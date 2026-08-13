//! SparkOS First Rust Userspace Application Skeleton
//! 
//! This file is designed to be compiled as a standalone `#![no_std]` ELF binary
//! for SparkOS userspace. It is NOT part of the kernel build itself.
//! 
//! Compile with (hypothetical separate Cargo project):
//! `cargo rustc --target x86_64-unknown-none -- -C relocation-model=pie`

#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

/// Raw system call wrapper for `int 0x80`
#[inline(always)]
unsafe fn syscall(num: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    let ret: u64;
    asm!(
        "int 0x80",
        in("rax") num,
        in("rdi") arg1,
        in("rsi") arg2,
        in("rdx") arg3,
        lateout("rax") ret,
        options(nostack, preserves_flags)
    );
    ret
}

/// Syscall wrappers
fn sys_exit(code: u64) -> ! {
    unsafe { syscall(1, code, 0, 0) };
    loop {}
}

fn sys_write(fd: u64, buf: &[u8]) -> u64 {
    unsafe { syscall(4, fd, buf.as_ptr() as u64, buf.len() as u64) }
}

/// Application entry point
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let msg = b"Hello from SparkOS userspace!\n";
    sys_write(1, msg);
    sys_exit(0);
}

// Note: In a real standalone crate, we would need our own panic handler.
// We provide one here for completeness of a no_std binary.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe { syscall(1, 1, 0, 0) }; // sys_exit(1)
    loop {}
}
