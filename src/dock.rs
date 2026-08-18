//! SparkOS Unified Top Panel 2.0 (Top Bar)
//!
//! Provides a single unified top panel replacing the bottom dock:
//! - Top-Left: Start / Launcher trigger button (`[ ❖ SparkOS ]`)
//! - Center-Left: Dynamic open window tabs (Active, Minimized, Focus states & icons)
//! - Top-Right: Real-time System HUD (HD Network Icon, Memory, Clock/Uptime)

use crate::wm::{Window, WindowState};
use crate::network_manager::{NetworkState, NETWORK_MANAGER};

pub struct Dock;

impl Dock {
    pub fn render(
        screen_w: u16,
        _screen_h: u16,
        windows: &[Window],
        focused_window: Option<u64>,
        launcher_open: bool,
        hovered_dock_tab: Option<usize>,
    ) {
        let top_bar_h = crate::wm::DOCK_HEIGHT; // 26px
        let active_theme = crate::theme::THEME_MANAGER.lock().current_theme;

        // 1. Top Bar background & bottom border
        crate::gui::draw_rect(0, 0, screen_w, top_bar_h, active_theme.dock_background);
        crate::gui::draw_rect(0, top_bar_h - 1, screen_w, 1, active_theme.border_color);

        // 2. Sol Üst: Başlat (Start / SparkOS) Butonu
        let launcher_bg = if launcher_open { 0x002563EB } else { 0x001E293B };
        crate::gui::draw_rect(4, 2, 88, 22, launcher_bg);
        crate::gui::draw_rect(4, 2, 88, 1, if launcher_open { 0x0060A5FA } else { 0x00334155 });
        crate::gui::draw_icon_glyph(8, 5, crate::app_registry::AppIcon::Logo, 0x0038BDF8, launcher_bg);
        crate::gui::draw_string(28, 7, "SparkOS", 0x00FFFFFF, launcher_bg);

        // Ayrıcı çizgi
        crate::gui::draw_rect(96, 3, 1, 20, 0x00334155);

        // 3. Orta-Sol: Açık Pencere Sekmeleri
        let mut tab_x = 102u16;
        let right_hud_w = 210u16;
        for (idx, win) in windows.iter().enumerate() {
            if tab_x + 90 > screen_w.saturating_sub(right_hud_w) {
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

            // Sekme arkaplanı ve çerçevesi
            crate::gui::draw_rect(tab_x, 2, 88, 22, tab_bg);
            let border_col = if is_focused { 0x0060A5FA } else { 0x00334155 };
            crate::gui::draw_rect(tab_x, 2, 88, 1, border_col);
            crate::gui::draw_rect(tab_x, 23, 88, 1, border_col);

            // Dikey ortalanmış 16x16 HD ikon ve başlık metni
            crate::gui::draw_icon_glyph(tab_x + 5, 5, win.icon, tab_fg, tab_bg);
            let short_title = if win.title.len() > 7 { &win.title[..7] } else { &win.title };
            crate::gui::draw_string(tab_x + 24, 7, short_title, tab_fg, tab_bg);

            // Aktif göstergesi
            if is_focused {
                crate::gui::draw_rect(tab_x + 36, 21, 16, 2, 0x0060A5FA);
            } else if win.state != WindowState::Minimized {
                crate::gui::draw_rect(tab_x + 40, 22, 8, 1, 0x0094A3B8);
            }

            tab_x += 92;
        }

        // 4. En Sağ Üst: Saat, Sistem Süresi ve Ağ/Bellek Bilgisi
        let hud_w = 206u16;
        let hud_x = screen_w.saturating_sub(hud_w + 4);
        crate::gui::draw_rect(hud_x, 2, hud_w, 22, 0x001E293B);
        crate::gui::draw_rect(hud_x, 2, hud_w, 1, 0x00334155);

        // HD Ağ İkonu (Ethernet / Wi-Fi / Bağlantı Yok)
        let is_connected = NETWORK_MANAGER.lock().state != NetworkState::Disconnected;
        let is_wifi = false; // Ethernet öncelikli
        crate::icons::render_network_icon(hud_x + 6, 5, is_wifi, is_connected);

        // Ağ etiketi
        let net_lbl = if is_connected { "Online" } else { "Offline" };
        let net_col = if is_connected { 0x0034D399 } else { 0x0094A3B8 };
        crate::gui::draw_string(hud_x + 24, 7, net_lbl, net_col, 0x001E293B);

        // Bellek
        let (used_mem, _) = crate::memory::get_memory_stats();
        let mem_str = alloc::format!("{}M", (used_mem / 1048576).max(1));
        crate::gui::draw_string(hud_x + 76, 7, &mem_str, 0x0038BDF8, 0x001E293B);

        // Saat / Süre (Uptime) Kutusu
        let ticks = crate::interrupts::get_tick();
        let seconds = ticks / 1000;
        let mins = seconds / 60;
        let s = seconds % 60;
        let time_str = alloc::format!("{:02}:{:02}s", mins, s);

        crate::gui::draw_rect(hud_x + 110, 3, 92, 20, 0x000F172A);
        crate::icons::render_icon_16(hud_x + 114, 5, crate::app_registry::AppIcon::SysMon, 0x000F172A);
        crate::gui::draw_string(hud_x + 134, 7, &time_str, 0x00F8FAFC, 0x000F172A);
    }
}
