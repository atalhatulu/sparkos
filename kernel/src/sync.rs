use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::hint::spin_loop;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use alloc::collections::VecDeque;
use spin::Mutex as SpinMutex;

/// 1. Spinlock — Raw CPU spinlock using atomic swap
pub struct Spinlock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for Spinlock<T> {}
unsafe impl<T: Send> Send for Spinlock<T> {}

impl<T> Spinlock<T> {
    pub const fn new(data: T) -> Self {
        Spinlock {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> SpinlockGuard<'_, T> {
        while self.locked.swap(true, Ordering::Acquire) {
            while self.locked.load(Ordering::Relaxed) {
                spin_loop();
            }
        }
        SpinlockGuard { lock: self }
    }
}

pub struct SpinlockGuard<'a, T> {
    lock: &'a Spinlock<T>,
}

impl<T> Deref for SpinlockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SpinlockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SpinlockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

/// IRQ-Safe Spinlock that disables interrupts while the lock is held.
pub struct IrqSafeSpinlock<T> {
    lock: Spinlock<T>,
}

unsafe impl<T: Send> Sync for IrqSafeSpinlock<T> {}
unsafe impl<T: Send> Send for IrqSafeSpinlock<T> {}

impl<T> IrqSafeSpinlock<T> {
    pub const fn new(data: T) -> Self {
        IrqSafeSpinlock {
            lock: Spinlock::new(data),
        }
    }
    
    pub fn lock(&self) -> IrqSafeSpinlockGuard<'_, T> {
        let interrupts_enabled = x86_64::instructions::interrupts::are_enabled();
        x86_64::instructions::interrupts::disable();
        let guard = self.lock.lock();
        IrqSafeSpinlockGuard {
            guard: core::mem::ManuallyDrop::new(guard),
            interrupts_were_enabled: interrupts_enabled,
        }
    }
}

pub struct IrqSafeSpinlockGuard<'a, T> {
    guard: core::mem::ManuallyDrop<SpinlockGuard<'a, T>>,
    interrupts_were_enabled: bool,
}

impl<T> Deref for IrqSafeSpinlockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.guard.deref()
    }
}
impl<T> DerefMut for IrqSafeSpinlockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.guard.deref_mut()
    }
}
impl<T> Drop for IrqSafeSpinlockGuard<'_, T> {
    fn drop(&mut self) {
        unsafe {
            core::mem::ManuallyDrop::drop(&mut self.guard);
        }
        if self.interrupts_were_enabled {
            x86_64::instructions::interrupts::enable();
        }
    }
}

// ---------------------------------------------------------------------------
// Faz 0.4: IrqMutex<T> — Kesme güvenli spin kilidi
// ---------------------------------------------------------------------------
pub use crate::irq_mutex::*;

/// 2. Mutex — Built on top of spin::Mutex, provides a non-spinning lock that yields to the scheduler
pub struct Mutex<T> {
    inner: SpinMutex<T>,
}

pub type MutexGuard<'a, T> = spin::MutexGuard<'a, T>;

impl<T> Mutex<T> {
    pub const fn new(data: T) -> Self {
        Mutex { inner: SpinMutex::new(data) }
    }

    pub fn lock(&self) -> MutexGuard<'_, T> {
        let mut spins = 0;
        loop {
            if let Some(guard) = self.inner.try_lock() {
                return guard;
            }
            spins += 1;
            if spins < 32 {
                core::hint::spin_loop();
            } else {
                if crate::sync::irq_locks_held() == 0 && x86_64::instructions::interrupts::are_enabled() {
                    crate::task::process::schedule();
                } else {
                    x86_64::instructions::hlt();
                }
            }
        }
    }
    
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        self.inner.try_lock()
    }
}

/// 3. Semaphore — Atomic counter with a WaitQueue
pub struct Semaphore {
    count: AtomicUsize,
    queue: crate::task::process::WaitQueue,
}

impl Semaphore {
    pub const fn new(initial: usize) -> Self {
        Semaphore {
            count: AtomicUsize::new(initial),
            queue: crate::task::process::WaitQueue::new(),
        }
    }

    pub fn try_wait(&self) -> bool {
        let mut current = self.count.load(Ordering::Relaxed);
        loop {
            if current == 0 {
                return false;
            }
            match self.count.compare_exchange_weak(current, current - 1, Ordering::Acquire, Ordering::Relaxed) {
                Ok(_) => return true,
                Err(val) => current = val,
            }
        }
    }

    pub fn wait(&self) {
        loop {
            if self.try_wait() {
                return;
            }
            {
                let mut s = crate::task::process::SCHEDULER.lock();
                self.queue.prepare_wait(&mut s, crate::task::process::BlockReason::Generic);
            }
            if self.try_wait() {
                let mut s = crate::task::process::SCHEDULER.lock();
                self.queue.finish_wait(&mut s);
                return;
            }
            crate::task::process::schedule();
            {
                let mut s = crate::task::process::SCHEDULER.lock();
                self.queue.finish_wait(&mut s);
            }
        }
    }

    pub fn signal(&self) {
        self.count.fetch_add(1, Ordering::Release);
        let mut s = crate::task::process::SCHEDULER.lock();
        self.queue.wake_one(&mut s);
    }
}

/// 4. Condvar — WaitQueue-backed Condition Variable
pub struct Condvar {
    queue: crate::task::process::WaitQueue,
}

impl Condvar {
    pub const fn new() -> Self {
        Condvar {
            queue: crate::task::process::WaitQueue::new(),
        }
    }

    pub fn wait<'a, T>(&self, guard: IrqGuard<'a, T>, mutex: &'a IrqMutex<T>) -> IrqGuard<'a, T> {
        {
            let mut s = crate::task::process::SCHEDULER.lock();
            self.queue.prepare_wait(&mut s, crate::task::process::BlockReason::Condvar);
        }
        drop(guard);
        crate::task::process::schedule();
        {
            let mut s = crate::task::process::SCHEDULER.lock();
            self.queue.finish_wait(&mut s);
        }
        mutex.lock()
    }

    pub fn wait_irq<'a, T>(&self, guard: IrqGuard<'a, T>, mutex: &'a IrqMutex<T>) -> IrqGuard<'a, T> {
        self.wait(guard, mutex)
    }

    pub fn notify_one(&self) {
        let mut s = crate::task::process::SCHEDULER.lock();
        self.queue.wake_one(&mut s);
    }

    pub fn notify_all(&self) {
        let mut s = crate::task::process::SCHEDULER.lock();
        self.queue.wake_all(&mut s);
    }
}

/// 5. BlockingChannel — Blocking channel for IPC (IrqMutex-protected, WaitQueue-backed)
pub struct BlockingChannel<M> {
    buffer: IrqMutex<VecDeque<M>>,
    not_empty: crate::task::process::WaitQueue,
    not_full: crate::task::process::WaitQueue,
    capacity: usize,
}

impl<M> BlockingChannel<M> {
    pub const fn new(capacity: usize) -> Self {
        BlockingChannel {
            buffer: IrqMutex::new(VecDeque::new()),
            not_empty: crate::task::process::WaitQueue::new(),
            not_full: crate::task::process::WaitQueue::new(),
            capacity,
        }
    }

    pub fn wait_queue_not_empty(&self) -> &crate::task::process::WaitQueue {
        &self.not_empty
    }

    pub fn wait_queue_not_full(&self) -> &crate::task::process::WaitQueue {
        &self.not_full
    }

    pub fn send(&self, msg: M) -> Option<u64> {
        let mut pending = Some(msg);
        loop {
            let mut guard = self.buffer.lock();
            if guard.len() < self.capacity {
                guard.push_back(pending.take().unwrap());
                drop(guard);
                let mut s = crate::task::process::SCHEDULER.lock();
                let woken = self.not_empty.wake_one(&mut s);
                return woken;
            }
            {
                let mut s = crate::task::process::SCHEDULER.lock();
                self.not_full.prepare_wait(&mut s, crate::task::process::BlockReason::ChannelSend);
            }
            drop(guard);
            crate::task::process::schedule();
            {
                let mut s = crate::task::process::SCHEDULER.lock();
                self.not_full.finish_wait(&mut s);
            }
        }
    }
    
    pub fn try_send(&self, msg: M) -> Result<(), M> {
        let mut guard = self.buffer.lock();
        if guard.len() >= self.capacity {
            return Err(msg);
        }
        guard.push_back(msg);
        drop(guard);
        let mut s = crate::task::process::SCHEDULER.lock();
        self.not_empty.wake_one(&mut s);
        Ok(())
    }

    pub fn recv(&self) -> M {
        loop {
            let mut guard = self.buffer.lock();
            if let Some(msg) = guard.pop_front() {
                drop(guard);
                let mut s = crate::task::process::SCHEDULER.lock();
                self.not_full.wake_one(&mut s);
                return msg;
            }
            {
                let mut s = crate::task::process::SCHEDULER.lock();
                self.not_empty.prepare_wait(&mut s, crate::task::process::BlockReason::ChannelRecv);
            }
            drop(guard);
            crate::task::process::schedule();
            {
                let mut s = crate::task::process::SCHEDULER.lock();
                self.not_empty.finish_wait(&mut s);
            }
        }
    }
    
    pub fn try_recv(&self) -> Option<M> {
        let mut guard = self.buffer.lock();
        if let Some(msg) = guard.pop_front() {
            drop(guard);
            let mut s = crate::task::process::SCHEDULER.lock();
            self.not_full.wake_one(&mut s);
            Some(msg)
        } else {
            None
        }
    }
}
