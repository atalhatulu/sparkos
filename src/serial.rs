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

use spin::Once;
use crossbeam_queue::ArrayQueue;

pub static SERIAL_RX_QUEUE: Once<ArrayQueue<u8>> = Once::new();

pub fn init_rx() {
    SERIAL_RX_QUEUE.call_once(|| ArrayQueue::new(256));
}

pub fn push_rx_byte(byte: u8) {
    if let Some(q) = SERIAL_RX_QUEUE.get() {
        let _ = q.push(byte);
    }
}

pub fn try_read_byte() -> Option<u8> {
    if let Some(q) = SERIAL_RX_QUEUE.get() {
        if let Some(b) = q.pop() {
            return Some(b);
        }
    }
    use x86_64::instructions::port::PortReadOnly;
    unsafe {
        let mut lsr: PortReadOnly<u8> = PortReadOnly::new(0x3FDu16);
        if lsr.read() & 1 != 0 {
            let mut rbr: PortReadOnly<u8> = PortReadOnly::new(0x3F8u16);
            Some(rbr.read())
        } else {
            None
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
