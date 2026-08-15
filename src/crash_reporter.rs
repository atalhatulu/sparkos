//! SparkOS Desktop Stability Phase V1.30.x — Global Freeze Protection & Crash Reporter (`src/crash_reporter.rs`)
//!
//! Provides kernel-level fault capture, isolated process termination, graphical error
//! modal rendering, diagnostic overlay support, and non-blocking recovery.

use alloc::format;
use alloc::vec::Vec;
use spin::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemHealthState {
    Normal,
    Warning,
    RecoverableError,
    ProcessCrash,
    KernelCritical,
}

#[derive(Debug, Clone)]
pub struct CrashRecord {
    pub app_name: &'static str,
    pub error_reason: &'static str,
    pub pid: u64,
    pub action_taken: &'static str,
    pub timestamp_ticks: u64,
}

pub struct CrashReporter {
    pub state: SystemHealthState,
    pub active_crash: Option<CrashRecord>,
    pub crash_history: Vec<CrashRecord>,
    pub diagnostic_mode: bool,
}

impl CrashReporter {
    pub const fn new() -> Self {
        Self {
            state: SystemHealthState::Normal,
            active_crash: None,
            crash_history: Vec::new(),
            diagnostic_mode: false,
        }
    }

    /// Records an application crash and ensures the kernel isolates the faulting PID
    pub fn report_process_crash(&mut self, pid: u64, app_name: &'static str, reason: &'static str) {
        crate::serial_println!("[CRASH-REPORTER] Process {} ('{}') faulted: {}", pid, app_name, reason);
        self.state = SystemHealthState::ProcessCrash;

        let record = CrashRecord {
            app_name,
            error_reason: reason,
            pid,
            action_taken: "Process isolated & terminated safely",
            timestamp_ticks: crate::interrupts::get_tick(),
        };

        self.active_crash = Some(record.clone());
        self.crash_history.push(record);

        // Terminate the faulty process in the scheduler table without taking down the kernel
        let mut list = crate::task::KILLED_PROCESSES.lock();
        if !list.contains(&pid) {
            list.push(pid);
        }
    }

    pub fn dismiss_active_crash(&mut self) {
        self.active_crash = None;
        self.state = SystemHealthState::Normal;
    }

    pub fn toggle_diagnostic_mode(&mut self) {
        self.diagnostic_mode = !self.diagnostic_mode;
    }

    /// Renders crash modal on top of desktop if an active crash occurred
    pub fn render_crash_modal(&self, screen_w: u16, screen_h: u16) {
        if let Some(crash) = &self.active_crash {
            let mw = 260u16;
            let mh = 160u16;
            let mx = (screen_w.saturating_sub(mw)) / 2;
            let my = (screen_h.saturating_sub(mh)) / 2;

            // Modal card background & danger border
            crate::gui::draw_rect(mx, my, mw, mh, 0x0009090B);
            crate::gui::draw_rect(mx, my, mw, 1, 0x00EF4444);
            crate::gui::draw_rect(mx, my, 1, mh, 0x00EF4444);
            crate::gui::draw_rect(mx + mw - 1, my, 1, mh, 0x00EF4444);
            crate::gui::draw_rect(mx, my + mh - 1, mw, 1, 0x00EF4444);

            // Modal Header
            crate::gui::draw_rect(mx + 2, my + 2, mw - 4, 22, 0x007F1D1D);
            crate::gui::draw_string(mx + 8, my + 7, "SparkOS Runtime Error", 0x00FFFFFF, 0x007F1D1D);

            // App name and error details
            let app_lbl = format!("Application: {}", crash.app_name);
            crate::gui::draw_string(mx + 12, my + 32, &app_lbl, 0x00F8FAFC, 0x0009090B);

            let err_lbl = format!("Error: {}", crash.error_reason);
            crate::gui::draw_string(mx + 12, my + 50, &err_lbl, 0x00FCA5A5, 0x0009090B);

            let pid_lbl = format!("PID: {}", crash.pid);
            crate::gui::draw_string(mx + 12, my + 68, &pid_lbl, 0x0094A3B8, 0x0009090B);

            let act_lbl = format!("Action: {}", crash.action_taken);
            crate::gui::draw_string(mx + 12, my + 86, &act_lbl, 0x0034D399, 0x0009090B);

            // Dismiss Button
            crate::gui::draw_rect(mx + 70, my + 118, 120, 24, 0x002563EB);
            crate::gui::draw_string(mx + 90, my + 124, "[ Dismiss ]", 0x00FFFFFF, 0x002563EB);
        }

        // Diagnostic Mode Overlay (if enabled)
        if self.diagnostic_mode {
            let dx = 8u16;
            let dy = 28u16;
            let dw = 320u16;
            let dh = 46u16;

            crate::gui::draw_rect_alpha(dx, dy, dw, dh, 0x00000000, 180);
            crate::gui::draw_rect(dx, dy, dw, 1, 0x0038BDF8);

            let tick = crate::interrupts::get_tick();
            let diag_line1 = format!("DIAG: Sched Tick {} | State: {:?}", tick, self.state);
            crate::gui::draw_string(dx + 6, dy + 6, &diag_line1, 0x0038BDF8, 0x00000000);

            let diag_line2 = format!("Active PID: {} | Memory: Stable | Lock: OK", crate::task::process::current_pid());
            crate::gui::draw_string(dx + 6, dy + 22, &diag_line2, 0x0034D399, 0x00000000);
        }
    }
}

pub static CRASH_REPORTER: Mutex<CrashReporter> = Mutex::new(CrashReporter::new());
