//! SparkOS Dock 2.0
//!
//! Renders bottom dock, launcher trigger button, active/minimized/fullscreen window tabs
//! with dynamic state badges, icon glyphs, short titles, and hover highlights.

use crate::wm::{Window, WindowState};

pub struct Dock;

impl Dock {
    pub fn render(
        screen_w: u16,
        screen_h: u16,
        windows: &[Window],
        focused_window: Option<u64>,
        launcher_open: bool,
        hovered_dock_tab: Option<usize>,
    ) {
        let dock_y = screen_h.saturating_sub(crate::wm::DOCK_HEIGHT);
        let active_theme = crate::theme::THEME_MANAGER.lock().current_theme;

        // Dock background
        crate::gui::draw_rect(0, dock_y, screen_w, crate::wm::DOCK_HEIGHT, active_theme.dock_background);
        crate::gui::draw_rect(0, dock_y, screen_w, 1, active_theme.border_color);

        // Launcher Button
        let launcher_bg = if launcher_open { 0x002563EB } else { 0x001E293B };
        crate::gui::draw_rect(4, dock_y + 2, 74, 20, launcher_bg);
        crate::gui::draw_icon_glyph(8, dock_y + 8, crate::app_registry::AppIcon::Logo, 0x00FFFFFF, launcher_bg);
        crate::gui::draw_string(22, dock_y + 7, "SparkOS", 0x00FFFFFF, launcher_bg);

        // Separator
        crate::gui::draw_rect(81, dock_y + 3, 1, 18, 0x00334155);

        // Window Tabs
        let mut tab_x = 86u16;
        for (idx, win) in windows.iter().enumerate() {
            if tab_x + 84 > screen_w.saturating_sub(80) {
                break;
            }

            let is_focused = focused_window == Some(win.window_id);
            let is_hovered = hovered_dock_tab == Some(idx);

            let tab_bg = if is_focused {
                0x002563EB // Vibrant Blue
            } else if win.state == WindowState::Minimized {
                if is_hovered { 0x001E293B } else { 0x000F172A }
            } else if is_hovered {
                0x00334155
            } else {
                0x001E293B
            };

            let tab_fg = if is_focused {
                0x00FFFFFF
            } else if win.state == WindowState::Minimized {
                0x0064748B
            } else {
                0x00E2E8F0
            };

            // Tab background & border
            crate::gui::draw_rect(tab_x, dock_y + 2, 80, 20, tab_bg);
            let border_col = if is_focused { 0x0060A5FA } else { 0x00334155 };
            crate::gui::draw_rect(tab_x, dock_y + 2, 80, 1, border_col);
            crate::gui::draw_rect(tab_x, dock_y + 21, 80, 1, border_col);

            // App icon & title from Window metadata cache
            crate::gui::draw_icon_glyph(tab_x + 4, dock_y + 8, win.icon, tab_fg, tab_bg);
            
            // Short label
            let short_title = if win.title.len() > 7 { &win.title[..7] } else { &win.title };
            crate::gui::draw_string(tab_x + 16, dock_y + 7, short_title, tab_fg, tab_bg);

            // Active indicator
            if is_focused {
                crate::gui::draw_rect(tab_x + 36, dock_y + 19, 8, 2, 0x0060A5FA);
            } else if win.state != WindowState::Minimized {
                crate::gui::draw_rect(tab_x + 38, dock_y + 20, 4, 1, 0x0094A3B8);
            }

            tab_x += 84;
        }
    }
}
