//! src/klog.rs — Seviyeli kernel loglama sistemi.
//!
//! Hedefler: QEMU seri port (`serial.rs`) + VGA (`vga_buffer.rs`).
//! Cikti formati: `0 [LEVEL] msg` (TIME su an sabit 0 tick).
//! Kullanilana kadar main.rs tarafindan gosterilmeyen infra modulu —
//! bu yuzden dead_code uyarilari bastirildi.

#![allow(dead_code)]

use core::fmt;
use core::fmt::Write;
use spin::Mutex;

/// Log seviyeleri. Oncelik: Error > Warn > Info > Debug > Trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LogLevel {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
    Trace = 4,
}

impl LogLevel {
    pub fn tag(self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
            LogLevel::Trace => "TRACE",
        }
    }
}

/// Log satirinin sinirli buyuklugu (alloc yok, tasmada budanir).
const LINE_CAP: usize = 256;

/// Kucuk, alloc-free format buffer'i.
struct LineWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> LineWriter<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        LineWriter { buf, pos: 0 }
    }
}

impl fmt::Write for LineWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let remaining = self.buf.len().saturating_sub(self.pos);
        let n = s.len().min(remaining);
        self.buf[self.pos..self.pos + n].copy_from_slice(&s.as_bytes()[..n]);
        self.pos += n;
        if n < s.len() {
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

/// Kernel logger. Seviye filtrelemesi yapar, ciktiyi serial + VGA'ya yazar.
pub struct KLogger {
    level: Mutex<LogLevel>,
}

pub static LOGGER: KLogger = KLogger {
    level: Mutex::new(LogLevel::Info),
};

impl KLogger {
    pub fn set_level(&self, lvl: LogLevel) {
        *self.level.lock() = lvl;
    }

    pub fn level(&self) -> LogLevel {
        *self.level.lock()
    }

    /// `lvl` mevcut esigin altindaysa mesaji serial + VGA'ya basar.
    pub fn log(&self, lvl: LogLevel, args: fmt::Arguments) {
        if (lvl as u8) > (*self.level.lock() as u8) {
            return;
        }

        let mut line = [0u8; LINE_CAP];
        let pos;
        {
            let mut w = LineWriter::new(&mut line);
            let _ = core::fmt::write(&mut w, format_args!("0 [{}] ", lvl.tag()));
            let _ = core::fmt::write(&mut w, args);
            pos = w.pos;
        }
        let s = core::str::from_utf8(&line[..pos]).unwrap_or("<invalid utf8>");

        // Hedef 1: QEMU seri port
        crate::serial::SerialWriter::force_write(s);
        crate::serial::SerialWriter::force_write("\n");

        // Hedef 2: VGA (GUI modundaysa gui::WRITERS uzerinden cizilir)
        let mut vga = crate::vga_buffer::WRITE_LOCK.lock();
        let _ = writeln!(vga, "{}", s);
    }
}

#[macro_export]
macro_rules! klog_error {
    ($($arg:tt)*) => {
        $crate::klog::LOGGER.log($crate::klog::LogLevel::Error, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! klog_warn {
    ($($arg:tt)*) => {
        $crate::klog::LOGGER.log($crate::klog::LogLevel::Warn, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! klog_info {
    ($($arg:tt)*) => {
        $crate::klog::LOGGER.log($crate::klog::LogLevel::Info, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! klog_debug {
    ($($arg:tt)*) => {
        $crate::klog::LOGGER.log($crate::klog::LogLevel::Debug, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! klog_trace {
    ($($arg:tt)*) => {
        $crate::klog::LOGGER.log($crate::klog::LogLevel::Trace, format_args!($($arg)*))
    };
}
