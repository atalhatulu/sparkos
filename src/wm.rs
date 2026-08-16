//! SparkOS — Window Manager, Compositor, Dock & Launcher Subsystem (Desktop V1.1 / Steps 1-6)
//!
//! Provides Ring-3 / Microkernel Window Management, Z-Order Back-to-Front Compositing,
//! Window Chrome (Titlebar & Buttons), Window Geometry & Resize Hardening, Bottom Dock,
//! Application Launcher, Application Registry, and Icon Rendering.

use alloc::vec::Vec;
use spin::Mutex;

pub const WORK_AREA_TOP: i32 = 20;
pub const DOCK_HEIGHT: u16 = 24;
pub const MIN_WINDOW_WIDTH: u32 = 120;
pub const MIN_WINDOW_HEIGHT: u32 = 60;

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
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    None,
    Left,
    Right,
    Bottom,
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
    pub saved_geom: Option<(i32, i32, u32, u32)>,
}

pub struct WindowManager {
    /// Windows ordered from Back (index 0) to Front (index len - 1).
    pub windows: Vec<Window>,
    pub next_window_id: u64,
    pub focused_window: Option<u64>,
    pub dragging_window: Option<(u64, i32, i32)>,
    pub resizing_window: Option<(u64, ResizeEdge, i32, i32, i32, i32, u32, u32)>,
    pub hovered_target: Option<HoverTarget>,
    pub launcher_open: bool,
    pub pending_spawn_app: Option<u8>,
    pub last_titlebar_click: Option<(u64, u64)>, // (window_id, tick)
}

impl WindowManager {
    pub const fn new() -> Self {
        Self {
            windows: Vec::new(),
            next_window_id: 1,
            focused_window: None,
            dragging_window: None,
            resizing_window: None,
            hovered_target: None,
            launcher_open: false,
            pending_spawn_app: None,
            last_titlebar_click: None,
        }
    }

    /// Creates a new window associated with an existing Surface.
    pub fn create_window(
        &mut self,
        owner_pid: u64,
        surface_id: u64,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<u64, WmError> {
        let max_w = unsafe { crate::gui::VESA.width as u32 };
        let max_h = unsafe { crate::gui::VESA.height as u32 };
        if width == 0 || height == 0 || width > max_w || height > max_h {
            return Err(WmError::InvalidDimensions);
        }

        // Hardening Check: Verify surface ownership if a surface is bound
        if surface_id != 0 {
            let reg = crate::surface::SURFACE_REGISTRY.lock();
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
            saved_geom: None,
        };

        // Appended at the end (top-most in Z-order)
        self.windows.push(win);
        self.focused_window = Some(window_id);

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

    /// Minimizes a window, hiding it and transferring focus. Enforces caller ownership.
    pub fn minimize_window(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        let win = self.windows.iter_mut().find(|w| w.window_id == window_id)
            .ok_or(WmError::NotFound)?;

        if win.owner_pid != caller_pid {
            return Err(WmError::PermissionDenied);
        }

        win.visible = false;
        win.focused = false;
        win.state = WindowState::Minimized;

        if self.focused_window == Some(window_id) {
            self.focused_window = self.windows.iter().rev().find(|w| w.visible).map(|w| w.window_id);
            if let Some(fid) = self.focused_window {
                if let Some(w) = self.windows.iter_mut().find(|w| w.window_id == fid) {
                    w.focused = true;
                }
            }
        }
        Ok(())
    }

    /// Restores a minimized window to normal and raises to top. Enforces caller ownership.
    pub fn restore_window(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        {
            let win = self.windows.iter_mut().find(|w| w.window_id == window_id)
                .ok_or(WmError::NotFound)?;

            if win.owner_pid != caller_pid {
                return Err(WmError::PermissionDenied);
            }

            win.visible = true;
            win.state = WindowState::Normal;
        }
        self.raise_to_top_internal(window_id)
    }

    /// Toggles maximize state of a window, preserving previous geometry. Enforces caller ownership.
    pub fn toggle_maximize_window(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        let max_w = unsafe { crate::gui::VESA.width as u32 };
        let max_h = unsafe { crate::gui::VESA.height as u32 };
        let work_h = max_h.saturating_sub(WORK_AREA_TOP as u32 + DOCK_HEIGHT as u32 + 20);

        let win = self.windows.iter_mut().find(|w| w.window_id == window_id)
            .ok_or(WmError::NotFound)?;

        if win.owner_pid != caller_pid {
            return Err(WmError::PermissionDenied);
        }

        if win.state == WindowState::Maximized {
            // Restore previous geometry
            if let Some((px, py, pw, ph)) = win.saved_geom.take() {
                win.x = px.clamp(0, (max_w.saturating_sub(MIN_WINDOW_WIDTH)) as i32);
                win.y = py.clamp(WORK_AREA_TOP, (max_h.saturating_sub(MIN_WINDOW_HEIGHT + DOCK_HEIGHT as u32 + 20)) as i32);
                win.width = pw.clamp(MIN_WINDOW_WIDTH, max_w);
                win.height = ph.clamp(MIN_WINDOW_HEIGHT, work_h);
            } else {
                win.x = 30;
                win.y = 35;
                win.width = 220;
                win.height = 110;
            }
            win.state = WindowState::Normal;
        } else {
            // Save current geometry and maximize to full desktop workspace (between top bar and dock)
            win.saved_geom = Some((win.x, win.y, win.width, win.height));
            win.x = 0;
            win.y = WORK_AREA_TOP;
            win.width = max_w;
            win.height = work_h;
            win.state = WindowState::Maximized;
        }

        self.raise_to_top_internal(window_id)
    }

    /// Toggles true fullscreen mode for a window covering (0, 0, screen_w, screen_h).
    pub fn toggle_fullscreen(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        let max_w = unsafe { crate::gui::VESA.width as u32 };
        let max_h = unsafe { crate::gui::VESA.height as u32 };

        let win = self.windows.iter_mut().find(|w| w.window_id == window_id)
            .ok_or(WmError::NotFound)?;

        if win.owner_pid != caller_pid {
            return Err(WmError::PermissionDenied);
        }

        if win.state == WindowState::Fullscreen {
            if let Some((px, py, pw, ph)) = win.saved_geom.take() {
                win.x = px.clamp(0, (max_w.saturating_sub(MIN_WINDOW_WIDTH)) as i32);
                win.y = py.clamp(WORK_AREA_TOP, (max_h.saturating_sub(MIN_WINDOW_HEIGHT + DOCK_HEIGHT as u32 + 20)) as i32);
                win.width = pw.clamp(MIN_WINDOW_WIDTH, max_w);
                win.height = ph.clamp(MIN_WINDOW_HEIGHT, max_h);
            } else {
                win.x = 40;
                win.y = 40;
                win.width = 420;
                win.height = 240;
            }
            win.state = WindowState::Normal;
        } else {
            if win.state != WindowState::Maximized {
                win.saved_geom = Some((win.x, win.y, win.width, win.height));
            }
            win.x = 0;
            win.y = 0;
            win.width = max_w;
            win.height = max_h;
            win.state = WindowState::Fullscreen;
        }

        self.raise_to_top_internal(window_id)
    }

    /// Cycles through open windows via Alt-Tab, restoring minimized windows and raising target to top
    pub fn alt_tab_cycle(&mut self) -> Option<u64> {
        if self.windows.is_empty() { return None; }

        let cur_focus = self.focused_window;
        let mut cur_idx = self.windows.len().saturating_sub(1);
        if let Some(fid) = cur_focus {
            if let Some(pos) = self.windows.iter().position(|w| w.window_id == fid) {
                cur_idx = pos;
            }
        }

        // Cycle to next window (wrapping around)
        let next_idx = if cur_idx == 0 { self.windows.len() - 1 } else { cur_idx - 1 };
        let next_wid = self.windows[next_idx].window_id;
        let owner = self.windows[next_idx].owner_pid;

        if self.windows[next_idx].state == WindowState::Minimized {
            let _ = self.restore_window(owner, next_wid);
        } else {
            let _ = self.raise_to_top_internal(next_wid);
        }

        self.focused_window
    }

    /// Destroys a window with strict ownership verification, surface reclamation, and process cleanup.
    pub fn destroy_window(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        let idx = self.windows.iter().position(|w| w.window_id == window_id)
            .ok_or(WmError::NotFound)?;

        if self.windows[idx].owner_pid != caller_pid {
            return Err(WmError::PermissionDenied);
        }

        let surf_id = self.windows[idx].surface_id;
        self.windows.remove(idx);

        if self.focused_window == Some(window_id) {
            // Transfer focus to the next topmost visible window
            self.focused_window = self.windows.iter().rev().find(|w| w.visible && w.state != WindowState::Minimized).map(|w| w.window_id);
            if let Some(fid) = self.focused_window {
                if let Some(w) = self.windows.iter_mut().find(|w| w.window_id == fid) {
                    w.focused = true;
                }
            }
        }

        // Clean up surface registry for this window and uncharge accounting
        if surf_id != 0 {
            let mut reg = crate::surface::SURFACE_REGISTRY.lock();
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

        // Clean up per-window terminal instance if attached
        crate::terminal_app::cleanup_terminal_for_window(window_id);
        crate::files_app::cleanup_files_for_window(window_id);
        crate::editor_app::cleanup_editor_for_window(window_id);

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
        }

        crate::serial_println!("[WM] Process {} destroyed Window {} (Surface {} cleaned, Remaining windows: {})",
            caller_pid, window_id, surf_id, remaining_windows);
        Ok(())
    }

    /// Moves a window to new coordinates with strict ownership verification.
    pub fn move_window(&mut self, caller_pid: u64, window_id: u64, new_x: i32, new_y: i32) -> Result<(), WmError> {
        let max_w = unsafe { crate::gui::VESA.width as i32 };
        let max_h = unsafe { crate::gui::VESA.height as i32 };

        let win = self.windows.iter_mut().find(|w| w.window_id == window_id)
            .ok_or(WmError::NotFound)?;

        if win.owner_pid != caller_pid {
            return Err(WmError::PermissionDenied);
        }

        win.x = new_x.clamp(-100, max_w - 50);
        win.y = new_y.clamp(WORK_AREA_TOP, max_h - (DOCK_HEIGHT as i32 + 30));
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
        let idx = self.windows.iter().position(|w| w.window_id == window_id)
            .ok_or(WmError::NotFound)?;

        let mut win = self.windows.remove(idx);

        for w in self.windows.iter_mut() {
            w.focused = false;
        }

        win.focused = true;
        self.focused_window = Some(window_id);
        self.windows.push(win);

        crate::serial_println!("[WM] Window {} raised to Top & FOCUSED", window_id);
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
    pub fn composite_desktop(&self, _mouse_x: i32, _mouse_y: i32) {
        unsafe {
            if crate::gui::BACKBUFFER.is_null() { return; }
        }

        let screen_w = unsafe { crate::gui::VESA.width as u16 };
        let screen_h = unsafe { crate::gui::VESA.height as u16 };
        let dock_y = screen_h.saturating_sub(DOCK_HEIGHT);
        let active_theme = crate::theme::THEME_MANAGER.lock().current_theme;

        // 1. Desktop Environment (Wallpaper Gradient Engine & Desktop Icons)
        crate::desktop::DESKTOP_ENV.lock().render(screen_w, screen_h);

        // 3. Composite Windows in Z-Order (Back-to-Front)
        let surf_reg = crate::surface::SURFACE_REGISTRY.lock();
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
                // 3-FS. True Fullscreen: Scaled blit across entire display area
                if let Some(surface) = surf_reg.iter().find(|s| s.surface_id == win.surface_id) {
                    let phys_addr = surface.shmem_phys_addr;
                    let src_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *const u32 };
                    let target_w = screen_w as usize;
                    let target_h = screen_h as usize;

                    unsafe {
                        if !crate::gui::BACKBUFFER.is_null() && target_w > 0 && target_h > 0 {
                            let surf_w = surface.width as usize;
                            let surf_h = surface.height as usize;

                            if surf_w == target_w && surf_h == target_h {
                                for r in 0..target_h {
                                    let src_offset = r * surf_w;
                                    let dst_offset = r * (screen_w as usize);
                                    core::ptr::copy_nonoverlapping(
                                        src_ptr.add(src_offset),
                                        crate::gui::BACKBUFFER.add(dst_offset),
                                        target_w,
                                    );
                                }
                            } else {
                                let step_x = ((surf_w as u64) << 16) / (target_w as u64);
                                let step_y = ((surf_h as u64) << 16) / (target_h as u64);

                                for r in 0..target_h {
                                    let src_y = (((r as u64) * step_y) >> 16) as usize;
                                    let src_row_offset = src_y.min(surf_h.saturating_sub(1)) * surf_w;
                                    let dst_row_offset = r * (screen_w as usize);

                                    for c in 0..target_w {
                                        let src_x = (((c as u64) * step_x) >> 16) as usize;
                                        let pixel = *src_ptr.add(src_row_offset + src_x.min(surf_w.saturating_sub(1)));
                                        *crate::gui::BACKBUFFER.add(dst_row_offset + c) = pixel;
                                    }
                                }
                            }
                        }
                    }
                }
                continue;
            }

            // 3a. Titlebar (20px high)
            crate::gui::draw_rect(wx, wy, ww, 20, title_bg);
            
            // Draw App Icon
            let icon_type = match win.owner_pid {
                1 => crate::app_registry::AppIcon::Terminal,
                2 => crate::app_registry::AppIcon::Files,
                3 => crate::app_registry::AppIcon::Generic,
                _ => crate::app_registry::AppIcon::Generic,
            };
            crate::gui::draw_icon_glyph(wx + 6, wy + 6, icon_type, title_fg, title_bg);

            // Title text (App Name / PID)
            let app_name = match win.owner_pid {
                1 => "Terminal",
                2 => "Demo App",
                3 => "Files",
                4 => "Settings",
                5 => "Task Manager",
                6 => "Web Browser",
                7 => "System Monitor",
                8 => "Text Editor",
                _ => "SparkOS Application",
            };
            crate::gui::draw_string(wx + 18, wy + 6, app_name, title_fg, title_bg);

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

            // 3c. Blit shared-memory surface if present (with 1:1 or scaled fill)
            if let Some(surface) = surf_reg.iter().find(|s| s.surface_id == win.surface_id) {
                let phys_addr = surface.shmem_phys_addr;
                let src_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *const u32 };
                let target_w = (ww as usize).min((screen_w.saturating_sub(wx)) as usize);
                let target_h = (wh as usize).min((dock_y.saturating_sub(wy + 20)) as usize);

                unsafe {
                    if !crate::gui::BACKBUFFER.is_null() && target_w > 0 && target_h > 0 {
                        let surf_w = surface.width as usize;
                        let surf_h = surface.height as usize;

                        if surf_w == target_w && surf_h == target_h {
                            for r in 0..target_h {
                                let dst_row = (wy + 20) as usize + r;
                                let src_offset = r * surf_w;
                                let dst_offset = dst_row * (screen_w as usize) + (wx as usize);
                                core::ptr::copy_nonoverlapping(
                                    src_ptr.add(src_offset),
                                    crate::gui::BACKBUFFER.add(dst_offset),
                                    target_w,
                                );
                            }
                        } else {
                            let step_x = ((surf_w as u64) << 16) / (target_w as u64);
                            let step_y = ((surf_h as u64) << 16) / (target_h as u64);

                            for r in 0..target_h {
                                let dst_row = (wy + 20) as usize + r;
                                let src_y = (((r as u64) * step_y) >> 16) as usize;
                                let src_row_offset = src_y.min(surf_h.saturating_sub(1)) * surf_w;
                                let dst_row_offset = dst_row * (screen_w as usize) + (wx as usize);

                                for c in 0..target_w {
                                    let src_x = (((c as u64) * step_x) >> 16) as usize;
                                    let pixel = *src_ptr.add(src_row_offset + src_x.min(surf_w.saturating_sub(1)));
                                    *crate::gui::BACKBUFFER.add(dst_row_offset + c) = pixel;
                                }
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

        // 4. Bottom Bar / Dock (y = dock_y, h = 24)
        crate::gui::draw_rect(0, dock_y, screen_w, DOCK_HEIGHT, active_theme.dock_background);
        crate::gui::draw_rect(0, dock_y, screen_w, 1, active_theme.border_color); // Top border

        // 4a. SparkOS Launcher Button on Dock
        let l_bg = if self.launcher_open { 0x002563EB } else { 0x001E293B };
        crate::gui::draw_rect(4, dock_y + 2, 74, 20, l_bg);
        crate::gui::draw_icon_glyph(8, dock_y + 8, crate::app_registry::AppIcon::Logo, 0x0038BDF8, l_bg);
        crate::gui::draw_string(20, dock_y + 7, "SparkOS", 0x00FFFFFF, l_bg);

        // 4b. Window Tabs on Dock
        let mut tab_x = 84u16;
        for win in self.windows.iter() {
            let is_focused = self.focused_window == Some(win.window_id);
            let tab_bg = if is_focused {
                0x002563EB // Focused Blue
            } else if win.state == WindowState::Minimized {
                0x0009090B // Minimized dark
            } else {
                0x001E293B // Unfocused normal
            };
            let tab_fg = if is_focused { 0x00FFFFFF } else if win.state == WindowState::Minimized { 0x0064748B } else { 0x00E2E8F0 };

            if tab_x + 84 <= screen_w.saturating_sub(80) {
                crate::gui::draw_rect(tab_x, dock_y + 2, 80, 20, tab_bg);
                let tab_icon = match win.owner_pid {
                    1 => crate::app_registry::AppIcon::Terminal,
                    2 => crate::app_registry::AppIcon::Demo,
                    3 => crate::app_registry::AppIcon::Files,
                    _ => crate::app_registry::AppIcon::Generic,
                };
                crate::gui::draw_icon_glyph(tab_x + 4, dock_y + 8, tab_icon, tab_fg, tab_bg);
                let app_prefix = match win.owner_pid {
                    1 => "Term",
                    2 => "Demo",
                    3 => "Files",
                    4 => "Set",
                    5 => "Task",
                    6 => "Web",
                    7 => "Sys",
                    8 => "Edit",
                    _ => "Win",
                };
                let short_name = alloc::format!("{}-{}", app_prefix, win.window_id);
                crate::gui::draw_string(tab_x + 16, dock_y + 7, &short_name, tab_fg, tab_bg);

                // Active dot indicator
                if is_focused {
                    crate::gui::draw_rect(tab_x + 36, dock_y + 19, 8, 2, 0x0060A5FA);
                }
                tab_x += 84;
            }
        }

        // 4c. Dock Background Completed

        // 5. System Top Bar (Top layer above windows and dock, below cursor)
        crate::system_bar::SYSTEM_BAR.lock().render(screen_w, screen_h);

        // 6. Application Launcher Popup (if open)
        if self.launcher_open {
            let px = 4u16;
            let pw = 154u16;
            let total_apps = crate::app_registry::REGISTERED_APPS.len() as u16;
            let ph = 34 + total_apps * 28 + 26;
            let py = dock_y.saturating_sub(ph + 4);

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
            for app in crate::app_registry::REGISTERED_APPS.iter() {
                crate::gui::draw_rect(px + 4, item_y, pw - 8, 24, 0x001E293B);
                crate::gui::draw_icon_glyph(px + 8, item_y + 8, app.icon, 0x00FFFFFF, 0x001E293B);
                crate::gui::draw_string(px + 22, item_y + 7, app.name, 0x00E2E8F0, 0x001E293B);
                item_y += 28;
            }

            // Close launcher button
            crate::gui::draw_rect(px + 4, item_y, pw - 8, 20, 0x00334155);
            crate::gui::draw_string(px + 36, item_y + 5, "Close Menu", 0x0094A3B8, 0x00334155);
        }

        // 6. Global Freeze Protection / Crash Modal & Diagnostic Overlay
        crate::crash_reporter::CRASH_REPORTER.lock().render_crash_modal(screen_w, screen_h);

        // 7. Draw topmost mouse cursor layer
        crate::cursor::draw_cursor_layer();

        // 8. Swap buffers to hardware VESA Framebuffer
        crate::gui::swap_buffers();
    }

    /// Handles mouse down: performs titlebar dragging, resize, close/maximize/minimize buttons, dock interaction, or client focus
    pub fn handle_mouse_down(&mut self, mx: i32, my: i32) -> Option<(u64, u64)> {
        let screen_h = unsafe { crate::gui::VESA.height as i32 };
        let dock_y = screen_h.saturating_sub(DOCK_HEIGHT as i32);

        // 1. Check Launcher Popup Click (if open)
        if self.launcher_open {
            let px = 4;
            let pw = 154;
            let total_apps = crate::app_registry::REGISTERED_APPS.len() as i32;
            let ph = 34 + total_apps * 28 + 26;
            let py = dock_y.saturating_sub(ph + 4);

            if mx >= px && mx < px + pw && my >= py && my < py + ph {
                let mut cur_y = py + 28;
                for app in crate::app_registry::REGISTERED_APPS.iter() {
                    if my >= cur_y && my < cur_y + 24 {
                        self.pending_spawn_app = Some(app.id);
                        self.launcher_open = false;
                        return None;
                    }
                    cur_y += 28;
                }
                // Close Menu button
                if my >= cur_y && my < cur_y + 24 {
                    self.launcher_open = false;
                    return None;
                }
            } else if !(mx >= 4 && mx <= 80 && my >= dock_y) {
                // Clicked outside launcher popup -> close launcher
                self.launcher_open = false;
            }
        }

        // 2. Check Bottom Bar / Dock Click (y >= dock_y)
        if my >= dock_y {
            // Launcher button click (x in 4..80)
            if mx >= 4 && mx <= 80 {
                self.launcher_open = !self.launcher_open;
                return None;
            }

            // Window Tab click (x >= 84)
            if mx >= 84 {
                let tab_idx = ((mx - 84) / 84) as usize;
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

            // 3a. Check Resize Borders (4px margin around window edges)
            if win.state != WindowState::Maximized {
                // Bottom-Right Corner (8x8)
                if mx >= wx + ww - 8 && mx <= wx + ww + 4 && my >= wy + 20 + wh - 8 && my <= wy + 20 + wh + 4 {
                    self.resizing_window = Some((wid, ResizeEdge::BottomRight, mx, my, wx, wy, win.width, win.height));
                    let _ = self.raise_to_top_internal(wid);
                    return Some((wid, owner));
                }
                // Bottom-Left Corner (8x8)
                if mx >= wx - 4 && mx <= wx + 8 && my >= wy + 20 + wh - 8 && my <= wy + 20 + wh + 4 {
                    self.resizing_window = Some((wid, ResizeEdge::BottomLeft, mx, my, wx, wy, win.width, win.height));
                    let _ = self.raise_to_top_internal(wid);
                    return Some((wid, owner));
                }
                // Right Edge
                if mx >= wx + ww - 4 && mx <= wx + ww + 4 && my >= wy && my <= wy + 20 + wh {
                    self.resizing_window = Some((wid, ResizeEdge::Right, mx, my, wx, wy, win.width, win.height));
                    let _ = self.raise_to_top_internal(wid);
                    return Some((wid, owner));
                }
                // Left Edge
                if mx >= wx - 4 && mx <= wx + 4 && my >= wy && my <= wy + 20 + wh {
                    self.resizing_window = Some((wid, ResizeEdge::Left, mx, my, wx, wy, win.width, win.height));
                    let _ = self.raise_to_top_internal(wid);
                    return Some((wid, owner));
                }
                // Bottom Edge
                if mx >= wx && mx <= wx + ww && my >= wy + 20 + wh - 4 && my <= wy + 20 + wh + 4 {
                    self.resizing_window = Some((wid, ResizeEdge::Bottom, mx, my, wx, wy, win.width, win.height));
                    let _ = self.raise_to_top_internal(wid);
                    return Some((wid, owner));
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
                            if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.owner_pid == owner) {
                                let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + surface.shmem_phys_addr) as *mut u32 };
                                editor_state.render_to_surface(surf_ptr, crate::editor_app::EDITOR_WIDTH, crate::editor_app::EDITOR_HEIGHT);
                            }
                            drop(editors);
                            return Some((wid, owner));
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
                    return Some((wid, owner));
                } else {
                    self.last_titlebar_click = Some((wid, now_tick));
                }

                // 5. Titlebar Drag start (Clicking on titlebar itself, not buttons)
                self.dragging_window = Some((wid, mx - wx, my - wy));
                let _ = self.raise_to_top_internal(wid);
                return Some((wid, owner));
            }

            // 3c. Check Client area click -> focus only (Surface content click does NOT start dragging)
            if mx >= wx && mx < wx + ww && my >= wy + 20 && my < wy + 20 + wh {
                let _ = self.raise_to_top_internal(wid);
                return Some((wid, owner));
            }
        }
        None
    }

    /// Handles mouse up: stops dragging and resizing, resets cursor
    pub fn handle_mouse_up(&mut self) -> Option<(u64, u64)> {
        self.dragging_window = None;
        self.resizing_window = None;
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
        let dock_y = max_h.saturating_sub(DOCK_HEIGHT as i32);

        // 1. Handle Window Resizing
        if let Some((wid, edge, start_mx, start_my, orig_x, orig_y, orig_w, orig_h)) = self.resizing_window {
            let dx = mx - start_mx;
            let dy = my - start_my;

            if let Some(win) = self.windows.iter_mut().find(|w| w.window_id == wid) {
                let max_avail_h = (dock_y - (orig_y + 20)).max(MIN_WINDOW_HEIGHT as i32) as u32;

                match edge {
                    ResizeEdge::Right => {
                        let new_w = ((orig_w as i32) + dx).clamp(MIN_WINDOW_WIDTH as i32, max_w - orig_x) as u32;
                        win.width = new_w;
                    }
                    ResizeEdge::Bottom => {
                        let new_h = ((orig_h as i32) + dy).clamp(MIN_WINDOW_HEIGHT as i32, max_avail_h as i32) as u32;
                        win.height = new_h;
                    }
                    ResizeEdge::BottomRight => {
                        let new_w = ((orig_w as i32) + dx).clamp(MIN_WINDOW_WIDTH as i32, max_w - orig_x) as u32;
                        let new_h = ((orig_h as i32) + dy).clamp(MIN_WINDOW_HEIGHT as i32, max_avail_h as i32) as u32;
                        win.width = new_w;
                        win.height = new_h;
                    }
                    ResizeEdge::Left => {
                        let max_left = orig_x + (orig_w as i32) - (MIN_WINDOW_WIDTH as i32);
                        let new_x = (orig_x + dx).clamp(0, max_left);
                        let new_w = ((orig_x + (orig_w as i32)) - new_x) as u32;
                        win.x = new_x;
                        win.width = new_w;
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
                    _ => {}
                }
            }
            crate::cursor::set_cursor_type(crate::cursor::CursorType::ResizeDiagonal);
            return;
        }

        // 2. Handle Window Dragging
        if let Some((wid, ox, oy)) = self.dragging_window {
            if let Some(win) = self.windows.iter_mut().find(|w| w.window_id == wid) {
                // If dragged while maximized, restore normal state and continue dragging smoothly
                if win.state == WindowState::Maximized {
                    if let Some((_, _, pw, ph)) = win.saved_geom.take() {
                        win.width = pw;
                        win.height = ph;
                    }
                    win.state = WindowState::Normal;
                }
                win.x = (mx - ox).clamp(-100, max_w.saturating_sub(50));
                win.y = (my - oy).clamp(WORK_AREA_TOP, dock_y.saturating_sub(30));
            }
            crate::cursor::set_cursor_type(crate::cursor::CursorType::Hand);
            return;
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
            if mx >= wx - 4 && mx <= wx + ww + 4 && my >= wy && my <= wy + 20 + wh + 4 {
                // Resize corners & edges (only if not maximized)
                if win.state != WindowState::Maximized {
                    if (mx >= wx + ww - 8 && mx <= wx + ww + 4 && my >= wy + 20 + wh - 8 && my <= wy + 20 + wh + 4) ||
                       (mx >= wx - 4 && mx <= wx + 8 && my >= wy + 20 + wh - 8 && my <= wy + 20 + wh + 4) {
                        hovered = Some(HoverTarget {
                            window_id: wid,
                            button: ChromeButton::None,
                            is_titlebar: false,
                            resize_edge: ResizeEdge::BottomRight,
                        });
                        cursor_type = crate::cursor::CursorType::ResizeDiagonal;
                        break;
                    } else if (mx >= wx + ww - 4 && mx <= wx + ww + 4 && my >= wy && my <= wy + 20 + wh) ||
                              (mx >= wx - 4 && mx <= wx + 4 && my >= wy && my <= wy + 20 + wh) {
                        hovered = Some(HoverTarget {
                            window_id: wid,
                            button: ChromeButton::None,
                            is_titlebar: false,
                            resize_edge: ResizeEdge::Right,
                        });
                        cursor_type = crate::cursor::CursorType::ResizeHorizontal;
                        break;
                    } else if mx >= wx && mx <= wx + ww && my >= wy + 20 + wh - 4 && my <= wy + 20 + wh + 4 {
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
