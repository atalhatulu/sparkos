//! SparkOS — Window Manager, Compositor, Dock & Launcher Subsystem (Desktop V1.1 / Steps 1-6)
//!
//! Provides Ring-3 / Microkernel Window Management, Z-Order Back-to-Front Compositing,
//! Window Chrome (Titlebar & Buttons), Window Geometry & Resize Hardening, Bottom Dock,
//! Application Launcher, Application Registry, and Icon Rendering.

use alloc::vec::Vec;
use spin::Mutex;

pub const WORK_AREA_TOP: i32 = 24;
pub const DOCK_HEIGHT: u16 = 24;
pub const MIN_WINDOW_WIDTH: u32 = 120;
pub const MIN_WINDOW_HEIGHT: u32 = 60;
pub const EDGE_SNAP_THRESHOLD: i32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WmError {
    InvalidDimensions,
    NotFound,
    PermissionDenied,
    SurfaceNotFound,
    StaleHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Minimized,
    Maximized,
    Fullscreen,
    SnappedLeft,
    SnappedRight,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    None,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeButton {
    None,
    Minimize,
    Maximize,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoverTarget {
    pub window_id: u64,
    pub button: ChromeButton,
    pub is_titlebar: bool,
    pub resize_edge: ResizeEdge,
}

#[derive(Debug, Clone)]
pub struct Window {
    pub window_id: u64,
    pub owner_pid: u64,
    pub surface_id: u64,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub visible: bool,
    pub focused: bool,
    pub state: WindowState,
    pub normal_geom: (i32, i32, u32, u32),
    pub prev_state: WindowState,
    pub saved_geom: Option<(i32, i32, u32, u32)>,
    pub title: alloc::string::String,
    pub icon: crate::app_registry::AppIcon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapPreview {
    None,
    Left,
    Right,
    Maximized,
}

#[derive(Debug, Clone)]
pub struct AltTabSwitcher {
    pub active: bool,
    pub selected_index: usize,
    pub candidates: Vec<u64>,
}

impl AltTabSwitcher {
    pub const fn new() -> Self {
        Self {
            active: false,
            selected_index: 0,
            candidates: Vec::new(),
        }
    }
}

pub struct WindowManager {
    /// Windows ordered from Back (index 0) to Front (index len - 1).
    pub windows: Vec<Window>,
    pub next_window_id: u64,
    pub focused_window: Option<u64>,
    pub mru_list: Vec<u64>,
    pub alt_tab: AltTabSwitcher,
    pub snap_preview: SnapPreview,
    pub dragging_window: Option<(u64, i32, i32)>,
    pub resizing_window: Option<(u64, ResizeEdge, i32, i32, i32, i32, u32, u32)>,
    pub hovered_target: Option<HoverTarget>,
    pub hovered_dock_tab: Option<usize>,
    pub launcher_open: bool,
    pub launcher_selected: usize,
    pub pending_spawn_app: Option<u8>,
    pub last_titlebar_click: Option<(u64, u64)>, // (window_id, tick)
    pub damage_tracker: crate::damage::DamageTracker,
}

impl WindowManager {
    pub const fn new() -> Self {
        Self {
            windows: Vec::new(),
            next_window_id: 1,
            focused_window: None,
            mru_list: Vec::new(),
            alt_tab: AltTabSwitcher::new(),
            snap_preview: SnapPreview::None,
            dragging_window: None,
            resizing_window: None,
            hovered_target: None,
            hovered_dock_tab: None,
            launcher_open: false,
            launcher_selected: 0,
            pending_spawn_app: None,
            last_titlebar_click: None,
            damage_tracker: crate::damage::DamageTracker::new(),
        }
    }

    pub fn launcher_nav_up(&mut self) {
        let total = crate::app_registry::REGISTERED_APPS.len();
        if total > 0 {
            if self.launcher_selected == 0 {
                self.launcher_selected = total - 1;
            } else {
                self.launcher_selected -= 1;
            }
            let screen_h = unsafe { crate::gui::VESA.height as i32 };
            let dock_y = screen_h.saturating_sub(DOCK_HEIGHT as i32);
            self.mark_damage(4, (dock_y - 250).max(0), 164, 260);
        }
    }

    pub fn launcher_nav_down(&mut self) {
        let total = crate::app_registry::REGISTERED_APPS.len();
        if total > 0 {
            self.launcher_selected = (self.launcher_selected + 1) % total;
            let screen_h = unsafe { crate::gui::VESA.height as i32 };
            let dock_y = screen_h.saturating_sub(DOCK_HEIGHT as i32);
            self.mark_damage(4, (dock_y - 250).max(0), 164, 260);
        }
    }

    pub fn launcher_get_selected_app(&self) -> Option<u8> {
        let total = crate::app_registry::REGISTERED_APPS.len();
        if self.launcher_open && self.launcher_selected < total {
            Some(crate::app_registry::REGISTERED_APPS[self.launcher_selected].id)
        } else {
            None
        }
    }

    /// Marks a generic rectangular region as damaged.
    pub fn mark_damage(&mut self, x: i32, y: i32, width: u32, height: u32) {
        self.damage_tracker.add_bounds(x, y, width, height);
    }

    /// Marks an entire window area (including titlebar, borders, and margins) as damaged.
    pub fn mark_window_damage(&mut self, x: i32, y: i32, width: u32, height: u32) {
        self.damage_tracker.add_bounds(x.saturating_sub(4), y.saturating_sub(4), width + 8, height + 28);
    }

    /// Forces a full-screen redraw on next composition cycle.
    pub fn mark_full_damage(&mut self) {
        let sw = unsafe { crate::gui::VESA.width as u32 };
        let sh = unsafe { crate::gui::VESA.height as u32 };
        self.damage_tracker.add_full_screen(sw, sh);
    }

    /// Marks the unified top panel as damaged.
    pub fn mark_top_bar_damage(&mut self) {
        let max_w = unsafe { crate::gui::VESA.width as u32 };
        self.mark_damage(0, 0, max_w, DOCK_HEIGHT as u32 + 2);
    }

    /// Creates a new window with custom metadata (title and icon).
    pub fn create_window_with_meta(
        &mut self,
        owner_pid: u64,
        surface_id: u64,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        title: alloc::string::String,
        icon: crate::app_registry::AppIcon,
    ) -> Result<u64, WmError> {
        let max_w = unsafe { crate::gui::VESA.width as u32 };
        let max_h = unsafe { crate::gui::VESA.height as u32 };
        if width == 0 || height == 0 || width > max_w || height > max_h {
            return Err(WmError::InvalidDimensions);
        }

        // Hardening Check: Verify surface ownership if a surface is bound
        if surface_id != 0 {
            let reg = crate::surface::SURFACE_REGISTRY.read();
            let is_owner = reg.iter().any(|s| s.surface_id == surface_id && s.owner_pid == owner_pid);
            if !is_owner {
                return Err(WmError::SurfaceNotFound);
            }
        }

        let clamped_w = width.clamp(MIN_WINDOW_WIDTH, max_w);
        let clamped_h = height.clamp(MIN_WINDOW_HEIGHT, max_h.saturating_sub(WORK_AREA_TOP as u32 + DOCK_HEIGHT as u32));
        let clamped_x = x.clamp(0, (max_w.saturating_sub(clamped_w)) as i32);
        let clamped_y = y.clamp(WORK_AREA_TOP, (max_h.saturating_sub(clamped_h + DOCK_HEIGHT as u32 + 20)) as i32);

        let window_id = self.next_window_id;
        self.next_window_id += 1;

        // Reset previous focus
        for w in self.windows.iter_mut() {
            w.focused = false;
        }

        let win = Window {
            window_id,
            owner_pid,
            surface_id,
            x: clamped_x,
            y: clamped_y,
            width: clamped_w,
            height: clamped_h,
            visible: true,
            focused: true,
            state: WindowState::Normal,
            normal_geom: (clamped_x, clamped_y, clamped_w, clamped_h),
            prev_state: WindowState::Normal,
            saved_geom: None,
            title,
            icon,
        };

        // Appended at the end (top-most in Z-order)
        self.windows.push(win);
        self.focused_window = Some(window_id);
        self.touch_mru(window_id);

        self.mark_window_damage(clamped_x, clamped_y, clamped_w, clamped_h);
        self.mark_top_bar_damage();

        // Register window attachment in process PCB
        {
            let mut sched = crate::task::process::SCHEDULER.lock();
            if let Some(proc) = sched.get_process_mut(owner_pid) {
                let _ = proc.increment_window_count();
                if !proc.owned_windows.contains(&window_id) {
                    proc.owned_windows.push(window_id);
                }
            }
        }

        crate::serial_println!("[WM] Process {} created Window {} (surface {}, [{}, {}, {}, {}], FOCUSED)",
            owner_pid, window_id, surface_id, clamped_x, clamped_y, clamped_w, clamped_h);

        Ok(window_id)
    }

    /// Creates a new window associated with an existing Surface, resolving metadata automatically from PCB.
    pub fn create_window(
        &mut self,
        owner_pid: u64,
        surface_id: u64,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<u64, WmError> {
        let (title, icon) = {
            let sched = crate::task::process::SCHEDULER.lock();
            if let Some(p) = sched.get_process(owner_pid) {
                let icon = match p.name.as_str() {
                    "terminal.app" => crate::app_registry::AppIcon::Terminal,
                    "files.app" => crate::app_registry::AppIcon::Files,
                    "editor.app" => crate::app_registry::AppIcon::Editor,
                    "taskmgr.app" => crate::app_registry::AppIcon::TaskMgr,
                    "sysmon.app" => crate::app_registry::AppIcon::SysMon,
                    "settings.app" => crate::app_registry::AppIcon::Settings,
                    "browser.app" => crate::app_registry::AppIcon::Browser,
                    "live_demo_app" => crate::app_registry::AppIcon::Demo,
                    _ => crate::app_registry::AppIcon::Generic,
                };
                let title = match p.name.as_str() {
                    "terminal.app" => alloc::string::String::from("Terminal"),
                    "files.app" => alloc::string::String::from("Files"),
                    "editor.app" => alloc::string::String::from("Text Editor"),
                    "taskmgr.app" => alloc::string::String::from("Task Manager"),
                    "sysmon.app" => alloc::string::String::from("System Monitor"),
                    "settings.app" => alloc::string::String::from("Settings"),
                    "browser.app" => alloc::string::String::from("Web Browser"),
                    "live_demo_app" => alloc::string::String::from("Demo App"),
                    _ => alloc::string::String::from("SparkOS Application"),
                };
                (title, icon)
            } else {
                (alloc::string::String::from("SparkOS Application"), crate::app_registry::AppIcon::Generic)
            }
        };

        self.create_window_with_meta(owner_pid, surface_id, x, y, width, height, title, icon)
    }

    /// Updates the title of an existing window and marks it as damaged.
    pub fn set_window_title(&mut self, window_id: u64, title: alloc::string::String) -> Result<(), WmError> {
        let (wx, wy, ww, wh) = {
            let win = self.windows.iter_mut().find(|w| w.window_id == window_id).ok_or(WmError::NotFound)?;
            win.title = title;
            (win.x, win.y, win.width, win.height)
        };
        self.mark_window_damage(wx, wy, ww, wh);
        Ok(())
    }

    /// Minimizes a window, hiding it, recording previous state and transferring focus. Enforces caller ownership.
    pub fn minimize_window(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        let max_w = unsafe { crate::gui::VESA.width as u32 };
        let max_h = unsafe { crate::gui::VESA.height as u32 };

        let (win_x, win_y, win_w, win_h) = {
            let win = self.windows.iter_mut().find(|w| w.window_id == window_id)
                .ok_or(WmError::NotFound)?;

            if win.owner_pid != caller_pid {
                return Err(WmError::PermissionDenied);
            }

            if win.state != WindowState::Minimized {
                win.prev_state = win.state;
            }
            win.visible = false;
            win.focused = false;
            win.state = WindowState::Minimized;
            (win.x, win.y, win.width, win.height)
        };

        self.mark_window_damage(win_x, win_y, win_w, win_h);
        self.mark_top_bar_damage();

        if self.focused_window == Some(window_id) {
            self.focused_window = self.windows.iter().rev().find(|w| w.visible && w.state != WindowState::Minimized).map(|w| w.window_id);
            let mut focused_geom = None;
            for w in self.windows.iter_mut() {
                w.focused = Some(w.window_id) == self.focused_window;
                if w.focused {
                    focused_geom = Some((w.x, w.y, w.width, w.height));
                }
            }
            if let Some((fx, fy, fw, fh)) = focused_geom {
                self.mark_window_damage(fx, fy, fw, fh);
            }
        }
        Ok(())
    }

    /// Restores a minimized window to its previous exact state and geometry, raising to top. Enforces caller ownership.
    pub fn restore_window(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        let max_w = unsafe { crate::gui::VESA.width as u32 };
        let max_h = unsafe { crate::gui::VESA.height as u32 };
        let work_h = max_h.saturating_sub(WORK_AREA_TOP as u32);

        let (win_x, win_y, win_w, win_h) = {
            let win = self.windows.iter_mut().find(|w| w.window_id == window_id)
                .ok_or(WmError::NotFound)?;

            if win.owner_pid != caller_pid {
                return Err(WmError::PermissionDenied);
            }

            win.visible = true;
            match win.prev_state {
                WindowState::Maximized => {
                    win.state = WindowState::Maximized;
                    win.x = 0;
                    win.y = WORK_AREA_TOP;
                    win.width = max_w;
                    win.height = work_h;
                }
                WindowState::Fullscreen => {
                    win.state = WindowState::Fullscreen;
                    win.x = 0;
                    win.y = 0;
                    win.width = max_w;
                    win.height = max_h;
                }
                WindowState::SnappedLeft => {
                    win.state = WindowState::SnappedLeft;
                    win.x = 0;
                    win.y = WORK_AREA_TOP;
                    win.width = max_w / 2;
                    win.height = work_h;
                }
                WindowState::SnappedRight => {
                    win.state = WindowState::SnappedRight;
                    win.x = (max_w / 2) as i32;
                    win.y = WORK_AREA_TOP;
                    win.width = max_w - (max_w / 2);
                    win.height = work_h;
                }
                _ => {
                    win.state = WindowState::Normal;
                    let (nx, ny, nw, nh) = win.normal_geom;
                    win.x = nx;
                    win.y = ny;
                    win.width = nw;
                    win.height = nh;
                }
            }
            (win.x, win.y, win.width, win.height)
        };
        crate::app_registry::rerender_app_for_window(window_id, win_w, win_h);
        self.mark_window_damage(win_x, win_y, win_w, win_h);
        self.mark_top_bar_damage();
        self.raise_to_top_internal(window_id)
    }

    /// Snaps a window to the left half of the screen.
    pub fn snap_left(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        let max_w = unsafe { crate::gui::VESA.width as u32 };
        let max_h = unsafe { crate::gui::VESA.height as u32 };
        let work_h = max_h.saturating_sub(WORK_AREA_TOP as u32);

        let (old_x, old_y, old_w, old_h) = {
            let win = self.windows.iter_mut().find(|w| w.window_id == window_id).ok_or(WmError::NotFound)?;
            if win.owner_pid != caller_pid {
                return Err(WmError::PermissionDenied);
            }
            if win.state == WindowState::Normal {
                win.normal_geom = (win.x, win.y, win.width, win.height);
            }
            let old_geom = (win.x, win.y, win.width, win.height);
            win.x = 0;
            win.y = WORK_AREA_TOP;
            win.width = max_w / 2;
            win.height = work_h;
            win.state = WindowState::SnappedLeft;
            win.prev_state = WindowState::Normal;
            old_geom
        };

        crate::app_registry::rerender_app_for_window(window_id, max_w / 2, work_h);
        let min_x = old_x.min(0) - 4;
        let min_y = old_y.min(WORK_AREA_TOP) - 4;
        let max_bw = (old_w.max(max_w / 2)) + (old_x - 0).abs() as u32 + 8;
        let max_bh = (old_h.max(work_h)) + (old_y - WORK_AREA_TOP).abs() as u32 + 28;
        self.damage_tracker.add_bounds(min_x, min_y, max_bw, max_bh);
        self.raise_to_top_internal(window_id)
    }

    /// Snaps a window to the right half of the screen.
    pub fn snap_right(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        let max_w = unsafe { crate::gui::VESA.width as u32 };
        let max_h = unsafe { crate::gui::VESA.height as u32 };
        let work_h = max_h.saturating_sub(WORK_AREA_TOP as u32);
        let target_x = (max_w / 2) as i32;
        let target_w = max_w - (max_w / 2);

        let (old_x, old_y, old_w, old_h) = {
            let win = self.windows.iter_mut().find(|w| w.window_id == window_id).ok_or(WmError::NotFound)?;
            if win.owner_pid != caller_pid {
                return Err(WmError::PermissionDenied);
            }
            if win.state == WindowState::Normal {
                win.normal_geom = (win.x, win.y, win.width, win.height);
            }
            let old_geom = (win.x, win.y, win.width, win.height);
            win.x = target_x;
            win.y = WORK_AREA_TOP;
            win.width = target_w;
            win.height = work_h;
            win.state = WindowState::SnappedRight;
            win.prev_state = WindowState::Normal;
            old_geom
        };

        crate::app_registry::rerender_app_for_window(window_id, target_w, work_h);
        let min_x = old_x.min(target_x) - 4;
        let min_y = old_y.min(WORK_AREA_TOP) - 4;
        let max_bw = (old_w.max(target_w)) + (old_x - target_x).abs() as u32 + 8;
        let max_bh = (old_h.max(work_h)) + (old_y - WORK_AREA_TOP).abs() as u32 + 28;
        self.damage_tracker.add_bounds(min_x, min_y, max_bw, max_bh);
        self.raise_to_top_internal(window_id)
    }

    /// Toggles maximize state of a window, preserving normal geometry. Enforces caller ownership.
    pub fn toggle_maximize_window(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        self.mark_full_damage();
        let max_w = unsafe { crate::gui::VESA.width as u32 };
        let max_h = unsafe { crate::gui::VESA.height as u32 };
        let work_h = max_h.saturating_sub(WORK_AREA_TOP as u32);

        let (new_w, new_h) = {
            let win = self.windows.iter_mut().find(|w| w.window_id == window_id)
                .ok_or(WmError::NotFound)?;

            if win.owner_pid != caller_pid {
                return Err(WmError::PermissionDenied);
            }

            if win.state == WindowState::Maximized {
                // Restore to Normal geometry
                let (nx, ny, nw, nh) = win.normal_geom;
                win.x = nx.clamp(0, (max_w.saturating_sub(MIN_WINDOW_WIDTH)) as i32);
                win.y = ny.clamp(WORK_AREA_TOP, (max_h.saturating_sub(MIN_WINDOW_HEIGHT + 20)) as i32);
                win.width = nw.clamp(MIN_WINDOW_WIDTH, max_w);
                win.height = nh.clamp(MIN_WINDOW_HEIGHT, work_h);
                win.state = WindowState::Normal;
                win.prev_state = WindowState::Normal;
                win.saved_geom = None;
            } else {
                // Preserve normal geometry before maximizing
                if win.state == WindowState::Normal {
                    win.normal_geom = (win.x, win.y, win.width, win.height);
                }
                win.saved_geom = Some(win.normal_geom);
                win.x = 0;
                win.y = WORK_AREA_TOP;
                win.width = max_w;
                win.height = work_h;
                win.state = WindowState::Maximized;
                win.prev_state = WindowState::Normal;
            }
            (win.width, win.height)
        };

        crate::app_registry::rerender_app_for_window(window_id, new_w, new_h);
        self.raise_to_top_internal(window_id)
    }

    /// Toggles true fullscreen mode covering (0, 0, screen_w, screen_h), preserving previous state.
    pub fn toggle_fullscreen(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        self.mark_full_damage();
        let max_w = unsafe { crate::gui::VESA.width as u32 };
        let max_h = unsafe { crate::gui::VESA.height as u32 };
        let work_h = max_h.saturating_sub(WORK_AREA_TOP as u32);

        let (new_w, new_h) = {
            let win = self.windows.iter_mut().find(|w| w.window_id == window_id)
                .ok_or(WmError::NotFound)?;

            if win.owner_pid != caller_pid {
                return Err(WmError::PermissionDenied);
            }

            if win.state == WindowState::Fullscreen {
                // Revert to prev_state (either Maximized or Normal)
                if win.prev_state == WindowState::Maximized {
                    win.x = 0;
                    win.y = WORK_AREA_TOP;
                    win.width = max_w;
                    win.height = work_h;
                    win.state = WindowState::Maximized;
                } else {
                    let (nx, ny, nw, nh) = win.normal_geom;
                    win.x = nx.clamp(0, (max_w.saturating_sub(MIN_WINDOW_WIDTH)) as i32);
                    win.y = ny.clamp(WORK_AREA_TOP, (max_h.saturating_sub(MIN_WINDOW_HEIGHT + 20)) as i32);
                    win.width = nw.clamp(MIN_WINDOW_WIDTH, max_w);
                    win.height = nh.clamp(MIN_WINDOW_HEIGHT, max_h);
                    win.state = WindowState::Normal;
                    win.saved_geom = None;
                }
                win.prev_state = WindowState::Normal;
            } else {
                // Save state before entering Fullscreen
                win.prev_state = win.state;
                if win.state == WindowState::Normal {
                    win.normal_geom = (win.x, win.y, win.width, win.height);
                }
                win.saved_geom = Some(win.normal_geom);
                win.x = 0;
                win.y = 0;
                win.width = max_w;
                win.height = max_h;
                win.state = WindowState::Fullscreen;
            }
            (win.width, win.height)
        };

        crate::app_registry::rerender_app_for_window(window_id, new_w, new_h);
        self.raise_to_top_internal(window_id)
    }

    /// Updates the MRU list bringing the specified window to the front.
    pub fn touch_mru(&mut self, window_id: u64) {
        self.mru_list.retain(|&id| id != window_id);
        self.mru_list.insert(0, window_id);
        self.clean_mru();
    }

    /// Cleans up nonexistent or closed window IDs from MRU list.
    pub fn clean_mru(&mut self) {
        let existing: Vec<u64> = self.windows.iter().filter(|w| w.state != WindowState::Closed).map(|w| w.window_id).collect();
        self.mru_list.retain(|id| existing.contains(id));
    }

    /// Handles Alt+Tab keypress: activates switcher HUD or advances selection.
    pub fn alt_tab_press(&mut self, is_shift: bool) {
        self.clean_mru();
        if self.windows.is_empty() {
            return;
        }

        let candidates: Vec<u64> = self.mru_list.iter()
            .copied()
            .filter(|&id| self.windows.iter().any(|w| w.window_id == id && w.state != WindowState::Closed))
            .collect();

        if candidates.is_empty() {
            return;
        }

        let sw = unsafe { crate::gui::VESA.width as i32 };
        let sh = unsafe { crate::gui::VESA.height as i32 };
        let hud_w = 400.min(sw - 40) as u32;
        let hud_h = 100 as u32;
        let hud_x = (sw - hud_w as i32) / 2;
        let hud_y = (sh - hud_h as i32) / 2;

        if !self.alt_tab.active {
            self.alt_tab.active = true;
            self.alt_tab.candidates = candidates;
            if self.alt_tab.candidates.len() >= 2 {
                self.alt_tab.selected_index = if is_shift { self.alt_tab.candidates.len() - 1 } else { 1 };
            } else {
                self.alt_tab.selected_index = 0;
            }
        } else {
            self.alt_tab.candidates = candidates;
            let count = self.alt_tab.candidates.len();
            if count > 0 {
                if is_shift {
                    self.alt_tab.selected_index = if self.alt_tab.selected_index == 0 { count - 1 } else { self.alt_tab.selected_index - 1 };
                } else {
                    self.alt_tab.selected_index = (self.alt_tab.selected_index + 1) % count;
                }
            }
        }

        self.mark_damage(hud_x - 4, hud_y - 4, hud_w + 8, hud_h + 8);
    }

    /// Commits the Alt+Tab selection when Alt key is released.
    pub fn alt_tab_commit(&mut self) -> Option<u64> {
        if !self.alt_tab.active {
            return None;
        }

        let sw = unsafe { crate::gui::VESA.width as i32 };
        let sh = unsafe { crate::gui::VESA.height as i32 };
        let hud_w = 400.min(sw - 40) as u32;
        let hud_h = 100 as u32;
        let hud_x = (sw - hud_w as i32) / 2;
        let hud_y = (sh - hud_h as i32) / 2;
        self.mark_damage(hud_x - 4, hud_y - 4, hud_w + 8, hud_h + 8);

        self.alt_tab.active = false;
        if self.alt_tab.candidates.is_empty() {
            return None;
        }

        let target_idx = self.alt_tab.selected_index.min(self.alt_tab.candidates.len() - 1);
        let target_id = self.alt_tab.candidates[target_idx];

        if let Some(win) = self.windows.iter().find(|w| w.window_id == target_id) {
            let owner = win.owner_pid;
            let is_minimized = win.state == WindowState::Minimized;
            if is_minimized {
                let _ = self.restore_window(owner, target_id);
            } else {
                let _ = self.raise_to_top_internal(target_id);
            }
            self.touch_mru(target_id);
            Some(target_id)
        } else {
            None
        }
    }

    /// Cancels Alt+Tab selection without switching window.
    pub fn alt_tab_cancel(&mut self) {
        if !self.alt_tab.active {
            return;
        }
        let sw = unsafe { crate::gui::VESA.width as i32 };
        let sh = unsafe { crate::gui::VESA.height as i32 };
        let hud_w = 400.min(sw - 40) as u32;
        let hud_h = 100 as u32;
        let hud_x = (sw - hud_w as i32) / 2;
        let hud_y = (sh - hud_h as i32) / 2;
        self.mark_damage(hud_x - 4, hud_y - 4, hud_w + 8, hud_h + 8);

        self.alt_tab.active = false;
    }

    /// Cycles through open windows via Alt-Tab, restoring minimized windows and raising target to top
    pub fn alt_tab_cycle(&mut self) -> Option<u64> {
        self.clean_mru();
        if self.windows.is_empty() { return None; }

        let candidates: Vec<u64> = self.mru_list.iter()
            .copied()
            .filter(|&id| self.windows.iter().any(|w| w.window_id == id && w.state != WindowState::Closed))
            .collect();

        if candidates.is_empty() {
            return None;
        }

        let target_id = if candidates.len() >= 2 { candidates[1] } else { candidates[0] };
        if let Some(win) = self.windows.iter().find(|w| w.window_id == target_id) {
            let owner = win.owner_pid;
            if win.state == WindowState::Minimized {
                let _ = self.restore_window(owner, target_id);
            } else {
                let _ = self.raise_to_top_internal(target_id);
            }
            self.touch_mru(target_id);
        }

        self.focused_window
    }

    /// Destroys a window with strict ownership verification, surface reclamation, and process cleanup.
    pub fn destroy_window(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        let max_w = unsafe { crate::gui::VESA.width as u32 };
        let max_h = unsafe { crate::gui::VESA.height as u32 };

        let idx = self.windows.iter().position(|w| w.window_id == window_id)
            .ok_or(WmError::NotFound)?;

        if self.windows[idx].owner_pid != caller_pid {
            return Err(WmError::PermissionDenied);
        }

        let old_x = self.windows[idx].x;
        let old_y = self.windows[idx].y;
        let old_w = self.windows[idx].width;
        let old_h = self.windows[idx].height;
        let surf_id = self.windows[idx].surface_id;
        self.windows.remove(idx);
        self.mru_list.retain(|&id| id != window_id);
        self.alt_tab.candidates.retain(|&id| id != window_id);
        self.clean_mru();

        self.mark_window_damage(old_x, old_y, old_w, old_h);
        self.mark_top_bar_damage();

        if self.focused_window == Some(window_id) {
            // Transfer focus to the next topmost visible window
            self.focused_window = self.windows.iter().rev().find(|w| w.visible && w.state != WindowState::Minimized).map(|w| w.window_id);
            let mut focused_geom = None;
            for w in self.windows.iter_mut() {
                w.focused = Some(w.window_id) == self.focused_window;
                if w.focused {
                    focused_geom = Some((w.x, w.y, w.width, w.height));
                }
            }
            if let Some((fx, fy, fw, fh)) = focused_geom {
                self.mark_window_damage(fx, fy, fw, fh);
            }
        }

        // Clean up surface registry for this window and uncharge accounting
        if surf_id != 0 {
            let mut reg = crate::surface::SURFACE_REGISTRY.write();
            if let Some(pos) = reg.iter().position(|s| s.surface_id == surf_id && s.owner_pid == caller_pid) {
                let surf = reg.remove(pos);
                drop(reg);

                let mut sched = crate::task::process::SCHEDULER.lock();
                if let Some(proc) = sched.get_process_mut(caller_pid) {
                    proc.uncharge_memory(surf.shmem_size as u64);
                    proc.decrement_surface_count();
                }
            }
        }

        // Clean up per-window terminal/files/editor/browser instance if attached
        crate::terminal_app::cleanup_terminal_for_window(window_id);
        crate::files_app::cleanup_files_for_window(window_id);
        crate::editor_app::cleanup_editor_for_window(window_id);
        crate::browser_app::cleanup_browser_for_window(window_id);

        // Unregister window from process PCB, decrement window count, and check remaining windows
        let remaining_windows = self.windows.iter().any(|w| w.owner_pid == caller_pid);
        {
            let mut sched = crate::task::process::SCHEDULER.lock();
            let mut need_purge = false;
            if let Some(proc) = sched.get_process_mut(caller_pid) {
                proc.decrement_window_count();
                proc.owned_windows.retain(|&wid| wid != window_id);

                // If process is UI-bound and has no remaining windows, mark it exited
                if !remaining_windows && proc.kind == crate::task::process::ProcessKind::UIBound {
                    proc.state = crate::task::process::ProcessState::Exited;
                    proc.exited = true;
                    proc.reaped = true;
                    crate::cap::destroy_process_cspace(&mut proc.cap_table);
                    crate::ipc::hangup_channel_for_pid(caller_pid as u32);
                    need_purge = true;
                }
            } else if !remaining_windows {
                need_purge = true;
            }

            if need_purge {
                sched.purge_pid(caller_pid);
            }
        }

        if !remaining_windows {
            crate::input::cleanup_input_for_pid(caller_pid);
            crate::permission::PERMISSION_MANAGER.lock().unregister_process(caller_pid);
        }

        crate::serial_println!("[WM] Process {} destroyed Window {} (Surface {} cleaned, Remaining windows: {})",
            caller_pid, window_id, surf_id, remaining_windows);
        Ok(())
    }

    /// Moves a window to new coordinates with strict ownership verification.
    pub fn move_window(&mut self, caller_pid: u64, window_id: u64, new_x: i32, new_y: i32) -> Result<(), WmError> {
        let max_w = unsafe { crate::gui::VESA.width as i32 };
        let max_h = unsafe { crate::gui::VESA.height as i32 };

        let (old_x, old_y, win_w, win_h, clamped_x, clamped_y) = {
            let win = self.windows.iter_mut().find(|w| w.window_id == window_id)
                .ok_or(WmError::NotFound)?;

            if win.owner_pid != caller_pid {
                return Err(WmError::PermissionDenied);
            }

            let new_clamped_x = new_x.clamp(-100, max_w - 50);
            let new_clamped_y = new_y.clamp(WORK_AREA_TOP, max_h - (DOCK_HEIGHT as i32 + 30));
            let (ox, oy, ow, oh) = (win.x, win.y, win.width, win.height);
            win.x = new_clamped_x;
            win.y = new_clamped_y;
            if win.state == WindowState::Normal {
                win.normal_geom = (win.x, win.y, win.width, win.height);
            }
            (ox, oy, ow, oh, new_clamped_x, new_clamped_y)
        };

        self.mark_window_damage(old_x, old_y, win_w, win_h);
        self.mark_window_damage(clamped_x, clamped_y, win_w, win_h);
        Ok(())
    }

    /// Raises a window to the top of the Z-order with caller ownership check.
    pub fn raise_to_top(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        let win = self.windows.iter().find(|w| w.window_id == window_id)
            .ok_or(WmError::NotFound)?;

        if win.owner_pid != caller_pid {
            return Err(WmError::PermissionDenied);
        }

        self.raise_to_top_internal(window_id)
    }

    /// Internal compositor elevation (e.g. on mouse click hit-test).
    pub fn raise_to_top_internal(&mut self, window_id: u64) -> Result<(), WmError> {
        let len = self.windows.len();
        if len == 0 { return Err(WmError::NotFound); }
        if self.windows[len - 1].window_id == window_id && self.focused_window == Some(window_id) {
            return Ok(());
        }

        let idx = self.windows.iter().position(|w| w.window_id == window_id)
            .ok_or(WmError::NotFound)?;

        let mut win = self.windows.remove(idx);

        for w in self.windows.iter_mut() {
            w.focused = false;
        }

        win.focused = true;
        self.focused_window = Some(window_id);
        self.mark_window_damage(win.x, win.y, win.width, win.height);
        self.mark_top_bar_damage();
        self.windows.push(win);
        self.touch_mru(window_id);
        Ok(())
    }

    /// Hit-tests screen coordinates (mx, my) from Top-to-Bottom.
    pub fn hit_test(&self, mx: i32, my: i32) -> Option<u64> {
        for win in self.windows.iter().rev() {
            if win.visible &&
               mx >= win.x && mx < win.x + (win.width as i32) &&
               my >= win.y && my < win.y + (win.height as i32 + 20) {
                return Some(win.window_id);
            }
        }
        None
    }

    /// Dispatches mouse clicks: performs hit-test, elevates focus, and returns (window_id, owner_pid).
    pub fn dispatch_mouse_click(&mut self, mx: i32, my: i32, _button: u8) -> Option<(u64, u64)> {
        if let Some(target_id) = self.hit_test(mx, my) {
            let _ = self.raise_to_top_internal(target_id);
            let owner_pid = self.windows.iter().find(|w| w.window_id == target_id).map(|w| w.owner_pid)?;
            return Some((target_id, owner_pid));
        }
        None
    }

    /// Dispatches keyboard input strictly to the single focused window (Keylogger Isolation).
    pub fn dispatch_keyboard_input(&self, _key_code: u8) -> Option<(u64, u64)> {
        let target_id = self.focused_window?;
        let owner_pid = self.windows.iter().find(|w| w.window_id == target_id).map(|w| w.owner_pid)?;
        Some((target_id, owner_pid))
    }

    /// Composite all windows, surfaces, dock, launcher, and cursor to the hardware backbuffer
    pub fn composite_desktop(&mut self, _mouse_x: i32, _mouse_y: i32) {
        unsafe {
            if crate::gui::BACKBUFFER.is_null() { return; }
        }

        let damage = match self.damage_tracker.take_damage() {
            Some(d) => d,
            None => return, // Zero damage: skip redraw completely
        };

        let screen_w = unsafe { crate::gui::VESA.width as u16 };
        let screen_h = unsafe { crate::gui::VESA.height as u16 };
        let clamped = damage.clamp_to_screen(screen_w as u32, screen_h as u32);
        if clamped.is_empty() { return; }

        let is_full = clamped.x == 0 && clamped.y == 0 && clamped.width >= screen_w as u32 && clamped.height >= screen_h as u32;

        if is_full {
            crate::gui::set_clip(None);
        } else {
            crate::gui::set_clip(Some((clamped.x as u16, clamped.y as u16, clamped.width as u16, clamped.height as u16)));
        }

        let dock_y = screen_h.saturating_sub(DOCK_HEIGHT);
        let active_theme = crate::theme::THEME_MANAGER.lock().current_theme;

        // 1. Desktop Environment (Wallpaper Gradient Engine & Desktop Icons)
        crate::desktop::DESKTOP_ENV.lock().render(screen_w, screen_h);

        // 3. Composite Windows in Z-Order (Back-to-Front)
        let surf_reg = crate::surface::SURFACE_REGISTRY.read();
        for win in self.windows.iter() {
            if !win.visible || win.state == WindowState::Minimized {
                continue;
            }

            let wx = win.x.max(0) as u16;
            let wy = win.y.max(0) as u16;
            let ww = win.width as u16;
            let wh = win.height as u16;

            let is_focused = self.focused_window == Some(win.window_id);
            let title_bg = if is_focused { active_theme.titlebar_active } else { active_theme.titlebar_inactive };
            let title_fg = if is_focused { active_theme.text_color } else { 0x0094A3B8 };
            let border_col = if is_focused { active_theme.accent_color } else { active_theme.border_color };

            if win.state == WindowState::Fullscreen {
                // 3-FS. True Fullscreen: 1:1 Aspect-Preserved Fast Centered Blit (Zero Lag, Crisp Pixels)
                if let Some(surface) = surf_reg.iter().find(|s| s.surface_id == win.surface_id) {
                    let phys_addr = surface.shmem_phys_addr;
                    let src_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *const u32 };
                    let surf_w = surface.width as usize;
                    let surf_h = surface.height as usize;
                    let screen_w_usize = screen_w as usize;
                    let screen_h_usize = screen_h as usize;

                    // Clean dark backdrop for fullscreen
                    crate::gui::draw_rect(0, 0, screen_w, screen_h, active_theme.window_active);

                    let offset_x = screen_w_usize.saturating_sub(surf_w) / 2;
                    let offset_y = screen_h_usize.saturating_sub(surf_h) / 2;
                    let copy_w = surf_w.min(screen_w_usize);
                    let copy_h = surf_h.min(screen_h_usize);

                    unsafe {
                        if !crate::gui::BACKBUFFER.is_null() && copy_w > 0 && copy_h > 0 {
                            let (clip_x, clip_y, clip_w, clip_h) = match crate::gui::CLIP_RECT {
                                Some(r) => (r.0 as usize, r.1 as usize, r.2 as usize, r.3 as usize),
                                None => (0, 0, screen_w_usize, screen_h_usize),
                            };

                            for r in 0..copy_h {
                                let dst_row = offset_y + r;
                                if dst_row < clip_y || dst_row >= clip_y + clip_h {
                                    continue;
                                }
                                let row_start = offset_x.max(clip_x);
                                let row_end = (offset_x + copy_w).min(clip_x + clip_w);
                                if row_end > row_start {
                                    let row_copy_w = row_end - row_start;
                                    let src_x_off = row_start - offset_x;
                                    let src_offset = r * surf_w + src_x_off;
                                    let dst_offset = dst_row * screen_w_usize + row_start;
                                    core::ptr::copy_nonoverlapping(
                                        src_ptr.add(src_offset),
                                        crate::gui::BACKBUFFER.add(dst_offset),
                                        row_copy_w,
                                    );
                                }
                            }
                        }
                    }
                }
                continue;
            }

            // 3a. Titlebar (20px high)
            crate::gui::draw_rect(wx, wy, ww, 20, title_bg);
            
            // Draw App Icon & Title strictly from cached Window metadata (Dikey & Yatay Tam Ortalı)
            crate::gui::draw_icon_glyph(wx + 6, wy + 2, win.icon, title_fg, title_bg);
            crate::gui::draw_string(wx + 26, wy + 4, &win.title, title_fg, title_bg);

            // Control Buttons on Right (— □ ×)
            let is_hover_target = self.hovered_target.as_ref().map(|h| h.window_id == win.window_id).unwrap_or(false);
            let hovered_btn = if is_hover_target {
                self.hovered_target.as_ref().map(|h| h.button).unwrap_or(ChromeButton::None)
            } else {
                ChromeButton::None
            };

            // Minimize Button [—]
            if ww > 60 {
                let min_x = wx + ww - 54;
                let min_bg = if hovered_btn == ChromeButton::Minimize { 0x00475569 } else { active_theme.dock_background };
                crate::gui::draw_rect(min_x, wy + 3, 14, 14, min_bg);
                crate::gui::draw_char(min_x + 4, wy + 5, '-', 0x00E2E8F0, min_bg);
            }

            // Maximize Button [□]
            if ww > 40 {
                let max_x = wx + ww - 36;
                let max_bg = if hovered_btn == ChromeButton::Maximize { active_theme.accent_color } else { active_theme.dock_background };
                crate::gui::draw_rect(max_x, wy + 3, 14, 14, max_bg);
                let max_sym = if win.state == WindowState::Maximized { '^' } else { '+' };
                crate::gui::draw_char(max_x + 4, wy + 5, max_sym, 0x00E2E8F0, max_bg);
            }

            // Close Button [×]
            if ww > 20 {
                let close_x = wx + ww - 18;
                let close_bg = if hovered_btn == ChromeButton::Close { active_theme.close_button_hover } else { 0x00DC2626 };
                crate::gui::draw_rect(close_x, wy + 3, 14, 14, close_bg);
                crate::gui::draw_char(close_x + 4, wy + 5, 'x', 0x00FFFFFF, close_bg);
            }

            // 3b. Client Area Background
            crate::gui::draw_rect(wx, wy + 20, ww, wh, active_theme.window_active);

            // 3c. Blit shared-memory surface if present (1:1 Crisp Fast Row Copy - Zero Lag)
            if let Some(surface) = surf_reg.iter().find(|s| s.surface_id == win.surface_id) {
                let phys_addr = surface.shmem_phys_addr;
                let src_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *const u32 };
                let surf_w = surface.width as usize;
                let surf_h = surface.height as usize;
                let target_w = (ww as usize).min((screen_w.saturating_sub(wx)) as usize);
                let target_h = (wh as usize).min((dock_y.saturating_sub(wy + 20)) as usize);

                let copy_w = surf_w.min(target_w);
                let copy_h = surf_h.min(target_h);

                unsafe {
                    if !crate::gui::BACKBUFFER.is_null() && copy_w > 0 && copy_h > 0 {
                        let (clip_x, clip_y, clip_w, clip_h) = match crate::gui::CLIP_RECT {
                            Some(r) => (r.0 as usize, r.1 as usize, r.2 as usize, r.3 as usize),
                            None => (0, 0, screen_w as usize, screen_h as usize),
                        };

                        for r in 0..copy_h {
                            let dst_row = (wy + 20) as usize + r;
                            if dst_row < clip_y || dst_row >= clip_y + clip_h {
                                continue;
                            }
                            let dst_x_start = wx as usize;
                            let dst_x_end = dst_x_start + copy_w;

                            let row_start = dst_x_start.max(clip_x);
                            let row_end = dst_x_end.min(clip_x + clip_w);

                            if row_end > row_start {
                                let row_copy_w = row_end - row_start;
                                let src_x_offset = row_start - dst_x_start;
                                let src_offset = r * surf_w + src_x_offset;
                                let dst_offset = dst_row * (screen_w as usize) + row_start;
                                core::ptr::copy_nonoverlapping(
                                    src_ptr.add(src_offset),
                                    crate::gui::BACKBUFFER.add(dst_offset),
                                    row_copy_w,
                                );
                            }
                        }
                    }
                }
            }

            // 3d. 1px window border
            crate::gui::draw_rect(wx, wy, ww, 1, border_col);
            crate::gui::draw_rect(wx, wy + 20 + wh - 1, ww, 1, border_col);
            crate::gui::draw_rect(wx, wy, 1, 20 + wh, border_col);
            crate::gui::draw_rect(wx + ww - 1, wy, 1, 20 + wh, border_col);
        }
        drop(surf_reg);

        // 3e. Snap Preview Box Overlay (when window dragging near edge)
        match self.snap_preview {
            SnapPreview::Left => {
                let pw = screen_w / 2;
                let ph = dock_y.saturating_sub(WORK_AREA_TOP as u16 + 20);
                crate::gui::draw_rect(0, WORK_AREA_TOP as u16, pw, ph, 0x001E293B);
                crate::gui::draw_rect(0, WORK_AREA_TOP as u16, pw, 2, 0x003B82F6);
                crate::gui::draw_rect(0, WORK_AREA_TOP as u16, 2, ph, 0x003B82F6);
                crate::gui::draw_rect(pw.saturating_sub(2), WORK_AREA_TOP as u16, 2, ph, 0x003B82F6);
                crate::gui::draw_rect(0, (WORK_AREA_TOP as u16) + ph.saturating_sub(2), pw, 2, 0x003B82F6);
            }
            SnapPreview::Right => {
                let px = screen_w / 2;
                let pw = screen_w - px;
                let ph = dock_y.saturating_sub(WORK_AREA_TOP as u16 + 20);
                crate::gui::draw_rect(px, WORK_AREA_TOP as u16, pw, ph, 0x001E293B);
                crate::gui::draw_rect(px, WORK_AREA_TOP as u16, pw, 2, 0x003B82F6);
                crate::gui::draw_rect(px, WORK_AREA_TOP as u16, 2, ph, 0x003B82F6);
                crate::gui::draw_rect(px + pw.saturating_sub(2), WORK_AREA_TOP as u16, 2, ph, 0x003B82F6);
                crate::gui::draw_rect(px, (WORK_AREA_TOP as u16) + ph.saturating_sub(2), pw, 2, 0x003B82F6);
            }
            SnapPreview::Maximized => {
                let pw = screen_w;
                let ph = dock_y.saturating_sub(WORK_AREA_TOP as u16 + 20);
                crate::gui::draw_rect(0, WORK_AREA_TOP as u16, pw, ph, 0x001E293B);
                crate::gui::draw_rect(0, WORK_AREA_TOP as u16, pw, 2, 0x003B82F6);
                crate::gui::draw_rect(0, WORK_AREA_TOP as u16, 2, ph, 0x003B82F6);
                crate::gui::draw_rect(pw.saturating_sub(2), WORK_AREA_TOP as u16, 2, ph, 0x003B82F6);
                crate::gui::draw_rect(0, (WORK_AREA_TOP as u16) + ph.saturating_sub(2), pw, 2, 0x003B82F6);
            }
            SnapPreview::None => {}
        }

        // 4. Unified Top Panel 2.0 (Start button, Window tabs, System HUD)
        // Ensure top panel is never partially clipped when damaged
        let touches_top_bar = is_full || clamped.y < (DOCK_HEIGHT as i32 + 2);
        if touches_top_bar {
            crate::gui::set_clip(None);
            crate::dock::Dock::render(
                screen_w,
                screen_h,
                &self.windows,
                self.focused_window,
                self.launcher_open,
                self.hovered_dock_tab,
            );
            if !is_full {
                crate::gui::set_clip(Some((clamped.x as u16, clamped.y as u16, clamped.width as u16, clamped.height as u16)));
            }
        }

        // 5. Application Launcher 2.0 Dropdown Popup (if open)
        if self.launcher_open {
            crate::gui::set_clip(None);
            let launcher_state = crate::launcher::LauncherState {
                open: self.launcher_open,
                selected_index: self.launcher_selected,
            };
            launcher_state.render(screen_w, screen_h);
            if !is_full {
                crate::gui::set_clip(Some((clamped.x as u16, clamped.y as u16, clamped.width as u16, clamped.height as u16)));
            }
        }

        // 6. Global Freeze Protection / Crash Modal & Diagnostic Overlay
        crate::crash_reporter::CRASH_REPORTER.lock().render_crash_modal(screen_w, screen_h);

        // 6b. Alt+Tab Window Switcher HUD Overlay
        if self.alt_tab.active && !self.alt_tab.candidates.is_empty() {
            let total_items = self.alt_tab.candidates.len().min(8) as u16;
            let card_w = 90u16;
            let card_h = 70u16;
            let hud_w = (total_items * (card_w + 8) + 12).min(screen_w - 40);
            let hud_h = 90u16;
            let hud_x = (screen_w - hud_w) / 2;
            let hud_y = (screen_h - hud_h) / 2;

            // HUD Outer Box & Border
            crate::gui::draw_rect(hud_x, hud_y, hud_w, hud_h, 0x000F172A);
            crate::gui::draw_rect(hud_x, hud_y, hud_w, 1, 0x003B82F6);
            crate::gui::draw_rect(hud_x, hud_y, 1, hud_h, 0x003B82F6);
            crate::gui::draw_rect(hud_x + hud_w - 1, hud_y, 1, hud_h, 0x003B82F6);
            crate::gui::draw_rect(hud_x, hud_y + hud_h - 1, hud_w, 1, 0x003B82F6);

            let mut card_x = hud_x + 8;
            let card_y = hud_y + 10;

            for (idx, &wid) in self.alt_tab.candidates.iter().take(8).enumerate() {
                let is_selected = idx == self.alt_tab.selected_index;
                let bg_color = if is_selected { 0x002563EB } else { 0x001E293B };
                let border_color = if is_selected { 0x0060A5FA } else { 0x00334155 };
                let text_color = if is_selected { 0x00FFFFFF } else { 0x0094A3B8 };

                // Card background & selection border
                crate::gui::draw_rect(card_x, card_y, card_w, card_h, bg_color);
                crate::gui::draw_rect(card_x, card_y, card_w, 1, border_color);
                crate::gui::draw_rect(card_x, card_y, 1, card_h, border_color);
                crate::gui::draw_rect(card_x + card_w - 1, card_y, 1, card_h, border_color);
                crate::gui::draw_rect(card_x, card_y + card_h - 1, card_w, 1, border_color);

                if let Some(win) = self.windows.iter().find(|w| w.window_id == wid) {
                    // Icon
                    crate::gui::draw_icon_glyph(card_x + (card_w / 2) - 8, card_y + 12, win.icon, text_color, bg_color);
                    // Title
                    let title_text = if win.title.len() > 9 { &win.title[..9] } else { &win.title };
                    crate::gui::draw_string(card_x + 6, card_y + 44, title_text, text_color, bg_color);
                }

                card_x += card_w + 8;
            }
        }

        // 7. Draw topmost mouse cursor layer
        crate::cursor::draw_cursor_layer();

        // 8. Transfer to Framebuffer (Full swap vs Dirty Rect flush)
        if is_full {
            crate::gui::swap_buffers();
        } else {
            crate::gui::flush_rect(clamped.x as u16, clamped.y as u16, clamped.width as u16, clamped.height as u16);
        }

        crate::gui::set_clip(None);
    }

    /// Handles mouse down: performs titlebar dragging, resize, close/maximize/minimize buttons, dock interaction, or client focus
    pub fn handle_mouse_down(&mut self, mx: i32, my: i32) -> Option<(u64, u64)> {
        let screen_w = unsafe { crate::gui::VESA.width as i32 };
        let top_bar_h = DOCK_HEIGHT as i32;

        // 1. Check Launcher Dropdown Click (if open)
        if self.launcher_open {
            let px = 4;
            let pw = 164;
            let total_apps = crate::app_registry::REGISTERED_APPS.len() as i32;
            let ph = 34 + total_apps * 28 + 26;
            let py = top_bar_h + 2;

            if mx >= px && mx < px + pw && my >= py && my < py + ph {
                let mut cur_y = py + 28;
                for app in crate::app_registry::REGISTERED_APPS.iter() {
                    if my >= cur_y && my < cur_y + 24 {
                        self.pending_spawn_app = Some(app.id);
                        self.launcher_open = false;
                        self.mark_damage(px, py, pw as u32, ph as u32);
                        return None;
                    }
                    cur_y += 28;
                }
                // Close Menu button
                if my >= cur_y && my < cur_y + 24 {
                    self.launcher_open = false;
                    self.mark_damage(px, py, pw as u32, ph as u32);
                    return None;
                }
            } else if !(mx >= 4 && mx <= 80 && my < top_bar_h) {
                // Clicked outside launcher dropdown -> close launcher
                self.launcher_open = false;
                self.mark_damage(px, py, pw as u32, ph as u32);
            }
        }

        // 2. Check Top Bar Click (my < top_bar_h)
        if my < top_bar_h {
            // Launcher button click (x in 4..92)
            if mx >= 4 && mx <= 92 {
                self.launcher_open = !self.launcher_open;
                self.mark_damage(0, 0, screen_w as u32, 350);
                return None;
            }

            // Window Tab click (x in 102 .. screen_w - 200)
            if mx >= 102 && mx <= screen_w.saturating_sub(200) {
                let tab_idx = ((mx - 102) / 92) as usize;
                if tab_idx < self.windows.len() {
                    let win = &self.windows[tab_idx];
                    let wid = win.window_id;
                    let owner = win.owner_pid;
                    let is_focused = self.focused_window == Some(wid);

                    if win.state == WindowState::Minimized {
                        let _ = self.restore_window(owner, wid);
                    } else if is_focused {
                        let _ = self.minimize_window(owner, wid);
                    } else {
                        let _ = self.raise_to_top_internal(wid);
                    }
                    self.mark_top_bar_damage();
                    return Some((wid, owner));
                }
            }
            return None;
        }

        // 3. Check Windows Click (Top-to-Bottom)
        for i in (0..self.windows.len()).rev() {
            let win = &self.windows[i];
            if !win.visible || win.state == WindowState::Minimized {
                continue;
            }

            let wx = win.x;
            let wy = win.y;
            let ww = win.width as i32;
            let wh = win.height as i32;
            let wid = win.window_id;
            let owner = win.owner_pid;

            // 3-FS. Fullscreen Window Click
            if win.state == WindowState::Fullscreen {
                let _ = self.raise_to_top_internal(wid);
                return Some((wid, owner));
            }

            // 3a. Check Resize Borders (6px margin around window edges)
            if win.state != WindowState::Maximized && win.state != WindowState::Fullscreen {
                // 1. Top-Left Corner (10x10)
                if mx >= wx - 6 && mx <= wx + 8 && my >= wy - 6 && my <= wy + 8 {
                    self.resizing_window = Some((wid, ResizeEdge::TopLeft, mx, my, wx, wy, win.width, win.height));
                    let _ = self.raise_to_top_internal(wid);
                    return None;
                }
                // 2. Top-Right Corner (10x10)
                if mx >= wx + ww - 8 && mx <= wx + ww + 6 && my >= wy - 6 && my <= wy + 8 {
                    self.resizing_window = Some((wid, ResizeEdge::TopRight, mx, my, wx, wy, win.width, win.height));
                    let _ = self.raise_to_top_internal(wid);
                    return None;
                }
                // 3. Bottom-Left Corner (10x10)
                if mx >= wx - 6 && mx <= wx + 8 && my >= wy + 20 + wh - 8 && my <= wy + 20 + wh + 6 {
                    self.resizing_window = Some((wid, ResizeEdge::BottomLeft, mx, my, wx, wy, win.width, win.height));
                    let _ = self.raise_to_top_internal(wid);
                    return None;
                }
                // 4. Bottom-Right Corner (10x10)
                if mx >= wx + ww - 8 && mx <= wx + ww + 6 && my >= wy + 20 + wh - 8 && my <= wy + 20 + wh + 6 {
                    self.resizing_window = Some((wid, ResizeEdge::BottomRight, mx, my, wx, wy, win.width, win.height));
                    let _ = self.raise_to_top_internal(wid);
                    return None;
                }
                // 5. Left Edge
                if mx >= wx - 6 && mx <= wx + 4 && my >= wy && my <= wy + 20 + wh {
                    self.resizing_window = Some((wid, ResizeEdge::Left, mx, my, wx, wy, win.width, win.height));
                    let _ = self.raise_to_top_internal(wid);
                    return None;
                }
                // 6. Right Edge
                if mx >= wx + ww - 4 && mx <= wx + ww + 6 && my >= wy && my <= wy + 20 + wh {
                    self.resizing_window = Some((wid, ResizeEdge::Right, mx, my, wx, wy, win.width, win.height));
                    let _ = self.raise_to_top_internal(wid);
                    return None;
                }
                // 7. Top Edge
                if mx >= wx && mx <= wx + ww && my >= wy - 6 && my <= wy + 4 {
                    self.resizing_window = Some((wid, ResizeEdge::Top, mx, my, wx, wy, win.width, win.height));
                    let _ = self.raise_to_top_internal(wid);
                    return None;
                }
                // 8. Bottom Edge
                if mx >= wx && mx <= wx + ww && my >= wy + 20 + wh - 4 && my <= wy + 20 + wh + 6 {
                    self.resizing_window = Some((wid, ResizeEdge::Bottom, mx, my, wx, wy, win.width, win.height));
                    let _ = self.raise_to_top_internal(wid);
                    return None;
                }
            }

            // 3b. Check Titlebar click (20px high)
            if mx >= wx && mx < wx + ww && my >= wy && my < wy + 20 {
                // 1. Close button [×] (Rightmost: wx + ww - 20 .. wx + ww - 4)
                if mx >= wx + ww - 20 && mx <= wx + ww - 4 && my >= wy + 2 && my <= wy + 18 {
                    let mut editors = crate::editor_app::EDITOR_INSTANCES.lock();
                    if let Some(editor_state) = editors.get_mut(&wid) {
                        if editor_state.is_dirty {
                            editor_state.show_unsaved_dialog = true;
                            if let Some(surface) = crate::surface::SURFACE_REGISTRY.read().iter().find(|s| s.surface_id == win.surface_id || s.owner_pid == owner) {
                                let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + surface.shmem_phys_addr) as *mut u32 };
                                editor_state.render_to_surface(surf_ptr, crate::editor_app::EDITOR_WIDTH, crate::editor_app::EDITOR_HEIGHT);
                            }
                            drop(editors);
                            return None;
                        }
                    }
                    drop(editors);

                    let _ = self.destroy_window(owner, wid);
                    return None;
                }
                // 2. Maximize button [□] (Middle: wx + ww - 38 .. wx + ww - 22)
                if mx >= wx + ww - 38 && mx <= wx + ww - 22 && my >= wy + 2 && my <= wy + 18 {
                    let _ = self.toggle_maximize_window(owner, wid);
                    return None;
                }
                // 3. Minimize button [—] (Left: wx + ww - 56 .. wx + ww - 40)
                if mx >= wx + ww - 56 && mx <= wx + ww - 40 && my >= wy + 2 && my <= wy + 18 {
                    let _ = self.minimize_window(owner, wid);
                    return None;
                }

                // 4. Check Titlebar double click (Maximize/Restore shortcut)
                let now_tick = crate::interrupts::get_tick();
                let is_double_click = if let Some((last_id, last_tick)) = self.last_titlebar_click {
                    last_id == wid && now_tick.saturating_sub(last_tick) <= 300
                } else {
                    false
                };

                if is_double_click {
                    self.last_titlebar_click = None;
                    let _ = self.toggle_maximize_window(owner, wid);
                    return None;
                } else {
                    self.last_titlebar_click = Some((wid, now_tick));
                }

                // 5. Titlebar Drag start (Clicking on titlebar itself, not buttons)
                self.dragging_window = Some((wid, mx - wx, my - wy));
                let _ = self.raise_to_top_internal(wid);
                return None;
            }

            // 3c. Check Client area click -> deliver event to client surface
            if mx >= wx && mx < wx + ww && my >= wy + 20 && my < wy + 20 + wh {
                let _ = self.raise_to_top_internal(wid);
                return Some((wid, owner));
            }
        }
        None
    }

    /// Handles mouse up: stops dragging and resizing with edge snapping, resets cursor
    pub fn handle_mouse_up(&mut self) -> Option<(u64, u64)> {
        let max_w = unsafe { crate::gui::VESA.width as i32 };
        let max_h = unsafe { crate::gui::VESA.height as i32 };
        let work_h = max_h.saturating_sub(WORK_AREA_TOP) as u32;

        if let Some((wid, _, _)) = self.dragging_window.take() {
            if let Some(win) = self.windows.iter_mut().find(|w| w.window_id == wid) {
                let old_x = win.x;
                let old_y = win.y;
                let old_w = win.width;
                let old_h = win.height;

                if win.y <= WORK_AREA_TOP + EDGE_SNAP_THRESHOLD {
                    // Snap Top -> Maximized
                    win.x = 0;
                    win.y = WORK_AREA_TOP;
                    win.width = max_w as u32;
                    win.height = work_h;
                    win.state = WindowState::Maximized;
                    win.prev_state = WindowState::Normal;
                } else if win.x <= EDGE_SNAP_THRESHOLD {
                    // Snap Left
                    win.x = 0;
                    win.y = WORK_AREA_TOP;
                    win.width = (max_w as u32) / 2;
                    win.height = work_h;
                    win.state = WindowState::SnappedLeft;
                    win.prev_state = WindowState::Normal;
                } else if win.x + (win.width as i32) >= max_w - EDGE_SNAP_THRESHOLD {
                    // Snap Right
                    let tw = (max_w as u32) - ((max_w as u32) / 2);
                    win.x = (max_w / 2) as i32;
                    win.y = WORK_AREA_TOP;
                    win.width = tw;
                    win.height = work_h;
                    win.state = WindowState::SnappedRight;
                    win.prev_state = WindowState::Normal;
                } else {
                    win.state = WindowState::Normal;
                    win.normal_geom = (win.x, win.y, win.width, win.height);
                }

                let (ww, wh) = (win.width, win.height);
                crate::app_registry::rerender_app_for_window(wid, ww, wh);
                let min_box_x = old_x.min(win.x) - 4;
                let min_box_y = old_y.min(win.y) - 4;
                let max_box_w = (old_w.max(win.width)) + (old_x - win.x).abs() as u32 + 8;
                let max_box_h = (old_h.max(win.height)) + (old_y - win.y).abs() as u32 + 28;
                self.damage_tracker.add_bounds(min_box_x, min_box_y, max_box_w, max_box_h);
            }
        }

        if self.snap_preview != SnapPreview::None {
            self.snap_preview = SnapPreview::None;
            self.damage_tracker.add_bounds(0, WORK_AREA_TOP, max_w as u32, work_h);
        }

        if let Some((wid, ..)) = self.resizing_window.take() {
            if let Some(win) = self.windows.iter().find(|w| w.window_id == wid) {
                crate::app_registry::rerender_app_for_window(wid, win.width, win.height);
            }
        }
        crate::cursor::set_cursor_type(crate::cursor::CursorType::Default);
        let target_id = self.focused_window?;
        let owner_pid = self.windows.iter().find(|w| w.window_id == target_id).map(|w| w.owner_pid)?;
        Some((target_id, owner_pid))
    }

    /// Handles mouse move: updates dragging or resizing window coordinates with strict bounds clamping,
    /// and performs hover hit-testing across Z-order to update cursor type and button hover states.
    pub fn handle_mouse_move(&mut self, mx: i32, my: i32) {
        let max_w = unsafe { crate::gui::VESA.width as i32 };
        let max_h = unsafe { crate::gui::VESA.height as i32 };

        // 1. Handle Window Resizing (8-Way)
        if let Some((wid, edge, start_mx, start_my, orig_x, orig_y, orig_w, orig_h)) = self.resizing_window {
            let dx = mx - start_mx;
            let dy = my - start_my;

            if let Some(win) = self.windows.iter_mut().find(|w| w.window_id == wid) {
                let max_avail_h = (max_h - (orig_y + 20)).max(MIN_WINDOW_HEIGHT as i32) as u32;

                let old_w = win.width;
                let old_h = win.height;
                let old_x = win.x;
                let old_y = win.y;

                match edge {
                    ResizeEdge::Right => {
                        let new_w = ((orig_w as i32) + dx).clamp(MIN_WINDOW_WIDTH as i32, max_w - orig_x) as u32;
                        win.width = new_w;
                    }
                    ResizeEdge::Left => {
                        let max_left = orig_x + (orig_w as i32) - (MIN_WINDOW_WIDTH as i32);
                        let new_x = (orig_x + dx).clamp(0, max_left);
                        let new_w = ((orig_x + (orig_w as i32)) - new_x) as u32;
                        win.x = new_x;
                        win.width = new_w;
                    }
                    ResizeEdge::Bottom => {
                        let new_h = ((orig_h as i32) + dy).clamp(MIN_WINDOW_HEIGHT as i32, max_avail_h as i32) as u32;
                        win.height = new_h;
                    }
                    ResizeEdge::Top => {
                        let max_top = orig_y + (orig_h as i32) - (MIN_WINDOW_HEIGHT as i32);
                        let new_y = (orig_y + dy).clamp(WORK_AREA_TOP, max_top);
                        let new_h = ((orig_y + (orig_h as i32)) - new_y) as u32;
                        win.y = new_y;
                        win.height = new_h;
                    }
                    ResizeEdge::TopLeft => {
                        let max_left = orig_x + (orig_w as i32) - (MIN_WINDOW_WIDTH as i32);
                        let new_x = (orig_x + dx).clamp(0, max_left);
                        let new_w = ((orig_x + (orig_w as i32)) - new_x) as u32;
                        let max_top = orig_y + (orig_h as i32) - (MIN_WINDOW_HEIGHT as i32);
                        let new_y = (orig_y + dy).clamp(WORK_AREA_TOP, max_top);
                        let new_h = ((orig_y + (orig_h as i32)) - new_y) as u32;
                        win.x = new_x;
                        win.width = new_w;
                        win.y = new_y;
                        win.height = new_h;
                    }
                    ResizeEdge::TopRight => {
                        let new_w = ((orig_w as i32) + dx).clamp(MIN_WINDOW_WIDTH as i32, max_w - orig_x) as u32;
                        let max_top = orig_y + (orig_h as i32) - (MIN_WINDOW_HEIGHT as i32);
                        let new_y = (orig_y + dy).clamp(WORK_AREA_TOP, max_top);
                        let new_h = ((orig_y + (orig_h as i32)) - new_y) as u32;
                        win.width = new_w;
                        win.y = new_y;
                        win.height = new_h;
                    }
                    ResizeEdge::BottomLeft => {
                        let max_left = orig_x + (orig_w as i32) - (MIN_WINDOW_WIDTH as i32);
                        let new_x = (orig_x + dx).clamp(0, max_left);
                        let new_w = ((orig_x + (orig_w as i32)) - new_x) as u32;
                        let new_h = ((orig_h as i32) + dy).clamp(MIN_WINDOW_HEIGHT as i32, max_avail_h as i32) as u32;
                        win.x = new_x;
                        win.width = new_w;
                        win.height = new_h;
                    }
                    ResizeEdge::BottomRight => {
                        let new_w = ((orig_w as i32) + dx).clamp(MIN_WINDOW_WIDTH as i32, max_w - orig_x) as u32;
                        let new_h = ((orig_h as i32) + dy).clamp(MIN_WINDOW_HEIGHT as i32, max_avail_h as i32) as u32;
                        win.width = new_w;
                        win.height = new_h;
                    }
                    _ => {}
                }

                let min_box_x = old_x.min(win.x) - 4;
                let min_box_y = old_y.min(win.y) - 4;
                let max_box_w = (old_w.max(win.width)) + (old_x - win.x).abs() as u32 + 8;
                let max_box_h = (old_h.max(win.height)) + (old_y - win.y).abs() as u32 + 28;
                self.damage_tracker.add_bounds(min_box_x, min_box_y, max_box_w, max_box_h);
                win.state = WindowState::Normal;
                win.normal_geom = (win.x, win.y, win.width, win.height);
            }
            let ctype = match edge {
                ResizeEdge::Left | ResizeEdge::Right => crate::cursor::CursorType::ResizeHorizontal,
                ResizeEdge::Top | ResizeEdge::Bottom => crate::cursor::CursorType::ResizeVertical,
                _ => crate::cursor::CursorType::ResizeDiagonal,
            };
            crate::cursor::set_cursor_type(ctype);
            return;
        }

        // 2. Handle Window Dragging
        if let Some((wid, ox, oy)) = self.dragging_window {
            if let Some(win) = self.windows.iter_mut().find(|w| w.window_id == wid) {
                // If dragged while maximized or snapped, restore normal state and continue dragging smoothly
                if win.state == WindowState::Maximized || win.state == WindowState::SnappedLeft || win.state == WindowState::SnappedRight {
                    let (_, _, pw, ph) = win.normal_geom;
                    win.width = pw;
                    win.height = ph;
                    win.state = WindowState::Normal;
                    win.prev_state = WindowState::Normal;
                    win.saved_geom = None;
                }
                let prev_x = win.x;
                let prev_y = win.y;
                let new_x = (mx - ox).clamp(-100, max_w.saturating_sub(50));
                let new_y = (my - oy).clamp(WORK_AREA_TOP, max_h.saturating_sub(30));

                let min_box_x = prev_x.min(new_x) - 4;
                let min_box_y = prev_y.min(new_y) - 4;
                let max_box_w = win.width + (prev_x - new_x).abs() as u32 + 8;
                let max_box_h = win.height + (prev_y - new_y).abs() as u32 + 28;
                self.damage_tracker.add_bounds(min_box_x, min_box_y, max_box_w, max_box_h);

                win.x = new_x;
                win.y = new_y;
                if win.state == WindowState::Normal {
                    win.normal_geom = (new_x, new_y, win.width, win.height);
                }

                let new_preview = if win.y <= WORK_AREA_TOP + EDGE_SNAP_THRESHOLD {
                    SnapPreview::Maximized
                } else if win.x <= EDGE_SNAP_THRESHOLD {
                    SnapPreview::Left
                } else if win.x + (win.width as i32) >= max_w - EDGE_SNAP_THRESHOLD {
                    SnapPreview::Right
                } else {
                    SnapPreview::None
                };

                if self.snap_preview != new_preview {
                    self.snap_preview = new_preview;
                    let work_h = max_h.saturating_sub(WORK_AREA_TOP + DOCK_HEIGHT as i32 + 20) as u32;
                    self.damage_tracker.add_bounds(0, WORK_AREA_TOP, max_w as u32, work_h);
                }
            }
            crate::cursor::set_cursor_type(crate::cursor::CursorType::Hand);
            return;
        }

        // 2b. Check Top Bar Tab Hover (my < DOCK_HEIGHT as i32)
        if my < DOCK_HEIGHT as i32 {
            let new_tab = if mx >= 102 && mx <= max_w.saturating_sub(210) {
                let tab_idx = ((mx - 102) / 92) as usize;
                if tab_idx < self.windows.len() { Some(tab_idx) } else { None }
            } else {
                None
            };

            if self.hovered_dock_tab != new_tab {
                self.hovered_dock_tab = new_tab;
                self.mark_top_bar_damage();
            }

            crate::cursor::set_cursor_type(if mx <= 92 || new_tab.is_some() {
                crate::cursor::CursorType::Hand
            } else {
                crate::cursor::CursorType::Default
            });
            return;
        } else if self.hovered_dock_tab.is_some() {
            self.hovered_dock_tab = None;
            self.mark_top_bar_damage();
        }

        // 3. Hover Detection and Cursor Type Selection (Z-Order Top-to-Bottom)
        let mut hovered = None;
        let mut cursor_type = crate::cursor::CursorType::Default;

        for win in self.windows.iter().rev() {
            if !win.visible || win.state == WindowState::Minimized {
                continue;
            }

            let wx = win.x;
            let wy = win.y;
            let ww = win.width as i32;
            let wh = win.height as i32;
            let wid = win.window_id;

            // Check if mouse is within window bounding box (including resize borders)
            if win.state == WindowState::Fullscreen {
                hovered = Some(HoverTarget {
                    window_id: wid,
                    button: ChromeButton::None,
                    is_titlebar: false,
                    resize_edge: ResizeEdge::None,
                });
                cursor_type = crate::cursor::CursorType::Default;
                break;
            }

            if mx >= wx - 6 && mx <= wx + ww + 6 && my >= wy - 6 && my <= wy + 20 + wh + 6 {
                // Resize corners & edges (only if not maximized or fullscreen)
                if win.state != WindowState::Maximized && win.state != WindowState::Fullscreen {
                    if (mx >= wx + ww - 8 && mx <= wx + ww + 6 && my >= wy + 20 + wh - 8 && my <= wy + 20 + wh + 6) ||
                       (mx >= wx - 6 && mx <= wx + 8 && my >= wy - 6 && my <= wy + 8) {
                        hovered = Some(HoverTarget {
                            window_id: wid,
                            button: ChromeButton::None,
                            is_titlebar: false,
                            resize_edge: ResizeEdge::BottomRight,
                        });
                        cursor_type = crate::cursor::CursorType::ResizeDiagonal;
                        break;
                    } else if (mx >= wx - 6 && mx <= wx + 8 && my >= wy + 20 + wh - 8 && my <= wy + 20 + wh + 6) ||
                              (mx >= wx + ww - 8 && mx <= wx + ww + 6 && my >= wy - 6 && my <= wy + 8) {
                        hovered = Some(HoverTarget {
                            window_id: wid,
                            button: ChromeButton::None,
                            is_titlebar: false,
                            resize_edge: ResizeEdge::BottomLeft,
                        });
                        cursor_type = crate::cursor::CursorType::ResizeDiagonal;
                        break;
                    } else if (mx >= wx + ww - 4 && mx <= wx + ww + 6 && my >= wy && my <= wy + 20 + wh) ||
                              (mx >= wx - 6 && mx <= wx + 4 && my >= wy && my <= wy + 20 + wh) {
                        hovered = Some(HoverTarget {
                            window_id: wid,
                            button: ChromeButton::None,
                            is_titlebar: false,
                            resize_edge: ResizeEdge::Right,
                        });
                        cursor_type = crate::cursor::CursorType::ResizeHorizontal;
                        break;
                    } else if (mx >= wx && mx <= wx + ww && my >= wy + 20 + wh - 4 && my <= wy + 20 + wh + 6) ||
                              (mx >= wx && mx <= wx + ww && my >= wy - 6 && my <= wy + 4) {
                        hovered = Some(HoverTarget {
                            window_id: wid,
                            button: ChromeButton::None,
                            is_titlebar: false,
                            resize_edge: ResizeEdge::Bottom,
                        });
                        cursor_type = crate::cursor::CursorType::ResizeVertical;
                        break;
                    }
                }

                // Check Titlebar (my in wy .. wy + 20)
                if mx >= wx && mx < wx + ww && my >= wy && my < wy + 20 {
                    // Close button [×] (Rightmost: wx + ww - 20 .. wx + ww - 4)
                    if mx >= wx + ww - 20 && mx <= wx + ww - 4 && my >= wy + 2 && my <= wy + 18 {
                        hovered = Some(HoverTarget {
                            window_id: wid,
                            button: ChromeButton::Close,
                            is_titlebar: false,
                            resize_edge: ResizeEdge::None,
                        });
                        cursor_type = crate::cursor::CursorType::Hand;
                        break;
                    }
                    // Maximize button [□] (Middle: wx + ww - 38 .. wx + ww - 22)
                    if mx >= wx + ww - 38 && mx <= wx + ww - 22 && my >= wy + 2 && my <= wy + 18 {
                        hovered = Some(HoverTarget {
                            window_id: wid,
                            button: ChromeButton::Maximize,
                            is_titlebar: false,
                            resize_edge: ResizeEdge::None,
                        });
                        cursor_type = crate::cursor::CursorType::Hand;
                        break;
                    }
                    // Minimize button [—] (Left: wx + ww - 56 .. wx + ww - 40)
                    if mx >= wx + ww - 56 && mx <= wx + ww - 40 && my >= wy + 2 && my <= wy + 18 {
                        hovered = Some(HoverTarget {
                            window_id: wid,
                            button: ChromeButton::Minimize,
                            is_titlebar: false,
                            resize_edge: ResizeEdge::None,
                        });
                        cursor_type = crate::cursor::CursorType::Hand;
                        break;
                    }

                    // Titlebar drag area
                    hovered = Some(HoverTarget {
                        window_id: wid,
                        button: ChromeButton::None,
                        is_titlebar: true,
                        resize_edge: ResizeEdge::None,
                    });
                    cursor_type = crate::cursor::CursorType::Hand;
                    break;
                }

                // Client Area
                if mx >= wx && mx < wx + ww && my >= wy + 20 && my < wy + 20 + wh {
                    hovered = Some(HoverTarget {
                        window_id: wid,
                        button: ChromeButton::None,
                        is_titlebar: false,
                        resize_edge: ResizeEdge::None,
                    });
                    cursor_type = crate::cursor::CursorType::Default;
                    break;
                }
            }
        }

        if self.hovered_target != hovered {
            if let Some(ref h) = self.hovered_target {
                if let Some(w) = self.windows.iter().find(|win| win.window_id == h.window_id) {
                    self.damage_tracker.add_bounds(w.x, w.y, w.width, 20);
                }
            }
            if let Some(ref h) = hovered {
                if let Some(w) = self.windows.iter().find(|win| win.window_id == h.window_id) {
                    self.damage_tracker.add_bounds(w.x, w.y, w.width, 20);
                }
            }
        }

        self.hovered_target = hovered;
        crate::cursor::set_cursor_type(cursor_type);
    }

    /// Cleans up all windows owned by a terminating process and re-evaluates focus.
    pub fn cleanup_windows_for_pid(&mut self, pid: u64) {
        let initial_count = self.windows.len();
        self.windows.retain(|w| w.owner_pid != pid);
        let cleaned = initial_count - self.windows.len();

        if cleaned > 0 {
            // Re-evaluate focus if previous focused window was destroyed
            if let Some(fid) = self.focused_window {
                if !self.windows.iter().any(|w| w.window_id == fid) {
                    self.focused_window = self.windows.iter().rev().find(|w| w.visible).map(|w| w.window_id);
                    if let Some(new_fid) = self.focused_window {
                        if let Some(w) = self.windows.iter_mut().find(|w| w.window_id == new_fid) {
                            w.focused = true;
                        }
                    }
                }
            }
            crate::serial_println!("[WM] Cleaned up {} orphaned window(s) for terminating PID {}", cleaned, pid);
        }
    }
}

pub static WM: Mutex<WindowManager> = Mutex::new(WindowManager::new());

/// Public helper to clean up windows on process exit
pub fn cleanup_windows_for_pid(pid: u64) {
    WM.lock().cleanup_windows_for_pid(pid);
}

/// Target frame interval for 60 FPS in milliseconds (PIT 1000 Hz timer ticks)
pub const FRAME_INTERVAL_TICKS: u64 = 16; // ~16.67 ms (60 FPS)

/// Decoupled, paced compositor task running cooperatively in SimpleExecutor.
/// Only performs rendering if damage exists and at most once every ~16 ms.
pub async fn compositor_task() {
    let mut last_render_tick = 0u64;

    loop {
        if crate::vga_buffer::GUI_MODE.load(core::sync::atomic::Ordering::Relaxed) {
            let now = crate::interrupts::get_tick();
            let elapsed = now.saturating_sub(last_render_tick);

            if elapsed >= FRAME_INTERVAL_TICKS {
                let mut wm = WM.lock();
                if wm.damage_tracker.is_damaged() {
                    let cx = crate::mouse::MOUSE_X.load(core::sync::atomic::Ordering::Relaxed);
                    let cy = crate::mouse::MOUSE_Y.load(core::sync::atomic::Ordering::Relaxed);
                    wm.composite_desktop(cx as i32, cy as i32);
                    last_render_tick = now;
                }
            }
        }

        crate::task::yield_now().await;
    }
}
