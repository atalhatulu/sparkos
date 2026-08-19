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
    }

    pub fn toggle_theme(&mut self) {
        THEME_MANAGER.lock().toggle_theme();
        let mut store = SETTINGS_STORE.lock();
        store.dark_theme = !store.dark_theme;
        self.status_message = format!("Theme toggled (Dark: {})", store.dark_theme);
    }

    pub fn handle_mouse_click(&mut self, local_x: u32, local_y: u32) {
        // 1. Sidebar clicks (x: 0..110)
        if local_x <= 110 {
            let item_height = 26u32;
            if local_y >= 36 && local_y < 36 + 5 * item_height {
                let idx = ((local_y - 36) / item_height) as usize;
                self.selected_nav_idx = idx;
                self.active_category = match idx {
                    0 => SettingsCategory::Appearance,
                    1 => SettingsCategory::Display,
                    2 => SettingsCategory::Desktop,
                    3 => SettingsCategory::Keyboard,
                    _ => SettingsCategory::System,
                };
                self.status_message = format!("Viewing {:?}", self.active_category);
                return;
            }
        }

        // 2. Action buttons in content pane
        if self.active_category == SettingsCategory::Appearance {
            // Toggle Theme button (x: 124..264, y: 108..132)
            if local_x >= 124 && local_x <= 264 && local_y >= 108 && local_y <= 132 {
                self.toggle_theme();
            }
        }
    }

    pub fn render_to_surface(&self, surface_ptr: *mut u32, w: u32, h: u32) {
        if surface_ptr.is_null() { return; }
        let bg_color = 0x000F172A;     // Deep Navy Slate
        let sidebar_bg = 0x00090E17;   // Darker Slate Sidebar
        let border_col = 0x00334155;
        let text_color = 0x00F8FAFC;
        let accent_color = 0x0038BDF8;
        let muted_color = 0x0094A3B8;

        crate::terminal_app::clear_surface(surface_ptr, w, h, bg_color);

        // 1. Left Flat Sidebar (x = 0..110)
        let sidebar_w = 110u32;
        let content_h = h.saturating_sub(18);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, 0, sidebar_w, content_h, sidebar_bg);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, sidebar_w - 1, 0, 1, content_h, border_col);

        crate::font::draw_text(surface_ptr, w, h, 10, 10, "SETTINGS", 0x0064748B, sidebar_bg);

        let categories = [
            (SettingsCategory::Appearance, "Appearance"),
            (SettingsCategory::Display, "Display"),
            (SettingsCategory::Desktop, "Desktop"),
            (SettingsCategory::Keyboard, "Shortcuts"),
            (SettingsCategory::System, "System"),
        ];

        let mut cat_y = 28u32;
        for (cat, label) in categories.iter() {
            let is_active = self.active_category == *cat;
            let item_bg = if is_active { 0x002563EB } else { 0x00131C2E };
            let item_fg = if is_active { 0x00FFFFFF } else { muted_color };

            crate::files_app::draw_surf_rect(surface_ptr, w, h, 6, cat_y, sidebar_w - 12, 22, item_bg);
            crate::font::draw_text(surface_ptr, w, h, 14, cat_y + 4, label, item_fg, item_bg);
            cat_y += 26;
        }

        // 2. Main Content Pane on Right (x = 116..w)
        let cx = sidebar_w + 10;
        let cy = 10u32;
        let main_w = w.saturating_sub(cx + 8);

        // Category Header Strip
        crate::files_app::draw_surf_rect(surface_ptr, w, h, cx, cy, main_w, 24, 0x001E293B);
        let cat_title = match self.active_category {
            SettingsCategory::Appearance => "Appearance & Theme",
            SettingsCategory::Display => "Display & Resolution",
            SettingsCategory::Desktop => "Desktop Configuration",
            SettingsCategory::Keyboard => "Global Shortcuts",
            SettingsCategory::System => "System Information",
        };
        crate::font::draw_text(surface_ptr, w, h, cx + 8, cy + 4, cat_title, accent_color, 0x001E293B);

        let card_y = cy + 32;

        match self.active_category {
            SettingsCategory::Appearance => {
                let theme_name = THEME_MANAGER.lock().current_theme.name;
                let theme_label = format!("Active Theme: {}", theme_name);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y, &theme_label, text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 20, "Accent Color: Sky Blue (#38BDF8)", text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 40, "Window Chrome: Flat 2D Modern", muted_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 60, "Top Panel: Unified Single Bar", muted_color, bg_color);

                // Theme Toggle Flat Button
                crate::files_app::draw_surf_rect(surface_ptr, w, h, cx, card_y + 86, 150, 24, 0x002563EB);
                crate::font::draw_text(surface_ptr, w, h, cx + 12, card_y + 90, "Toggle Dark / Light", 0x00FFFFFF, 0x002563EB);
            }
            SettingsCategory::Display => {
                let vesa_w = unsafe { crate::gui::VESA.width };
                let vesa_h = unsafe { crate::gui::VESA.height };
                let res_label = format!("Resolution: {}x{} @ 60 Hz", vesa_w, vesa_h);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y, &res_label, text_color, bg_color);

                let work_h = vesa_h.saturating_sub(crate::wm::WORK_AREA_TOP as u16 + 20);
                let work_label = format!("Work Area: {}x{}", vesa_w, work_h);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 20, &work_label, text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 40, "Driver: VESA Linear Framebuffer", muted_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 60, "Compositor: 60 FPS Damage Tracking", 0x0034D399, bg_color);
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
                    let kline = format!("• {}:", keys);
                    crate::font::draw_text(surface_ptr, w, h, cx, sy, &kline, 0x00F8FAFC, bg_color);
                    crate::font::draw_text(surface_ptr, w, h, cx + 12, sy + 14, desc, muted_color, bg_color);
                    sy += 28;
                }
            }
            SettingsCategory::System => {
                crate::font::draw_text(surface_ptr, w, h, cx, card_y, "OS: SparkOS Microkernel v2.0", 0x0034D399, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 20, "CPU: x86_64 SMP (Preemptive)", text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 40, "Memory: 256 MB Physical RAM", text_color, bg_color);
                let win_count = crate::wm::WM.lock().windows.len();
                let win_label = format!("Active Windows: {}", win_count);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 60, &win_label, text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, card_y + 80, "Security: Capability CSpace Model", text_color, bg_color);
            }
        }

        // 3. Status Bar at bottom
        let status_y = h.saturating_sub(18);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 18, 0x001E293B);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 1, border_col);
        crate::font::draw_text(surface_ptr, w, h, 8, status_y + 4, &self.status_message, 0x0034D399, 0x001E293B);
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
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frame for settings.app")?;
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
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        alloc::vec![],
    );

    let surf_id = crate::surface::create_surface_for_pid(pid, SETTINGS_WIDTH, SETTINGS_HEIGHT)?;
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

    Ok(pid)
}
