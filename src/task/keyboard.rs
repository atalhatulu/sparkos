//! SparkOS Keyboard Async Task & Safe Interrupt Queue
//!
//! Separates hardware IRQ scancode ingestion from Mutex-locking event dispatching,
//! ensuring deadlock-free keyboard processing across the Window Manager.

use spin::Once;
use crossbeam_queue::ArrayQueue;
use core::{future::Future, pin::Pin, task::{Context, Poll}};

pub static SCANCODE_QUEUE: Once<ArrayQueue<u8>> = Once::new();

pub fn init() {
    SCANCODE_QUEUE.call_once(|| ArrayQueue::new(256));
}

/// Called by the keyboard interrupt handler (IRQ1) - STRICTLY LOCK-FREE
pub(crate) fn add_scancode(scancode: u8) {
    if let Some(queue) = SCANCODE_QUEUE.get() {
        if queue.push(scancode).is_err() {
            crate::serial_println!("WARNING: scancode queue full; dropping keyboard input");
        }
    } else {
        crate::serial_println!("WARNING: scancode queue uninitialized");
    }
    // Feed low-level ring buffer for VGA shell
    crate::keyboard::handle_key(scancode);
    // IRQ notification (IRQ 1)
    crate::ipc::irq_event(1, scancode);
}

pub struct ScancodeFuture;

impl Future for ScancodeFuture {
    type Output = u8;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<u8> {
        if let Some(queue) = SCANCODE_QUEUE.get() {
            if let Some(scancode) = queue.pop() {
                return Poll::Ready(scancode);
            }
        }
        Poll::Pending
    }
}

pub async fn read_scancode() -> u8 {
    ScancodeFuture.await
}

/// Dedicated asynchronous keyboard task running in cooperative executor context (Deadlock-Free!)
pub async fn keyboard_task() {
    let mut is_ctrl = false;
    let mut is_shift = false;
    let mut is_alt = false;
    let mut is_extended = false;

    loop {
        let scancode = read_scancode().await;
        if scancode == 0xE0 {
            is_extended = true;
            continue;
        }

        if crate::vga_buffer::GUI_MODE.load(core::sync::atomic::Ordering::Relaxed) {
            let pressed = (scancode & 0x80) == 0;
            let key_code = scancode & 0x7F;

            match key_code {
                0x1D => is_ctrl = pressed,
                0x2A | 0x36 => is_shift = pressed,
                0x38 => is_alt = pressed,
                _ => {}
            }

            let modifiers = (if is_ctrl || crate::keyboard::is_ctrl_pressed() { 1 } else { 0 })
                | (if is_shift || crate::keyboard::is_shift_pressed() { 2 } else { 0 })
                | (if is_alt || crate::keyboard::is_alt_pressed() { 8 } else { 0 });

            crate::input::dispatch_keyboard_event(key_code, pressed, modifiers);
        }
        let _ = is_extended;
        is_extended = false;
    }
}
