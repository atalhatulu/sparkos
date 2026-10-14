//! SparkOS File System IPC Protocol & On-Disk Structs (Faz 5.1)
//!
//! Provides ABI structures for Ring-3 File System Service (SPFS v1/v2),
//! Capability-gated File Handles, VFS Operations, and On-Disk Layout definitions.


pub const MAX_FS_PATH_LEN: usize = 64;

pub const FS_OP_OPEN: u32 = 1;
pub const FS_OP_READ: u32 = 2;
pub const FS_OP_WRITE: u32 = 3;
pub const FS_OP_STAT: u32 = 4;
pub const FS_OP_READDIR: u32 = 5;
pub const FS_OP_MKDIR: u32 = 6;
pub const FS_OP_UNLINK: u32 = 7;
pub const FS_OP_RENAME: u32 = 8;
pub const FS_OP_SYNC: u32 = 9;
pub const FS_OP_CLOSE: u32 = 10;

pub const O_RDONLY: u32 = 0;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
pub const O_CREAT: u32 = 0x40;
pub const O_TRUNC: u32 = 0x200;
pub const O_APPEND: u32 = 0x400;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FsRequest {
    pub opcode: u32,
    pub handle: u32,
    pub flags: u32,
    pub mode: u32,
    pub offset: u64,
    pub length: u64,
    pub path: [u8; MAX_FS_PATH_LEN],
    pub path_len: u32,
}

impl FsRequest {
    pub const fn empty() -> Self {
        Self {
            opcode: 0,
            handle: 0,
            flags: 0,
            mode: 0,
            offset: 0,
            length: 0,
            path: [0u8; MAX_FS_PATH_LEN],
            path_len: 0,
        }
    }

    pub fn new_open(path: &str, flags: u32, mode: u32) -> Self {
        let mut req = Self::empty();
        req.opcode = FS_OP_OPEN;
        req.flags = flags;
        req.mode = mode;
        req.set_path(path);
        req
    }

    pub fn new_read(handle: u32, offset: u64, length: u64) -> Self {
        let mut req = Self::empty();
        req.opcode = FS_OP_READ;
        req.handle = handle;
        req.offset = offset;
        req.length = length;
        req
    }

    pub fn new_write(handle: u32, offset: u64, length: u64) -> Self {
        let mut req = Self::empty();
        req.opcode = FS_OP_WRITE;
        req.handle = handle;
        req.offset = offset;
        req.length = length;
        req
    }

    pub fn new_stat(path: &str) -> Self {
        let mut req = Self::empty();
        req.opcode = FS_OP_STAT;
        req.set_path(path);
        req
    }

    pub fn new_close(handle: u32) -> Self {
        let mut req = Self::empty();
        req.opcode = FS_OP_CLOSE;
        req.handle = handle;
        req
    }

    pub fn set_path(&mut self, path: &str) {
        let bytes = path.as_bytes();
        let len = bytes.len().min(MAX_FS_PATH_LEN);
        self.path = [0u8; MAX_FS_PATH_LEN];
        self.path[..len].copy_from_slice(&bytes[..len]);
        self.path_len = len as u32;
    }

    pub fn path_str(&self) -> &str {
        let len = (self.path_len as usize).min(MAX_FS_PATH_LEN);
        core::str::from_utf8(&self.path[..len]).unwrap_or("")
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FsStat {
    pub inode_id: u32,
    pub size: u64,
    pub mode: u32,
    pub blocks: u64,
    pub is_dir: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FsResponse {
    pub status: u32,
    pub handle: u32,
    pub bytes_transferred: u64,
    pub inode_id: u32,
    pub size: u64,
    pub is_dir: bool,
}

impl FsResponse {
    pub const fn ok(handle: u32, bytes: u64) -> Self {
        Self {
            status: 0,
            handle,
            bytes_transferred: bytes,
            inode_id: 0,
            size: 0,
            is_dir: false,
        }
    }

    pub const fn err(errno_val: u32) -> Self {
        Self {
            status: errno_val,
            handle: 0,
            bytes_transferred: 0,
            inode_id: 0,
            size: 0,
            is_dir: false,
        }
    }

    pub fn is_ok(&self) -> bool {
        self.status == 0
    }
}
