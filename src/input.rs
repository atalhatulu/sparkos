//! SparkOS — Input & Event Subsystem (Faz 13)
//!
//! Provides Hardware Input Capture, Fixed 32-Byte Wire-Format `InputEvent` ABI,
//! Per-Client Ring Buffers, Mouse Local Coordinate Translation, Focus-Gated Key Routing,
//! Coalescing Queue Overflow Backpressure, and Automated Teardown.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use spin::Mutex;

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
    pub event_type: u8,       // 1 byte
    pub modifiers: u8,        // 1 byte (Bit 0: Shift, Bit 1: Ctrl, Bit 2: Alt)
    pub key_code: u8,         // 1 byte
    pub mouse_button: u8,     // 1 byte (1: Left, 2: Right, 3: Middle)
    pub wheel_delta: i8,      // 1 byte (+1: Up, -1: Down)
    pub _reserved: [u8; 3],   // 3 bytes padding
    pub mouse_x: i32,         // 4 bytes (Pencere-içi yerel X)
    pub mouse_y: i32,         // 4 bytes (Pencere-içi yerel Y)
    pub timestamp: u64,       // 8 bytes (Sistem zaman damgası)
    pub _padding: [u8; 8],    // 8 bytes padding -> Toplam 32 byte
}

// 32-byte wire-format ABI doğrulaması
const _: () = assert!(core::mem::size_of::<InputEvent>() == 32);

pub const EVENT_QUEUE_CAPACITY: usize = 32;

#[derive(Debug, Clone)]
pub struct EventQueue {
    pub buffer: Vec<InputEvent>,
}

impl EventQueue {
    pub const fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Olayı kuyruğa ekler; kuyruk doluysa MouseMove'u birleştirir (coalescing),
    /// tuş olaylarını öncelikli korur.
    pub fn push(&mut self, ev: InputEvent) {
        if self.buffer.len() >= EVENT_QUEUE_CAPACITY {
            // Eğer yeni gelen olay MouseMove ise ve kuyrukta önceki MouseMove varsa birleştir
            if ev.event_type == EventType::MouseMove as u8 {
                if let Some(last_move) = self.buffer.iter_mut().rev().find(|e| e.event_type == EventType::MouseMove as u8) {
                    last_move.mouse_x = ev.mouse_x;
                    last_move.mouse_y = ev.mouse_y;
                    last_move.timestamp = ev.timestamp;
                    return;
                }
            }
            // En eski MouseMove'u silmeyi dene
            if let Some(pos) = self.buffer.iter().position(|e| e.event_type == EventType::MouseMove as u8) {
                self.buffer.remove(pos);
            } else {
                // Eğer kuyruk tamamen tuş olaylarıyla doluysa, en eskiyi at (Tanımlı backpressure)
                self.buffer.remove(0);
            }
        }
        self.buffer.push(ev);
    }

    pub fn pop(&mut self) -> Option<InputEvent> {
        if self.buffer.is_empty() {
            None
        } else {
            Some(self.buffer.remove(0))
        }
    }
}

pub static INPUT_QUEUES: Mutex<BTreeMap<u64, EventQueue>> = Mutex::new(BTreeMap::new());

/// Bir sürece olay gönderir
pub fn deliver_event_to_pid(pid: u64, ev: InputEvent) {
    let mut queues = INPUT_QUEUES.lock();
    queues.entry(pid).or_insert_with(EventQueue::new).push(ev);
}

/// Süreç sonlandığında olay kuyruğunu sızıntısız temizler
pub fn cleanup_input_for_pid(pid: u64) {
    let mut queues = INPUT_QUEUES.lock();
    if queues.remove(&pid).is_some() {
        crate::serial_println!("[INPUT] Cleaned up event queue for terminating PID {}", pid);
    }
}

/// Fare olayını işler: Hit-test yapar, global koordinatları yerel pencere koordinatına
/// dönüştürür ve yalnızca hedef pencerenin sahibine iletir.
pub fn dispatch_mouse_event(global_x: i32, global_y: i32, button: u8, pressed: bool) -> Option<u64> {
    let mut wm = crate::wm::WM.lock();
    if let Some(win_id) = wm.hit_test(global_x, global_y) {
        if pressed {
            let _ = wm.raise_to_top_internal(win_id);
        }
        if let Some(win) = wm.windows.iter().find(|w| w.window_id == win_id) {
            let owner_pid = win.owner_pid;
            let local_x = global_x - win.x;
            let local_y = global_y - win.y;

            let ev_type = if button > 0 {
                if pressed { EventType::MouseDown } else { EventType::MouseUp }
            } else {
                EventType::MouseMove
            };

            let ev = InputEvent {
                event_type: ev_type as u8,
                modifiers: 0,
                key_code: 0,
                mouse_button: button,
                wheel_delta: 0,
                _reserved: [0; 3],
                mouse_x: local_x,
                mouse_y: local_y,
                timestamp: 1000,
                _padding: [0; 8],
            };

            deliver_event_to_pid(owner_pid, ev);
            return Some(owner_pid);
        }
    }
    None
}

/// Klavye olayını işler: Kesinlikle YALNIZCA o an odaklı pencerenin sahibine iletir (Focus-Gated Routing).
pub fn dispatch_keyboard_event(key_code: u8, pressed: bool, modifiers: u8) -> Option<u64> {
    let wm = crate::wm::WM.lock();
    let focused_id = wm.focused_window?;
    let win = wm.windows.iter().find(|w| w.window_id == focused_id)?;
    let owner_pid = win.owner_pid;

    let ev_type = if pressed { EventType::KeyDown } else { EventType::KeyUp };
    let ev = InputEvent {
        event_type: ev_type as u8,
        modifiers,
        key_code,
        mouse_button: 0,
        wheel_delta: 0,
        _reserved: [0; 3],
        mouse_x: 0,
        mouse_y: 0,
        timestamp: 1000,
        _padding: [0; 8],
    };

    deliver_event_to_pid(owner_pid, ev);
    Some(owner_pid)
}
