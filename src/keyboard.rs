use spin::Mutex;

/// PS/2 klavye scancode → ASCII çevirici (US QWERTY)
/// IRQ handler'ından çağrılır, karakterleri ring buffer'a koyar.

const BUF_SIZE: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Ascii(u8),
    Backspace,
    Delete,
    Enter,
    Escape,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    CtrlC,
    Unknown,
}

pub struct Keyboard {
    buffer: [Key; BUF_SIZE],
    read: usize,
    write: usize,
    shift: bool,
    ctrl: bool,
    extended: bool,
}

impl Keyboard {
    pub const fn new() -> Self {
        Keyboard {
            buffer: [Key::Unknown; BUF_SIZE],
            read: 0,
            write: 0,
            shift: false,
            ctrl: false,
            extended: false,
        }
    }

    pub fn clear(&mut self) {
        self.read = 0;
        self.write = 0;
        self.shift = false;
        self.ctrl = false;
        self.extended = false;
    }

    fn push(&mut self, key: Key) {
        let next = (self.write + 1) % BUF_SIZE;
        if next != self.read {
            self.buffer[self.write] = key;
            self.write = next;
        }
    }

    pub fn pop(&mut self) -> Option<Key> {
        if self.read == self.write {
            None
        } else {
            let key = core::mem::replace(&mut self.buffer[self.read], Key::Unknown);
            self.read = (self.read + 1) % BUF_SIZE;
            Some(key)
        }
    }

    /// Scancode işle — IRQ handler'ından çağrılır
    pub fn handle_scancode(&mut self, scancode: u8) {
        // Extended prefix (0xE0) — tusun ikinci byte'ı geliyor
        if self.extended {
            self.extended = false;
            match scancode {
                0x53 => self.push(Key::Delete),    // Delete
                0x4B => self.push(Key::Left),      // Left arrow
                0x4D => self.push(Key::Right),     // Right arrow
                0x48 => self.push(Key::Up),        // Up arrow
                0x50 => self.push(Key::Down),      // Down arrow
                0x47 => self.push(Key::Home),      // Home
                0x4F => self.push(Key::End),       // End
                0x52 => self.push(Key::Delete),    // Insert
                0x1D => self.ctrl = true,          // Right Ctrl make
                0x9D => self.ctrl = false,         // Right Ctrl break
                _ => {}
            }
            return;
        }

        // Extended baslangıcı
        if scancode == 0xE0 {
            self.extended = true;
            return;
        }

        // Break code'ları işle (bit 7 set)
        if scancode & 0x80 != 0 {
            let make = scancode & 0x7F;
            if make == 0x2A || make == 0x36 {
                self.shift = false;
            }
            if make == 0x1D {
                self.ctrl = false;
            }
            if make == 0x38 {
                ALT_PRESSED.store(false, core::sync::atomic::Ordering::Relaxed);
            }
            return;
        }

        // Normal tuslar (Make code)
        match scancode {
            0x1D => self.ctrl = true,                    // Ctrl bas
            0x2A | 0x36 => self.shift = true,           // Shift bas
            0x38 => {
                ALT_PRESSED.store(true, core::sync::atomic::Ordering::Relaxed);
            }
            0x1C => self.push(Key::Enter),               // Enter
            0x0E => self.push(Key::Backspace),           // Backspace
            0x01 => self.push(Key::Escape),              // Escape
            0x0F => self.push(Key::Tab),                 // Tab
            0x39 => self.push(Key::Ascii(b' ')),         // Space
            0x53 => self.push(Key::Delete),              // Delete (keypad)
            0x2E => {                                    // 'c' or Ctrl+C
                if self.ctrl {
                    self.push(Key::CtrlC);
                } else if let Some(c) = scancode_to_ascii(scancode, self.shift) {
                    self.push(Key::Ascii(c));
                }
            }
            _ => {
                if let Some(c) = scancode_to_ascii(scancode, self.shift) {
                    self.push(Key::Ascii(c));
                }
            }
        }
    }
}

pub static ALT_PRESSED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub fn is_alt_pressed() -> bool {
    ALT_PRESSED.load(core::sync::atomic::Ordering::Relaxed)
}

pub static KEYBOARD: Mutex<Keyboard> = Mutex::new(Keyboard::new());

/// IRQ handler'dan çağrılacak
pub fn handle_key(scancode: u8) {
    KEYBOARD.lock().handle_scancode(scancode);
}

/// Shell'in okuması için
pub fn read_key() -> Option<Key> {
    KEYBOARD.lock().pop()
}

// US QWERTY scancode -> ASCII (Set 1 make codes)
const KEYMAP_NORMAL: [u8; 128] = [
    0,    0,   b'1', b'2', b'3', b'4', b'5', b'6',
    b'7', b'8', b'9', b'0', b'-', b'=', 0,    0,
    b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i',
    b'o', b'p', b'[', b']', 0,    0,   b'a', b's',
    b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';',
    b'\'',b'`', 0,   b'\\',b'z', b'x', b'c', b'v',
    b'b', b'n', b'm', b',', b'.', b'/', 0,   b'*',
    0,   b' ', 0,    0,    0,    0,    0,    0,
    0,    0,    0,    0,    0,    0,    0,    0,
    b'7', b'8', b'9', b'-', b'4', b'5', b'6', b'+',
    b'1', b'2', b'3', b'0', b'.', 0,    0,    0,
    0,    0,    0,    0,    0,    0,    0,    0,
    0,    0,    0,    0,    0,    0,    0,    0,
    0,    0,    0,    0,    0,    0,    0,    0,
    0,    0,    0,    0,    0,    0,    0,    0,
    0,    0,    0,    0,    0,    0,    0,    0,
];

const KEYMAP_SHIFT: [u8; 128] = [
    0,    0,   b'!', b'@', b'#', b'$', b'%', b'^',
    b'&', b'*', b'(', b')', b'_', b'+', 0,    0,
    b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I',
    b'O', b'P', b'{', b'}', 0,    0,   b'A', b'S',
    b'D', b'F', b'G', b'H', b'J', b'K', b'L', b':',
    b'"', b'~', 0,   b'|', b'Z', b'X', b'C', b'V',
    b'B', b'N', b'M', b'<', b'>', b'?', 0,   b'*',
    0,   b' ', 0,    0,    0,    0,    0,    0,
    0,    0,    0,    0,    0,    0,    0,    0,
    b'7', b'8', b'9', b'-', b'4', b'5', b'6', b'+',
    b'1', b'2', b'3', b'0', b'.', 0,    0,    0,
    0,    0,    0,    0,    0,    0,    0,    0,
    0,    0,    0,    0,    0,    0,    0,    0,
    0,    0,    0,    0,    0,    0,    0,    0,
    0,    0,    0,    0,    0,    0,    0,    0,
    0,    0,    0,    0,    0,    0,    0,    0,
];

pub fn scancode_to_ascii(scancode: u8, shift: bool) -> Option<u8> {
    let idx = scancode as usize;
    if idx >= 128 { return None; }
    let c = if shift { KEYMAP_SHIFT[idx] } else { KEYMAP_NORMAL[idx] };
    if c == 0 { None } else { Some(c) }
}
