use crate::serial_println;

pub const SYS_EXIT: u64 = 1;
pub const SYS_WRITE: u64 = 4;
pub const SYS_READ: u64 = 0;
pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_LSEEK: u64 = 8;
// More syscalls can be added here, e.g. SYS_YIELD

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
    match syscall_num {
        SYS_EXIT => sys_exit(arg1),
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
            // Return an error code (e.g., -1) for unknown syscalls
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

fn sys_write(fd: u64, buf_ptr: u64, len: u64) -> u64 {
    if fd == 1 || fd == 2 { // stdout or stderr
        // Check if memory is readable user memory (simplified: assume it is valid for now)
        let bytes = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
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
    
    // Only stdout (1) and stderr (2) supported for now
    crate::serial_println!("[SYSCALL] sys_write Error: Unsupported fd {}", fd);
    u64::MAX
}
