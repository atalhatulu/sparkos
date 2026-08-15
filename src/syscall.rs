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
pub const SYS_MAP_DMA: u64 = 26;
pub const SYS_NET_SEND_FRAME: u64 = 27;
pub const SYS_NET_RECV_FRAME: u64 = 28;
pub const SYS_IPC_CANCEL: u64 = 29;
/// Aşama 6.3: netdrv bir DMA bölgesi içindeki alt-aralığı (ör. RX ring'de gelen
/// frame) üst yığına zero-copy ulaştırmak için bir "buffer capability" üretir.
/// dma_fd üzerinden (MAP|DMA haklı) bir ObjectKind::Memory slot cap oluşturulur;
/// bu cap SLOT_MAP'te o bölgenin fiziksel adresi + offset/len ile kaydedilir.
/// Return: yeni slot cap'inin fd'si (>= 1000); hata -13 (EACCES) / -14 (EFAULT).
pub const SYS_IPC_CREATE_SLOT: u64 = 30;
pub const SYS_CREATE_SURFACE: u64 = 31;
pub const SYS_PRESENT_SURFACE: u64 = 32;
pub const SYS_DESTROY_SURFACE: u64 = 33;
pub const SYS_CREATE_WINDOW: u64 = 34;
pub const SYS_DESTROY_WINDOW: u64 = 35;
pub const SYS_MOVE_WINDOW: u64 = 36;
pub const SYS_MINIMIZE_WINDOW: u64 = 37;
pub const SYS_RESTORE_WINDOW: u64 = 38;
pub const SYS_POLL_EVENT: u64 = 39;

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



static FIRST_EXEC_PIDS: spin::Mutex<alloc::vec::Vec<u64>> = spin::Mutex::new(alloc::vec::Vec::new());

#[no_mangle]
pub extern "C" fn syscall_dispatcher(
    syscall_num: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
) -> u64 {
    let pid = crate::task::process::current_pid();
    if pid > 0 {
        let mut executed = FIRST_EXEC_PIDS.lock();
        if !executed.contains(&pid) {
            executed.push(pid);
            crate::serial_println!("[USER-EXEC] pid={} reached entrypoint", pid);
        }
    }
    if syscall_num != SYS_POLL_EVENT && syscall_num != SYS_YIELD {
        crate::serial_println!("[SYSCALL] pid={} num={} arg1={:#x}", pid, syscall_num, arg1);
    }

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
        SYS_IPC_RECV => sys_ipc_recv(arg1, arg2, arg3, arg4, arg5),
        SYS_IPC_TRY_RECV => sys_ipc_try_recv(arg1, arg2, arg3, arg4, arg5),
        SYS_IOPERM => sys_ioperm(arg1, arg2, arg3),
        // Aşama 5.2: user-space servis çerçevesi. Her ikisi de process modeli
        // altında çalışmalı — kernel executor içinden çağrılırsa EACCES.
        SYS_IPC_CREATE_ENDPOINT => sys_ipc_create_endpoint(arg1),
        SYS_IPC_BIND_IRQ => sys_ipc_bind_irq(arg1, arg2, arg3),
        SYS_MAP_DMA => sys_map_dma(arg1, arg2, arg3),
        SYS_NET_SEND_FRAME => sys_net_send_frame(arg1, arg2),
        SYS_NET_RECV_FRAME => sys_net_recv_frame(arg1, arg2),
        SYS_IPC_CANCEL => sys_ipc_cancel(arg1),
        SYS_IPC_CREATE_SLOT => sys_ipc_create_slot(arg1, arg2, arg3),
        SYS_CREATE_SURFACE => {
            match crate::surface::create_surface(arg1 as u32, arg2 as u32) {
                Ok(id) => id,
                Err(_) => u64::MAX,
            }
        }
        SYS_PRESENT_SURFACE => {
            let x = ((arg2 >> 48) & 0xFFFF) as u32;
            let y = ((arg2 >> 32) & 0xFFFF) as u32;
            let w = ((arg2 >> 16) & 0xFFFF) as u32;
            let h = (arg2 & 0xFFFF) as u32;
            match crate::surface::present_surface(arg1, x, y, w, h) {
                Ok(_) => 0,
                Err(_) => u64::MAX,
            }
        }
        SYS_DESTROY_SURFACE => {
            match crate::surface::destroy_surface(arg1) {
                Ok(_) => 0,
                Err(_) => u64::MAX,
            }
        }
        SYS_CREATE_WINDOW => {
            let pid = crate::task::process::current_pid();
            match crate::wm::WM.lock().create_window(pid, arg1, arg2 as i32, arg3 as i32, arg4 as u32, arg5 as u32) {
                Ok(id) => id,
                Err(_) => u64::MAX,
            }
        }
        SYS_DESTROY_WINDOW => {
            let pid = crate::task::process::current_pid();
            match crate::wm::WM.lock().destroy_window(pid, arg1) {
                Ok(_) => 0,
                Err(_) => u64::MAX,
            }
        }
        SYS_MOVE_WINDOW => {
            let pid = crate::task::process::current_pid();
            match crate::wm::WM.lock().move_window(pid, arg1, arg2 as i32, arg3 as i32) {
                Ok(_) => 0,
                Err(_) => u64::MAX,
            }
        }
        SYS_MINIMIZE_WINDOW => {
            let pid = crate::task::process::current_pid();
            match crate::wm::WM.lock().minimize_window(pid, arg1) {
                Ok(_) => 0,
                Err(_) => u64::MAX,
            }
        }
        SYS_RESTORE_WINDOW => {
            let pid = crate::task::process::current_pid();
            match crate::wm::WM.lock().restore_window(pid, arg1) {
                Ok(_) => 0,
                Err(_) => u64::MAX,
            }
        }
        SYS_POLL_EVENT => {
            let pid = crate::task::process::current_pid();
            if let Some(ev) = crate::input::INPUT_QUEUES.lock().get_mut(&pid).and_then(|q| q.pop()) {
                if let Ok(dst_slice) = crate::sec_mem::validate_user_ptr_mut(arg1, core::mem::size_of::<crate::input::InputEvent>()) {
                    unsafe {
                        let ev_bytes = core::slice::from_raw_parts(&ev as *const _ as *const u8, 32);
                        dst_slice.copy_from_slice(ev_bytes);
                    }
                    1
                } else {
                    EFAULT
                }
            } else {
                0
            }
        }
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

/// Alınan IPC mesajını kullanıcı tamponuna kopyalar;
/// - Payload her zaman kullanıcı tamponuna kopyalanır ve kopyalanan bayt sayısı (payload len) döndürülür.
/// - Capability durumu ayrık olarak raporlanır (CAP_INV-7/11, Görev B):
///   - Meşru (valid) ise: `out_cap_ptr`'ye (slot + gen, 8 bayt) yazılır; `out_status_ptr`'ye 0 (OK) yazılır.
///   - Mesaj kuyruktayken revoke edilmişse: `out_cap_ptr`'ye 0 yazılır; `out_status_ptr`'ye 1 (CAP_REVOKED) yazılır.
///     SESSİZ DROP YOKTUR: Revoke edilmiş capability taşıyan mesajda dahi payload veri kaybı olmadan teslim edilir.
///   - Geçersiz (invalid) ise: `out_cap_ptr`'ye 0 yazılır; `out_status_ptr`'ye 2 (CAP_INVALID) yazılır.
///   - Capability yoksa: `out_status_ptr`'ye 0 yazılır.
fn copy_ipc_msg_to_user(
    msg: crate::ipc::CapMessage<alloc::vec::Vec<u8>>,
    out_buf: &mut [u8],
    out_cap_ptr: u64,
    out_status_ptr: u64,
    max_len: u64,
) -> u64 {
    let n = core::cmp::min(msg.payload.len(), max_len as usize);
    out_buf[..n].copy_from_slice(&msg.payload[..n]);

    let mut cap_status: u32 = 0; // 0 = OK / No cap, 1 = CAP_REVOKED, 2 = CAP_INVALID

    if let Some(cap) = msg.capability {
        match crate::cap::check_rights(cap, crate::cap::Rights::empty()) {
            Ok(_) => {
                if out_cap_ptr != 0 {
                    if let Ok(cap_bytes) = crate::sec_mem::validate_user_ptr_mut(out_cap_ptr, 8) {
                        cap_bytes[..4].copy_from_slice(&cap.slot.to_le_bytes());
                        cap_bytes[4..8].copy_from_slice(&cap.generation.to_le_bytes());
                    }
                }
                cap_status = 0;
            }
            Err(crate::cap::CapError::Revoked) => {
                if out_cap_ptr != 0 {
                    if let Ok(cap_bytes) = crate::sec_mem::validate_user_ptr_mut(out_cap_ptr, 8) {
                        cap_bytes.fill(0);
                    }
                }
                cap_status = 1; // CAP_REVOKED
            }
            Err(_) => {
                if out_cap_ptr != 0 {
                    if let Ok(cap_bytes) = crate::sec_mem::validate_user_ptr_mut(out_cap_ptr, 8) {
                        cap_bytes.fill(0);
                    }
                }
                cap_status = 2; // CAP_INVALID
            }
        }
    }

    if out_status_ptr != 0 {
        if let Ok(status_bytes) = crate::sec_mem::validate_user_ptr_mut(out_status_ptr, 4) {
            status_bytes.copy_from_slice(&cap_status.to_le_bytes());
        }
    }

    n as u64
}

fn sys_ipc_recv(ep_id: u64, buf_ptr: u64, max_len: u64, out_cap_ptr: u64, out_status_ptr: u64) -> u64 {
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
        Ok(msg) => copy_ipc_msg_to_user(msg, out_buf, out_cap_ptr, out_status_ptr, max_len),
        Err(crate::cap::CapError::NoRights) => EACCES,
        Err(_) => u64::MAX,
    }
}

/// Non-blocking IPC alımı. Kuyruk boşsa EAGAIN döner (CPU kilitlemez, bekletmez).
/// User-space servisler bunu poll edip SYS_YIELD ile zaman dilimlerini verir.
fn sys_ipc_try_recv(ep_id: u64, buf_ptr: u64, max_len: u64, out_cap_ptr: u64, out_status_ptr: u64) -> u64 {
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
        Ok(Some(msg)) => copy_ipc_msg_to_user(msg, out_buf, out_cap_ptr, out_status_ptr, max_len),
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

    // SEC-12: Bounded capacity validation to prevent Kernel Heap OOM DoS
    if capacity == 0 || capacity > crate::ipc::MAX_ENDPOINT_CAPACITY as u64 {
        serial_println!("[SYSCALL] SYS_IPC_CREATE_ENDPOINT EINVAL: capacity {} out of bounds (1..{})",
            capacity, crate::ipc::MAX_ENDPOINT_CAPACITY);
        return (-22i64) as u64; // -EINVAL
    }

    let pid = crate::task::process::current_pid();

    let (ep_id, ep_root) = match crate::ipc::create_raw_endpoint(capacity as usize) {
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
        crate::ipc::register_endpoint_owner(ep_id, pid as u32);
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
/// terminating the app.
fn sys_yield() -> u64 {
    0
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

/// Aşama 6.2: Capability-Gated DMA bölgesi eşleme köprüsü.
/// Sürücü süreci kendi Ring-3 adres alanına tahsisli DMA bölgesini eşler.
fn sys_map_dma(dma_slot: u64, virt_addr: u64, pages: u64) -> u64 {
    if !crate::task::process::current_is_user_process() {
        serial_println!("[SYSCALL] SYS_MAP_DMA EACCES: not a user process");
        return EACCES;
    }
    let pid = crate::task::process::current_pid();

    let total_bytes = match (pages as usize).checked_mul(4096) {
        Some(b) => b,
        None => return EFAULT,
    };

    if !crate::memory::is_user_range(virt_addr, total_bytes) {
        serial_println!("[SYSCALL] SYS_MAP_DMA EFAULT: virt_addr 0x{:x} outside user space", virt_addr);
        return EFAULT;
    }

    if !crate::dma_region::is_page_aligned(virt_addr) {
        serial_println!("[SYSCALL] SYS_MAP_DMA EFAULT: virt_addr 0x{:x} not page aligned", virt_addr);
        return EFAULT;
    }

    let cap_handle = match crate::task::process::with_cap_table(pid, |t| {
        crate::syscall_cap::find_fd_in_table(t, dma_slot as u32)
    }) {
        Some(Some(h)) => h,
        _ => return EACCES,
    };

    // Rights::MAP(4) | Rights::DMA(16) kontrolü
    if crate::cap::check_rights(cap_handle, crate::cap::Rights(4 | 16)).is_err() {
        serial_println!("[SYSCALL] SYS_MAP_DMA EACCES: missing MAP|DMA rights");
        return EACCES;
    }

    let (phys_addr, max_pages) = match crate::dma_region::lookup_dma_region(cap_handle.slot) {
        Some(pair) => pair,
        None => {
            serial_println!("[SYSCALL] SYS_MAP_DMA EACCES: no registered DMA region for cap slot {}", cap_handle.slot);
            return EACCES;
        }
    };

    if pages > max_pages {
        serial_println!("[SYSCALL] SYS_MAP_DMA EFAULT: requested {} pages exceeds region capacity {}", pages, max_pages);
        return EFAULT;
    }

    match crate::memory::map_user_phys_range(virt_addr, x86_64::PhysAddr::new(phys_addr), pages, true) {
        Ok(_) => {
            serial_println!("[SYSCALL] SYS_MAP_DMA: mapped {} pages at virt 0x{:x} -> phys 0x{:x} for pid {}",
                pages, virt_addr, phys_addr, pid);
            0
        }
        Err(e) => {
            serial_println!("[SYSCALL] SYS_MAP_DMA failed: {}", e);
            EFAULT
        }
    }
}

/// Aşama 6.3: DmaSlot Buffer-Cap Oluşturma Köprüsü.
///
/// netdrv (Ring 3), elindeki DMA bölgesi capability'siyle (`dma_fd`) bölge
/// içindeki bir alt-aralığı (ör. RX ring'de gelen frame) üst yığına zero-copy
/// ulaştırmak için yeni bir `ObjectKind::Memory` slot cap üretir. Slot, SLOT_MAP
/// kaydına bölgenin fiziksel adresi + `offset`/`len` olarak bağlanır; üst yığın
/// `dma_region::resolve_slot_cap` ile aynı fiziksel sayfaların kernel-görünür
/// sanal adresini alır — veri hiç kopyalanmaz.
///
/// Gate'ler (sys_map_dma ile aynı kalıp):
/// - process modeli altında çalışmalı → EACCES
/// - `dma_fd` tabloda bulunmalı → EACCES
/// - handle MAP(4)|DMA(16) hakkı taşımalı → EACCES
/// - handle kayıtlı bir DMA bölgesine işaret etmeli → EACCES
/// - `len==0` veya `offset+len` bölge kapasitesini aşmalı → EFAULT
///
/// Return: yeni slot cap'inin fd'si (>= 1000).
fn sys_ipc_create_slot(dma_fd: u64, offset: u64, len: u64) -> u64 {
    if !crate::task::process::current_is_user_process() {
        serial_println!("[SYSCALL] SYS_IPC_CREATE_SLOT EACCES: not a user process");
        return EACCES;
    }
    let pid = crate::task::process::current_pid();

    let dma_cap = match crate::task::process::with_cap_table(pid, |t| {
        crate::syscall_cap::find_fd_in_table(t, dma_fd as u32)
    }) {
        Some(Some(h)) => h,
        _ => return EACCES,
    };

    // Rights::MAP(4) | Rights::DMA(16) kontrolü
    if crate::cap::check_rights(dma_cap, crate::cap::Rights(4 | 16)).is_err() {
        serial_println!("[SYSCALL] SYS_IPC_CREATE_SLOT EACCES: missing MAP|DMA rights");
        return EACCES;
    }

    let (region_phys, pages) = match crate::dma_region::lookup_dma_region(dma_cap.slot) {
        Some(pair) => pair,
        None => {
            serial_println!("[SYSCALL] SYS_IPC_CREATE_SLOT EACCES: no registered DMA region for cap slot {}", dma_cap.slot);
            return EACCES;
        }
    };

    let offset_usize = offset as usize;
    let len_usize = len as usize;
    let region_bytes = (pages as usize) * 4096;
    if len_usize == 0 || offset_usize.checked_add(len_usize).map_or(true, |end| end > region_bytes) {
        serial_println!("[SYSCALL] SYS_IPC_CREATE_SLOT EFAULT: slot range out of DMA region bounds");
        return EFAULT;
    }

    // Yeni slot cap: bağımsız Memory object (lineage'dan ayrı yaşar).
    let slot_cap = match crate::cap::create_object(crate::cap::ObjectKind::Memory) {
        Ok(h) => h,
        Err(e) => {
            serial_println!("[SYSCALL] SYS_IPC_CREATE_SLOT failed: capability store exhausted ({:?})", e);
            return u64::MAX;
        }
    };
    let (kind, object_idx) = match crate::cap::object_identity(slot_cap) {
        Ok(pair) => pair,
        Err(e) => {
            serial_println!("[SYSCALL] SYS_IPC_CREATE_SLOT failed: object_identity {:?}", e);
            return u64::MAX;
        }
    };
    debug_assert_eq!(kind, crate::cap::ObjectKind::Memory);
    crate::dma_region::register_slot(object_idx, region_phys, offset_usize, len_usize);

    // Boş fd'yi 1000'den itibaren tara; cap_table'a ekle ve fd'yi döndür.
    let fd = match crate::task::process::with_cap_table(pid, |t| {
        let mut fd = 1000u32;
        while t.iter().any(|(f, _)| *f == fd) {
            fd += 1;
        }
        t.push((fd, slot_cap));
        fd
    }) {
        Some(fd) => fd,
        None => {
            serial_println!("[SYSCALL] SYS_IPC_CREATE_SLOT EACCES: process table vanished");
            return EACCES;
        }
    };

    serial_println!("[SYSCALL] pid {} created slot cap fd {} (phys 0x{:x}+{} len {})",
        pid, fd, region_phys, offset_usize, len_usize);
    fd as u64
}

/// Aşama 6.3: Zero-Copy L2 Frame Gönderim Köprüsü.
#[allow(static_mut_refs)]
fn sys_net_send_frame(buf_ptr: u64, len: u64) -> u64 {
    if !crate::task::process::current_is_user_process() {
        return EACCES;
    }
    let bytes = match crate::sec_mem::validate_user_ptr(buf_ptr, len as usize) {
        Ok(b) => b,
        Err(_) => return EFAULT,
    };
    unsafe {
        if let Some(dev) = &mut crate::rtl8139::RTL8139_DEV {
            dev.send_packet(bytes);
            return len;
        }
    }
    EACCES
}

/// Aşama 6.3: Zero-Copy / Direct L2 Frame Alım Köprüsü.
#[allow(static_mut_refs)]
fn sys_net_recv_frame(buf_ptr: u64, max_len: u64) -> u64 {
    if !crate::task::process::current_is_user_process() {
        return EACCES;
    }
    let user_buf = match crate::sec_mem::validate_user_ptr_mut(buf_ptr, max_len as usize) {
        Ok(b) => b,
        Err(_) => return EFAULT,
    };
    unsafe {
        if let Some(dev) = &mut crate::rtl8139::RTL8139_DEV {
            if let Some(packet) = dev.poll_rx() {
                let copy_len = packet.len().min(max_len as usize);
                user_buf[..copy_len].copy_from_slice(&packet[..copy_len]);
                return copy_len as u64;
            }
        }
    }
    EAGAIN
}

/// Aşama 7.1: Cooperative IPC İptal Köprüsü.
fn sys_ipc_cancel(ep_id: u64) -> u64 {
    let pid = crate::task::process::current_pid();
    let cap_handle = match crate::task::process::with_cap_table(pid, |t| {
        crate::syscall_cap::find_fd_in_table(t, ep_id as u32)
    }) {
        Some(Some(h)) => h,
        _ => return EACCES,
    };

    if crate::cap::check_rights(cap_handle, crate::cap::Rights(1 | 2)).is_err() {
        return EACCES;
    }

    match crate::ipc::cancel_endpoint(ep_id as u32, cap_handle) {
        Ok(canceled_count) => {
            serial_println!("[SYSCALL] sys_ipc_cancel: canceled {} in-flight messages on ep {}", canceled_count, ep_id);
            0
        }
        Err(_) => EACCES,
    }
}
