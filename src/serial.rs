use spin::Mutex;
use uart_16550::SerialPort;

pub static SERIAL: Mutex<Option<SerialPort>> = Mutex::new(None);

pub struct SerialWriter;

impl SerialWriter {
    pub fn init() {
        let mut port = unsafe { SerialPort::new(0x3F8) };
        port.init();
        *SERIAL.lock() = Some(port);
    }

    pub fn force_write(s: &str) {
        if let Some(ref mut port) = *SERIAL.lock() {
            for byte in s.bytes() {
                port.send(byte);
            }
        }
    }
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        if let Some(ref mut port) = *$crate::serial::SERIAL.lock() {
            let _ = core::fmt::Write::write_fmt(port, format_args!($($arg)*));
        }
    };
}

#[macro_export]
macro_rules! serial_println {
    () => {
        if let Some(ref mut port) = *$crate::serial::SERIAL.lock() {
            let _ = core::fmt::Write::write_str(port, "\n");
        }
    };
    ($($arg:tt)*) => {
        if let Some(ref mut port) = *$crate::serial::SERIAL.lock() {
            let _ = core::fmt::Write::write_fmt(port, format_args!($($arg)*));
            let _ = core::fmt::Write::write_str(port, "\n");
        }
    };
}
