// Asama 2 — Syscall yetki kontrolü köprüsü (syscall_cap)
//
// PURE core (slice üzerinde test edilebilir): fcc Q1 slice-pure refactor'una göre,
// karar mantığı global'e (SCHEDULER)a bağlanmaz — dilim argümanı alır. Bu sayede
// aynı fonksiyon hem host unit test'te (düz Vec) hem kernel'da çalışır.
//
// Kernel wrapper'ları: AGY'nin stub bıraktığı yerleri, `process::with_cap_table`
// (SCHEDULER lock'u içinde closure) üzerinden GERÇEK erişimle doldurur.
// AGY bu kodlarda `Ok(())` stub'ı bırakmıştı; Hermes glue olarak bağladı.
//
// Not: Bu dosya `#![no_std]` / `extern crate alloc` İÇERMEZ — `main.rs` crate kökü
// üzerinden modül olarak derlenir (crate no_std + alloc sağlar). Asama 1'de bu ikisini
// içeren modül derlenemedi.

use alloc::vec::Vec;
use crate::cap::{self, CapHandle, CapError, Rights, ObjectKind};
use crate::task::process;

/// Saf (pure) dilim üzerinde FD yetki kontrolü. (Q1 refactor)
/// `check_rights` KULLANILIR (deref değil): deref+drop, refcount'u 0'a düşürüp
/// obje'yi valid=false yapar (ikinci check Invalid verirdi). fd-capability kalıcıdır.
pub fn check_fd_access_in_table(table: &[(u32, CapHandle)], fd: u32, needed: Rights) -> cap::Result<()> {
    let (_, handle) = table.iter().find(|(t_fd, _)| *t_fd == fd).ok_or(CapError::NotFound)?;
    cap::check_rights(*handle, needed)
}

/// Saf (pure) dilim üzerinde Process EXECUTE yetki kontrolü. (B3)
/// Tabloda EXECUTE (256) yetkili herhangi bir handle varsa OK.
pub fn check_process_exec_in_table(table: &[(u32, CapHandle)]) -> cap::Result<()> {
    for (_, handle) in table.iter() {
        if cap::check_rights(*handle, Rights(256)).is_ok() {
            return Ok(());
        }
    }
    Err(CapError::NoRights)
}

/// Tabloya yeni FD yetkisi ekle. Yeni capability `parent`'tan `req` rights ile grant edilir.
pub fn grant_fd_in_table(table: &mut Vec<(u32, CapHandle)>, fd: u32, parent: CapHandle, req: Rights) -> cap::Result<()> {
    if table.iter().any(|(t_fd, _)| *t_fd == fd) {
        return Err(CapError::AlreadyExists);
    }
    let granted = cap::grant(parent, req)?;
    table.push((fd, granted));
    Ok(())
}

/// Tablodan FD kaldır ve capability'yi close et. (G1)
pub fn close_fd_in_table(table: &mut Vec<(u32, CapHandle)>, fd: u32) -> cap::Result<()> {
    let index = table.iter().position(|(t_fd, _)| *t_fd == fd).ok_or(CapError::NotFound)?;
    let (_, handle) = table.remove(index);
    cap::close(handle)?;
    Ok(())
}

/// Process stdio seed (B1). fd 0/1/2'ye READ|WRITE capability verir.
/// `create_user_process` / spawn sonunda bir kez çağrılır; aksi halde tüm user çıktısı
/// (SYS_WRITE fd 1/2) EACCES alır ve sistem "bozulur".
pub fn seed_stdio(table: &mut Vec<(u32, CapHandle)>) -> cap::Result<()> {
    let stdio_cap = cap::create_object(ObjectKind::Fd)?;
    let rw_rights = Rights(3); // READ(1) | WRITE(2)
    table.push((0, cap::grant(stdio_cap, rw_rights)?));
    table.push((1, cap::grant(stdio_cap, rw_rights)?));
    table.push((2, cap::grant(stdio_cap, rw_rights)?));
    Ok(())
}

/// Process kendi üzerinde EXECUTE hakkı seed (B3). fork/exec yapabilmesi için.
/// fd alanına `u32::MAX` (geri dönüşsüz sentinel) yerleştirilir — gerçek fd'lerle çakışmaz.
pub fn seed_process_exec(table: &mut Vec<(u32, CapHandle)>) -> cap::Result<()> {
    let proc_cap = cap::create_object(ObjectKind::Process)?;
    let exec_cap = cap::grant(proc_cap, Rights(256))?; // EXECUTE = 256
    table.push((u32::MAX, exec_cap));
    Ok(())
}

// =============================================================================
// KERNEL WRAPPERS — gerçek SCHEDULER erişimi (Hermes glue; AGY stub bırakmıştı)
// =============================================================================

/// Aktif process'te FD erişim kontrolü. SCHEDULER'a current_pid ile lock açılır;
/// cap_table kopyası PURE fonksiyona verilir (guard lifetime taşmaz).
pub fn check_fd_access(fd: u32, needed: Rights) -> cap::Result<()> {
    let pid = process::current_pid();
    process::with_cap_table(pid, |table| check_fd_access_in_table(table, fd, needed))
        .unwrap_or(Err(CapError::NotFound))
}

/// Aktif process'te fork/exec tetikleme yetkisi (EXECUTE right).
pub fn check_process_exec() -> cap::Result<()> {
    let pid = process::current_pid();
    process::with_cap_table(pid, |t| check_process_exec_in_table(t))
        .unwrap_or(Err(CapError::NotFound))
}

/// Aktif process tablosuna yeni fd ekle (sys_open / sys_socket provision).
/// `cap` zaten `create_object` ile üretilmiş handle'dır; `req` rights ile grant edilir.
pub fn add_fd_to_current(fd: u32, cap: CapHandle, req: Rights) -> cap::Result<()> {
    let pid = process::current_pid();
    process::with_cap_table(pid, |t| grant_fd_in_table(t, fd, cap, req))
        .unwrap_or(Err(CapError::NotFound))
}

/// Aktif process tablosundan fd kaldır + close (sys_close G1).
pub fn remove_fd_from_current(fd: u32) -> cap::Result<()> {
    let pid = process::current_pid();
    process::with_cap_table(pid, |t| close_fd_in_table(t, fd))
        .unwrap_or(Err(CapError::NotFound))
}

/// Yeni bir process doğduğunda (spawn / fork / exec) çalışır: stdio + process-exec seed.
/// `create_user_process` yolunda çağrılması planlanır.
pub fn seed_new_process(table: &mut Vec<(u32, CapHandle)>) -> cap::Result<()> {
    seed_stdio(table)?;
    seed_process_exec(table)
}

// =============================================================================
// HOST TESTS (PURE fonksiyonlar — kernel bağımlılığı yok)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cap::{Rights, ObjectKind, CapError};

    #[test]
    fn test_fd_access_and_seed() {
        cap::init(); // STATE'i tazele — aksi halde deref Invalid döner
        let mut table = Vec::new();
        // Henüz seed yok: fd 1 NotFound
        assert!(matches!(check_fd_access_in_table(&table, 1, Rights(1)), Err(CapError::NotFound)));

        seed_stdio(&mut table).unwrap();
        assert!(check_fd_access_in_table(&table, 1, Rights(2)).is_ok()); // stdout WRITE OK
        assert!(check_fd_access_in_table(&table, 0, Rights(1)).is_ok()); // stdin READ OK
        // DESTROY (128) hakkı yok → NoRights
        assert!(matches!(check_fd_access_in_table(&table, 1, Rights(128)), Err(CapError::NoRights)));

        // G1: close sonrası entry yok
        close_fd_in_table(&mut table, 1).unwrap();
        assert!(matches!(check_fd_access_in_table(&table, 1, Rights(2)), Err(CapError::NotFound)));
    }

    #[test]
    fn test_process_exec() {
        cap::init();
        let mut table = Vec::new();
        assert!(matches!(check_process_exec_in_table(&table), Err(CapError::NoRights)));

        seed_process_exec(&mut table).unwrap();
        assert!(check_process_exec_in_table(&table).is_ok());
    }

    #[test]
    fn test_grant_duplicate_fd_rejected() {
        cap::init();
        let mut table = Vec::new();
        seed_stdio(&mut table).unwrap();
        // fd 1 zaten seed'li → AlreadyExists
        let parent = cap::create_object(ObjectKind::Fd).unwrap();
        assert!(matches!(grant_fd_in_table(&mut table, 1, parent, Rights(1)), Err(CapError::AlreadyExists)));
    }
}
