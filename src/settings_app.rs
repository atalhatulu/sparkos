//! SparkOS Desktop — Settings 2.0 Control Center (`settings.app`)
//!
//! Provides a categorized system configuration center:
//! - Appearance (Theme, Accent Color, Dock & Launcher visual settings)
//! - Display (Resolution, Screen & Work area bounds)
//! - Desktop (Wallpaper, Desktop Icons visibility)
//! - Keyboard (Global shortcut map)
//! - System (Kernel version, CPU, RAM, Uptime, Process & Window stats)
//!
//! Includes an extensible `SettingsStore` abstraction, isolated multi-instance states,
//! localized damage tracking, and zero-scheduler-lock rendering.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use spin::Mutex;
use crate::theme::THEME_MANAGER;

pub const SETTINGS_WIDTH: u32 = 440;
pub const SETTINGS_HEIGHT: u32 = 280;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsCategory {
    Appearance,
    Display,
    Desktop,
    Keyboard,
    System,
}

#[derive(Debug, Clone)]
pub struct SystemSettings {
    pub dark_theme: bool,
    pub accent_color: u32,
    pub wallpaper_name: &'static str,
    pub show_desktop_icons: bool,
    pub dock_auto_hide: bool,
}

impl Default for SystemSettings {
    fn default() -> Self {
        Self {
            dark_theme: true,
            accent_color: 0x0038BDF8,
            wallpaper_name: "Oceanic Spark Gradient",
            show_desktop_icons: true,
            dock_auto_hide: false,
        }
    }
}

pub static SETTINGS_STORE: Mutex<SystemSettings> = Mutex::new(SystemSettings {
    dark_theme: true,
    accent_color: 0x0038BDF8,
    wallpaper_name: "Oceanic Spark Gradient",
    show_desktop_icons: true,
    dock_auto_hide: false,
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction { None, Redraw, ThemeChanged }

pub struct SettingsLayout {
    pub sidebar_w: u32,
    pub nav_start_y: u32,
    pub nav_item_h: u32,
    pub theme_btn_x: u32,
    pub theme_btn_y: u32,
    pub theme_btn_w: u32,
    pub theme_btn_h: u32,
}

pub const SETTINGS_LAYOUT: SettingsLayout = SettingsLayout {
    sidebar_w: 110,
    nav_start_y: 28,
    nav_item_h: 26,
    theme_btn_x: 120, // cx = sidebar_w + 10 = 120
    theme_btn_y: 128, // cy (10) + 32 + 86 = 128
    theme_btn_w: 150,
    theme_btn_h: 24,
};

#[derive(Debug, Clone)]
pub struct SettingsAppState {
    pub window_id: u64,
    pub pid: u64,
    pub active_category: SettingsCategory,
    pub selected_nav_idx: usize,
    pub status_message: String,
}

impl SettingsAppState {
    pub fn new(window_id: u64, pid: u64) -> Self {
        Self {
            window_id,
            pid,
            active_category: SettingsCategory::Appearance,
            selected_nav_idx: 0,
            status_message: String::from("System Settings Ready"),
        }
    }

    pub fn set_category(&mut self, category: SettingsCategory) {
        self.active_category = category;
        self.selected_nav_idx = match category {
            SettingsCategory::Appearance => 0,
            SettingsCategory::Display => 1,
            SettingsCategory::Desktop => 2,
            SettingsCategory::Keyboard => 3,
            SettingsCategory::System => 4,
        };
        self.status_message = format!("Viewing {:?}", category);
    }

    pub fn nav_up(&mut self) {
        if self.selected_nav_idx == 0 {
            self.selected_nav_idx = 4;
        } else {
            self.selected_nav_idx -= 1;
        }
        self.active_category = match self.selected_nav_idx {
            0 => SettingsCategory::Appearance,
            1 => SettingsCategory::Display,
            2 => SettingsCategory::Desktop,
            3 => SettingsCategory::Keyboard,
            _ => SettingsCategory::System,
        };
        self.status_message = format!("Viewing {:?}", self.active_category);
    }

    pub fn nav_down(&mut self) {
        self.selected_nav_idx = (self.selected_nav_idx + 1) % 5;
        self.active_category = match self.selected_nav_idx {
            0 => SettingsCategory::Appearance,
            1 => SettingsCategory::Display,
            2 => SettingsCategory::Desktop,
            3 => SettingsCategory::Keyboard,
            _ => SettingsCategory::System,
        };
        self.status_message = format!("Viewing {:?}", self.active_category);
    }

    pub fn toggle_theme(&mut self) {
        let mut tm = THEME_MANAGER.lock();
        tm.toggle_theme();
        let is_dark = tm.current_mode == crate::theme::ThemeMode::SparkDark;
        drop(tm);
        let mut store = SETTINGS_STORE.lock();
        store.dark_theme = is_dark;
        self.status_message = format!("Theme toggled (Dark: {})", store.dark_theme);
    }

    pub fn handle_mouse_click(&mut self, local_x: u32, local_y: u32) -> SettingsAction {
        let layout = &SETTINGS_LAYOUT;
        // The four-pixel gap between rows is not a clickable category.
        if local_x >= 6 && local_x < layout.sidebar_w - 6 && local_y >= layout.nav_start_y {
            let offset = local_y - layout.nav_start_y;
            let idx = (offset / layout.nav_item_h) as usize;
            if idx < 5 && offset % layout.nav_item_h < 22 {
                self.set_category([SettingsCategory::Appearance, SettingsCategory::Display,
                    SettingsCategory::Desktop, SettingsCategory::Keyboard, SettingsCategory::System][idx]);
                return SettingsAction::Redraw;
            }
        }
        if self.active_category == SettingsCategory::Appearance
            && local_x >= layout.theme_btn_x && local_x < layout.theme_btn_x + layout.theme_btn_w
            && local_y >= layout.theme_btn_y && local_y < layout.theme_btn_y + layout.theme_btn_h {
            self.toggle_theme();
            return SettingsAction::ThemeChanged;
        }
        SettingsAction::None
    }

    pub fn handle_key_input(&mut self, key_code: u8, pressed: bool) -> SettingsAction {
        if !pressed { return SettingsAction::None; }
        match key_code {
            0x48 => { self.nav_up(); SettingsAction::Redraw }
            0x50 => { self.nav_down(); SettingsAction::Redraw }
            0x1C | 0x39 if self.active_category == SettingsCategory::Appearance => {
                self.toggle_theme();
                SettingsAction::ThemeChanged
            }
            _ => SettingsAction::None,
        }
    }

    pub fn render_to_surface(&self, surface_ptr: *mut u32, w: u32, h: u32) {
        let _timing = crate::perf::Span::new(crate::perf::Metric::Settings);
        if surface_ptr.is_null() { return; }
        let theme = THEME_MANAGER.lock().current_theme;
        let bg_color = theme.window_active;
        let sidebar_bg = theme.panel_background;
        let border_col = theme.border_color;
        let text_color = theme.text_color;
        let accent_color = theme.accent_color;
        let muted_color = theme.text_muted;

        crate::terminal_app::clear_surface(surface_ptr, w, h, bg_color);

        // 1. Left Flat Sidebar (x = 0..sidebar_w)
        let sidebar_w = SETTINGS_LAYOUT.sidebar_w;
        let content_h = h.saturating_sub(18);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, 0, sidebar_w, content_h, sidebar_bg);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, sidebar_w - 1, 0, 1, content_h, border_col);

        crate::font::draw_text(surface_ptr, w, h, 10, 10, "SETTINGS", muted_color, sidebar_bg);

        let categories = [
            (SettingsCategory::Appearance, "Appearance"),
            (SettingsCategory::Display, "Display"),
            (SettingsCategory::Desktop, "Desktop"),
            (SettingsCategory::Keyboard, "Shortcuts"),
            (SettingsCategory::System, "System"),
        ];

        let mut cat_y = SETTINGS_LAYOUT.nav_start_y;
        for (cat, label) in categories.iter() {
            let is_active = self.active_category == *cat;
            let item_bg = if is_active { accent_color } else { sidebar_bg };
            let item_fg = if is_active { 0x00FFFFFF } else { muted_color };

            crate::files_app::draw_surf_rect(surface_ptr, w, h, 6, cat_y, sidebar_w - 12, 22, item_bg);
            crate::font::draw_text(surface_ptr, w, h, 14, cat_y + 4, label, item_fg, item_bg);
            cat_y += SETTINGS_LAYOUT.nav_item_h;
        }

        // 2. Main Content Pane on Right (x = sidebar_w + 10 .. w)
        let cx = sidebar_w + 10;
        let cy = 10u32;
        let main_w = w.saturating_sub(cx + 8);

        // Category Header Strip
        crate::files_app::draw_surf_rect(surface_ptr, w, h, cx, cy, main_w, 24, sidebar_bg);
        let cat_title = match self.active_category {
            SettingsCategory::Appearance => "Appearance & Theme",
            SettingsCategory::Display => "Display & Resolution",
            SettingsCategory::Desktop => "Desktop Configuration",
            SettingsCategory::Keyboard => "Global Shortcuts",
            SettingsCategory::System => "System Information",
        };
        crate::font::draw_text(surface_ptr, w, h, cx + 8, cy + 4, cat_title, accent_color, sidebar_bg);

        let card_y = cy + 32;

        match self.active_category {
            SettingsCategory::Appearance => {
                let theme_name = theme.name;
                let theme_label = format!("Active Theme: {}", theme_name);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y, &theme_label, text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 20, &format!("Accent Color: #{:06X}", accent_color), text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 40, "Window Chrome: Flat 2D Modern", muted_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 60, "Top Panel: Unified Single Bar", muted_color, bg_color);

                // Theme Toggle Flat Button (from layout)
                let bx = SETTINGS_LAYOUT.theme_btn_x;
                let by = SETTINGS_LAYOUT.theme_btn_y;
                let bw = SETTINGS_LAYOUT.theme_btn_w;
                let bh = SETTINGS_LAYOUT.theme_btn_h;
                crate::files_app::draw_surf_rect(surface_ptr, w, h, bx, by, bw, bh, accent_color);
                crate::font::draw_text(surface_ptr, w, h, bx + 12, by + 4, "Switch theme", 0x00FFFFFF, accent_color);
            }
            SettingsCategory::Display => {
                let vesa_w = unsafe { crate::gui::VESA.width };
                let vesa_h = unsafe { crate::gui::VESA.height };
                let res_label = format!("Resolution: {}x{}", vesa_w, vesa_h);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y, &res_label, text_color, bg_color);

                let work_h = vesa_h.saturating_sub(crate::wm::WORK_AREA_TOP as u16 + 20);
                let work_label = format!("Work Area: {}x{}", vesa_w, work_h);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 20, &work_label, text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 40, "Driver: VESA Linear Framebuffer", muted_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 60, "Compositor: Damage Tracking", 0x0034D399, bg_color);
            }
            SettingsCategory::Desktop => {
                let store = SETTINGS_STORE.lock();
                let wp_label = format!("Wallpaper: {}", store.wallpaper_name);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y, &wp_label, text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 20, "Desktop Icons: Enabled (Visible)", text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 40, "Top Panel: Top Taskbar & Launcher", muted_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 60, "Launcher: Full Application Menu", muted_color, bg_color);
            }
            SettingsCategory::Keyboard => {
                let shortcuts = [
                    ("F1 / Ctrl+Alt+T", "Spawn Terminal Instance"),
                    ("Alt + Tab", "MRU Window Switcher"),
                    ("Ctrl + Escape", "Application Launcher"),
                    ("Alt + F4", "Close Focused Window"),
                ];
                let mut sy = card_y;
                for (keys, desc) in shortcuts.iter() {
                    let kline = format!("{}:", keys);
                    crate::font::draw_text(surface_ptr, w, h, cx, sy, &kline, text_color, bg_color);
                    crate::font::draw_text(surface_ptr, w, h, cx + 12, sy + 14, desc, muted_color, bg_color);
                    sy += 28;
                }
            }
            SettingsCategory::System => {
                crate::font::draw_text(surface_ptr, w, h, cx, card_y, "OS: SparkOS Microkernel v2.0", 0x0034D399, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 20, "Architecture: x86_64", text_color, bg_color);
                // Rendering also runs with WM held during resize. Never re-enter WM here.
                let (used, capacity) = crate::memory::get_memory_stats();
                let memory_label = format!("Frame pool: {} / {} MiB", used / 1048576, capacity / 1048576);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 40, &memory_label, text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 60, "Built-in desktop: kernel mode", text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 80, "Security: Capability CSpace Model", text_color, bg_color);
            }
        }

        // 3. Status Bar at bottom
        let status_y = h.saturating_sub(18);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 18, sidebar_bg);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 1, border_col);
        crate::font::draw_text(surface_ptr, w, h, 8, status_y + 4, &self.status_message, accent_color, sidebar_bg);
    }
}

pub static SETTINGS_INSTANCES: Mutex<BTreeMap<u64, SettingsAppState>> = Mutex::new(BTreeMap::new());

pub fn cleanup_settings_for_window(window_id: u64) {
    let mut instances = SETTINGS_INSTANCES.lock();
    if instances.remove(&window_id).is_some() {
        crate::serial_println!("[SETTINGS] Cleaned up Settings state for Window {}", window_id);
    }
}

pub fn spawn_settings_app(name: &str) -> Result<u64, &'static str> {
    let address_space = crate::memory::AddressSpaceBuilder::new()?;
    let cr3 = address_space.cr3();
    let code = crate::terminal_app::terminal_machine_code();
    let code_base = crate::memory::USER_ADDR_BASE;
    crate::memory::map_user_region_in_cr3(cr3, code_base, 0x3000, true)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &code, 0x1000);

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let pid = crate::task::process::create_user_process_with_caps(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        address_space.commit(),
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        alloc::vec![],
    );
    let pending = crate::task::process::PendingProcess::new(pid);

    let surf_id = crate::surface::create_surface_for_pid(pid, crate::surface::MAX_SURFACE_WIDTH, crate::surface::MAX_SURFACE_HEIGHT)?;
    if let Some(s) = crate::surface::SURFACE_REGISTRY.write().iter_mut().find(|s| s.surface_id == surf_id) {
        s.width = SETTINGS_WIDTH;
        s.height = SETTINGS_HEIGHT;
    }
    let win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, 80, 80, SETTINGS_WIDTH, SETTINGS_HEIGHT)
        .map_err(|_| "window creation failed")?;

    {
        let state = SettingsAppState::new(win_id, pid);
        if let Some(surface) = crate::surface::SURFACE_REGISTRY.read().iter().find(|s| s.surface_id == surf_id) {
            let phys_addr = surface.shmem_phys_addr;
            let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *mut u32 };
            state.render_to_surface(surf_ptr, SETTINGS_WIDTH, SETTINGS_HEIGHT);
        }
        SETTINGS_INSTANCES.lock().insert(win_id, state);
    }

    let _ = crate::surface::present_surface(surf_id, 0, 0, SETTINGS_WIDTH, SETTINGS_HEIGHT);
    crate::serial_println!("[APP-REGISTRY] Successfully launched '{}' (PID {}, Entry 0x{:x}, Surface {}, Window {})",
        name, pid, code_base, surf_id, win_id);

    crate::input::register_kernel_input_owner(pid);
    pending.commit();
    Ok(pid)
}
