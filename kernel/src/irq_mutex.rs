//! IRQ-Safe Mutex implementation.
//! Guarantees that interrupts are disabled before acquiring the inner spinlock
//! and properly restored to their previous IF state upon guard drop.
//! Safe against single-core and multi-core deadlocks during hardware IRQs.

use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
use spin::{Mutex as SpinMutex, MutexGuard as SpinMutexGuard};

pub trait InterruptController {
    fn are_enabled() -> bool;
    fn disable();
    fn enable();
}

#[cfg(target_os = "none")]
pub struct NativeInterrupts;

#[cfg(target_os = "none")]
impl InterruptController for NativeInterrupts {
    #[inline]
    fn are_enabled() -> bool {
        x86_64::instructions::interrupts::are_enabled()
    }
    #[inline]
    fn disable() {
        x86_64::instructions::interrupts::disable();
    }
    #[inline]
    fn enable() {
        x86_64::instructions::interrupts::enable();
    }
}

#[cfg(not(target_os = "none"))]
pub struct NativeInterrupts;

#[cfg(not(target_os = "none"))]
static HOST_IF: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

#[cfg(not(target_os = "none"))]
impl InterruptController for NativeInterrupts {
    #[inline]
    fn are_enabled() -> bool {
        HOST_IF.load(core::sync::atomic::Ordering::SeqCst)
    }
    #[inline]
    fn disable() {
        HOST_IF.store(false, core::sync::atomic::Ordering::SeqCst);
    }
    #[inline]
    fn enable() {
        HOST_IF.store(true, core::sync::atomic::Ordering::SeqCst);
    }
}

pub struct GenericIrqMutex<T, C: InterruptController = NativeInterrupts> {
    inner: SpinMutex<T>,
    _ctrl: PhantomData<C>,
}

unsafe impl<T: Send, C: InterruptController> Sync for GenericIrqMutex<T, C> {}
unsafe impl<T: Send, C: InterruptController> Send for GenericIrqMutex<T, C> {}

pub struct GenericIrqGuard<'a, T, C: InterruptController = NativeInterrupts> {
    guard: ManuallyDrop<SpinMutexGuard<'a, T>>,
    was_enabled: bool,
    start_tsc: u64,
    _not_send: PhantomData<*mut ()>,
    _ctrl: PhantomData<C>,
}

pub type IrqMutex<T> = GenericIrqMutex<T, NativeInterrupts>;
pub type IrqGuard<'a, T> = GenericIrqGuard<'a, T, NativeInterrupts>;

pub static HELD_IRQ_LOCKS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[inline]
pub fn irq_locks_held() -> usize {
    HELD_IRQ_LOCKS.load(core::sync::atomic::Ordering::Relaxed)
}

impl<T, C: InterruptController> GenericIrqMutex<T, C> {
    pub const fn new(data: T) -> Self {
        Self {
            inner: SpinMutex::new(data),
            _ctrl: PhantomData,
        }
    }

    pub fn lock(&self) -> GenericIrqGuard<'_, T, C> {
        let was_enabled = C::are_enabled();
        C::disable();
        let start_tsc = if was_enabled { crate::perf::rdtsc() } else { 0 };
        let guard = self.inner.lock();
        HELD_IRQ_LOCKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        GenericIrqGuard {
            guard: ManuallyDrop::new(guard),
            was_enabled,
            start_tsc,
            _not_send: PhantomData,
            _ctrl: PhantomData,
        }
    }

    pub fn try_lock(&self) -> Option<GenericIrqGuard<'_, T, C>> {
        let was_enabled = C::are_enabled();
        C::disable();
        let start_tsc = if was_enabled { crate::perf::rdtsc() } else { 0 };
        if let Some(guard) = self.inner.try_lock() {
            HELD_IRQ_LOCKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            Some(GenericIrqGuard {
                guard: ManuallyDrop::new(guard),
                was_enabled,
                start_tsc,
                _not_send: PhantomData,
                _ctrl: PhantomData,
            })
        } else {
            if was_enabled {
                C::enable();
            }
            None
        }
    }

    pub fn is_locked(&self) -> bool {
        self.inner.is_locked()
    }
}

impl<T, C: InterruptController> Deref for GenericIrqGuard<'_, T, C> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T, C: InterruptController> DerefMut for GenericIrqGuard<'_, T, C> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T, C: InterruptController> Drop for GenericIrqGuard<'_, T, C> {
    fn drop(&mut self) {
        HELD_IRQ_LOCKS.fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
        // SAFETY: The inner spinlock guard is explicitly dropped before restoring
        // interrupt flags. This ensures any hardware IRQ will see the spinlock
        // fully unlocked when interrupts are re-enabled.
        unsafe {
            ManuallyDrop::drop(&mut self.guard);
        }
        if self.was_enabled {
            let elapsed = crate::perf::rdtsc().saturating_sub(self.start_tsc);
            crate::perf::record_irq_off_cycles(elapsed);
            C::enable();
        }
    }
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use core::sync::atomic::{AtomicBool, Ordering};

    pub static MOCK_IF_STATE: AtomicBool = AtomicBool::new(true);

    pub struct MockInterrupts;
    impl InterruptController for MockInterrupts {
        fn are_enabled() -> bool {
            MOCK_IF_STATE.load(Ordering::SeqCst)
        }
        fn disable() {
            MOCK_IF_STATE.store(false, Ordering::SeqCst);
        }
        fn enable() {
            MOCK_IF_STATE.store(true, Ordering::SeqCst);
        }
    }

    pub type MockIrqMutex<T> = GenericIrqMutex<T, MockInterrupts>;

    #[test]
    fn test_irq_mutex_basic() {
        MOCK_IF_STATE.store(true, Ordering::SeqCst);
        let m = MockIrqMutex::new(42);
        {
            let g = m.lock();
            assert_eq!(*g, 42);
            assert!(!MockInterrupts::are_enabled());
        }
        assert!(MockInterrupts::are_enabled());
    }

    #[test]
    fn test_irq_mutex_nested_drop_order() {
        MOCK_IF_STATE.store(true, Ordering::SeqCst);
        let m1 = MockIrqMutex::new(10);
        let m2 = MockIrqMutex::new(20);
        {
            let g1 = m1.lock();
            assert!(!MockInterrupts::are_enabled());
            {
                let g2 = m2.lock();
                assert!(!MockInterrupts::are_enabled());
                assert_eq!(*g1, 10);
                assert_eq!(*g2, 20);
            }
            // Inner guard dropped, outer still active -> IF must still be disabled
            assert!(!MockInterrupts::are_enabled());
        }
        // Outer guard dropped -> IF must be restored to enabled
        assert!(MockInterrupts::are_enabled());
    }

    #[test]
    fn test_irq_mutex_try_lock() {
        MOCK_IF_STATE.store(true, Ordering::SeqCst);
        let m = MockIrqMutex::new(99);
        let g1 = m.try_lock();
        assert!(g1.is_some());
        assert!(!MockInterrupts::are_enabled());

        let g2 = m.try_lock();
        assert!(g2.is_none());
        assert!(!MockInterrupts::are_enabled());

        drop(g1);
        assert!(MockInterrupts::are_enabled());
    }
}
