#![no_std]

//! # libspark — Official SparkOS Userspace SDK
//!
//! SparkOS kullanıcı alanı (Ring 3) uygulamaları geliştirmek için resmî SDK kütüphanesi.
//! Çekirdek iç yapısını bilmeye gerek kalmadan standart ABI üzerinden sistem çağrıları sağlar.

// -----------------------------------------------------------------------------
// Syscall Constants (int 0x80 ABI)
// -----------------------------------------------------------------------------

pub const SYS_READ: u64 = 0;
pub const SYS_EXIT: u64 = 1;
pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_WRITE: u64 = 4;
pub const SYS_YIELD: u64 = 9;

pub const SYS_SOCKET: u64 = 10;
pub const SYS_CONNECT: u64 = 11;
pub const SYS_SEND: u64 = 12;
pub const SYS_RECV: u64 = 13;

pub const SYS_IPC_SEND: u64 = 20;
pub const SYS_IPC_RECV: u64 = 21;
pub const SYS_IPC_CANCEL: u64 = 29;

// -----------------------------------------------------------------------------
// Raw Syscall ABI
// -----------------------------------------------------------------------------

#[inline(always)]
pub unsafe fn raw_syscall3(n: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "int 0x80",
        inout("rax") n => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        options(nostack)
    );
    ret
}

// -----------------------------------------------------------------------------
// Public Userspace API
// -----------------------------------------------------------------------------

/// Standart dosya veya terminale veri yazar (fd 1: stdout, fd 2: stderr)
pub fn write(fd: u64, buf: &[u8]) -> i64 {
    unsafe { raw_syscall3(SYS_WRITE, fd, buf.as_ptr() as u64, buf.len() as u64) as i64 }
}

/// Standart girişten veya dosyadan veri okur (fd 0: stdin)
pub fn read(fd: u64, buf: &mut [u8]) -> i64 {
    unsafe { raw_syscall3(SYS_READ, fd, buf.as_mut_ptr() as u64, buf.len() as u64) as i64 }
}

/// Dosya açar ve dosya tanıtıcısı (FD) döner
pub fn open(path: &str, flags: u64) -> i64 {
    unsafe { raw_syscall3(SYS_OPEN, path.as_ptr() as u64, path.len() as u64, flags) as i64 }
}

/// Açık bir dosya tanıtıcısını kapatır
pub fn close(fd: u64) -> i64 {
    unsafe { raw_syscall3(SYS_CLOSE, fd, 0, 0) as i64 }
}

/// Süreci belirtilen çıkış koduyla sonlandırır
pub fn exit(code: u64) -> ! {
    unsafe {
        raw_syscall3(SYS_EXIT, code, 0, 0);
        loop { core::arch::asm!("hlt"); }
    }
}

/// CPU zamanını diğer süreçlere devreder
pub fn yield_cpu() {
    unsafe { raw_syscall3(SYS_YIELD, 0, 0, 0); }
}

// -----------------------------------------------------------------------------
// Print formatting
// -----------------------------------------------------------------------------

pub struct Stdout;

impl core::fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        write(1, s.as_bytes());
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    let _ = Stdout.write_fmt(args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}
