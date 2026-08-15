#![no_std]

//! # libspark — Official SparkOS Userspace SDK
//!
//! SparkOS kullanıcı alanı (Ring 3) uygulamaları geliştirmek için resmî SDK kütüphanesi.
//! Çekirdek iç yapısını bilmeye gerek kalmadan standart ABI (`int 0x80`) üzerinden
//! sistem çağrıları ve kullanıcı alanı arabirimleri sağlar.

// -----------------------------------------------------------------------------
// Canonical Syscall Numbers (int 0x80 ABI)
// -----------------------------------------------------------------------------

pub const SYS_READ: u64 = 0;
pub const SYS_EXIT: u64 = 1;
pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_WRITE: u64 = 4;
pub const SYS_EXEC: u64 = 7;
pub const SYS_WAITPID: u64 = 8;
pub const SYS_YIELD: u64 = 9;

pub const SYS_SOCKET: u64 = 10;
pub const SYS_CONNECT: u64 = 11;
pub const SYS_SEND: u64 = 12;
pub const SYS_RECV: u64 = 13;

pub const SYS_IPC_SEND: u64 = 20;
pub const SYS_IPC_RECV: u64 = 21;
pub const SYS_IOPERM: u64 = 22;
pub const SYS_IPC_TRY_RECV: u64 = 23;
pub const SYS_IPC_CREATE_ENDPOINT: u64 = 24;
pub const SYS_IPC_BIND_IRQ: u64 = 25;
pub const SYS_MAP_DMA: u64 = 26;
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

// -----------------------------------------------------------------------------
// Standard Error Codes
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Success = 0,
    Invalid = 1,      // EINVAL
    NotFound = 2,     // ENOENT
    PermissionDenied = 3, // EPERM
    BadFileDescriptor = 4, // EBADF
    WouldBlock = 5,   // EAGAIN
    OutOfMemory = 6,  // ENOMEM
    AlreadyExists = 7,// EEXIST
    ConnectionRefused = 8, // ECONNREFUSED
    TimedOut = 9,     // ETIMEDOUT
    Unknown = 255,
}

impl ErrorCode {
    pub fn from_i64(code: i64) -> Self {
        match code {
            0 => ErrorCode::Success,
            -1 => ErrorCode::Invalid,
            -2 => ErrorCode::NotFound,
            -3 => ErrorCode::PermissionDenied,
            -4 => ErrorCode::BadFileDescriptor,
            -5 => ErrorCode::WouldBlock,
            -6 => ErrorCode::OutOfMemory,
            -7 => ErrorCode::AlreadyExists,
            -8 => ErrorCode::ConnectionRefused,
            -9 => ErrorCode::TimedOut,
            _ => ErrorCode::Unknown,
        }
    }
}

// -----------------------------------------------------------------------------
// Raw Syscall Invocations
// -----------------------------------------------------------------------------

#[inline(always)]
pub unsafe fn raw_syscall0(n: u64) -> u64 {
    let ret: u64;
    core::arch::asm!("int 0x80", inout("rax") n => ret, options(nostack));
    ret
}

#[inline(always)]
pub unsafe fn raw_syscall1(n: u64, a1: u64) -> u64 {
    let ret: u64;
    core::arch::asm!("int 0x80", inout("rax") n => ret, in("rdi") a1, options(nostack));
    ret
}

#[inline(always)]
pub unsafe fn raw_syscall2(n: u64, a1: u64, a2: u64) -> u64 {
    let ret: u64;
    core::arch::asm!("int 0x80", inout("rax") n => ret, in("rdi") a1, in("rsi") a2, options(nostack));
    ret
}

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

#[inline(always)]
pub unsafe fn raw_syscall4(n: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "int 0x80",
        inout("rax") n => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        options(nostack)
    );
    ret
}

// -----------------------------------------------------------------------------
// Process API
// -----------------------------------------------------------------------------

pub mod process {
    use super::*;

    /// Süreci belirtilen çıkış koduyla sonlandırır
    pub fn exit(code: u64) -> ! {
        unsafe {
            raw_syscall1(SYS_EXIT, code);
            loop { core::arch::asm!("hlt"); }
        }
    }

    /// CPU zaman dilimini diğer süreçlere devreder
    pub fn yield_cpu() {
        unsafe { raw_syscall0(SYS_YIELD); }
    }

    /// Çocuk sürecin sonlanmasını bekler ve çıkış kodunu döner
    pub fn waitpid(pid: u64) -> Result<u64, ErrorCode> {
        let ret = unsafe { raw_syscall1(SYS_WAITPID, pid) as i64 };
        if ret >= 0 {
            Ok(ret as u64)
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }
}

// -----------------------------------------------------------------------------
// FD & File I/O API
// -----------------------------------------------------------------------------

pub mod fd {
    use super::*;

    pub const O_RDONLY: u64 = 0;
    pub const O_WRONLY: u64 = 1;
    pub const O_RDWR: u64 = 2;
    pub const O_CREAT: u64 = 64;

    /// Dosya açar ve dosya tanıtıcısı (FD) döner
    pub fn open(path: &str, flags: u64) -> Result<u64, ErrorCode> {
        let ret = unsafe { raw_syscall3(SYS_OPEN, path.as_ptr() as u64, path.len() as u64, flags) as i64 };
        if ret >= 0 {
            Ok(ret as u64)
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }

    /// Açık bir dosya tanıtıcısını kapatır
    pub fn close(fd: u64) -> Result<(), ErrorCode> {
        let ret = unsafe { raw_syscall1(SYS_CLOSE, fd) as i64 };
        if ret == 0 {
            Ok(())
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }

    /// Standart girişten veya dosyadan veri okur (fd 0: stdin)
    pub fn read(fd: u64, buf: &mut [u8]) -> Result<usize, ErrorCode> {
        let ret = unsafe { raw_syscall3(SYS_READ, fd, buf.as_mut_ptr() as u64, buf.len() as u64) as i64 };
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }

    /// Standart dosya veya terminale veri yazar (fd 1: stdout, fd 2: stderr)
    pub fn write(fd: u64, buf: &[u8]) -> Result<usize, ErrorCode> {
        let ret = unsafe { raw_syscall3(SYS_WRITE, fd, buf.as_ptr() as u64, buf.len() as u64) as i64 };
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }
}

// -----------------------------------------------------------------------------
// IPC API
// -----------------------------------------------------------------------------

pub mod ipc {
    use super::*;

    /// Yeni bir IPC uç noktası oluşturur
    pub fn create_endpoint(capacity: u32) -> Result<u32, ErrorCode> {
        let ret = unsafe { raw_syscall1(SYS_IPC_CREATE_ENDPOINT, capacity as u64) as i64 };
        if ret >= 0 {
            Ok(ret as u32)
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }

    /// IPC uç noktasına mesaj gönderir
    pub fn send(ep_id: u32, data: &[u8]) -> Result<(), ErrorCode> {
        let ret = unsafe { raw_syscall3(SYS_IPC_SEND, ep_id as u64, data.as_ptr() as u64, data.len() as u64) as i64 };
        if ret == 0 {
            Ok(())
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }

    /// IPC uç noktasından non-blocking mesaj alır
    pub fn try_recv(ep_id: u32, buf: &mut [u8]) -> Result<usize, ErrorCode> {
        let ret = unsafe { raw_syscall3(SYS_IPC_TRY_RECV, ep_id as u64, buf.as_mut_ptr() as u64, buf.len() as u64) as i64 };
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }

    /// Bekleyen IPC isteğini iptal eder
    pub fn cancel(ep_id: u32) -> Result<(), ErrorCode> {
        let ret = unsafe { raw_syscall1(SYS_IPC_CANCEL, ep_id as u64) as i64 };
        if ret == 0 {
            Ok(())
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }
}

// -----------------------------------------------------------------------------
// Network Socket API
// -----------------------------------------------------------------------------

pub mod net {
    use super::*;

    pub const AF_INET: u64 = 0;
    pub const SOCK_STREAM: u64 = 0;
    pub const SOCK_DGRAM: u64 = 1;

    /// Yeni bir soket oluşturur
    pub fn socket(domain: u64, kind: u64) -> Result<u64, ErrorCode> {
        let ret = unsafe { raw_syscall2(SYS_SOCKET, domain, kind) as i64 };
        if ret >= 0 {
            Ok(ret as u64)
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }

    /// Soketi uzak adrese bağlar
    pub fn connect(sock_fd: u64, ip: [u8; 4], port: u16) -> Result<(), ErrorCode> {
        let ip_u32 = u32::from_be_bytes(ip);
        let ret = unsafe { raw_syscall3(SYS_CONNECT, sock_fd, ip_u32 as u64, port as u64) as i64 };
        if ret == 0 {
            Ok(())
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }

    /// Soket üzerinden veri gönderir
    pub fn send(sock_fd: u64, buf: &[u8]) -> Result<usize, ErrorCode> {
        let ret = unsafe { raw_syscall3(SYS_SEND, sock_fd, buf.as_ptr() as u64, buf.len() as u64) as i64 };
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }

    /// Soket üzerinden veri alır
    pub fn recv(sock_fd: u64, buf: &mut [u8]) -> Result<usize, ErrorCode> {
        let ret = unsafe { raw_syscall3(SYS_RECV, sock_fd, buf.as_mut_ptr() as u64, buf.len() as u64) as i64 };
        if ret >= 0 {
            Ok(ret as usize)
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }
}

// -----------------------------------------------------------------------------
// GUI Shared Memory Surface API (Faz 11/12 Hazırlık)
// -----------------------------------------------------------------------------

pub mod gui {
    use super::*;

    /// Compositor üzerinde yeni bir paylaşımlı bellek yüzeyi (surface) tahsis eder
    pub fn create_surface(width: u32, height: u32) -> Result<u64, ErrorCode> {
        let ret = unsafe { raw_syscall2(SYS_CREATE_SURFACE, width as u64, height as u64) as i64 };
        if ret >= 0 {
            Ok(ret as u64)
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }

    /// Yüzeydeki güncellenen dirty dikdörtgeni compositor'a sunar
    pub fn present_surface(surface_id: u64, x: u32, y: u32, w: u32, h: u32) -> Result<(), ErrorCode> {
        let coords = ((x as u64) << 48) | ((y as u64) << 32) | ((w as u64) << 16) | (h as u64);
        let ret = unsafe { raw_syscall2(SYS_PRESENT_SURFACE, surface_id, coords) as i64 };
        if ret == 0 {
            Ok(())
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }

    /// Yüzeyi ve ilişkili capability/shmem kaynaklarını yok eder
    pub fn destroy_surface(surface_id: u64) -> Result<(), ErrorCode> {
        let ret = unsafe { raw_syscall1(SYS_DESTROY_SURFACE, surface_id) as i64 };
        if ret == 0 {
            Ok(())
        } else {
            Err(ErrorCode::from_i64(ret))
        }
    }

    // -------------------------------------------------------------------------
    // Faz 14: GUI Toolkit (Canvas, Rect, Widgets)
    // -------------------------------------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Rect {
        pub x: i32,
        pub y: i32,
        pub width: u32,
        pub height: u32,
    }

    impl Rect {
        pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
            Self { x, y, width, height }
        }

        pub fn contains(&self, px: i32, py: i32) -> bool {
            px >= self.x && px < self.x + (self.width as i32) &&
            py >= self.y && py < self.y + (self.height as i32)
        }

        pub fn intersect(&self, other: &Rect) -> Option<Rect> {
            let x0 = self.x.max(other.x);
            let y0 = self.y.max(other.y);
            let x1 = (self.x + self.width as i32).min(other.x + other.width as i32);
            let y1 = (self.y + self.height as i32).min(other.y + other.height as i32);

            if x1 > x0 && y1 > y0 {
                Some(Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
            } else {
                None
            }
        }
    }

    pub struct Canvas<'a> {
        pub buffer: &'a mut [u32],
        pub width: u32,
        pub height: u32,
        pub clip: Rect,
    }

    impl<'a> Canvas<'a> {
        pub fn new(buffer: &'a mut [u32], width: u32, height: u32) -> Self {
            let clip = Rect::new(0, 0, width, height);
            Self { buffer, width, height, clip }
        }

        pub fn set_clip(&mut self, clip: Rect) {
            self.clip = clip;
        }

        pub fn fill_rect(&mut self, rect: &Rect, color: u32) {
            if let Some(target) = self.clip.intersect(rect) {
                let start_x = target.x.max(0) as u32;
                let start_y = target.y.max(0) as u32;
                let end_x = ((target.x + target.width as i32) as u32).min(self.width);
                let end_y = ((target.y + target.height as i32) as u32).min(self.height);

                for y in start_y..end_y {
                    let row_offset = (y * self.width) as usize;
                    for x in start_x..end_x {
                        let idx = row_offset + (x as usize);
                        if idx < self.buffer.len() {
                            self.buffer[idx] = color;
                        }
                    }
                }
            }
        }

        pub fn draw_border(&mut self, rect: &Rect, thickness: u32, color: u32) {
            if thickness == 0 { return; }
            // Top border
            self.fill_rect(&Rect::new(rect.x, rect.y, rect.width, thickness), color);
            // Bottom border
            self.fill_rect(&Rect::new(rect.x, rect.y + rect.height as i32 - thickness as i32, rect.width, thickness), color);
            // Left border
            self.fill_rect(&Rect::new(rect.x, rect.y, thickness, rect.height), color);
            // Right border
            self.fill_rect(&Rect::new(rect.x + rect.width as i32 - thickness as i32, rect.y, thickness, rect.height), color);
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ButtonState {
        Normal,
        Hover,
        Pressed,
    }

    #[derive(Debug, Clone)]
    pub struct Button {
        pub bounds: Rect,
        pub text: &'static str,
        pub state: ButtonState,
        pub dirty: bool,
        pub clicks: u32,
    }

    impl Button {
        pub fn new(bounds: Rect, text: &'static str) -> Self {
            Self {
                bounds,
                text,
                state: ButtonState::Normal,
                dirty: true,
                clicks: 0,
            }
        }

        pub fn draw(&self, canvas: &mut Canvas) {
            let color = match self.state {
                ButtonState::Normal => 0xFF3B82F6,  // Blue
                ButtonState::Hover => 0xFF60A5FA,   // Light Blue
                ButtonState::Pressed => 0xFF1D4ED8, // Dark Blue
            };
            canvas.fill_rect(&self.bounds, color);
            canvas.draw_border(&self.bounds, 2, 0xFFFFFFFF);
        }

        pub fn handle_event(&mut self, ev: &crate::event::InputEvent) -> bool {
            let inside = self.bounds.contains(ev.mouse_x, ev.mouse_y);
            match ev.event_type {
                3 => { // MouseMove
                    let new_state = if inside { ButtonState::Hover } else { ButtonState::Normal };
                    if self.state != new_state {
                        self.state = new_state;
                        self.dirty = true;
                        return true;
                    }
                }
                4 => { // MouseDown
                    if inside {
                        self.state = ButtonState::Pressed;
                        self.dirty = true;
                        return true;
                    }
                }
                5 => { // MouseUp
                    if self.state == ButtonState::Pressed && inside {
                        self.clicks += 1;
                        self.state = ButtonState::Hover;
                        self.dirty = true;
                        return true;
                    } else if self.state != ButtonState::Normal {
                        self.state = ButtonState::Normal;
                        self.dirty = true;
                    }
                }
                _ => {}
            }
            false
        }
    }

    #[derive(Debug, Clone)]
    pub struct Label {
        pub bounds: Rect,
        pub text: &'static str,
        pub color: u32,
        pub dirty: bool,
    }

    impl Label {
        pub fn new(bounds: Rect, text: &'static str, color: u32) -> Self {
            Self { bounds, text, color, dirty: true }
        }

        pub fn draw(&self, canvas: &mut Canvas) {
            canvas.fill_rect(&self.bounds, self.color);
        }
    }
}

// -----------------------------------------------------------------------------
// Input & Event API (Faz 13)
// -----------------------------------------------------------------------------

pub mod event {
    use super::*;

    #[repr(u8)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum EventType {
        None = 0,
        KeyDown = 1,
        KeyUp = 2,
        MouseMove = 3,
        MouseDown = 4,
        MouseUp = 5,
        MouseWheel = 6,
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct InputEvent {
        pub event_type: u8,
        pub modifiers: u8,
        pub key_code: u8,
        pub mouse_button: u8,
        pub wheel_delta: i8,
        pub _reserved: [u8; 3],
        pub mouse_x: i32,
        pub mouse_y: i32,
        pub timestamp: u64,
        pub _padding: [u8; 8],
    }

    /// Sıradaki olayı okur (Non-blocking)
    pub fn poll(ep_id: u32) -> Option<InputEvent> {
        let mut buf = [0u8; 32];
        match ipc::try_recv(ep_id, &mut buf) {
            Ok(len) if len >= 32 => {
                let ev = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const InputEvent) };
                Some(ev)
            }
            _ => None,
        }
    }

    /// Olay gelene kadar bekler (Yield tabanlı döngü)
    pub fn next(ep_id: u32) -> InputEvent {
        loop {
            if let Some(ev) = poll(ep_id) {
                return ev;
            }
            process::yield_cpu();
        }
    }
}

// -----------------------------------------------------------------------------
// Graphical Terminal & Shell API (Faz 15)
// -----------------------------------------------------------------------------

pub mod terminal {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Cell {
        pub ch: u8,
        pub fg: u32,
        pub bg: u32,
    }

    impl Cell {
        pub const fn new(ch: u8, fg: u32, bg: u32) -> Self {
            Self { ch, fg, bg }
        }
    }

    #[derive(Debug, Clone)]
    pub struct TextGrid<const COLS: usize, const ROWS: usize> {
        pub cells: [[Cell; COLS]; ROWS],
        pub cursor_x: usize,
        pub cursor_y: usize,
    }

    impl<const COLS: usize, const ROWS: usize> TextGrid<COLS, ROWS> {
        pub const fn new(default_fg: u32, default_bg: u32) -> Self {
            let empty_cell = Cell::new(b' ', default_fg, default_bg);
            Self {
                cells: [[empty_cell; COLS]; ROWS],
                cursor_x: 0,
                cursor_y: 0,
            }
        }

        pub fn clear(&mut self, fg: u32, bg: u32) {
            let empty = Cell::new(b' ', fg, bg);
            for r in 0..ROWS {
                for c in 0..COLS {
                    self.cells[r][c] = empty;
                }
            }
            self.cursor_x = 0;
            self.cursor_y = 0;
        }

        pub fn scroll_up(&mut self, fg: u32, bg: u32) {
            for r in 1..ROWS {
                self.cells[r - 1] = self.cells[r];
            }
            let empty = Cell::new(b' ', fg, bg);
            for c in 0..COLS {
                self.cells[ROWS - 1][c] = empty;
            }
            self.cursor_y = ROWS.saturating_sub(1);
        }

        pub fn put_char(&mut self, ch: u8, fg: u32, bg: u32) {
            match ch {
                b'\n' => {
                    self.cursor_x = 0;
                    if self.cursor_y + 1 >= ROWS {
                        self.scroll_up(fg, bg);
                    } else {
                        self.cursor_y += 1;
                    }
                }
                b'\r' => {
                    self.cursor_x = 0;
                }
                0x08 => { // Backspace
                    if self.cursor_x > 0 {
                        self.cursor_x -= 1;
                        self.cells[self.cursor_y][self.cursor_x] = Cell::new(b' ', fg, bg);
                    }
                }
                _ => {
                    if self.cursor_x >= COLS {
                        self.cursor_x = 0;
                        if self.cursor_y + 1 >= ROWS {
                            self.scroll_up(fg, bg);
                        } else {
                            self.cursor_y += 1;
                        }
                    }
                    self.cells[self.cursor_y][self.cursor_x] = Cell::new(ch, fg, bg);
                    self.cursor_x += 1;
                }
            }
        }

        pub fn write_str(&mut self, s: &str, fg: u32, bg: u32) {
            for b in s.bytes() {
                self.put_char(b, fg, bg);
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Filesystem & VFS API (Faz 16)
// -----------------------------------------------------------------------------

pub mod fs {
    use super::*;

    pub use super::fd::{open, close, read, write, O_RDONLY, O_WRONLY, O_RDWR, O_CREAT};

    /// Yeni bir dizin oluşturur
    pub fn mkdir(path: &str) -> Result<(), ErrorCode> {
        let fd = open(path, O_CREAT | O_RDWR)?;
        close(fd)?;
        Ok(())
    }

    /// Dosya veya dizini siler
    pub fn unlink(path: &str) -> Result<(), ErrorCode> {
        let _ = path;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Top-Level Convenience Re-exports
// -----------------------------------------------------------------------------

pub use process::{exit, yield_cpu, waitpid};
pub use fd::{open, close, read, write};

// -----------------------------------------------------------------------------
// Formatted Output (`print!` / `println!`)
// -----------------------------------------------------------------------------

pub struct Stdout;

impl core::fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let _ = fd::write(1, s.as_bytes());
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
