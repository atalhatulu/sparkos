//! SparkOS Desktop V1.36 — Real Desktop Environment & Wallpaper Engine (`src/desktop.rs`)
//!
//! Provides the primary desktop background layer, theme-reactive wallpaper gradient engine,
//! desktop icons (Home, Computer, Trash, Applications), single-click selection,
//! double-click activation, and capability-isolated launcher dispatching.

use spin::Mutex;
use crate::app_registry::AppIcon;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopIconAction {
    OpenHome,
    OpenComputer,
    OpenTrash,
    OpenApplications,
}

#[derive(Debug, Clone, Copy)]
pub struct DesktopIcon {
    pub id: u32,
    pub position: (u16, u16),
    pub icon: AppIcon,
    pub label: &'static str,
    pub action: DesktopIconAction,
}

pub struct DesktopEnvironment {
    pub icons: [DesktopIcon; 4],
    pub selected_icon_id: Option<u32>,
    pub last_click_id: Option<u32>,
    pub last_click_tick: u64,
    pub wallpaper_start_color: u32,
    pub wallpaper_end_color: u32,
}

impl DesktopEnvironment {
    pub const fn new() -> Self {
        Self {
            icons: [
                DesktopIcon {
                    id: 1,
                    position: (24, 40),
                    icon: AppIcon::Files,
                    label: "Home",
                    action: DesktopIconAction::OpenHome,
                },
                DesktopIcon {
                    id: 2,
                    position: (24, 115),
                    icon: AppIcon::Generic,
                    label: "Computer",
                    action: DesktopIconAction::OpenComputer,
                },
                DesktopIcon {
                    id: 3,
                    position: (24, 190),
                    icon: AppIcon::Generic,
                    label: "Trash",
                    action: DesktopIconAction::OpenTrash,
                },
                DesktopIcon {
                    id: 4,
                    position: (24, 265),
                    icon: AppIcon::Logo,
                    label: "Apps",
                    action: DesktopIconAction::OpenApplications,
                },
            ],
            selected_icon_id: None,
            last_click_id: None,
            last_click_tick: 0,
            wallpaper_start_color: 0x000F2027, // Oceanic Slate
            wallpaper_end_color: 0x00203A43,   // Deep Teal
        }
    }

    /// Renders desktop wallpaper gradient and desktop icons
    pub fn render(&self, screen_w: u16, screen_h: u16) {
        // 1. Wallpaper Engine: Render Gradient Background
        let active_theme = crate::theme::THEME_MANAGER.lock().current_theme;
        let start_col = if active_theme.name == "Spark Light" { 0x00E2E8F0 } else { self.wallpaper_start_color };
        let end_col = if active_theme.name == "Spark Light" { 0x00CBD5E1 } else { self.wallpaper_end_color };

        crate::gui::draw_background(start_col, end_col);

        // 2. Render Desktop Icons
        for icon in &self.icons {
            let (ix, iy) = icon.position;
            if ix + 60 >= screen_w || iy + 65 >= screen_h { continue; }

            let is_selected = self.selected_icon_id == Some(icon.id);
            if is_selected {
                crate::gui::draw_rect_alpha(ix - 4, iy - 4, 64, 62, 0x003B82F6, 120);
                crate::gui::draw_rect(ix - 4, iy - 4, 64, 1, 0x0060A5FA);
                crate::gui::draw_rect(ix - 4, iy - 4, 1, 62, 0x0060A5FA);
                crate::gui::draw_rect(ix + 60 - 1, iy - 4, 1, 62, 0x0060A5FA);
                crate::gui::draw_rect(ix - 4, iy + 62 - 5, 64, 1, 0x0060A5FA);
            }

            // Draw Icon Glyph
            crate::gui::draw_icon_glyph(ix + 12, iy + 6, icon.icon, 0x0038BDF8, 0x00000000);

            // Draw Label with drop-shadow for crisp readability
            crate::gui::draw_string(ix + 2, iy + 41, icon.label, 0x00000000, 0x00000000);
            crate::gui::draw_string(ix + 1, iy + 40, icon.label, 0x00F8FAFC, 0x00000000);
        }
    }

    /// Handles mouse clicks on desktop: single-click selects, double-click activates launcher action
    pub fn handle_mouse_click(&mut self, mx: u16, my: u16, current_tick: u64) -> Option<DesktopIconAction> {
        for icon in &self.icons {
            let (ix, iy) = icon.position;
            if mx >= ix - 4 && mx <= ix + 60 && my >= iy - 4 && my <= iy + 60 {
                // Check for double click (within 30 ticks / ~500ms)
                if self.last_click_id == Some(icon.id) && current_tick.saturating_sub(self.last_click_tick) <= 30 {
                    self.last_click_id = None;
                    self.last_click_tick = 0;
                    crate::serial_println!("[DESKTOP] Double-click activated icon '{}'", icon.label);
                    return Some(icon.action);
                } else {
                    // Single click: select icon
                    self.selected_icon_id = Some(icon.id);
                    self.last_click_id = Some(icon.id);
                    self.last_click_tick = current_tick;
                    return None;
                }
            }
        }

        // Clicked on empty desktop space: deselect
        self.selected_icon_id = None;
        self.last_click_id = None;
        None
    }
}

pub static DESKTOP_ENV: Mutex<DesktopEnvironment> = Mutex::new(DesktopEnvironment::new());
