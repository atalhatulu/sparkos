#![no_std]

pub mod errno {
    pub const EPERM: u64 = 1;
    pub const ENOENT: u64 = 2;
    pub const ESRCH: u64 = 3;
    pub const EINTR: u64 = 4;
    pub const EIO: u64 = 5;
    pub const EBADF: u64 = 9;
    pub const EAGAIN: u64 = 11;
    pub const ENOMEM: u64 = 12;
    pub const EACCES: u64 = 13;
    pub const EFAULT: u64 = 14;
    pub const EINVAL: u64 = 22;
    pub const ENOSYS: u64 = 38;
    pub const EDQUOT: u64 = 122;
    pub const EQUOTA: u64 = 122;

    pub const EFAULT_RET: u64 = (-14i64) as u64;
    pub const EACCES_RET: u64 = (-13i64) as u64;
    pub const EAGAIN_RET: u64 = (-11i64) as u64;
    pub const EQUOTA_RET: u64 = (-122i64) as u64;
}

pub mod syscall {
    pub const SYS_EXIT: u64 = 1;
    pub const SYS_WRITE: u64 = 4;
    pub const SYS_READ: u64 = 0;
    pub const SYS_OPEN: u64 = 2;
    pub const SYS_CLOSE: u64 = 3;
    pub const SYS_LSEEK: u64 = 8;
    pub const SYS_YIELD: u64 = 9;

    // Sockets (Linux-compatible)
    pub const SYS_SOCKET: u64 = 10;
    pub const SYS_CONNECT: u64 = 11;
    pub const SYS_SEND: u64 = 12;
    pub const SYS_RECV: u64 = 13;

    // Process model
    pub const SYS_FORK: u64 = 14;
    pub const SYS_EXEC: u64 = 15;
    pub const SYS_WAITPID: u64 = 16;

    // Microkernel IPC & Device Port
    pub const SYS_IPC_SEND: u64 = 20;
    pub const SYS_IPC_RECV: u64 = 21;
    pub const SYS_IOPERM: u64 = 22;
    pub const SYS_IPC_TRY_RECV: u64 = 23;
    pub const SYS_IPC_CREATE_ENDPOINT: u64 = 24;
    pub const SYS_IPC_BIND_IRQ: u64 = 25;
    pub const SYS_MAP_DMA: u64 = 26;
    pub const SYS_NET_SEND_FRAME: u64 = 27;
    pub const SYS_NET_RECV_FRAME: u64 = 28;
    pub const SYS_IPC_CANCEL: u64 = 29;
    pub const SYS_IPC_CREATE_SLOT: u64 = 30;

    // Surface & Window Manager
    pub const SYS_CREATE_SURFACE: u64 = 31;
    pub const SYS_PRESENT_SURFACE: u64 = 32;
    pub const SYS_DESTROY_SURFACE: u64 = 33;
    pub const SYS_CREATE_WINDOW: u64 = 34;
    pub const SYS_DESTROY_WINDOW: u64 = 35;
    pub const SYS_MOVE_WINDOW: u64 = 36;
    pub const SYS_MINIMIZE_WINDOW: u64 = 37;
    pub const SYS_RESTORE_WINDOW: u64 = 38;
    pub const SYS_POLL_EVENT: u64 = 39;

    // Multiprocessing & Signals & FS
    pub const SYS_SETAFFINITY: u64 = 40;
    pub const SYS_GETAFFINITY: u64 = 41;
    pub const SYS_MKDIR: u64 = 42;
    pub const SYS_STAT: u64 = 43;
    pub const SYS_KILL: u64 = 44;
    pub const SYS_SIGACTION: u64 = 45;
    pub const SYS_SIGPROCMASK: u64 = 46;
}

pub mod cap {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum CapError {
        Invalid,
        Revoked,
        NoRights,
        NotFound,
        Exhausted,
        AlreadyExists,
        QuotaExceeded,
    }

    impl CapError {
        pub const fn to_errno(&self) -> u64 {
            match self {
                CapError::Invalid => crate::errno::EINVAL,
                CapError::Revoked | CapError::NoRights => crate::errno::EACCES,
                CapError::NotFound => crate::errno::ENOENT,
                CapError::Exhausted => crate::errno::ENOMEM,
                CapError::AlreadyExists => crate::errno::EAGAIN,
                CapError::QuotaExceeded => crate::errno::EQUOTA,
            }
        }
    }

    pub type Result<T> = core::result::Result<T, CapError>;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct CapHandle {
        pub slot: u32,
        pub generation: u32,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Rights(pub u32);

    impl Rights {
        pub const READ: Rights     = Rights(1 << 0);
        pub const WRITE: Rights    = Rights(1 << 1);
        pub const MAP: Rights      = Rights(1 << 2);
        pub const IO: Rights       = Rights(1 << 3);
        pub const DMA: Rights      = Rights(1 << 4);
        pub const TRANSFER: Rights = Rights(1 << 5);
        pub const GRANT: Rights    = Rights(1 << 6);
        pub const DESTROY: Rights  = Rights(1 << 7);
        pub const EXECUTE: Rights  = Rights(1 << 8);
        pub const MANAGE: Rights   = Rights(1 << 9);

        #[inline]
        pub const fn empty() -> Self { Rights(0) }
        #[inline]
        pub const fn all() -> Self { Rights(0x3FF) }

        #[inline]
        pub fn contains(&self, other: Rights) -> bool {
            (self.0 & other.0) == other.0
        }
    }

    impl core::ops::BitAnd for Rights {
        type Output = Self;
        #[inline]
        fn bitand(self, rhs: Self) -> Self {
            Rights(self.0 & rhs.0)
        }
    }

    impl core::ops::BitOr for Rights {
        type Output = Self;
        #[inline]
        fn bitor(self, rhs: Self) -> Self {
            Rights(self.0 | rhs.0)
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ObjectKind {
        Memory,
        Device,
        Endpoint,
        Generic,
        Fd,
        Process,
        MemObject,
    }
}

pub mod ipc {
    use super::cap::CapHandle;

    pub const SERVICE_DEVICE_FD: u32 = u32::MAX - 1;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TransferMode {
        None,
        /// Sahiplik aktarımı: kaynak eski lineage'dan tamamen koparılır, alıcı yeni root olur.
        Transfer,
        /// Geçici ödünç: gönderici mesaj kuyruktayken yetkiyi geri alabilir (revoke).
        Lend,
    }

    /// Capability taşıyabilen mesaj zarfı
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CapMessage<M> {
        pub payload: M,
        pub capability: Option<CapHandle>,
        pub transfer_mode: TransferMode,
    }

    impl<M> CapMessage<M> {
        pub fn new_plain(payload: M) -> Self {
            CapMessage {
                payload,
                capability: None,
                transfer_mode: TransferMode::None,
            }
        }

        pub fn new_with_cap(payload: M, cap: CapHandle, mode: TransferMode) -> Self {
            CapMessage {
                payload,
                capability: Some(cap),
                transfer_mode: mode,
            }
        }
    }
}

pub use cap::{CapError, CapHandle, ObjectKind, Rights};
pub use errno::*;
pub use ipc::{CapMessage, TransferMode, SERVICE_DEVICE_FD};
pub use syscall::*;

pub mod service;
pub mod fs;
pub mod input;
pub mod net;
pub mod compositor;

