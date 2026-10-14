//! SparkOS Keyboard Async Task & Safe Interrupt Queue
//!
//! Separates hardware IRQ scancode ingestion from Mutex-locking event dispatching,
//! ensuring deadlock-free keyboard processing across the Window Manager.

use spin::Once;
use crossbeam_queue::ArrayQueue;
use crate::task::process::{WaitQueue, SCHEDULER, BlockReason};

pub static SCANCODE_QUEUE: Once<ArrayQueue<u8>> = Once::new();
pub static KEYBOARD_WAIT_QUEUE: WaitQueue = WaitQueue::new();

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
    // Wake any thread waiting on keyboard input
    {
        let mut s = SCHEDULER.lock();
        KEYBOARD_WAIT_QUEUE.wake_one(&mut s);
    }
    // Feed low-level ring buffer for VGA shell
    crate::keyboard::handle_key(scancode);
    // IRQ notification (IRQ 1)
    crate::ipc::irq_event(1, scancode);
}

pub fn read_scancode() -> u8 {
    loop {
        if let Some(queue) = SCANCODE_QUEUE.get() {
            if let Some(scancode) = queue.pop() {
                return scancode;
            }
        }
        let mut s = SCHEDULER.lock();
        if let Some(queue) = SCANCODE_QUEUE.get() {
            if let Some(scancode) = queue.pop() {
                drop(s);
                return scancode;
            }
        }
        KEYBOARD_WAIT_QUEUE.prepare_wait(&mut s, BlockReason::Generic);
        drop(s);
        crate::task::process::schedule();
    }
}

/// Dedicated preemptive keyboard kernel thread (Deadlock-Free!)
pub extern "C" fn keyboard_thread() {
    let mut is_ctrl = false;
    let mut is_shift = false;
    let mut is_alt = false;
    let mut is_extended = false;

    loop {
        let scancode = read_scancode();
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

            // TODO(Faz5): keyboard dispatch to user-space input service
            #[cfg(feature = "legacy-desktop")]
            {
                let modifiers = (if is_ctrl || crate::keyboard::is_ctrl_pressed() { crate::input::MOD_CTRL } else { 0 })
                    | (if is_shift || crate::keyboard::is_shift_pressed() { crate::input::MOD_SHIFT } else { 0 })
                    | (if is_alt || crate::keyboard::is_alt_pressed() { crate::input::MOD_ALT } else { 0 });

                crate::input::dispatch_keyboard_event(key_code, pressed, modifiers);
            }
            #[cfg(not(feature = "legacy-desktop"))]
            {
                let _ = (is_ctrl, is_shift, is_alt, pressed, key_code);
            }
        }
        let _ = is_extended;
        is_extended = false;
    }
}
