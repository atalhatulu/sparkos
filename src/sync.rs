use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::hint::spin_loop;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use alloc::collections::VecDeque;
use core::task::Waker;
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

/// 2. Mutex — Built on top of spin::Mutex, provides a std::sync::Mutex like lock() that uses hlt to wait
pub struct Mutex<T> {
    inner: SpinMutex<T>,
}

pub type MutexGuard<'a, T> = spin::MutexGuard<'a, T>;

impl<T> Mutex<T> {
    pub const fn new(data: T) -> Self {
        Mutex { inner: SpinMutex::new(data) }
    }

    pub fn lock(&self) -> MutexGuard<'_, T> {
        loop {
            if let Some(guard) = self.inner.try_lock() {
                return guard;
            }
            x86_64::instructions::hlt();
        }
    }
    
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        self.inner.try_lock()
    }
}

/// 3. Semaphore — Atomic counter with an async wakelist
pub struct Semaphore {
    count: AtomicUsize,
    wakers: SpinMutex<VecDeque<Waker>>,
}

impl Semaphore {
    pub const fn new(initial: usize) -> Self {
        Semaphore {
            count: AtomicUsize::new(initial),
            wakers: SpinMutex::new(VecDeque::new()),
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
                break;
            }
            x86_64::instructions::hlt();
        }
    }
    
    pub async fn wait_async(&self) {
        core::future::poll_fn(|cx| {
            if self.try_wait() {
                core::task::Poll::Ready(())
            } else {
                self.wakers.lock().push_back(cx.waker().clone());
                core::task::Poll::Pending
            }
        }).await;
    }

    pub fn signal(&self) {
        self.count.fetch_add(1, Ordering::Release);
        if let Some(waker) = self.wakers.lock().pop_front() {
            waker.wake();
        }
    }
}

/// 4. Condvar — Condition variable compatible with spin::MutexGuard
pub struct Condvar {
    wakers: SpinMutex<VecDeque<Waker>>,
    generation: AtomicUsize,
}

impl Condvar {
    pub const fn new() -> Self {
        Condvar {
            wakers: SpinMutex::new(VecDeque::new()),
            generation: AtomicUsize::new(0),
        }
    }

    pub fn wait<'a, T>(&self, guard: spin::MutexGuard<'a, T>, mutex: &'a spin::Mutex<T>) -> spin::MutexGuard<'a, T> {
        let gen = self.generation.load(Ordering::SeqCst);
        
        drop(guard);
        
        // Wait for condition to change (spurious wakeups possible and handled by users)
        loop {
            if self.generation.load(Ordering::SeqCst) != gen {
                break;
            }
            x86_64::instructions::hlt();
        }
        
        loop {
            if let Some(g) = mutex.try_lock() {
                return g;
            }
            x86_64::instructions::hlt();
        }
    }

    pub fn notify_one(&self) {
        if let Some(waker) = self.wakers.lock().pop_front() {
            waker.wake();
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    pub fn notify_all(&self) {
        let mut wakers = self.wakers.lock();
        while let Some(waker) = wakers.pop_front() {
            waker.wake();
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

/// 5. BlockingChannel — Blocking channel for IPC
pub struct BlockingChannel<M> {
    buffer: SpinMutex<VecDeque<M>>,
    cond: Condvar,
    capacity: usize,
}

impl<M> BlockingChannel<M> {
    pub const fn new(capacity: usize) -> Self {
        BlockingChannel {
            buffer: SpinMutex::new(VecDeque::new()),
            cond: Condvar::new(),
            capacity,
        }
    }

    pub fn send(&self, msg: M) {
        let mut guard = self.buffer.lock();
        while guard.len() >= self.capacity {
            guard = self.cond.wait(guard, &self.buffer);
        }
        guard.push_back(msg);
        self.cond.notify_one();
    }
    
    pub fn try_send(&self, msg: M) -> Result<(), M> {
        let mut guard = self.buffer.lock();
        if guard.len() >= self.capacity {
            return Err(msg);
        }
        guard.push_back(msg);
        self.cond.notify_one();
        Ok(())
    }

    pub fn recv(&self) -> M {
        let mut guard = self.buffer.lock();
        loop {
            if let Some(msg) = guard.pop_front() {
                self.cond.notify_one();
                return msg;
            }
            guard = self.cond.wait(guard, &self.buffer);
        }
    }
    
    pub fn try_recv(&self) -> Option<M> {
        let mut guard = self.buffer.lock();
        if let Some(msg) = guard.pop_front() {
            self.cond.notify_one();
            Some(msg)
        } else {
            None
        }
    }
}
