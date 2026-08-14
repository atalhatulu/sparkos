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
    SyscallInfo {
        number: SYS_YIELD,
        name: "sys_yield",
        description: "Cooperatively yield the CPU so other kernel tasks progress.",
    },
    SyscallInfo {
        number: SYS_SOCKET,
        name: "sys_socket",
        description: "Create a socket (network domain) and return its file descriptor.",
    },
    SyscallInfo {
        number: SYS_CONNECT,
        name: "sys_connect",
        description: "Connect a socket to a remote (IP, port) endpoint.",
    },
    SyscallInfo {
        number: SYS_SEND,
        name: "sys_send",
        description: "Send data over a connected socket.",
    },
    SyscallInfo {
        number: SYS_RECV,
        name: "sys_recv",
        description: "Receive data from a socket into a buffer.",
    },
    SyscallInfo {
        number: SYS_FORK,
        name: "sys_fork",
        description: "Fork the current process (requires EXECUTE capability).",
    },
    SyscallInfo {
        number: SYS_EXEC,
        name: "sys_exec",
        description: "Replace the current process with a fresh ELF image (requires EXECUTE capability).",
    },
    SyscallInfo {
        number: SYS_IPC_SEND,
        name: "sys_ipc_send",
        description: "Send a byte payload + optional capability over an IPC endpoint (WRITE right).",
    },
    SyscallInfo {
        number: SYS_IPC_RECV,
        name: "sys_ipc_recv",
        description: "Blocking receive from an IPC endpoint (READ right).",
    },
    SyscallInfo {
        number: SYS_IPC_TRY_RECV,
        name: "sys_ipc_try_recv",
        description: "Non-blocking receive from an IPC endpoint; EAGAIN when empty.",
    },
    SyscallInfo {
        number: SYS_IOPERM,
        name: "sys_ioperm",
        description: "Bind a port I/O range to the calling process (requires IO right).",
    },
];

/// Returns the table of available system calls.
pub fn syscall_table() -> &'static [SyscallInfo] {
    SYSCALLS
}
