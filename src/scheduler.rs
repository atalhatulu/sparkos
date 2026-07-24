/// SparkOS Cooperative Scheduler (pre-heap)
/// 
/// Heap olmadan çalışan basit fonksiyon tabanlı scheduler.
/// Task'lar fonksiyon pointer'ı olarak saklanır.
/// İlerde async/await ve Box heap allocator eklenecek.

type TaskFn = fn() -> TaskState;

#[derive(Clone, Copy)]
pub enum TaskState {
    Running,
    Done,
}

pub struct Scheduler {
    tasks: [Option<(TaskFn, u64)>; 16],
    count: usize,
    current: usize,
}

impl Scheduler {
    pub const fn new() -> Self {
        Scheduler {
            tasks: [None; 16],
            count: 0,
            current: 0,
        }
    }
    
    pub fn spawn(&mut self, func: TaskFn, id: u64) -> bool {
        if self.count >= 16 {
            return false;
        }
        for slot in &mut self.tasks {
            if slot.is_none() {
                *slot = Some((func, id));
                self.count += 1;
                return true;
            }
        }
        false
    }
    
    pub fn run(&mut self) -> ! {
        loop {
            if self.count == 0 {
                x86_64::instructions::hlt();
                continue;
            }
            
            for _ in 0..16 {
                self.current = (self.current + 1) % 16;
                if let Some((func, _id)) = &self.tasks[self.current] {
                    match func() {
                        TaskState::Done => {
                            self.tasks[self.current] = None;
                            self.count -= 1;
                        }
                        TaskState::Running => {}
                    }
                    break;
                }
            }
        }
    }
}
