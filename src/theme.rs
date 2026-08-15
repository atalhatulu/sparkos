//! SparkOS Desktop V1.7 — Theme Engine
//!
//! Provides centralized, capability-controlled theme switching for Window Decor,
//! Dock, Launcher, Terminal, and user-space applications.

use spin::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    SparkDark,
    SparkLight,
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub desktop_background: u32,
    pub window_active: u32,
    pub window_inactive: u32,
    pub titlebar_active: u32,
    pub titlebar_inactive: u32,
    pub border_color: u32,
    pub text_color: u32,
    pub accent_color: u32,
    pub dock_background: u32,
    pub launcher_background: u32,
    pub close_button_hover: u32,
}

pub const SPARK_DARK: Theme = Theme {
    name: "Spark Dark",
    desktop_background: 0x001E293B, // Slate 800
    window_active: 0x000F172A,      // Slate 900
    window_inactive: 0x001E293B,    // Slate 800
    titlebar_active: 0x002563EB,    // Vibrant Royal Blue
    titlebar_inactive: 0x00334155,  // Muted Slate
    border_color: 0x00475569,       // Slate 600
    text_color: 0x00F8FAFC,         // Crisp White
    accent_color: 0x0038BDF8,       // Sky Blue
    dock_background: 0x000F172A,    // Dark Glass
    launcher_background: 0x001E293B,// Slate Modal
    close_button_hover: 0x00EF4444, // Red 500
};

pub const SPARK_LIGHT: Theme = Theme {
    name: "Spark Light",
    desktop_background: 0x00E2E8F0, // Slate 200
    window_active: 0x00FFFFFF,      // Pure White
    window_inactive: 0x00F1F5F9,    // Slate 100
    titlebar_active: 0x003B82F6,    // Blue 500
    titlebar_inactive: 0x0094A3B8,  // Slate 400
    border_color: 0x00CBD5E1,       // Slate 300
    text_color: 0x000F172A,         // Slate 900
    accent_color: 0x002563EB,       // Blue 600
    dock_background: 0x00F8FAFC,    // Light Glass
    launcher_background: 0x00FFFFFF,// White Modal
    close_button_hover: 0x00DC2626, // Red 600
};

pub struct ThemeManager {
    pub current_mode: ThemeMode,
    pub current_theme: Theme,
}

impl ThemeManager {
    pub const fn new() -> Self {
        Self {
            current_mode: ThemeMode::SparkDark,
            current_theme: SPARK_DARK,
        }
    }

    pub fn set_theme(&mut self, mode: ThemeMode) {
        self.current_mode = mode;
        self.current_theme = match mode {
            ThemeMode::SparkDark => SPARK_DARK,
            ThemeMode::SparkLight => SPARK_LIGHT,
        };
        crate::serial_println!("[THEME] Active theme changed to: {}", self.current_theme.name);
    }

    pub fn toggle_theme(&mut self) {
        match self.current_mode {
            ThemeMode::SparkDark => self.set_theme(ThemeMode::SparkLight),
            ThemeMode::SparkLight => self.set_theme(ThemeMode::SparkDark),
        }
    }
}

pub static THEME_MANAGER: Mutex<ThemeManager> = Mutex::new(ThemeManager::new());
