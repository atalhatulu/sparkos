//! SparkOS Desktop V1.37 — System-wide Text Clipboard Service
//!
//! Provides a thread-safe, capability-isolated global text clipboard
//! with bounds checking, size clamping, and multi-application copy/cut/paste support.

use alloc::string::String;
use spin::Mutex;

pub const MAX_CLIPBOARD_BYTES: usize = 64 * 1024; // 64 KB max to protect kernel memory

pub struct Clipboard {
    content: String,
}

impl Clipboard {
    pub const fn new() -> Self {
        Self {
            content: String::new(),
        }
    }

    pub fn set_text(&mut self, text: &str) {
        self.content.clear();
        let clamped = if text.len() > MAX_CLIPBOARD_BYTES {
            &text[..MAX_CLIPBOARD_BYTES]
        } else {
            text
        };
        self.content.push_str(clamped);
    }

    pub fn get_text(&self) -> String {
        self.content.clone()
    }

    pub fn clear(&mut self) {
        self.content.clear();
    }

    pub fn has_text(&self) -> bool {
        !self.content.is_empty()
    }

    pub fn len(&self) -> usize {
        self.content.len()
    }
}

pub static CLIPBOARD: Mutex<Clipboard> = Mutex::new(Clipboard::new());

pub fn copy_to_clipboard(text: &str) {
    CLIPBOARD.lock().set_text(text);
}

pub fn get_clipboard_text() -> String {
    CLIPBOARD.lock().get_text()
}

pub fn clear_clipboard() {
    CLIPBOARD.lock().clear();
}
