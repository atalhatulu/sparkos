use crate::fd::FD_TABLE;

pub const SYS_OPEN: u64 = 2;
pub const SYS_READ: u64 = 0;
pub const SYS_CLOSE: u64 = 3;
// pub const SYS_WRITE: u64 = 4; // Zaten syscall.rs'te var, ama storage için biz ele alacağız
pub const SYS_LSEEK: u64 = 8;

pub fn sys_open(path_ptr: u64, flags: u64) -> u64 {
    // Okuma işlemi (path null terminated string kabul edilir veya belirli uzunluk)
    // Güvenli varsayıyoruz:
    let mut len = 0;
    while unsafe { *((path_ptr as *const u8).add(len)) } != 0 {
        len += 1;
        if len > 255 { break; } // Maksimum yol uzunluğu sınırı
    }
    
    let bytes = unsafe { core::slice::from_raw_parts(path_ptr as *const u8, len) };
    if let Ok(path) = core::str::from_utf8(bytes) {
        match FD_TABLE.lock().open(path, flags as u32) {
            Ok(fd) => fd as u64,
            Err(e) => {
                crate::serial_println!("[SYSCALL] sys_open Error: {}", e);
                u64::MAX // -1 Error
            }
        }
    } else {
        u64::MAX
    }
}

pub fn sys_read(fd: u64, buf_ptr: u64, len: u64) -> u64 {
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
    match FD_TABLE.lock().read(fd as usize, buf) {
        Ok(bytes_read) => bytes_read as u64,
        Err(e) => {
            crate::serial_println!("[SYSCALL] sys_read Error: {}", e);
            u64::MAX
        }
    }
}

pub fn sys_write(fd: u64, buf_ptr: u64, len: u64) -> u64 {
    // 1 ve 2 (stdout, stderr) syscall.rs içinde ele alınabilir
    // ama storage'a da yazabilmek için burayı kullanacağız
    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
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
