use core::{future::Future, pin::Pin};
use core::task::{Context, Poll};
use alloc::boxed::Box;
use core::sync::atomic::{AtomicU64, Ordering};

use alloc::string::{String, ToString};
use alloc::collections::BTreeMap;
use spin::Mutex;

pub mod simple_executor;
pub mod keyboard;
pub mod yield_now;
pub mod process;

pub use yield_now::yield_now;

pub static PROCESS_LIST: spin::Lazy<Mutex<BTreeMap<u64, String>>> = spin::Lazy::new(|| Mutex::new(BTreeMap::new()));
pub static KILLED_PROCESSES: spin::Lazy<Mutex<alloc::vec::Vec<u64>>> = spin::Lazy::new(|| Mutex::new(alloc::vec::Vec::new()));

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskId(pub u64);

impl TaskId {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        TaskId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

pub struct Task {
    pub id: TaskId,
    pub name: String,
    future: Pin<Box<dyn Future<Output = ()>>>,
}

impl Task {
    pub fn new(name: &str, future: impl Future<Output = ()> + 'static) -> Task {
        let task_id = TaskId::new();
        PROCESS_LIST.lock().insert(task_id.0, name.to_string());
        Task {
            id: task_id,
            name: name.to_string(),
            future: Box::pin(future),
        }
    }
    
    fn poll(&mut self, context: &mut Context) -> Poll<()> {
        self.future.as_mut().poll(context)
    }
}
