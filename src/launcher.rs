//! SparkOS Launcher 2.0
//!
//! Provides dynamic application discovery via `app_registry`, keyboard navigation (Up/Down/Enter/Escape),
//! visual selection highlight, and zero-scheduler-lock rendering.

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

        let px = 4u16;
        let pw = 164u16;
        let total_apps = crate::app_registry::REGISTERED_APPS.len() as u16;
        let ph = 34 + total_apps * 28 + 26;
        let py = crate::wm::DOCK_HEIGHT + 2;

        // Background & Border
        crate::gui::draw_rect(px, py, pw, ph, 0x000F172A);
        crate::gui::draw_rect(px, py, pw, 1, 0x003B82F6);
        crate::gui::draw_rect(px, py, 1, ph, 0x003B82F6);
        crate::gui::draw_rect(px + pw - 1, py, 1, ph, 0x003B82F6);
        crate::gui::draw_rect(px, py + ph - 1, pw, 1, 0x003B82F6);

        // Header
        crate::gui::draw_rect(px + 2, py + 2, pw - 4, 22, 0x001E293B);
        crate::gui::draw_string(px + 8, py + 8, "SparkOS Launcher", 0x00FFFFFF, 0x001E293B);

        // Registered App Items
        let mut item_y = py + 28;
        for (idx, app) in crate::app_registry::REGISTERED_APPS.iter().enumerate() {
            let is_selected = idx == self.selected_index;
            let bg_col = if is_selected { 0x002563EB } else { 0x001E293B };
            let border_col = if is_selected { 0x0060A5FA } else { 0x00334155 };
            let text_col = if is_selected { 0x00FFFFFF } else { 0x00E2E8F0 };

            crate::gui::draw_rect(px + 4, item_y, pw - 8, 24, bg_col);
            crate::gui::draw_rect(px + 4, item_y, pw - 8, 1, border_col);
            crate::gui::draw_rect(px + 4, item_y, 1, 24, border_col);
            crate::gui::draw_rect(px + pw - 5, item_y, 1, 24, border_col);
            crate::gui::draw_rect(px + 4, item_y + 23, pw - 8, 1, border_col);

            crate::gui::draw_icon_glyph(px + 8, item_y + 8, app.icon, text_col, bg_col);
            crate::gui::draw_string(px + 24, item_y + 7, app.name, text_col, bg_col);
            item_y += 28;
        }

        // Close launcher button
        crate::gui::draw_rect(px + 4, item_y, pw - 8, 20, 0x00334155);
        crate::gui::draw_string(px + 40, item_y + 5, "Close Menu", 0x0094A3B8, 0x00334155);
    }
}
