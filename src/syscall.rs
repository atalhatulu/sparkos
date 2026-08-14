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

// Standard errno style return code: -(EFAULT). Negative errno values are
// returned to the user as their unsigned encoding.
const EFAULT: u64 = (-14i64) as u64;
// EACCES (permission denied) — Asama 2: capability gate'i reddettiğinde döner.
const EACCES: u64 = (-13i64) as u64;

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
        _ => {
            serial_println!("[SYSCALL] Unknown syscall number: {}", syscall_num);
            u64::MAX
        }
    }
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
