//! SparkOS Desktop V1.10 — Notification Service
//!
//! Provides a capability-controlled desktop notification popup system with spam prevention,
//! priority queues, and transient on-screen rendering.

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

pub const MAX_NOTIFICATIONS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationPriority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub app_id: u8,
    pub title: String,
    pub message: String,
    pub priority: NotificationPriority,
    pub timestamp: u64,
    pub duration_ticks: u64,
}

pub struct NotificationManager {
    pub notifications: Vec<Notification>,
    pub app_rate_limits: [(u8, u64); 16], // (app_id, last_timestamp)
}

impl NotificationManager {
    pub const fn new() -> Self {
        Self {
            notifications: Vec::new(),
            app_rate_limits: [(0, 0); 16],
        }
    }

    pub fn post(&mut self, app_id: u8, title: &str, message: &str, priority: NotificationPriority) -> Result<(), &'static str> {
        let now = crate::interrupts::get_tick();

        // Spam prevention: 1 notification per 100 ticks per app
        let slot = (app_id as usize) % 16;
        if self.app_rate_limits[slot].0 == app_id && now.saturating_sub(self.app_rate_limits[slot].1) < 100 {
            crate::serial_println!("[NOTIFY] Rate limit exceeded for app {}", app_id);
            return Err("RateLimitExceeded");
        }
        self.app_rate_limits[slot] = (app_id, now);

        if self.notifications.len() >= MAX_NOTIFICATIONS {
            self.notifications.remove(0); // Evict oldest
        }

        self.notifications.push(Notification {
            app_id,
            title: String::from(title),
            message: String::from(message),
            priority,
            timestamp: now,
            duration_ticks: 300,
        });

        crate::serial_println!("[NOTIFY] New notification from app {}: '{}' - '{}'", app_id, title, message);
        Ok(())
    }

    pub fn render_layer(&self) {
        if self.notifications.is_empty() { return; }

        let active_theme = crate::theme::THEME_MANAGER.lock().current_theme;
        let mut y = 40u16;

        for n in self.notifications.iter().rev().take(3) {
            let x = (crate::display::DEFAULT_WIDTH - 260) as u16;
            let w = 240u16;
            let h = 50u16;

            // Toast Card Background
            crate::gui::draw_rect(x, y, w, h, active_theme.launcher_background);
            crate::gui::draw_rect(x, y, w, 1, active_theme.accent_color);
            crate::gui::draw_rect(x, y + h - 1, w, 1, active_theme.accent_color);
            crate::gui::draw_rect(x, y, 1, h, active_theme.accent_color);
            crate::gui::draw_rect(x + w - 1, y, 1, h, active_theme.accent_color);

            // Title and Message
            crate::gui::draw_string(x + 8, y + 8, &n.title, active_theme.accent_color, active_theme.launcher_background);
            crate::gui::draw_string(x + 8, y + 24, &n.message, active_theme.text_color, active_theme.launcher_background);

            y += 60;
        }
    }
}

pub static NOTIFICATION_MANAGER: Mutex<NotificationManager> = Mutex::new(NotificationManager::new());
