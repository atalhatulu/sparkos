use crate::serial_println;
use crate::cap::Rights;
use crate::syscall_cap;

pub const SYS_EXIT: u64 = 1;
pub const SYS_WRITE: u64 = 4;
pub const SYS_READ: u64 = 0;
pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_LSEEK: u64 = 8;
/// Cooperative user→kernel yield. A Ring 3 app calls this during a long loop so
/// the kernel executor can poll other tasks (timer, IPC, input) instead of
/// freezing them for the full app run. Semantics identical to SYS_EXIT's return
/// path (resume at user.rs `2:` label via KERNEL_RSP/KERNEL_RIP); the app is
/// relaunched at its saved resume point by the async loop in `user.rs`.
pub const SYS_YIELD: u64 = 9;

// Socket syscalls (Linux-compatible numbers).
pub const SYS_SOCKET: u64 = 10;
pub const SYS_CONNECT: u64 = 11;
pub const SYS_SEND: u64 = 12;
pub const SYS_RECV: u64 = 13;

// Process syscalls (Linux-compatible numbers).
pub const SYS_FORK: u64 = 14;
pub const SYS_EXEC: u64 = 15;

// Microkernel IPC & Device Port syscalls (Aşama 4 & 5).
pub const SYS_IPC_SEND: u64 = 20;
pub const SYS_IPC_RECV: u64 = 21;
pub const SYS_IOPERM: u64 = 22;
/// Non-blocking IPC alımı (Aşama 5): kuyruk boşsa EAGAIN döner, CPU'yu kilitlemez.
pub const SYS_IPC_TRY_RECV: u64 = 23;
/// User-space servis: adres alanına yeni bir endpoint oluşturur (Aşama 5.2).
/// Başarılıysa endpoint id (ep_id) döner; bu id aynı zamanda çağıranın
/// cap_table'ındaki fd'dir — servis IPC okumak için onu kullanır.
pub const SYS_IPC_CREATE_ENDPOINT: u64 = 24;
/// User-space servis: bir cihaz capability'siyle IRQ'yu kendi endpoint'ine bağlar
/// (Aşama 5.2). Device cap üzerinde MANAGE hakkı + endpoint'te WRITE gerekir.
pub const SYS_IPC_BIND_IRQ: u64 = 25;

// Standard errno style return code: -(EFAULT). Negative errno values are
// returned to the user as their unsigned encoding.
const EFAULT: u64 = (-14i64) as u64;
// EACCES (permission denied) — Asama 2: capability gate'i reddettiğinde döner.
const EACCES: u64 = (-13i64) as u64;
// EAGAIN (would block) — non-blocking IPC recv, kuyruk boşken döner.
const EAGAIN: u64 = (-11i64) as u64;

pub fn init() {
    serial_println!("[OK] Syscall dispatcher initialized");
}



#[no_mangle]
pub extern "C" fn syscall_dispatcher(
    syscall_num: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
) -> u64 {
    // Preserve the original dispatcher structure; socket(net) syscalls use
    // arg1..arg3 only (the target IP is packed into a single u32 arg).
    let _ = (arg4, arg5);
    match syscall_num {
        SYS_EXIT => sys_exit(arg1),
        SYS_YIELD => sys_yield(),
        SYS_READ => {
            // Asama 2: fd reader yetkisi (Rights::READ). capability yoksa EACCES.
            if syscall_cap::check_fd_access(arg1 as u32, Rights(1)).is_err() {
                return EACCES;
            }
            crate::syscall_storage::sys_read(arg1, arg2, arg3)
        }
        SYS_OPEN => crate::syscall_storage::sys_open(arg1, arg2),
        SYS_CLOSE => {
            // Asama 2: fd destroy yetkisi (Rights::DESTROY=128), G1: close_fd.
            if syscall_cap::check_fd_access(arg1 as u32, Rights(128)).is_err() {
                return EACCES;
            }
            crate::syscall_storage::sys_close(arg1)
        }
        SYS_LSEEK => {
            // Asama 2: fd read-navigate yetkisi (Rights::READ).
            if syscall_cap::check_fd_access(arg1 as u32, Rights(1)).is_err() {
                return EACCES;
            }
            crate::syscall_storage::sys_lseek(arg1, arg2 as i64, arg3)
        }
        SYS_SOCKET => crate::net_socket::sys_socket(arg1, arg2),
        SYS_CONNECT | SYS_SEND | SYS_RECV => {
            // Asama 2: socket fd IO yetkisi (Rights::IO=8).
            if syscall_cap::check_fd_access(arg1 as u32, Rights(8)).is_err() {
                return EACCES;
            }
            match syscall_num {
                SYS_CONNECT => crate::net_socket::sys_connect(arg1, arg2, arg3),
                SYS_SEND => crate::net_socket::sys_send(arg1, arg2, arg3),
                SYS_RECV => crate::net_socket::sys_recv(arg1, arg2, arg3),
                _ => unreachable!(),
            }
        }
        SYS_FORK => {
            // Asama 2 (B3): process'in EXECUTE yetkisi olmadan fork yapamaz.
            if syscall_cap::check_process_exec().is_err() {
                return EACCES;
            }
            let pid = crate::task::process::fork_current();
            serial_println!("[SYSCALL] SYS_FORK from Ring 3 -> child pid {}", pid);
            pid as u64
        }
        SYS_EXEC => {
            // exec(elf_ptr, len): build a fresh user process from ELF bytes.
            serial_println!("[SYSCALL] SYS_EXEC ({:#x}, {}) from Ring 3", arg1, arg2);
            // Asama 2 (B3): EXECUTE yetkisi check.
            if syscall_cap::check_process_exec().is_err() {
                return EACCES;
            }
            // Asama 2.0 / 3: kernel adresini ve geçersiz kullanıcı tamponlarını engelle.
            let elf_bytes = match crate::sec_mem::validate_user_ptr(arg1, arg2 as usize) {
                Ok(b) => b,
                Err(_) => {
                    serial_println!("[SYSCALL] SYS_EXEC Error: invalid user buffer (EFAULT)");
                    return EFAULT;
                }
            };
            match crate::task::process::exec_elf_proc("execd", elf_bytes) {
                Ok(pid) => pid,
                Err(e) => {
                    serial_println!("[SYSCALL] SYS_EXEC failed: {}", e);
                    u64::MAX
                }
            }
        }
        SYS_WRITE => {
            // Asama 2: fd yazma yetkisi (Rights::WRITE=2). fd 1/2 stdio seed'li
            // oldugu icin gecer; fd>=3 icin backend yolu.
            if syscall_cap::check_fd_access(arg1 as u32, Rights(2)).is_err() {
                return EACCES;
            }
            // fd 1/2 (stdout/stderr) terminale, fd >= 3 dosyalara yazılır
            if arg1 == 1 || arg1 == 2 {
                sys_write(arg1, arg2, arg3)
            } else {
                crate::syscall_storage::sys_write(arg1, arg2, arg3)
            }
        }
        SYS_IPC_SEND => sys_ipc_send(arg1, arg2, arg3, arg4, arg5),
        SYS_IPC_RECV => sys_ipc_recv(arg1, arg2, arg3, arg4),
        SYS_IPC_TRY_RECV => sys_ipc_try_recv(arg1, arg2, arg3, arg4),
        SYS_IOPERM => sys_ioperm(arg1, arg2, arg3),
        // Aşama 5.2: user-space servis çerçevesi. Her ikisi de process modeli
        // altında çalışmalı — kernel executor içinden çağrılırsa EACCES.
        SYS_IPC_CREATE_ENDPOINT => sys_ipc_create_endpoint(arg1),
        SYS_IPC_BIND_IRQ => sys_ipc_bind_irq(arg1, arg2, arg3),
        _ => {
            serial_println!("[SYSCALL] Unknown syscall number: {}", syscall_num);
            u64::MAX
        }
    }
}

fn sys_ipc_send(ep_id: u64, buf_ptr: u64, len: u64, attach_slot: u64, mode_val: u64) -> u64 {
    let pid = crate::task::process::current_pid();
    let sender_cap = match crate::task::process::with_cap_table(pid, |t| {
        crate::syscall_cap::find_fd_in_table(t, ep_id as u32)
    }) {
        Some(Some(h)) => h,
        _ => return EACCES,
    };

    let data = match crate::sec_mem::validate_user_ptr(buf_ptr, len as usize) {
        Ok(d) => d,
        Err(_) => return EFAULT,
    };

    let attached_cap = if attach_slot != 0 {
        crate::task::process::with_cap_table(pid, |t| {
            crate::syscall_cap::find_fd_in_table(t, attach_slot as u32)
        }).flatten()
    } else {
        None
    };

    let mode = match mode_val {
        1 => crate::ipc::TransferMode::Transfer,
        2 => crate::ipc::TransferMode::Lend,
        _ => crate::ipc::TransferMode::None,
    };

    match crate::ipc::raw_ipc_send(ep_id as u32, sender_cap, data, attached_cap, mode) {
        Ok(_) => 0,
        Err(crate::cap::CapError::NoRights) => EACCES,
        Err(_) => u64::MAX,
    }
}

/// Alınan IPC mesajını kullanıcı tamponuna kopyalar; capability varsa handle'ını
/// (slot + generation, 8 bayt) out_cap_ptr adresine yazar. Kopyalanan bayt sayısını döndürür.
fn copy_ipc_msg_to_user(
    msg: crate::ipc::CapMessage<alloc::vec::Vec<u8>>,
    out_buf: &mut [u8],
    out_cap_ptr: u64,
    max_len: u64,
) -> u64 {
    let n = core::cmp::min(msg.payload.len(), max_len as usize);
    out_buf[..n].copy_from_slice(&msg.payload[..n]);

    if out_cap_ptr != 0 {
        if let Some(cap) = msg.capability {
            if let Ok(cap_bytes) = crate::sec_mem::validate_user_ptr_mut(out_cap_ptr, 8) {
                cap_bytes[..4].copy_from_slice(&cap.slot.to_le_bytes());
                cap_bytes[4..8].copy_from_slice(&cap.generation.to_le_bytes());
            }
        }
    }
    n as u64
}

fn sys_ipc_recv(ep_id: u64, buf_ptr: u64, max_len: u64, out_cap_ptr: u64) -> u64 {
    // Aşama 5.1: interrupt context'te birikmiş IRQ olaylarını (interrupt dışı) boşalt.
    // Aksi halde servis recv döngüsü bağlı olayları IRQ'lar arasında göremezdi.
    crate::ipc::deliver_pending_irqs();

    let pid = crate::task::process::current_pid();
    let receiver_cap = match crate::task::process::with_cap_table(pid, |t| {
        crate::syscall_cap::find_fd_in_table(t, ep_id as u32)
    }) {
        Some(Some(h)) => h,
        _ => return EACCES,
    };

    let out_buf = match crate::sec_mem::validate_user_ptr_mut(buf_ptr, max_len as usize) {
        Ok(b) => b,
        Err(_) => return EFAULT,
    };

    match crate::ipc::raw_ipc_recv(ep_id as u32, receiver_cap) {
        Ok(msg) => copy_ipc_msg_to_user(msg, out_buf, out_cap_ptr, max_len),
        Err(crate::cap::CapError::NoRights) => EACCES,
        Err(_) => u64::MAX,
    }
}

/// Non-blocking IPC alımı. Kuyruk boşsa EAGAIN döner (CPU kilitlemez, bekletmez).
/// User-space servisler bunu poll edip SYS_YIELD ile zaman dilimlerini verir.
fn sys_ipc_try_recv(ep_id: u64, buf_ptr: u64, max_len: u64, out_cap_ptr: u64) -> u64 {
    // Aşama 5.1: recv öncesi birikmiş IRQ olaylarını boşalt — try_recv poll döngüsü
    // (Aşama 5.2 keysvc) her iterasyonda buraya gelir, olayı hemen görür.
    crate::ipc::deliver_pending_irqs();

    let pid = crate::task::process::current_pid();
    let receiver_cap = match crate::task::process::with_cap_table(pid, |t| {
        crate::syscall_cap::find_fd_in_table(t, ep_id as u32)
    }) {
        Some(Some(h)) => h,
        _ => return EACCES,
    };

    let out_buf = match crate::sec_mem::validate_user_ptr_mut(buf_ptr, max_len as usize) {
        Ok(b) => b,
        Err(_) => return EFAULT,
    };

    match crate::ipc::raw_ipc_try_recv(ep_id as u32, receiver_cap) {
        Ok(Some(msg)) => copy_ipc_msg_to_user(msg, out_buf, out_cap_ptr, max_len),
        Ok(None) => EAGAIN,
        Err(crate::cap::CapError::NoRights) => EACCES,
        Err(_) => u64::MAX,
    }
}

/// Aşama 5.2: servis adres alanında yeni bir IPC endpoint oluşturur. Return değeri
/// (ep_id) aynı zamanda servisin cap_table'ındaki fd'dir — bu kısıt ipc.rs'in
/// "ep_id == cap_table fd" aramasıyla birlikte çalışmak için zorunludur.
///
/// Gate'ler:
/// - process modeli altında çalışmalı (kernel executor'dan çağrılamaz) → EACCES
/// - cap_table'da ep_id çakışması varsa oluşturma reddedilir → EACCES
fn sys_ipc_create_endpoint(capacity: u64) -> u64 {
    if !crate::task::process::current_is_user_process() {
        serial_println!("[SYSCALL] SYS_IPC_CREATE_ENDPOINT EACCES: not a user process");
        return EACCES;
    }
    let pid = crate::task::process::current_pid();

    let (ep_id, ep_root) = match crate::ipc::create_raw_endpoint(capacity.max(1) as usize) {
        Ok(pair) => pair,
        Err(e) => {
            serial_println!("[SYSCALL] SYS_IPC_CREATE_ENDPOINT failed: {:?}", e);
            return u64::MAX;
        }
    };
    serial_println!("[SYSCALL] pid {} created endpoint {}", pid, ep_id);

    // Root handle'ı ep_id fd'sine yerleştir. create_raw_endpoint cap_table'dan
    // bağımsız üretir (ep_id, monotonic NEXT_EP_ID ad alanından gelir, capability
    // object store'un handle.slot'undan değil); bu adım handle'ı tabloya ekler ve
    // böylece ipc.rs'in `ep_id == cap_table fd` araması (sys_ipc_try_recv vb.)
    // çağıran servis için de çalışır.
    let inserted = crate::task::process::with_cap_table(pid, |t| {
        // Çakışma koruması: aynı ep_id fd'si zaten doluysa üzerine yazma, reddet.
        if crate::syscall_cap::find_fd_in_table(t, ep_id).is_some() {
            return false;
        }
        t.push((ep_id, ep_root));
        true
    });

    if inserted == Some(true) {
        ep_id as u64
    } else {
        serial_println!("[SYSCALL] SYS_IPC_CREATE_ENDPOINT EACCES: fd {} already taken", ep_id);
        EACCES
    }
}

/// Aşama 5.2: servis, elindeki Device capability ile bir IRQ'yu kendi endpoint'ine
/// bağlar. Gate'ler (confused deputy koruması):
/// - process modeli altında çalışmalı → EACCES
/// - device_fd, Device tipinde bir handle'a MANAGE hakkıyla karşılık gelmeli
/// - ep_id, çağıranın cap_table'ında WRITE hakkıyla bulunmalı
/// - IRQ numarası 0..=15 aralığında olmalı (PIC)
fn sys_ipc_bind_irq(device_fd: u64, irq: u64, ep_id: u64) -> u64 {
    if !crate::task::process::current_is_user_process() {
        serial_println!("[SYSCALL] SYS_IPC_BIND_IRQ EACCES: not a user process");
        return EACCES;
    }
    if irq > 15 {
        serial_println!("[SYSCALL] SYS_IPC_BIND_IRQ EACCES: irq {} out of PIC range", irq);
        return EACCES;
    }
    let pid = crate::task::process::current_pid();

    let device_cap = match crate::task::process::with_cap_table(pid, |t| {
        crate::syscall_cap::find_fd_in_table(t, device_fd as u32)
    }) {
        Some(Some(h)) => h,
        _ => return EACCES,
    };
    let writer_cap = match crate::task::process::with_cap_table(pid, |t| {
        crate::syscall_cap::find_fd_in_table(t, ep_id as u32)
    }) {
        Some(Some(h)) => h,
        _ => return EACCES,
    };

    match crate::ipc::bind_irq(device_cap, irq as u8, ep_id as u32, writer_cap) {
        Ok(()) => {
            serial_println!("[SYSCALL] pid {} bound irq {} -> ep {}", pid, irq, ep_id);
            0
        }
        Err(e) => {
            serial_println!("[SYSCALL] SYS_IPC_BIND_IRQ failed: {:?}", e);
            EACCES
        }
    }
}

fn sys_ioperm(start_port: u64, end_port: u64, enable: u64) -> u64 {
    let pid = crate::task::process::current_pid();

    if start_port > end_port || end_port > 65535 {
        return u64::MAX;
    }

    // Per-device gate (Asama 4): process'in cap_table'ında ObjectKind::Device tipinde
    // bir capability olmalı VE istenen aralık o cihaza bağlı aralığın alt kümesi olmalı.
    // Socket fd'lerinin Rights::IO=8 taşıması nedeniyle eski boolean gate (`.any(.. |
    // check_rights(*h, Rights(8)).is_ok())`) bir ayrıcalık yükseltme açığıydı: ağ
    // erişimli bir process hiçbir cihaz bağlamadan TÜM portlara erişebiliyordu.
    let device_cap = match crate::task::process::with_cap_table(pid, |t| {
        t.iter()
            .map(|(_, h)| *h)
            .find(|h| crate::cap::object_identity(*h).ok().map(|(k, _)| k == crate::cap::ObjectKind::Device).unwrap_or(false))
    }) {
        Some(Some(h)) => h,
        _ => {
            serial_println!("[SYSCALL] sys_ioperm EACCES: pid {} has no device capability", pid);
            return EACCES;
        }
    };

    if crate::cap::port_range_allowed(device_cap, start_port as u16, end_port as u16).is_err() {
        serial_println!("[SYSCALL] sys_ioperm EACCES: pid {} ports {:#x}..={:#x} outside bound device range", pid, start_port, end_port);
        return EACCES;
    }

    if enable != 0 {
        crate::gdt::allow_port_range(start_port as u16, end_port as u16);
        crate::task::process::set_current_allowed_ports(Some((start_port as u16, end_port as u16)));
        serial_println!("[SYSCALL] sys_ioperm: enabled ports {:#x}..={:#x} for pid {}", start_port, end_port, pid);
    } else {
        crate::gdt::deny_port_range(start_port as u16, end_port as u16);
        crate::task::process::set_current_allowed_ports(None);
        serial_println!("[SYSCALL] sys_ioperm: disabled ports {:#x}..={:#x} for pid {}", start_port, end_port, pid);
    }

    0
}

fn sys_exit(status: u64) -> u64 {
    serial_println!("[SYSCALL] sys_exit({}) called from Ring 3", status);
    // Process-model path: terminate this process via the scheduler and switch
    // to the next ready process (never returns). Uses the per-PCB kernel
    // state, not the legacy KERNEL_RSP/KERNEL_RIP globals.
    if crate::task::process::current_is_user_process() {
        if let Some((pid, name)) = crate::task::process::current_process_info() {
            serial_println!("[PROC] pid {} ('{}') exiting with status {}", pid, name, status);
        }
        crate::task::process::exit_current();
    }
    // Legacy single-app path (shell.rs exec_elf): the process model is not
    // active, so return to the kernel at the saved global resume point.
    unsafe {
        core::arch::asm!(
            "cli",
            "mov rsp, {kernel_rsp}",
            "jmp {kernel_rip}",
            kernel_rsp = in(reg) crate::user::KERNEL_RSP,
            kernel_rip = in(reg) crate::user::KERNEL_RIP,
            options(noreturn)
        );
    }
}

/// Cooperative yield: returns control to the kernel executor without
/// terminating the app. The `exec_elf_async` loop in `user.rs` will relaunch
/// the app at its saved resume point on the next poll.
fn sys_yield() -> u64 {
    serial_println!("[SYSCALL] SYS_YIELD from Ring 3 (cooperative)");
    unsafe {
        core::arch::asm!(
            "cli",
            "mov rsp, {kernel_rsp}",
            "jmp {kernel_rip}",
            kernel_rsp = in(reg) crate::user::KERNEL_RSP,
            kernel_rip = in(reg) crate::user::KERNEL_RIP,
            options(noreturn)
        );
    }
}

fn sys_write(fd: u64, buf_ptr: u64, len: u64) -> u64 {
    if fd == 1 || fd == 2 { // stdout or stderr
        let len_usize = len as usize;
        let bytes = match crate::sec_mem::validate_user_ptr(buf_ptr, len_usize) {
            Ok(b) => b,
            Err(_) => {
                serial_println!("[SYSCALL] sys_write Error: invalid user buffer (EFAULT)");
                return EFAULT;
            }
        };
        if let Ok(s) = core::str::from_utf8(bytes) {
            let mut w = x86_64::instructions::interrupts::without_interrupts(|| crate::vga_buffer::WRITE_LOCK.lock());
            let color = if fd == 1 { crate::vga_buffer::Color::LightGreen } else { crate::vga_buffer::Color::LightRed };
            w.set_color(color, crate::vga_buffer::Color::Black);
            core::fmt::Write::write_str(&mut *w, s).unwrap();
            w.set_color(crate::vga_buffer::Color::White, crate::vga_buffer::Color::Black);
            crate::serial_println!("[USER PRINT (fd {})]: {}", fd, s);
            return len;
        } else {
            crate::serial_println!("[SYSCALL] sys_write Error: Invalid UTF-8");
            return u64::MAX;
        }
    }
    
    crate::serial_println!("[SYSCALL] sys_write Error: Unsupported fd {}", fd);
    u64::MAX
}
