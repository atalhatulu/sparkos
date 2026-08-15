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
}

impl WindowManager {
    pub const fn new() -> Self {
        Self {
            windows: Vec::new(),
            next_window_id: 1,
            focused_window: None,
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
        if width == 0 || height == 0 || width > 1920 || height > 1080 {
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
