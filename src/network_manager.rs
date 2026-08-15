//! SparkOS Desktop V1.32 — Network Status Manager & System Tray (`src/network_manager.rs`)
//!
//! Provides real-time network state tracking (Ethernet, WiFi, Disconnected),
//! capability-gated IPC access, system tray popup rendering, and offline fail-safe behavior.

use alloc::string::String;
use spin::Mutex;
use crate::net::Ipv4Addr;
use crate::permission::AppPermission;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkState {
    Disconnected,
    Ethernet {
        interface: &'static str,
        ip: Ipv4Addr,
    },
    Wifi {
        ssid: &'static str,
        signal_strength: u8,
    },
}

pub struct NetworkManager {
    pub state: NetworkState,
    pub popup_open: bool,
    pub last_updated_tick: u64,
}

impl NetworkManager {
    pub const fn new() -> Self {
        Self {
            state: NetworkState::Ethernet {
                interface: "eth0 (RTL8139)",
                ip: [10, 0, 2, 15],
            },
            popup_open: false,
            last_updated_tick: 0,
        }
    }

    pub fn set_disconnected(&mut self) {
        self.state = NetworkState::Disconnected;
    }

    pub fn set_ethernet(&mut self, ip: Ipv4Addr) {
        self.state = NetworkState::Ethernet {
            interface: "eth0 (RTL8139)",
            ip,
        };
    }

    pub fn set_wifi(&mut self, ssid: &'static str, signal: u8) {
        self.state = NetworkState::Wifi {
            ssid,
            signal_strength: signal.min(100),
        };
    }

    pub fn toggle_popup(&mut self) {
        self.popup_open = !self.popup_open;
    }

    /// Status icon symbol for top bar
    pub fn get_icon_symbol(&self) -> &'static str {
        match self.state {
            NetworkState::Disconnected => "x",
            NetworkState::Ethernet { .. } => "[ETH]",
            NetworkState::Wifi { .. } => "[WIFI]",
        }
    }

    /// Capability-gated query for user-space applications
    pub fn get_network_state_for_app(&self, caller_pid: u64) -> Result<NetworkState, &'static str> {
        if crate::permission::PERMISSION_MANAGER.lock().check_permission(caller_pid, AppPermission::NetworkAccess).is_err() {
            crate::serial_println!("[NET-MANAGER] Security: PID {} lacks Network capability to query network state", caller_pid);
            return Err("PermissionDenied");
        }
        Ok(self.state.clone())
    }

    /// Renders network system tray popup if open
    pub fn render_popup(&self, screen_w: u16, top_bar_h: u16) {
        if !self.popup_open { return; }

        let pw = 160u16;
        let ph = 110u16;
        let px = screen_w.saturating_sub(pw + 10);
        let py = top_bar_h + 4;

        // Popup background and border
        crate::gui::draw_rect(px, py, pw, ph, 0x000F172A);
        crate::gui::draw_rect(px, py, pw, 1, 0x003B82F6);
        crate::gui::draw_rect(px, py, 1, ph, 0x003B82F6);
        crate::gui::draw_rect(px + pw - 1, py, 1, ph, 0x003B82F6);
        crate::gui::draw_rect(px, py + ph - 1, pw, 1, 0x003B82F6);

        // Header
        crate::gui::draw_rect(px + 2, py + 2, pw - 4, 20, 0x001E293B);
        crate::gui::draw_string(px + 8, py + 6, "Network Status", 0x00FFFFFF, 0x001E293B);

        // Content
        match &self.state {
            NetworkState::Disconnected => {
                crate::gui::draw_string(px + 12, py + 30, "Status: Disconnected", 0x00EF4444, 0x000F172A);
                crate::gui::draw_string(px + 12, py + 48, "No active network link", 0x0094A3B8, 0x000F172A);
            }
            NetworkState::Ethernet { interface, ip } => {
                crate::gui::draw_string(px + 12, py + 28, "Ethernet Connected", 0x0010B981, 0x000F172A);
                crate::gui::draw_string(px + 12, py + 44, interface, 0x0094A3B8, 0x000F172A);
                let ip_str = alloc::format!("IP: {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
                crate::gui::draw_string(px + 12, py + 60, &ip_str, 0x00E2E8F0, 0x000F172A);
            }
            NetworkState::Wifi { ssid, signal_strength } => {
                crate::gui::draw_string(px + 12, py + 28, "WiFi Connected", 0x0010B981, 0x000F172A);
                crate::gui::draw_string(px + 12, py + 44, ssid, 0x00E2E8F0, 0x000F172A);
                let sig_str = alloc::format!("Signal: {}%", signal_strength);
                crate::gui::draw_string(px + 12, py + 60, &sig_str, 0x0094A3B8, 0x000F172A);
            }
        }

        // Settings Button
        crate::gui::draw_rect(px + 8, py + 82, pw - 16, 20, 0x002563EB);
        crate::gui::draw_string(px + 36, py + 86, "[ Settings ]", 0x00FFFFFF, 0x002563EB);
    }
}

pub static NETWORK_MANAGER: Mutex<NetworkManager> = Mutex::new(NetworkManager::new());
