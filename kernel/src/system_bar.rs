//! SparkOS Desktop V1.31 — System Top Bar Subsystem (`src/system_bar.rs`)
//!
//! Provides the primary system panel at the top of the display, presenting real-time
//! uptime-derived clock (HH:MM), network status indicator, audio level, memory stats,
//! active user profile, theme adaptation, and resolution-independent rendering.

use alloc::format;
use spin::Mutex;
use crate::network_manager::{NetworkState, NETWORK_MANAGER};

pub const SYSTEM_BAR_HEIGHT: usize = 24;

#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
}

impl SystemClock {
    pub const fn new(hours: u8, minutes: u8, seconds: u8) -> Self {
        Self { hours, minutes, seconds }
    }

    /// Converts system timer ticks / uptime into formatted clock
    pub fn from_uptime_seconds(sec: u64) -> Self {
        let total_min = sec / 60;
        let s = (sec % 60) as u8;
        let m = (total_min % 60) as u8;
        let h = ((total_min / 60) % 24) as u8;
        Self { hours: h, minutes: m, seconds: s }
    }

    pub fn format_hh_mm(&self) -> alloc::string::String {
        format!("{:02}:{:02}", self.hours, self.minutes)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MemoryStats {
    pub total_kb: u64,
    pub used_kb: u64,
}

#[derive(Debug, Clone)]
pub struct UserInfo {
    pub username: &'static str,
}

pub struct SystemBar {
    pub visible: bool,
    pub height: usize,
    pub clock: SystemClock,
    pub network_state: NetworkState,
    pub audio_state: u8,
    pub memory_usage: MemoryStats,
    pub active_user: UserInfo,
}

impl SystemBar {
    pub const fn new() -> Self {
        Self {
            visible: true,
            height: SYSTEM_BAR_HEIGHT,
            clock: SystemClock::new(12, 0, 0),
            network_state: NetworkState::Disconnected,
            audio_state: 80,
            memory_usage: MemoryStats { total_kb: 262144, used_kb: 43008 }, // 256MB Total, 42MB Used
            active_user: UserInfo { username: "teha" },
        }
    }

    /// Updates clock based on timer ticks
    pub fn update_clock(&mut self, uptime_sec: u64) {
        self.clock = SystemClock::from_uptime_seconds(uptime_sec);
    }

    /// Compositor layer rendering of the top bar
    pub fn render(&mut self, fb_w: u16, _fb_h: u16) {
        if !self.visible { return; }

        let active_theme = crate::theme::THEME_MANAGER.lock().current_theme;
        let bar_bg = active_theme.dock_background;
        let bar_fg = 0x00FFFFFF;
        let border_col = active_theme.border_color;
        let bar_h = self.height as u16;

        // 1. Top Bar Background & 1px bottom border
        crate::gui::draw_rect(0, 0, fb_w, bar_h, bar_bg);
        crate::gui::draw_rect(0, bar_h - 1, fb_w, 1, border_col);

        // 2. Left Section: System Branding & Active User
        crate::gui::draw_icon_glyph(8, 5, crate::app_registry::AppIcon::Logo, 0x0038BDF8, bar_bg);
        crate::gui::draw_string(22, 6, "SparkOS", bar_fg, bar_bg);

        let user_label = format!("@{}", self.active_user.username);
        crate::gui::draw_string(96, 6, &user_label, 0x0094A3B8, bar_bg);

        // 3. Right Section: [Network] [Audio] [Memory] [Clock]
        let net_symbol = NETWORK_MANAGER.lock().get_icon_symbol();
        let net_text = format!("[Net: {}]", net_symbol);
        let audio_text = format!("[Vol: {}%]", self.audio_state);
        let mem_mb = self.memory_usage.used_kb / 1024;
        let mem_text = format!("[Mem: {}MB]", mem_mb);
        let clock_text = self.clock.format_hh_mm();

        let mut rx = fb_w.saturating_sub(280);

        // Network Indicator
        crate::gui::draw_string(rx, 6, &net_text, 0x0034D399, bar_bg);
        rx += 70;

        // Audio Level
        crate::gui::draw_string(rx, 6, &audio_text, 0x00E2E8F0, bar_bg);
        rx += 74;

        // Memory Usage
        crate::gui::draw_string(rx, 6, &mem_text, 0x0038BDF8, bar_bg);
        rx += 80;

        // Real-time Clock (HH:MM)
        crate::gui::draw_rect(rx - 4, 3, 48, 18, 0x001E293B);
        crate::gui::draw_string(rx, 6, &clock_text, 0x00F8FAFC, 0x001E293B);

        // 4. Render Network popup if open
        NETWORK_MANAGER.lock().render_popup(fb_w, bar_h);
    }
}

pub static SYSTEM_BAR: Mutex<SystemBar> = Mutex::new(SystemBar::new());
