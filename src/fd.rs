use alloc::string::{String, ToString};
use spin::Mutex;
use crate::fs;

pub const O_RDONLY: u32 = 0;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
pub const O_CREAT: u32 = 64;

pub const SEEK_SET: u32 = 0;
pub const SEEK_CUR: u32 = 1;
pub const SEEK_END: u32 = 2;

#[derive(Clone)]
pub struct FileDescriptor {
    pub fd: usize,
    pub path: String, // node referansı/pointer yerine yol kullanıyoruz
    pub offset: usize,
    pub flags: u32,
    pub ref_count: usize,
}

pub struct FdTable {
    fds: [Option<FileDescriptor>; 256],
}

impl FdTable {
    pub const fn new() -> Self {
        const INIT: Option<FileDescriptor> = None;
        Self { fds: [INIT; 256] }
    }

    pub fn open(&mut self, path: &str, flags: u32) -> Result<usize, &'static str> {
        let fd = self.fds.iter().position(|f| f.is_none()).ok_or("No free fd")?;
        
        // VFS kontrolü. Seeded binaries (e.g. /bin/hello) are backed by the
        // binary-safe store and are always openable, even though their SPFS
        // tree node is only a placeholder.
        let exists = fs::file_exists(path) || !fs::is_dir(path) && {
            fs::read_file(path).is_ok() || fs::is_seeded_binary(path)
        };
        if !exists {
            if flags & O_CREAT != 0 {
                fs::write_file(path, "")?;
            } else {
                return Err("File not found");
            }
        }
        
        self.fds[fd] = Some(FileDescriptor {
            fd,
            path: path.to_string(),
            offset: 0,
            flags,
            ref_count: 1,
        });
        Ok(fd)
    }

    pub fn close(&mut self, fd: usize) -> Result<(), &'static str> {
        if fd >= 256 || self.fds[fd].is_none() {
            return Err("Invalid fd");
        }
        self.fds[fd] = None;
        Ok(())
    }

    pub fn read(&mut self, fd: usize, buf: &mut [u8]) -> Result<usize, &'static str> {
        let desc = self.fds.get_mut(fd).and_then(|f| f.as_mut()).ok_or("Invalid fd")?;
        if desc.flags & O_WRONLY != 0 && desc.flags & O_RDWR == 0 {
            return Err("Bad fd flags for reading");
        }
        
        // Route seeded binaries (byte-exact ELF) through the binary-safe
        // path reader; everything else keeps the legacy UTF-8 chunk path.
        let len = if fs::is_seeded_binary(&desc.path) {
            fs::read_file_from_path_chunk(&desc.path, desc.offset, buf)?
        } else {
            fs::read_file_chunk(&desc.path, desc.offset, buf)?
        };
        desc.offset += len;
        Ok(len)
    }

    pub fn write(&mut self, fd: usize, buf: &[u8]) -> Result<usize, &'static str> {
        let desc = self.fds.get_mut(fd).and_then(|f| f.as_mut()).ok_or("Invalid fd")?;
        if desc.flags & O_WRONLY == 0 && desc.flags & O_RDWR == 0 {
            return Err("Bad fd flags for writing");
        }
        
        let len = fs::write_file_chunk(&desc.path, desc.offset, buf)?;
        desc.offset += len;
        Ok(len)
    }

    pub fn lseek(&mut self, fd: usize, offset: isize, whence: u32) -> Result<usize, &'static str> {
        let desc = self.fds.get_mut(fd).and_then(|f| f.as_mut()).ok_or("Invalid fd")?;
        let content_len = if fs::is_seeded_binary(&desc.path) {
            fs::get_file_size_from_path(&desc.path)?
        } else {
            fs::get_file_size(&desc.path)?
        };
        
        let new_offset = match whence {
            SEEK_SET => offset,
            SEEK_CUR => desc.offset as isize + offset,
            SEEK_END => content_len as isize + offset,
            _ => return Err("Invalid whence"),
        };
        
        if new_offset < 0 {
            return Err("Invalid offset");
        }
        
        desc.offset = new_offset as usize;
        Ok(desc.offset)
    }
}

pub static FD_TABLE: spin::Lazy<Mutex<FdTable>> = spin::Lazy::new(|| Mutex::new(FdTable::new()));
