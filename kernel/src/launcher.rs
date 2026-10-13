//! SparkOS Launcher 2.0
//!
//! Provides dynamic application discovery via `app_registry`, keyboard navigation (Up/Down/Enter/Escape),
//! visual selection highlight, and zero-scheduler-lock rendering.

#[derive(Debug, Clone, Copy)]
pub struct LauncherLayout {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub item_start_y: i32,
    pub item_height: u32,
    pub item_spacing: u32,
    pub close_btn_height: u32,
}

impl LauncherLayout {
    pub fn new() -> Self {
        let total_apps = crate::app_registry::REGISTERED_APPS.len() as u32;
        let px = 4i32;
        let py = crate::wm::DOCK_HEIGHT as i32 + 2;
        let pw = 164u32;
        let ph = 34 + total_apps * 28 + 26;
        Self {
            x: px,
            y: py,
            width: pw,
            height: ph,
            item_start_y: py + 28,
            item_height: 24,
            item_spacing: 28,
            close_btn_height: 20,
        }
    }
}

pub const LAUNCHER_LAYOUT: LauncherLayout = LauncherLayout {
    x: 4,
    y: 26, // DOCK_HEIGHT (24) + 2
    width: 164,
    height: 34 + 6 * 28 + 26,
    item_start_y: 26 + 28,
    item_height: 24,
    item_spacing: 28,
    close_btn_height: 20,
};

#[derive(Debug, Clone)]
pub struct LauncherState {
    pub open: bool,
    pub selected_index: usize,
}

impl LauncherState {
    pub const fn new() -> Self {
        Self {
            open: false,
            selected_index: 0,
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.selected_index = 0;
        }
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn nav_up(&mut self) {
        let total = crate::app_registry::REGISTERED_APPS.len();
        if total > 0 {
            if self.selected_index == 0 {
                self.selected_index = total - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }

    pub fn nav_down(&mut self) {
        let total = crate::app_registry::REGISTERED_APPS.len();
        if total > 0 {
            self.selected_index = (self.selected_index + 1) % total;
        }
    }

    pub fn get_selected_app_id(&self) -> Option<u8> {
        let total = crate::app_registry::REGISTERED_APPS.len();
        if self.open && self.selected_index < total {
            Some(crate::app_registry::REGISTERED_APPS[self.selected_index].id)
        } else {
            None
        }
    }

    pub fn render(&self, _screen_w: u16, _screen_h: u16) {
        if !self.open { return; }

        let theme = crate::theme::THEME_MANAGER.lock().current_theme;
        let layout = LauncherLayout::new();
        let px = layout.x as u16;
        let py = layout.y as u16;
        let pw = layout.width as u16;
        let ph = layout.height as u16;

        let bg_color = theme.window_active;
        let header_bg = theme.panel_background;
        let border_col = theme.accent_color;
        let text_color = theme.text_color;
        let text_muted = theme.text_muted;
        let accent_color = theme.accent_color;

        // Background & Border
        crate::gui::draw_rect(px, py, pw, ph, bg_color);
        crate::gui::draw_rect(px, py, pw, 1, border_col);
        crate::gui::draw_rect(px, py, 1, ph, border_col);
        crate::gui::draw_rect(px + pw - 1, py, 1, ph, border_col);
        crate::gui::draw_rect(px, py + ph - 1, pw, 1, border_col);

        // Header
        crate::gui::draw_rect(px + 2, py + 2, pw - 4, 22, header_bg);
        crate::gui::draw_string(px + 12, py + 7, "SparkOS Apps", accent_color, header_bg);

        // Registered App Items
        let mut item_y = layout.item_start_y as u16;
        for (idx, app) in crate::app_registry::REGISTERED_APPS.iter().enumerate() {
            let is_selected = idx == self.selected_index;
            let bg_col = if is_selected { accent_color } else { header_bg };
            let b_col = if is_selected { 0x0060A5FA } else { theme.border_color };
            let t_col = if is_selected { 0x00FFFFFF } else { text_color };

            crate::gui::draw_rect(px + 4, item_y, pw - 8, layout.item_height as u16, bg_col);
            crate::gui::draw_rect(px + 4, item_y, pw - 8, 1, b_col);
            crate::gui::draw_rect(px + 4, item_y, 1, layout.item_height as u16, b_col);
            crate::gui::draw_rect(px + pw - 5, item_y, 1, layout.item_height as u16, b_col);
            crate::gui::draw_rect(px + 4, item_y + (layout.item_height as u16 - 1), pw - 8, 1, b_col);

            // Dikey ortalanmış 16x16 HD ikon ve metin
            crate::gui::draw_icon_glyph(px + 8, item_y + 4, app.icon, t_col, bg_col);
            crate::gui::draw_string(px + 30, item_y + 6, app.name, t_col, bg_col);
            item_y += layout.item_spacing as u16;
        }

        // Close launcher button
        crate::gui::draw_rect(px + 4, item_y, pw - 8, layout.close_btn_height as u16, header_bg);
        crate::gui::draw_string(px + 44, item_y + 4, "Close Menu", text_muted, header_bg);
    }
}
