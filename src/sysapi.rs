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
pub const SYS_IPC_CREATE_ENDPOINT: u64 = 24;
pub const SYS_IPC_BIND_IRQ: u64 = 25;
/// Capability-Gated DMA bölgesi eşleme (Aşama 6.2).
pub const SYS_MAP_DMA: u64 = 26;
/// Zero-Copy L2 Frame Gönderimi (Aşama 6.3).
pub const SYS_NET_SEND_FRAME: u64 = 27;
/// Zero-Copy L2 Frame Alımı (Aşama 6.3).
pub const SYS_NET_RECV_FRAME: u64 = 28;
/// Cooperative IPC İptali (Aşama 7.1).
pub const SYS_IPC_CANCEL: u64 = 29;
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

// -----------------------------------------------------------------------------
// Faz 7: Userspace Syscall Wrappers (`lib/sysapi` ABI)
// -----------------------------------------------------------------------------

/// Raw system call invocation via `int 0x80`
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

/// Standard output write
pub fn write(fd: u64, buf: &[u8]) -> i64 {
    unsafe { raw_syscall3(SYS_WRITE, fd, buf.as_ptr() as u64, buf.len() as u64) as i64 }
}

/// Standard input read
pub fn read(fd: u64, buf: &mut [u8]) -> i64 {
    unsafe { raw_syscall3(SYS_READ, fd, buf.as_mut_ptr() as u64, buf.len() as u64) as i64 }
}

/// Process exit
pub fn exit(code: u64) -> ! {
    unsafe {
        raw_syscall3(SYS_EXIT, code, 0, 0);
        loop { core::arch::asm!("hlt"); }
    }
}

/// Cooperative yield
pub fn yield_cpu() {
    unsafe { raw_syscall3(SYS_YIELD, 0, 0, 0); }
}
