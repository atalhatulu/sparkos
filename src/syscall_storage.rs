use crate::fd::FD_TABLE;

pub const SYS_OPEN: u64 = 2;
pub const SYS_READ: u64 = 0;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_LSEEK: u64 = 8;

/// Maximum accepted path length (bytes), including the trailing NUL.
const MAX_PATH: usize = 256;

/// errno for "bad address". Negative errno values are returned to the caller
/// as their unsigned encoding.
const EFAULT: u64 = (-14i64) as u64;

/// Scans a validated user buffer for the NUL terminator, enforcing a hard cap.
/// Returns the number of bytes up to (not including) the NUL, or `Err(EFAULT)`
/// if no terminator is found within `max`.
fn user_strn_len(buf: &[u8], max: usize) -> Result<usize, u64> {
    let scan = if buf.len() < max { buf.len() } else { max };
    for (i, &b) in buf[..scan].iter().enumerate() {
        if b == 0 {
            return Ok(i);
        }
    }
    Err(EFAULT)
}

pub fn sys_open(path_ptr: u64, flags: u64) -> u64 {
    // Validate the path pointer before dereferencing it: canonical, user half,
    // bounded length, every page user-mapped. Reject with -EFAULT.
    let path_buf = match crate::sec_mem::validate_user_ptr(path_ptr, MAX_PATH) {
        Ok(b) => b,
        Err(_) => {
            crate::serial_println!("[SYSCALL] sys_open Error: invalid path pointer (EFAULT)");
            return EFAULT;
        }
    };

    let len = match user_strn_len(path_buf, MAX_PATH) {
        Ok(l) => l,
        Err(e) => {
            crate::serial_println!("[SYSCALL] sys_open Error: unterminated path (EFAULT)");
            return e;
        }
    };

    if let Ok(path) = core::str::from_utf8(&path_buf[..len]) {
        match FD_TABLE.lock().open(path, flags as u32) {
            Ok(fd) => fd as u64,
            Err(e) => {
                crate::serial_println!("[SYSCALL] sys_open Error: {}", e);
                u64::MAX
            }
        }
    } else {
        u64::MAX
    }
}

pub fn sys_read(fd: u64, buf_ptr: u64, len: u64) -> u64 {
    // Validate the destination buffer is user-writable before writing to it.
    let len_usize = len as usize;
    let buf = match crate::sec_mem::validate_user_ptr_mut(buf_ptr, len_usize) {
        Ok(b) => b,
        Err(_) => {
            crate::serial_println!("[SYSCALL] sys_read Error: invalid buffer (EFAULT)");
            return EFAULT;
        }
    };

    match FD_TABLE.lock().read(fd as usize, &mut *buf) {
        Ok(bytes_read) => bytes_read as u64,
        Err(e) => {
            crate::serial_println!("[SYSCALL] sys_read Error: {}", e);
            u64::MAX
        }
    }
}

pub fn sys_write(fd: u64, buf_ptr: u64, len: u64) -> u64 {
    // Validate the source buffer is user-readable before reading it.
    let len_usize = len as usize;
    let buf = match crate::sec_mem::validate_user_ptr(buf_ptr, len_usize) {
        Ok(b) => b,
        Err(_) => {
            crate::serial_println!("[SYSCALL] sys_write Error: invalid buffer (EFAULT)");
            return EFAULT;
        }
    };

    match FD_TABLE.lock().write(fd as usize, buf) {
        Ok(bytes_written) => bytes_written as u64,
        Err(e) => {
            crate::serial_println!("[SYSCALL] sys_write Error: {}", e);
            u64::MAX
        }
    }
}

pub fn sys_close(fd: u64) -> u64 {
    match FD_TABLE.lock().close(fd as usize) {
        Ok(_) => 0,
        Err(e) => {
            crate::serial_println!("[SYSCALL] sys_close Error: {}", e);
            u64::MAX
        }
    }
}

pub fn sys_lseek(fd: u64, offset: i64, whence: u64) -> u64 {
    match FD_TABLE.lock().lseek(fd as usize, offset as isize, whence as u32) {
        Ok(new_offset) => new_offset as u64,
        Err(e) => {
            crate::serial_println!("[SYSCALL] sys_lseek Error: {}", e);
            u64::MAX
        }
    }
}
