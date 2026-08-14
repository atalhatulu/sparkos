//! src/ktrace.rs — Trace ring buffer + backtrace iskeleti.
//!
//! Sabit boyutlu 512 event'lik halka buffer. `trace!` makrosu event ekler,
//! `print_trace` son N event'i seri porta basar, `walk_stack` RBP zinciriyle
//! return adreslerini toplar. Kullanilana kadar main.rs tarafindan
//! gosterilmeyen infra modulu — bu yuzden dead_code uyarilari bastirildi.

#![allow(dead_code)]

use crate::klog::LogLevel;
use core::arch::asm;
use core::fmt;
use spin::Mutex;

pub const RING_CAP: usize = 512;
pub const DATA_CAP: usize = 128;
pub const MAX_FRAMES: usize = 32;

/// Tek trace event'i. `data` sabit boyutlu inline buffer (alloc yok).
#[derive(Debug, Clone, Copy)]
pub struct TraceEvent {
    pub id: u64,
    pub level: LogLevel,
    pub tick: u64,
    pub len: usize,
    pub data: [u8; DATA_CAP],
}

impl TraceEvent {
    pub const fn empty() -> Self {
        TraceEvent {
            id: 0,
            level: LogLevel::Trace,
            tick: 0,
            len: 0,
            data: [0u8; DATA_CAP],
        }
    }

    pub fn text(&self) -> &str {
        core::str::from_utf8(&self.data[..self.len]).unwrap_or("<invalid utf8>")
    }
}

/// Sabit boyutlu halka buffer.
pub struct RingBuffer {
    pub events: [TraceEvent; RING_CAP],
    pub head: usize,
    pub count: usize,
    pub next_id: u64,
}

impl RingBuffer {
    pub const fn new() -> Self {
        RingBuffer {
            events: [TraceEvent::empty(); RING_CAP],
            head: 0,
            count: 0,
            next_id: 0,
        }
    }

    /// Event'i halkaya ekler; doluysa en eskiyi ezer.
    pub fn push(&mut self, mut ev: TraceEvent) {
        ev.id = self.next_id;
        self.next_id += 1;

        let idx = (self.head + self.count) % RING_CAP;
        self.events[idx] = ev;
        if self.count < RING_CAP {
            self.count += 1;
        } else {
            self.head = (self.head + 1) % RING_CAP;
        }
    }
}

pub static TRACE_RING: Mutex<RingBuffer> = Mutex::new(RingBuffer::new());

/// Kucuk, alloc-free format buffer'i.
struct BufWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> BufWriter<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        BufWriter { buf, pos: 0 }
    }
}

impl fmt::Write for BufWriter<'_> {
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

/// Ring'e yeni bir trace event'i ekler.
pub fn log_trace(level: LogLevel, args: fmt::Arguments) {
    let mut ev = TraceEvent::empty();
    ev.level = level;
    ev.tick = crate::interrupts::get_tick();
    {
        let mut w = BufWriter::new(&mut ev.data);
        let _ = core::fmt::write(&mut w, args);
        ev.len = w.pos;
    }
    TRACE_RING.lock().push(ev);
}

/// Son `last` trace event'ini seri porta basar.
pub fn print_trace(last: usize) {
    let ring = TRACE_RING.lock();
    let n = ring.count.min(last);
    if n == 0 {
        crate::serial_println!("[ktrace] (bos)");
        return;
    }
    let start = if n >= ring.count {
        ring.head
    } else {
        (ring.head + ring.count - n) % RING_CAP
    };
    for i in 0..n {
        let idx = (start + i) % RING_CAP;
        let ev = &ring.events[idx];
        crate::serial_println!("#{} [{}] t={} {}", ev.id, ev.level.tag(), ev.tick, ev.text());
    }
}

/// RBP zincirini takip ederek call stack'teki return adreslerini toplar.
/// Simdilik sadece adresler yeterli; sembol isimlendirme sonraki katman.
pub fn walk_stack() -> [u64; MAX_FRAMES] {
    let mut frames = [0u64; MAX_FRAMES];
    let mut rbp_val: u64;
    unsafe {
        asm!("mov {}, rbp", out(reg) rbp_val);
    }

    let mut cur = rbp_val;
    let mut i = 0;
    while cur != 0 && i < MAX_FRAMES {
        // [cur]   = caller'in RBP'si
        // [cur+8] = return adresi
        let prev = unsafe { core::ptr::read_volatile(cur as *const u64) };
        let ret = unsafe { core::ptr::read_volatile((cur + 8) as *const u64) };
        if ret == 0 {
            break;
        }
        frames[i] = ret;
        i += 1;
        // Stack asagi dogru buyur; caller RBP'si daha yuksek adreste olmali.
        // Bozuk/corrupt zinciri takip etmeyi onle.
        if prev == 0 || prev <= cur {
            break;
        }
        cur = prev;
    }
    frames
}

#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {
        $crate::ktrace::log_trace($crate::klog::LogLevel::Trace, format_args!($($arg)*))
    };
}
