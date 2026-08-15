//! SparkOS — Window Manager & Compositor Subsystem (Faz 12)
//!
//! Provides Ring-3 / Microkernel Window Management, Z-Order Back-to-Front Compositing,
//! Hit-Testing, Focus Elevation, Isolated Input Routing, and Teardown Cleanup.

use alloc::vec::Vec;
use spin::Mutex;

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
    Closed,
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
}

pub struct WindowManager {
    /// Windows ordered from Back (index 0) to Front (index len - 1).
    pub windows: Vec<Window>,
    pub next_window_id: u64,
    pub focused_window: Option<u64>,
    pub dragging_window: Option<(u64, i32, i32)>,
}

impl WindowManager {
    pub const fn new() -> Self {
        Self {
            windows: Vec::new(),
            next_window_id: 1,
            focused_window: None,
            dragging_window: None,
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
        if width == 0 || height == 0 || width > 640 || height > 360 {
            return Err(WmError::InvalidDimensions);
        }

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
            x,
            y,
            width,
            height,
            visible: true,
            focused: true,
            state: WindowState::Normal,
        };

        // Appended at the end (top-most in Z-order)
        self.windows.push(win);
        self.focused_window = Some(window_id);

        crate::serial_println!("[WM] Process {} created Window {} (surface {}, [{}, {}, {}, {}], FOCUSED)",
            owner_pid, window_id, surface_id, x, y, width, height);

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

    /// Destroys a window with strict ownership verification.
    pub fn destroy_window(&mut self, caller_pid: u64, window_id: u64) -> Result<(), WmError> {
        let idx = self.windows.iter().position(|w| w.window_id == window_id)
            .ok_or(WmError::NotFound)?;

        if self.windows[idx].owner_pid != caller_pid {
            return Err(WmError::PermissionDenied);
        }

        self.windows.remove(idx);

        if self.focused_window == Some(window_id) {
            // Transfer focus to the next topmost visible window
            self.focused_window = self.windows.iter().rev().find(|w| w.visible).map(|w| w.window_id);
            if let Some(fid) = self.focused_window {
                if let Some(w) = self.windows.iter_mut().find(|w| w.window_id == fid) {
                    w.focused = true;
                }
            }
        }

        crate::serial_println!("[WM] Process {} destroyed Window {}", caller_pid, window_id);
        Ok(())
    }

    /// Moves a window to new coordinates with strict ownership verification.
    pub fn move_window(&mut self, caller_pid: u64, window_id: u64, new_x: i32, new_y: i32) -> Result<(), WmError> {
        let win = self.windows.iter_mut().find(|w| w.window_id == window_id)
            .ok_or(WmError::NotFound)?;

        if win.owner_pid != caller_pid {
            return Err(WmError::PermissionDenied);
        }

        win.x = new_x;
        win.y = new_y;
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
               my >= win.y && my < win.y + (win.height as i32) {
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

    /// Composite all windows and surfaces to the hardware backbuffer and swap to screen
    pub fn composite_desktop(&self, mouse_x: i32, mouse_y: i32) {
        unsafe {
            if crate::gui::BACKBUFFER.is_null() { return; }
        }

        let screen_w = unsafe { crate::gui::VESA.width };
        let screen_h = unsafe { crate::gui::VESA.height };

        // 1. Draw solid wallpaper / desktop background (Dark Slate #1E293B)
        crate::gui::draw_rect(0, 0, screen_w, screen_h, 0x001E293B);

        // 2. Top menu / status bar (Navy #0F172A)
        crate::gui::draw_rect(0, 0, screen_w, 20, 0x000F172A);
        crate::gui::draw_string(8, 6, "SparkOS Desktop v1.0 (640x360) | Isolated CSpace", 0x00E2E8F0, 0x000F172A);

        // 3. Composite windows Back-to-Front
        let surf_reg = crate::surface::SURFACE_REGISTRY.lock();
        for win in self.windows.iter() {
            if !win.visible || win.state == WindowState::Minimized {
                continue;
            }

            let wx = win.x.max(0).min(screen_w as i32 - 1) as u16;
            let wy = win.y.max(20).min(screen_h as i32 - 1) as u16;
            let ww = (win.width as u16).min(screen_w.saturating_sub(wx));
            let wh = (win.height as u16).min(screen_h.saturating_sub(wy + 20));

            if ww == 0 || wh == 0 {
                continue;
            }

            let is_focused = self.focused_window == Some(win.window_id);
            let title_bg = if is_focused { 0x002563EB /* Blue */ } else { 0x00475569 /* Slate Gray */ };

            // 3a. Titlebar (20px high)
            crate::gui::draw_rect(wx, wy, ww, 20, title_bg);
            
            // Title text (Window <id> - PID <pid>)
            let mut title_buf = [0u8; 32];
            let title_str = {
                use core::fmt::Write;
                struct BufWriter<'a> { buf: &'a mut [u8], pos: usize }
                impl<'a> Write for BufWriter<'a> {
                    fn write_str(&mut self, s: &str) -> core::fmt::Result {
                        for b in s.bytes() {
                            if self.pos < self.buf.len() {
                                self.buf[self.pos] = b;
                                self.pos += 1;
                            }
                        }
                        Ok(())
                    }
                }
                let mut bw = BufWriter { buf: &mut title_buf, pos: 0 };
                let _ = write!(bw, "Win {} (PID {})", win.window_id, win.owner_pid);
                core::str::from_utf8(&bw.buf[..bw.pos]).unwrap_or("Win")
            };
            crate::gui::draw_string(wx + 6, wy + 6, title_str, 0x00FFFFFF, title_bg);

            // Minimize Button [-]
            if ww > 40 {
                let min_x = wx + ww - 36;
                crate::gui::draw_rect(min_x, wy + 3, 14, 14, 0x00334155);
                crate::gui::draw_char(min_x + 4, wy + 6, '-', 0x00FFFFFF, 0x00334155);
            }

            // Close Button [X]
            if ww > 20 {
                let close_x = wx + ww - 18;
                crate::gui::draw_rect(close_x, wy + 3, 14, 14, 0x00DC2626);
                crate::gui::draw_char(close_x + 4, wy + 6, 'X', 0x00FFFFFF, 0x00DC2626);
            }

            // 3b. Client Area Background
            crate::gui::draw_rect(wx, wy + 20, ww, wh, 0x000F172A);

            // 3c. Blit shared-memory surface if present
            if let Some(surface) = surf_reg.iter().find(|s| s.surface_id == win.surface_id) {
                let phys_addr = surface.shmem_phys_addr;
                let src_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *const u32 };
                let copy_w = (surface.width.min(win.width) as usize).min(ww as usize);
                let copy_h = (surface.height.min(win.height) as usize).min(wh as usize);

                unsafe {
                    if !crate::gui::BACKBUFFER.is_null() {
                        for r in 0..copy_h {
                            let dst_row = (wy + 20 + r as u16) as usize;
                            if dst_row >= screen_h as usize { break; }
                            let dst_col = wx as usize;
                            let src_offset = r * (surface.width as usize);
                            let dst_offset = dst_row * (screen_w as usize) + dst_col;
                            core::ptr::copy_nonoverlapping(
                                src_ptr.add(src_offset),
                                crate::gui::BACKBUFFER.add(dst_offset),
                                copy_w,
                            );
                        }
                    }
                }
            }

            // 3d. 1px window border
            crate::gui::draw_rect(wx, wy, ww, 1, 0x0064748B);
            crate::gui::draw_rect(wx, wy + 20 + wh - 1, ww, 1, 0x0064748B);
            crate::gui::draw_rect(wx, wy, 1, 20 + wh, 0x0064748B);
            crate::gui::draw_rect(wx + ww - 1, wy, 1, 20 + wh, 0x0064748B);
        }
        drop(surf_reg);

        // 4. Draw mouse cursor
        let cur_x = (mouse_x.max(0).min(screen_w as i32 - 1)) as u16;
        let cur_y = (mouse_y.max(0).min(screen_h as i32 - 1)) as u16;
        crate::gui::draw_cursor(cur_x, cur_y);

        // 5. Swap buffers to hardware VESA Framebuffer
        crate::gui::swap_buffers();
    }

    /// Handles mouse down: performs titlebar dragging, close/minimize buttons, or client focus
    pub fn handle_mouse_down(&mut self, mx: i32, my: i32) -> Option<(u64, u64)> {
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

            // Check Titlebar click (20px high)
            if mx >= wx && mx < wx + ww && my >= wy && my < wy + 20 {
                // Close button [X]
                if mx >= wx + ww - 20 && mx <= wx + ww - 4 && my >= wy + 2 && my <= wy + 18 {
                    let _ = self.destroy_window(owner, wid);
                    return None;
                }
                // Minimize button [-]
                if mx >= wx + ww - 38 && mx <= wx + ww - 22 && my >= wy + 2 && my <= wy + 18 {
                    let _ = self.minimize_window(owner, wid);
                    return None;
                }

                // Drag start
                self.dragging_window = Some((wid, mx - wx, my - wy));
                let _ = self.raise_to_top_internal(wid);
                return Some((wid, owner));
            }

            // Check Client area click -> focus
            if mx >= wx && mx < wx + ww && my >= wy + 20 && my < wy + 20 + wh {
                let _ = self.raise_to_top_internal(wid);
                return Some((wid, owner));
            }
        }
        None
    }

    /// Handles mouse up: stops dragging
    pub fn handle_mouse_up(&mut self) -> Option<(u64, u64)> {
        self.dragging_window = None;
        let target_id = self.focused_window?;
        let owner_pid = self.windows.iter().find(|w| w.window_id == target_id).map(|w| w.owner_pid)?;
        Some((target_id, owner_pid))
    }

    /// Handles mouse move: updates dragging window coordinates
    pub fn handle_mouse_move(&mut self, mx: i32, my: i32) {
        if let Some((wid, ox, oy)) = self.dragging_window {
            if let Some(win) = self.windows.iter_mut().find(|w| w.window_id == wid) {
                win.x = (mx - ox).max(0).min(640 - win.width as i32);
                win.y = (my - oy).max(20).min(360 - win.height as i32 - 20);
            }
        }
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
