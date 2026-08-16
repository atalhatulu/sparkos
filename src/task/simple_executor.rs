use super::Task;
use alloc::collections::VecDeque;
use core::task::{Waker, RawWaker, RawWakerVTable, Context, Poll};

pub struct SimpleExecutor {
    task_queue: VecDeque<Task>,
}

impl SimpleExecutor {
    pub fn new() -> SimpleExecutor {
        SimpleExecutor {
            task_queue: VecDeque::new(),
        }
    }

    pub fn spawn(&mut self, task: Task) {
        self.task_queue.push_back(task);
    }

    pub fn run(&mut self) {
        loop {
            if self.task_queue.is_empty() {
                x86_64::instructions::hlt();
                continue;
            }

            let queue_len = self.task_queue.len();
            for _ in 0..queue_len {
                if let Some(mut task) = self.task_queue.pop_front() {
                    let task_id = task.id.0;
                    let is_system_task = task.name == "clock"
                        || task.name == "mouse"
                        || task.name == "keyboard"
                        || task.name == "boot_orchestrator";

                    if !is_system_task {
                        let mut killed = super::KILLED_PROCESSES.lock();
                        if let Some(pos) = killed.iter().position(|&id| id == task_id) {
                            killed.remove(pos);
                            super::PROCESS_LIST.lock().remove(&task_id);
                            continue; // Görev iptal edildi (kill), çalıştırma
                        }
                    }

                    let waker = dummy_waker();
                    let mut context = Context::from_waker(&waker);
                    match task.poll(&mut context) {
                        Poll::Ready(()) => {
                            super::PROCESS_LIST.lock().remove(&task_id);
                        }
                        Poll::Pending => self.task_queue.push_back(task), 
                    }
                }
            }

            // Bir turu tamamladik. Eger hala bekleyen gorevler varsa, 
            // bir sonraki donanim kesmesine (interrupt) kadar islemciyi uyut.
            x86_64::instructions::hlt();
        }
    }
}

fn dummy_raw_waker() -> RawWaker {
    fn no_op(_: *const ()) {}
    fn clone(_: *const ()) -> RawWaker { dummy_raw_waker() }
    let vtable = &RawWakerVTable::new(clone, no_op, no_op, no_op);
    RawWaker::new(core::ptr::null(), vtable)
}

fn dummy_waker() -> Waker {
    unsafe { Waker::from_raw(dummy_raw_waker()) }
}
