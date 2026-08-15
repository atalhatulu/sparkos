//! SparkOS Desktop V1.35 — Modern Settings Control Center (`settings.app`)
//!
//! Provides a full-featured multi-pane system configuration center featuring Appearance,
//! Network, System Information, Application Permissions reviewer, and About SparkOS panels.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::libspark_ui::{Button, Label, Widget};
use crate::network_manager::NETWORK_MANAGER;
use crate::theme::THEME_MANAGER;

pub const SETTINGS_WIDTH: u32 = 420;
pub const SETTINGS_HEIGHT: u32 = 260;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Appearance,
    Network,
    System,
    Applications,
    About,
}

pub struct SettingsState {
    pub active_tab: SettingsTab,
    pub accent_color: u32,
    pub wallpaper_name: &'static str,
}

impl SettingsState {
    pub const fn new() -> Self {
        Self {
            active_tab: SettingsTab::Appearance,
            accent_color: 0x0038BDF8,
            wallpaper_name: "Oceanic Spark Gradient",
        }
    }

    pub fn set_tab(&mut self, tab: SettingsTab) {
        self.active_tab = tab;
    }

    pub fn toggle_theme(&mut self) {
        THEME_MANAGER.lock().toggle_theme();
    }

    pub fn render_to_surface(&self, surface_ptr: *mut u32, w: u32, h: u32) {
        if surface_ptr.is_null() { return; }
        let bg_color = 0x000F172A; // Navy Slate
        let sidebar_bg = 0x001E293B; // Slate 800
        let text_color = 0x00F8FAFC;
        let accent_color = self.accent_color;
        let muted_color = 0x0094A3B8;

        crate::terminal_app::clear_surface(surface_ptr, w, h, bg_color);

        // 1. Sidebar (Tabs Navigation on Left: w = 110)
        let sidebar_w = 110u32;
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, 0, sidebar_w, h, sidebar_bg);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, sidebar_w - 1, 0, 1, h, 0x00334155);

        // Header
        crate::font::draw_text(surface_ptr, w, h, 12, 10, "Settings", 0x0038BDF8, sidebar_bg);

        let tabs = [
            (SettingsTab::Appearance, "Appearance"),
            (SettingsTab::Network, "Network"),
            (SettingsTab::System, "System"),
            (SettingsTab::Applications, "Apps"),
            (SettingsTab::About, "About"),
        ];

        let mut tab_y = 36u32;
        for (tab, label) in tabs.iter() {
            let is_active = self.active_tab == *tab;
            let item_bg = if is_active { 0x002563EB } else { sidebar_bg };
            let item_fg = if is_active { 0x00FFFFFF } else { muted_color };

            crate::files_app::draw_surf_rect(surface_ptr, w, h, 6, tab_y, sidebar_w - 12, 22, item_bg);
            crate::font::draw_text(surface_ptr, w, h, 14, tab_y + 4, label, item_fg, item_bg);
            tab_y += 26;
        }

        // 2. Main Content Pane on Right (x = 120..w)
        let cx = 124u32;
        let cy = 12u32;

        match self.active_tab {
            SettingsTab::Appearance => {
                crate::font::draw_text(surface_ptr, w, h, cx, cy, "Appearance Settings", 0x0038BDF8, bg_color);
                let theme_name = THEME_MANAGER.lock().current_theme.name;
                let theme_label = format!("Active Theme: {}", theme_name);
                crate::font::draw_text(surface_ptr, w, h, cx, cy + 26, &theme_label, text_color, bg_color);

                // Accent Color & Wallpaper
                let accent_label = "Accent Color: Sky Blue (#38BDF8)";
                crate::font::draw_text(surface_ptr, w, h, cx, cy + 46, accent_label, text_color, bg_color);

                let wp_label = format!("Wallpaper: {}", self.wallpaper_name);
                crate::font::draw_text(surface_ptr, w, h, cx, cy + 66, &wp_label, text_color, bg_color);

                // Theme Toggle Action Button
                crate::files_app::draw_surf_rect(surface_ptr, w, h, cx, cy + 96, 140, 24, 0x002563EB);
                crate::font::draw_text(surface_ptr, w, h, cx + 18, cy + 100, "[ Toggle Theme ]", 0x00FFFFFF, 0x002563EB);
            }
            SettingsTab::Network => {
                crate::font::draw_text(surface_ptr, w, h, cx, cy, "Network Configuration", 0x0038BDF8, bg_color);
                let net_state = NETWORK_MANAGER.lock().state.clone();
                match net_state {
                    crate::network_manager::NetworkState::Disconnected => {
                        crate::font::draw_text(surface_ptr, w, h, cx, cy + 26, "Status: Disconnected", 0x00EF4444, bg_color);
                        crate::font::draw_text(surface_ptr, w, h, cx, cy + 46, "Hardware: RTL8139 NIC (Idle)", muted_color, bg_color);
                    }
                    crate::network_manager::NetworkState::Ethernet { interface, ip } => {
                        crate::font::draw_text(surface_ptr, w, h, cx, cy + 26, "Status: Connected (Ethernet)", 0x0010B981, bg_color);
                        let if_str = format!("Interface: {}", interface);
                        crate::font::draw_text(surface_ptr, w, h, cx, cy + 46, &if_str, text_color, bg_color);
                        let ip_str = format!("IPv4 Address: {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
                        crate::font::draw_text(surface_ptr, w, h, cx, cy + 66, &ip_str, text_color, bg_color);
                    }
                    crate::network_manager::NetworkState::Wifi { ssid, signal_strength } => {
                        crate::font::draw_text(surface_ptr, w, h, cx, cy + 26, "Status: Connected (WiFi)", 0x0010B981, bg_color);
                        let ssid_str = format!("SSID: {}", ssid);
                        crate::font::draw_text(surface_ptr, w, h, cx, cy + 46, &ssid_str, text_color, bg_color);
                        let sig_str = format!("Signal: {}%", signal_strength);
                        crate::font::draw_text(surface_ptr, w, h, cx, cy + 66, &sig_str, text_color, bg_color);
                    }
                }
            }
            SettingsTab::System => {
                crate::font::draw_text(surface_ptr, w, h, cx, cy, "System Information", 0x0038BDF8, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, cy + 26, "CPU: x86_64 SMP (2 Cores Active)", text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, cy + 46, "RAM: 256 MB Total | 43 MB Used", text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, cy + 66, "Kernel: SparkOS Microkernel v1.35", 0x0034D399, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, cy + 86, "Desktop: SparkDesktop V1.35", 0x0038BDF8, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, cy + 106, "Isolation: CSpace Capabilities + CR3", text_color, bg_color);
            }
            SettingsTab::Applications => {
                crate::font::draw_text(surface_ptr, w, h, cx, cy, "Installed Applications & Permissions", 0x0038BDF8, bg_color);
                let apps = [
                    ("terminal.app", "FS_Read, FS_Write, Term_IO"),
                    ("files.app", "FS_Read, FS_Write, SPFS_IPC"),
                    ("browser.app", "NetworkAccess, HTTP_IPC"),
                    ("taskmgr.app", "Process_Enum, Task_Kill"),
                ];
                let mut app_y = cy + 24;
                for (app_name, perms) in apps.iter() {
                    let app_line = format!("• {}:", app_name);
                    crate::font::draw_text(surface_ptr, w, h, cx, app_y, &app_line, 0x00F8FAFC, bg_color);
                    crate::font::draw_text(surface_ptr, w, h, cx + 12, app_y + 14, perms, muted_color, bg_color);
                    app_y += 30;
                }
            }
            SettingsTab::About => {
                crate::font::draw_text(surface_ptr, w, h, cx, cy, "About SparkOS", 0x0038BDF8, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, cy + 26, "SparkOS Microkernel Operating System", 0x00FFFFFF, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, cy + 46, "Version: 1.35.0 (Release-Ready)", 0x0034D399, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, cy + 66, "Architecture: x86_64 SMP (Multi-Core)", text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, cy + 86, "Security Model: Capability-Based CSpace", text_color, bg_color);
                crate::font::draw_text(surface_ptr, w, h, cx, cy + 106, "License: MIT Open Source", muted_color, bg_color);
            }
        }
    }
}

pub fn render_settings_surface(surface_ptr: *mut u32, w: u32, h: u32) {
    let state = SettingsState::new();
    state.render_to_surface(surface_ptr, w, h);
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
    let _win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, 80, 80, SETTINGS_WIDTH, SETTINGS_HEIGHT)
        .map_err(|_| "window creation failed")?;

    if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.surface_id == surf_id) {
        let phys_addr = surface.shmem_phys_addr;
        let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *mut u32 };
        let state = SettingsState::new();
        state.render_to_surface(surf_ptr, SETTINGS_WIDTH, SETTINGS_HEIGHT);
    }

    let _ = crate::surface::present_surface(surf_id, 0, 0, SETTINGS_WIDTH, SETTINGS_HEIGHT);
    crate::serial_println!("[APP-REGISTRY] Successfully launched '{}' (PID {}, Entry 0x{:x}, Surface {}, Window)",
        name, pid, code_base, surf_id);

    Ok(pid)
}
