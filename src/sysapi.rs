//! SparkOS Userspace System Call API Documentation and Constants.
//!
//! This module defines the available system calls that can be executed from
//! Ring 3 applications via the `int 0x80` interrupt.

/// Represents information about a single system call.
#[derive(Debug, Clone, Copy)]
pub struct SyscallInfo {
    pub number: u64,
    pub name: &'static str,
    pub description: &'static str,
}

pub const SYS_READ: u64 = 0;
pub const SYS_EXIT: u64 = 1;
pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_WRITE: u64 = 4;
pub const SYS_LSEEK: u64 = 8;

/// Static table of all supported system calls in SparkOS 1.0.
pub static SYSCALLS: &[SyscallInfo] = &[
    SyscallInfo {
        number: SYS_READ,
        name: "sys_read",
        description: "Read from a file descriptor into a buffer.",
    },
    SyscallInfo {
        number: SYS_EXIT,
        name: "sys_exit",
        description: "Terminate the current application.",
    },
    SyscallInfo {
        number: SYS_OPEN,
        name: "sys_open",
        description: "Open a file and return a file descriptor.",
    },
    SyscallInfo {
        number: SYS_CLOSE,
        name: "sys_close",
        description: "Close an open file descriptor.",
    },
    SyscallInfo {
        number: SYS_WRITE,
        name: "sys_write",
        description: "Write from a buffer to a file descriptor (1/2 for terminal).",
    },
    SyscallInfo {
        number: SYS_LSEEK,
        name: "sys_lseek",
        description: "Reposition the file offset of a file descriptor.",
    },
];

/// Returns the table of available system calls.
pub fn syscall_table() -> &'static [SyscallInfo] {
    SYSCALLS
}
