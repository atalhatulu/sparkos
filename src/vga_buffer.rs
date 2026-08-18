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
