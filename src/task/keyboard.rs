use spin::Once;
use crossbeam_queue::ArrayQueue;
use core::{future::Future, pin::Pin, task::{Context, Poll}};

pub static SCANCODE_QUEUE: Once<ArrayQueue<u8>> = Once::new();

pub fn init() {
    SCANCODE_QUEUE.call_once(|| ArrayQueue::new(100));
}

/// Called by the keyboard interrupt handler
pub(crate) fn add_scancode(scancode: u8) {
    if let Some(queue) = SCANCODE_QUEUE.get() {
        if queue.push(scancode).is_err() {
            crate::serial_println!("WARNING: scancode queue full; dropping keyboard input");
        }
    } else {
        crate::serial_println!("WARNING: scancode queue uninitialized");
    }
    // Decode scancode into keyboard ring buffer for shell
    crate::keyboard::handle_key(scancode);
    // Aşama 5.1: IRQ notification (IRQ 1)
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
