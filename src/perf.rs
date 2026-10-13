//! Opt-in BSP desktop telemetry. No allocations or serial output in the hot path.
//! F9 starts a fresh recording; F10 freezes it and prints a snapshot to serial.
//! Durations are PIT-calibrated TSC estimates; framebuffer completion is not scanout.
#![allow(dead_code)]
#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Metric {
    Compose, Framebuffer, Terminal, Files, Editor, Browser, Settings, TaskMgr,
    InputDispatch, ExecutorBusy, TaskPoll, MouseIrqToDispatch, MouseToPresent,
}
const N: usize = 13;
const NAMES: [&str; N] = ["compose", "framebuffer", "terminal", "files", "editor", "browser",
    "settings", "taskmgr", "input_dispatch", "executor_busy", "task_poll", "mouse_irq_dispatch", "mouse_to_present"];

pub struct Span { metric: Metric, start: u64 }
impl Span {
    #[inline]
    pub fn new(metric: Metric) -> Self { Self { metric, start: stamp() } }
}
impl Drop for Span {
    #[inline]
    fn drop(&mut self) { finish(self.metric, self.start); }
}

#[cfg(not(feature = "perf-metrics"))]
mod disabled {
    use super::Metric;
    #[inline] pub fn stamp() -> u64 { 0 }
    #[inline] pub fn finish(_: Metric, _: u64) {}
    #[inline] pub fn init() {}
    #[inline] pub fn mouse_irq() {}
    #[inline] pub fn mouse_dispatched() {}
    #[inline] pub fn take_pending_mouse() -> u64 { 0 }
    #[inline] pub fn queue_sample(_: usize, _: bool) {}
    #[inline] pub fn start_recording() {}
    #[inline] pub fn stop_report() {}
}
#[cfg(not(feature = "perf-metrics"))]
pub use disabled::*;

#[cfg(feature = "perf-metrics")]
mod enabled {
    use super::*;
    use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use crate::perf_histogram::Histogram;
    static ENABLED: AtomicBool = AtomicBool::new(false);
    static HZ: AtomicU64 = AtomicU64::new(0);
    static LAST_MOUSE: AtomicU64 = AtomicU64::new(0);
    static PENDING_MOUSE: AtomicU64 = AtomicU64::new(0);
    static QUEUE_MAX: AtomicU64 = AtomicU64::new(0);
    static QUEUE_EVICTIONS: AtomicU64 = AtomicU64::new(0);
    static STATS: spin::Mutex<[Histogram; N]> = spin::Mutex::new([Histogram::new(); N]);
    static START: AtomicU64 = AtomicU64::new(0);
    static PIXELS: AtomicU64 = AtomicU64::new(0);
    static WINDOWS: AtomicU64 = AtomicU64::new(0);

    fn raw_stamp() -> u64 {
        let lo: u32; let hi: u32;
        unsafe { core::arch::asm!("lfence", "rdtsc", "lfence", out("eax") lo, out("edx") hi,
            options(nostack, preserves_flags)); }
        ((hi as u64) << 32) | lo as u64
    }
    pub fn init() {
        // BSP only, before the executor. PIT divisor is 1193 at 1193182 Hz.
        // Settle startup IRQ timing before five 100-tick calibration windows.
        let warmup = crate::interrupts::get_tick();
        while crate::interrupts::get_tick().saturating_sub(warmup) < 200 { x86_64::instructions::hlt(); }
        let mut samples = [0u64; 5];
        for sample in &mut samples {
            let edge = crate::interrupts::get_tick();
            while crate::interrupts::get_tick() == edge { x86_64::instructions::hlt(); }
            let tick = crate::interrupts::get_tick(); let start = raw_stamp();
            while crate::interrupts::get_tick().saturating_sub(tick) < 100 { x86_64::instructions::hlt(); }
            let elapsed = crate::interrupts::get_tick() - tick;
            *sample = (raw_stamp() - start).saturating_mul(1_193_182) / (elapsed * 1193);
        }
        samples.sort_unstable();
        let hz = samples[2];
        HZ.store(hz, Ordering::Relaxed);
        let jitter = (samples[4] - samples[0]).saturating_mul(1000) / hz.max(1);
        crate::serial_println!("[PERF] clock=PIT-calibrated-TSC hz={} samples={:?} spread_permille={} BSP_only=true", hz, samples, jitter);
        if jitter > 50 { crate::serial_println!("[PERF] WARNING calibration spread exceeds 5%; timing estimates are unreliable"); }
    }
    #[inline]
    pub fn stamp() -> u64 { if ENABLED.load(Ordering::Relaxed) { raw_stamp() } else { 0 } }
    pub fn finish(metric: Metric, start: u64) {
        if start == 0 || !ENABLED.load(Ordering::Relaxed) { return; }
        let ticks = raw_stamp().saturating_sub(start);
        let hz = HZ.load(Ordering::Relaxed);
        if hz != 0 { STATS.lock()[metric as usize].record(ticks.saturating_mul(1_000_000).saturating_add(hz - 1) / hz); }
    }
    pub fn mouse_irq() { LAST_MOUSE.store(stamp(), Ordering::Relaxed); }
    pub fn mouse_dispatched() {
        let now = stamp();
        if now == 0 { return; }
        let irq = LAST_MOUSE.swap(0, Ordering::Relaxed);
        finish(Metric::MouseIrqToDispatch, irq);
        let _ = PENDING_MOUSE.compare_exchange(0, now, Ordering::Relaxed, Ordering::Relaxed);
    }
    pub fn take_pending_mouse() -> u64 { PENDING_MOUSE.swap(0, Ordering::Relaxed) }
    pub fn queue_sample(len: usize, evicted: bool) {
        if ENABLED.load(Ordering::Relaxed) {
            QUEUE_MAX.fetch_max(len as u64, Ordering::Relaxed);
            if evicted { QUEUE_EVICTIONS.fetch_add(1, Ordering::Relaxed); }
        }
    }
    pub fn start_recording() {
        ENABLED.store(false, Ordering::Relaxed);
        *STATS.lock() = [Histogram::new(); N];
        LAST_MOUSE.store(0, Ordering::Relaxed); PENDING_MOUSE.store(0, Ordering::Relaxed);
        QUEUE_MAX.store(0, Ordering::Relaxed); QUEUE_EVICTIONS.store(0, Ordering::Relaxed);
        PIXELS.store(crate::wm::COMPOSITED_PIXELS.load(Ordering::Relaxed), Ordering::Relaxed);
        WINDOWS.store(crate::wm::COMPOSITED_WINDOWS.load(Ordering::Relaxed), Ordering::Relaxed);
        crate::serial_println!("[PERF] BEGIN");
        START.store(raw_stamp(), Ordering::Relaxed);
        ENABLED.store(true, Ordering::Relaxed);
    }
    pub fn stop_report() {
        let end = raw_stamp();
        if !ENABLED.swap(false, Ordering::Relaxed) { return; }
        let stats = *STATS.lock(); // Release lock before slow serial output.
        let hz = HZ.load(Ordering::Relaxed).max(1);
        let duration_us = end.saturating_sub(START.load(Ordering::Relaxed)).saturating_mul(1_000_000) / hz;
        crate::serial_println!("[PERF] REPORT duration_us={} pixels={} windows={} queue_max={} queue_evictions={}",
            duration_us, crate::wm::COMPOSITED_PIXELS.load(Ordering::Relaxed)-PIXELS.load(Ordering::Relaxed),
            crate::wm::COMPOSITED_WINDOWS.load(Ordering::Relaxed)-WINDOWS.load(Ordering::Relaxed),
            QUEUE_MAX.load(Ordering::Relaxed), QUEUE_EVICTIONS.load(Ordering::Relaxed));
        for (i, h) in stats.iter().enumerate() {
            crate::serial_println!("[PERF] metric={} count={} total_us={} mean_us={} p50_upper_us={} p95_upper_us={} max_us={}",
                NAMES[i], h.count, h.total_us, h.total_us / h.count.max(1), h.percentile_upper(50), h.percentile_upper(95), h.max_us);
        }
        crate::serial_println!("[PERF] END");
    }
}
#[cfg(feature = "perf-metrics")]
pub use enabled::*;
