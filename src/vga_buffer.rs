use core::fmt;

const BUFFER_HEIGHT: usize = 25;
const BUFFER_WIDTH: usize = 80;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
struct ColorCode(u8);

impl ColorCode {
    const fn new(foreground: Color, background: Color) -> ColorCode {
        ColorCode((background as u8) << 4 | (foreground as u8))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct ScreenChar {
    ascii_character: u8,
    color_code: ColorCode,
}

#[repr(transparent)]
struct Buffer {
    chars: [[ScreenChar; BUFFER_WIDTH]; BUFFER_HEIGHT],
}

pub static GUI_MODE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub struct VgaWriter {
    column: usize,
    row: usize,
    color: ColorCode,
    buffer: &'static mut Buffer,
}

impl VgaWriter {
    pub fn set_color(&mut self, fg: Color, bg: Color) {
        self.color = ColorCode::new(fg, bg);
    }

    pub fn write_at(&mut self, row: usize, col: usize, text: &str, fg: Color, bg: Color) {
        let color_code = ColorCode::new(fg, bg);
        let mut current_col = col;
        for byte in text.bytes() {
            if current_col >= BUFFER_WIDTH { break; }
            unsafe {
                core::ptr::write_volatile(
                    &mut self.buffer.chars[row][current_col],
                    ScreenChar {
                        ascii_character: byte,
                        color_code,
                    }
                );
            }
            current_col += 1;
        }
    }

    pub fn clear(&mut self) {
        if GUI_MODE.load(core::sync::atomic::Ordering::Relaxed) {
            let mut gw_arr = crate::gui::WRITERS.lock();
            let gw = &mut gw_arr[0];
            gw.clear();
            let wx = gw.win_x;
            let wy = gw.win_y;
            let ww = gw.win_w;
            let wh = gw.win_h;
            drop(gw_arr);
            crate::gui::flush_rect(wx, wy, ww, wh);
            return;
        }
        
        let blank = ScreenChar {
            ascii_character: b' ',
            color_code: self.color,
        };
        for row in 0..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                unsafe {
                    core::ptr::write_volatile(&mut self.buffer.chars[row][col], blank);
                }
            }
        }
        self.column = 0;
        self.row = 0;
        self.update_cursor();
    }

    fn update_cursor(&self) {
        let pos = self.row * BUFFER_WIDTH + self.column;
        let mut port_3d4 = x86_64::instructions::port::Port::new(0x3D4);
        let mut port_3d5 = x86_64::instructions::port::Port::new(0x3D5);
        unsafe {
            port_3d4.write(14u8);
            port_3d5.write((pos >> 8) as u8);
            port_3d4.write(15u8);
            port_3d5.write((pos & 0xFF) as u8);
        }
    }

    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            b'\x08' => { // Backspace: git bir geri, boşluk bas, tekrar git
                if self.column > 0 {
                    self.column -= 1;
                    unsafe {
                        core::ptr::write_volatile(
                            &mut self.buffer.chars[self.row][self.column],
                            ScreenChar {
                                ascii_character: b' ',
                                color_code: self.color,
                            }
                        );
                    }
                }
            }
            byte => {
                if self.column >= BUFFER_WIDTH {
                    self.new_line();
                }
                unsafe {
                    core::ptr::write_volatile(
                        &mut self.buffer.chars[self.row][self.column],
                        ScreenChar {
                            ascii_character: byte,
                            color_code: self.color,
                        }
                    );
                }
                self.column += 1;
            }
        }
        self.update_cursor();
    }

    fn new_line(&mut self) {
        if self.row < BUFFER_HEIGHT - 1 {
            self.row += 1;
        } else {
            for row in 1..BUFFER_HEIGHT {
                for col in 0..BUFFER_WIDTH {
                    unsafe {
                        let c = core::ptr::read_volatile(&self.buffer.chars[row][col]);
                        core::ptr::write_volatile(&mut self.buffer.chars[row - 1][col], c);
                    }
                }
            }
            let blank = ScreenChar {
                ascii_character: b' ',
                color_code: self.color,
            };
            for col in 0..BUFFER_WIDTH {
                unsafe {
                    core::ptr::write_volatile(&mut self.buffer.chars[BUFFER_HEIGHT - 1][col], blank);
                }
            }
        }
        self.column = 0;
    }
}

impl fmt::Write for VgaWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if GUI_MODE.load(core::sync::atomic::Ordering::Relaxed) {
            let fg_u32 = match self.color.0 & 0x0F {
                0 => 0x00000000, 1 => 0x000000AA, 2 => 0x0000AA00, 3 => 0x0000AAAA,
                4 => 0x00AA0000, 5 => 0x00AA00AA, 6 => 0x00AA5500, 7 => 0x00AAAAAA,
                8 => 0x00555555, 9 => 0x005555FF, 10 => 0x0055FF55, 11 => 0x0055FFFF,
                12 => 0x00FF5555, 13 => 0x00FF55FF, 14 => 0x00FFFF55, 15 => 0x00FFFFFF,
                _ => 0x00FFFFFF,
            };
            let bg_u32 = match self.color.0 >> 4 {
                0 => 0x001E1E1E, // Siyah yerine terminal gri
                1 => 0x000000AA, 2 => 0x0000AA00, 3 => 0x0000AAAA,
                4 => 0x00AA0000, 5 => 0x00AA00AA, 6 => 0x00AA5500, 7 => 0x00AAAAAA,
                8 => 0x00555555, 9 => 0x005555FF, 10 => 0x0055FF55, 11 => 0x0055FFFF,
                12 => 0x00FF5555, 13 => 0x00FF55FF, 14 => 0x00FFFF55, 15 => 0x00FFFFFF,
                _ => 0x001E1E1E,
            };
            
            let mut gw_arr = crate::gui::WRITERS.lock();
            let gw = &mut gw_arr[0];
            gw.set_color(fg_u32, bg_u32);
            let _ = core::fmt::Write::write_str(&mut *gw, s);
            let wx = gw.win_x;
            let wy = gw.win_y;
            let ww = gw.win_w;
            let wh = gw.win_h;
            drop(gw_arr); // Kilidi birak!
            crate::gui::flush_rect(wx, wy, ww, wh); // Sadece pencere alanini guncelle
            return Ok(());
        }
        
        for byte in s.bytes() {
            self.write_byte(byte);
        }
        if let Some(ref mut port) = *crate::serial::SERIAL.lock() {
            let _ = core::fmt::Write::write_str(port, s);
        }
        Ok(())
    }
}

pub static WRITE_LOCK: spin::Lazy<spin::Mutex<VgaWriter>> = spin::Lazy::new(|| {
    let vga_addr = unsafe {
        let addr = crate::memory::VGA_VIRT_ADDR;
        if addr == 0 { 0xB8000 as *mut u8 } else { addr as *mut u8 }
    };
    spin::Mutex::new(VgaWriter {
        column: 0,
        row: 0,
        color: ColorCode::new(Color::White, Color::Black),
        buffer: unsafe { &mut *(vga_addr as *mut Buffer) },
    })
});
