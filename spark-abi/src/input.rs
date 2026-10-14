//! SparkOS Input Service IPC Protocol (Faz 5.2)
//!
//! Provides ABI structures for Ring-3 Input Server, Scancode/Mouse Packet Delivery,
//! and Event Distribution to Compositor and Focused Applications.

pub const INPUT_OP_REGISTER_CLIENT: u32 = 1;
pub const INPUT_OP_POLL_EVENT: u32 = 2;
pub const INPUT_OP_SEND_EVENT: u32 = 3;
pub const INPUT_OP_SET_FOCUS: u32 = 4;

pub const EVENT_KEY_DOWN: u32 = 1;
pub const EVENT_KEY_UP: u32 = 2;
pub const EVENT_MOUSE_MOVE: u32 = 3;
pub const EVENT_MOUSE_DOWN: u32 = 4;
pub const EVENT_MOUSE_UP: u32 = 5;
pub const EVENT_MOUSE_SCROLL: u32 = 6;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputEvent {
    pub kind: u32,
    pub keycode: u32,
    pub modifiers: u32,
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub buttons: u32,
    pub timestamp_ms: u64,
}

impl InputEvent {
    pub const fn empty() -> Self {
        Self {
            kind: 0,
            keycode: 0,
            modifiers: 0,
            mouse_x: 0,
            mouse_y: 0,
            buttons: 0,
            timestamp_ms: 0,
        }
    }

    pub fn key_down(keycode: u32, modifiers: u32, timestamp_ms: u64) -> Self {
        Self {
            kind: EVENT_KEY_DOWN,
            keycode,
            modifiers,
            mouse_x: 0,
            mouse_y: 0,
            buttons: 0,
            timestamp_ms,
        }
    }

    pub fn mouse_move(x: i32, y: i32, buttons: u32, timestamp_ms: u64) -> Self {
        Self {
            kind: EVENT_MOUSE_MOVE,
            keycode: 0,
            modifiers: 0,
            mouse_x: x,
            mouse_y: y,
            buttons,
            timestamp_ms,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputRequest {
    pub opcode: u32,
    pub client_pid: u64,
    pub target_window: u32,
    pub event: InputEvent,
}

impl InputRequest {
    pub const fn empty() -> Self {
        Self {
            opcode: 0,
            client_pid: 0,
            target_window: 0,
            event: InputEvent::empty(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputResponse {
    pub status: u32,
    pub pending_count: u32,
    pub event: InputEvent,
}

impl InputResponse {
    pub const fn ok(event: InputEvent, pending: u32) -> Self {
        Self {
            status: 0,
            pending_count: pending,
            event,
        }
    }

    pub const fn empty_queue() -> Self {
        Self {
            status: 0,
            pending_count: 0,
            event: InputEvent::empty(),
        }
    }

    pub const fn err(errno_val: u32) -> Self {
        Self {
            status: errno_val,
            pending_count: 0,
            event: InputEvent::empty(),
        }
    }
}
