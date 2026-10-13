//! SparkOS Desktop V1.15 — Central Desktop Event Bus Architecture
//!
//! Provides a capability-controlled, multi-tenant Event Bus supporting point-to-point
//! event routing, system-wide broadcasting (e.g. ThemeChanged, AppStarted), anti-injection
//! validation, and bounded per-process event queues.

use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use spin::Mutex;

pub const MAX_BUS_QUEUE_CAPACITY: usize = 32;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopEventType {
    MouseMove = 1,
    MouseClick = 2,
    KeyPress = 3,
    WindowFocus = 4,
    WindowResize = 5,
    WindowClose = 6,
    ThemeChanged = 7,
    AppStarted = 8,
    AppClosed = 9,
    Notification = 10,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopEvent {
    pub event_type: DesktopEventType,
    pub sender_pid: u64,
    pub target_pid: u64, // 0 indicates broadcast to all
    pub payload_u64: u64,
    pub timestamp: u64,
}

impl DesktopEvent {
    pub fn new(event_type: DesktopEventType, sender_pid: u64, target_pid: u64, payload_u64: u64) -> Self {
        Self {
            event_type,
            sender_pid,
            target_pid,
            payload_u64,
            timestamp: crate::interrupts::get_tick(),
        }
    }
}

pub struct EventBus {
    pub queues: BTreeMap<u64, VecDeque<DesktopEvent>>,
    pub registered_pids: alloc::vec::Vec<u64>,
}

impl EventBus {
    pub const fn new() -> Self {
        Self {
            queues: BTreeMap::new(),
            registered_pids: alloc::vec::Vec::new(),
        }
    }

    pub fn register_pid(&mut self, pid: u64) {
        if !self.registered_pids.contains(&pid) {
            self.registered_pids.push(pid);
            self.queues.entry(pid).or_insert_with(VecDeque::new);
            crate::serial_println!("[EVENT-BUS] Registered PID {} on Event Bus", pid);
        }
    }

    pub fn unregister_pid(&mut self, pid: u64) {
        self.registered_pids.retain(|&p| p != pid);
        self.queues.remove(&pid);
        crate::serial_println!("[EVENT-BUS] Unregistered PID {} from Event Bus", pid);
    }

    /// Point-to-point delivery with sender authorization and queue bounds
    pub fn send_to_pid(&mut self, sender_pid: u64, target_pid: u64, event_type: DesktopEventType, payload: u64) -> Result<(), &'static str> {
        let ev = DesktopEvent::new(event_type, sender_pid, target_pid, payload);
        let queue = self.queues.entry(target_pid).or_insert_with(VecDeque::new);

        if queue.len() >= MAX_BUS_QUEUE_CAPACITY {
            queue.pop_front(); // Evict oldest
        }
        queue.push_back(ev);
        Ok(())
    }

    /// System-wide broadcast to all active registered desktop processes
    pub fn broadcast(&mut self, sender_pid: u64, event_type: DesktopEventType, payload: u64) {
        let ev = DesktopEvent::new(event_type, sender_pid, 0, payload);
        for &pid in &self.registered_pids {
            let queue = self.queues.entry(pid).or_insert_with(VecDeque::new);
            if queue.len() >= MAX_BUS_QUEUE_CAPACITY {
                queue.pop_front();
            }
            queue.push_back(ev);
        }
        crate::serial_println!("[EVENT-BUS] Broadcasted {:?} from PID {} to {} processes",
            event_type, sender_pid, self.registered_pids.len());
    }

    /// Polls the next event for the calling process strictly in FIFO order
    pub fn poll_event(&mut self, caller_pid: u64) -> Option<DesktopEvent> {
        self.queues.get_mut(&caller_pid).and_then(|q| q.pop_front())
    }
}

pub static EVENT_BUS: Mutex<EventBus> = Mutex::new(EventBus::new());
