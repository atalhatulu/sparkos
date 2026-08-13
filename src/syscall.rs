use crate::serial_println;

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

// Standard errno style return code: -(EFAULT). Negative errno values are
// returned to the user as their unsigned encoding.
const EFAULT: u64 = (-14i64) as u64;

pub fn init() {
    serial_println!("[OK] Syscall dispatcher initialized");
}

/// Validates that `ptr..ptr+len` is a readable user buffer (canonical, in the
/// user half, within USER_RANGE, and every page is user-mapped). On failure
/// returns `-EFAULT` rather than touching the buffer.
fn check_user_read(buf_ptr: u64, len: usize) -> Result<(), u64> {
    if crate::sec_mem::validate_user_ptr(buf_ptr, len).is_ok() {
        Ok(())
    } else {
        Err(EFAULT)
    }
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
    let _ = (arg4, arg5);
    match syscall_num {
        SYS_EXIT => sys_exit(arg1),
        SYS_YIELD => sys_yield(),
        SYS_READ => crate::syscall_storage::sys_read(arg1, arg2, arg3),
        SYS_OPEN => crate::syscall_storage::sys_open(arg1, arg2),
        SYS_CLOSE => crate::syscall_storage::sys_close(arg1),
        SYS_LSEEK => crate::syscall_storage::sys_lseek(arg1, arg2 as i64, arg3),
        SYS_WRITE => {
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
    // Uygulama sonlandığında executor'a dönmeli veya kernel loop'a dönmelidir.
    // Şimdilik KERNEL_RIP ve KERNEL_RSP kullanılıyor (user.rs'te iretq öncesi kaydedilir).
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
        // Validate the user buffer before touching it: canonical address, user
        // half, in-range length, every page user-mapped. Reject with -EFAULT.
        let len_usize = len as usize;
        if check_user_read(buf_ptr, len_usize).is_err() {
            serial_println!("[SYSCALL] sys_write Error: invalid user buffer (EFAULT)");
            return EFAULT;
        }
        let bytes = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len_usize) };
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
