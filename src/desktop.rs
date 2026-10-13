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
    OpenTerminal,
    OpenEditor,
    OpenTaskMgr,
    OpenSettings,
    OpenBrowser,
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
    pub icons: [DesktopIcon; 6],
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
                    label: "Files",
                    action: DesktopIconAction::OpenHome,
                },
                DesktopIcon {
                    id: 2,
                    position: (24, 115),
                    icon: AppIcon::Terminal,
                    label: "Terminal",
                    action: DesktopIconAction::OpenTerminal,
                },
                DesktopIcon {
                    id: 3,
                    position: (24, 190),
                    icon: AppIcon::Editor,
                    label: "Editor",
                    action: DesktopIconAction::OpenEditor,
                },
                DesktopIcon {
                    id: 4,
                    position: (24, 265),
                    icon: AppIcon::TaskMgr,
                    label: "TaskMgr",
                    action: DesktopIconAction::OpenTaskMgr,
                },
                DesktopIcon {
                    id: 5,
                    position: (24, 340),
                    icon: AppIcon::Settings,
                    label: "Settings",
                    action: DesktopIconAction::OpenSettings,
                },
                DesktopIcon {
                    id: 6,
                    position: (24, 415),
                    icon: AppIcon::Browser,
                    label: "Browser",
                    action: DesktopIconAction::OpenBrowser,
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

        // 2. Render Desktop Icons (Yalnızca CLIP_RECT ile kesişen simgeleri çiz)
        let clip = unsafe { crate::gui::CLIP_RECT };
        for icon in &self.icons {
            let (ix, iy) = icon.position;
            if ix + 60 >= screen_w || iy + 65 >= screen_h { continue; }

            // Damage Kırpma Optimizasyonu: Hasar bölgesinin dışındaki simgeleri atla
            if let Some((cx, cy, cw, ch)) = clip {
                let icon_right = ix + 64;
                let icon_bottom = iy + 64;
                let clip_right = cx + cw;
                let clip_bottom = cy + ch;
                if icon_right < cx || ix > clip_right || icon_bottom < cy || iy > clip_bottom {
                    continue;
                }
            }

            let is_selected = self.selected_icon_id == Some(icon.id);
            if is_selected {
                crate::gui::draw_rect_alpha(ix - 4, iy - 4, 64, 62, 0x003B82F6, 120);
                crate::gui::draw_rect(ix - 4, iy - 4, 64, 1, 0x0060A5FA);
                crate::gui::draw_rect(ix - 4, iy - 4, 1, 62, 0x0060A5FA);
                crate::gui::draw_rect(ix + 60 - 1, iy - 4, 1, 62, 0x0060A5FA);
                crate::gui::draw_rect(ix - 4, iy + 62 - 5, 64, 1, 0x0060A5FA);
            }

            // Draw Icon Glyph (Horizontally centered in 56px icon cell)
            let icon_center_x = ix + (56u16.saturating_sub(16)) / 2;
            crate::gui::draw_icon_glyph(icon_center_x, iy + 6, icon.icon, 0x0038BDF8, 0x00000000);

            // Draw Label with drop-shadow for crisp readability (Horizontally centered)
            let label_len = icon.label.len() as u16;
            let text_w = label_len * 8;
            let text_center_x = if text_w < 56 { ix + (56 - text_w) / 2 } else { ix };
            crate::gui::draw_string(text_center_x + 1, iy + 41, icon.label, 0x00000000, 0x00000000);
            crate::gui::draw_string(text_center_x, iy + 40, icon.label, 0x00F8FAFC, 0x00000000);
        }

        // 3. Desktop Resource Widget (Top-Right HUD - Yalnızca CLIP_RECT ile kesişiyorsa çiz)
        let widget_x = screen_w.saturating_sub(184);
        let widget_y = 36u16;
        let widget_w = 172u16;
        let widget_h = 70u16;

        let should_draw_widget = match clip {
            Some((cx, cy, cw, ch)) => {
                let w_right = widget_x + widget_w;
                let w_bottom = widget_y + widget_h;
                let c_right = cx + cw;
                let c_bottom = cy + ch;
                !(w_right < cx || widget_x > c_right || w_bottom < cy || widget_y > c_bottom)
            }
            None => true,
        };

        if should_draw_widget {
            let (used_mem, total_mem) = crate::memory::get_memory_stats();
            let used_mb = (used_mem / (1024 * 1024)).max(1);
            let total_mb = total_mem / (1024 * 1024);

            crate::gui::draw_rect_alpha(widget_x, widget_y, widget_w, widget_h, active_theme.dock_background, 210);
            crate::gui::draw_rect(widget_x, widget_y, widget_w, 1, active_theme.border_color);
            crate::gui::draw_rect(widget_x, widget_y, 1, widget_h, active_theme.border_color);
            crate::gui::draw_rect(widget_x + widget_w - 1, widget_y, 1, widget_h, active_theme.border_color);
            crate::gui::draw_rect(widget_x, widget_y + widget_h - 1, widget_w, 1, active_theme.border_color);

            let proc_count = crate::task::process::get_system_metrics_snapshot().len();

            let cpu_str = alloc::format!("CPU  {}% (PIDs: {})", (proc_count * 3).min(99), proc_count);
            let ram_str = alloc::format!("RAM  {} / {} MB", used_mb, total_mb);
            let disk_str = "DISK 12 / 64 MB";
            let gpu_str = "GPU  N/A";

            crate::gui::draw_string(widget_x + 8, widget_y + 6, &cpu_str, active_theme.accent_color, 0x00000000);
            crate::gui::draw_string(widget_x + 8, widget_y + 21, &ram_str, 0x0034D399, 0x00000000);
            crate::gui::draw_string(widget_x + 8, widget_y + 36, disk_str, active_theme.text_color, 0x00000000);
            crate::gui::draw_string(widget_x + 8, widget_y + 51, gpu_str, active_theme.text_muted, 0x00000000);
        }
    }

    /// Handles mouse clicks on desktop: single-click selects, double-click activates launcher action
    pub fn handle_mouse_click(&mut self, mx: u16, my: u16, current_tick: u64) -> Option<DesktopIconAction> {
        for icon in &self.icons {
            let (ix, iy) = icon.position;
            let in_x = mx >= ix.saturating_sub(6) && mx <= ix + 64;
            let in_y = my >= iy.saturating_sub(6) && my <= iy + 64;
            if in_x && in_y {
                // Check for double click (within 650 ticks / ~650ms)
                if self.last_click_id == Some(icon.id) && current_tick.saturating_sub(self.last_click_tick) <= 650 {
                    self.last_click_id = None;
                    self.last_click_tick = 0;
                    crate::serial_println!("[DESKTOP] Double-click activated icon '{}'", icon.label);
                    return Some(icon.action);
                } else {
                    // Single click: select icon
                    let prev_sel = self.selected_icon_id;
                    self.selected_icon_id = Some(icon.id);
                    self.last_click_id = Some(icon.id);
                    self.last_click_tick = current_tick;
                    if prev_sel != Some(icon.id) {
                        crate::wm::WM.lock().mark_damage(ix.saturating_sub(6) as i32, iy.saturating_sub(6) as i32, 72, 70);
                    }
                    return None;
                }
            }
        }

        // Clicked on empty desktop space: deselect
        if let Some(prev_id) = self.selected_icon_id {
            if let Some(icon) = self.icons.iter().find(|i| i.id == prev_id) {
                let (ix, iy) = icon.position;
                crate::wm::WM.lock().mark_damage(ix.saturating_sub(6) as i32, iy.saturating_sub(6) as i32, 72, 70);
            }
        }
        self.selected_icon_id = None;
        self.last_click_id = None;
        None
    }
}

pub static DESKTOP_ENV: Mutex<DesktopEnvironment> = Mutex::new(DesktopEnvironment::new());
