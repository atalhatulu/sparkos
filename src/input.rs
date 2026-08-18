//! SparkOS Desktop V1.4 — Input Event Architecture
//!
//! Provides a standardized 32-Byte Wire-Format `InputEvent` model, per-process event queues,
//! MouseMove event coalescing, focus-gated keyboard routing, window focus/resize events,
//! and bounded memory allocation.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use spin::Mutex;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    None = 0,
    MouseMove = 1,
    MouseButtonDown = 2,
    MouseButtonUp = 3,
    KeyDown = 4,
    KeyUp = 5,
    WindowFocus = 6,
    WindowResize = 7,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputEvent {
    pub event_type: u8,       // 1 byte (EventType)
    pub modifiers: u8,        // 1 byte (Bit 0: Shift, Bit 1: Ctrl, Bit 2: Alt, Bit 3: Meta)
    pub key_code: u8,         // 1 byte
    pub mouse_button: u8,     // 1 byte (1: Left, 2: Right, 3: Middle)
    pub wheel_delta: i8,      // 1 byte (+1: Up, -1: Down)
    pub _reserved: [u8; 3],   // 3 bytes padding
    pub mouse_x: i32,         // 4 bytes (Pencere-içi yerel X / Genişlik)
    pub mouse_y: i32,         // 4 bytes (Pencere-içi yerel Y / Yükseklik)
    pub timestamp: u64,       // 8 bytes (Sistem zaman damgası)
    pub _padding: [u8; 8],    // 8 bytes padding -> Toplam 32 byte
}

// 32-byte wire-format ABI doğrulaması
const _: () = assert!(core::mem::size_of::<InputEvent>() == 32);

pub const MAX_QUEUE_CAPACITY: usize = 64;

#[derive(Debug, Clone)]
pub struct EventQueue {
    pub buffer: Vec<InputEvent>,
}

impl EventQueue {
    pub const fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Olayı kuyruğa ekler; MouseMove olaylarını coalescing (birleştirme) yaparak
    /// kuyruğun dolmasını engeller, klavye ve pencere olaylarını FIFO korur.
    pub fn push(&mut self, ev: InputEvent) {
        // 1. MouseMove Coalescing: Eğer gelen olay MouseMove ise ve kuyruğun sonunda
        // zaten bir MouseMove varsa, yeni bir slot tüketmek yerine koordinatları güncelle
        if ev.event_type == EventType::MouseMove as u8 {
            if let Some(last_ev) = self.buffer.last_mut() {
                if last_ev.event_type == EventType::MouseMove as u8 {
                    last_ev.mouse_x = ev.mouse_x;
                    last_ev.mouse_y = ev.mouse_y;
                    last_ev.timestamp = ev.timestamp;
                    last_ev.modifiers = ev.modifiers;
                    return;
                }
            }
        }

        // 2. Kapasite Sınırı Denetimi
        if self.buffer.len() >= MAX_QUEUE_CAPACITY {
            // Önce kuyruktaki en eski MouseMove olayını düşür
            if let Some(pos) = self.buffer.iter().position(|e| e.event_type == EventType::MouseMove as u8) {
                self.buffer.remove(pos);
            } else {
                // Kuyruk sadece kritik olaylarla doluysa en eski olayı düşür (bounded memory)
                self.buffer.remove(0);
            }
        }

        self.buffer.push(ev);
    }

    pub fn pop(&mut self) -> Option<InputEvent> {
        if self.buffer.is_empty() {
            None
        } else {
            Some(self.buffer.remove(0))
        }
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

pub static INPUT_QUEUES: Mutex<BTreeMap<u64, EventQueue>> = Mutex::new(BTreeMap::new());

/// Bir sürece olay gönderir
pub fn deliver_event_to_pid(pid: u64, ev: InputEvent) {
    let mut queues = INPUT_QUEUES.lock();
    queues.entry(pid).or_insert_with(EventQueue::new).push(ev);
}

/// Süreç sonlandığında olay kuyruğunu sızıntısız temizler
pub fn cleanup_input_for_pid(pid: u64) {
    let mut queues = INPUT_QUEUES.lock();
    if queues.remove(&pid).is_some() {
        crate::serial_println!("[INPUT] Cleaned up event queue for terminating PID {}", pid);
    }
}

/// Routes mouse button down: handles window interactions (focus, drag, resize, buttons),
/// desktop icons, launcher, crash modals, and delivers InputEvent to client surface.
pub fn route_mouse_down(cx: i32, cy: i32) {
    let mut wm = crate::wm::WM.lock();
    wm.mark_damage(cx.saturating_sub(4), cy.saturating_sub(4), 28, 28);
    let event_target = wm.handle_mouse_down(cx, cy);
    let app_to_spawn = wm.pending_spawn_app.take();
    drop(wm);

    if let Some((wid, owner_pid)) = event_target {
        let (local_x, local_y, target_surf_id) = {
            let wm = crate::wm::WM.lock();
            if let Some(w) = wm.windows.iter().find(|w| w.window_id == wid) {
                let surf_reg = crate::surface::SURFACE_REGISTRY.read();
                let (surf_w, surf_h) = if let Some(surf) = surf_reg.iter().find(|s| s.surface_id == w.surface_id) {
                    (surf.width as i32, surf.height as i32)
                } else {
                    (w.width as i32, w.height as i32)
                };
                let (lx, ly) = if w.state == crate::wm::WindowState::Fullscreen {
                    let screen_w = unsafe { crate::gui::VESA.width as i32 };
                    let screen_h = unsafe { crate::gui::VESA.height as i32 };
                    let off_x = (screen_w.saturating_sub(surf_w)) / 2;
                    let off_y = (screen_h.saturating_sub(surf_h)) / 2;
                    (cx - off_x, cy - off_y)
                } else {
                    (cx - w.x, cy - (w.y + 20))
                };
                (lx, ly, Some(w.surface_id))
            } else {
                (cx, cy, None)
            }
        };

        let ev = InputEvent {
            event_type: EventType::MouseButtonDown as u8,
            modifiers: 0,
            key_code: 0,
            mouse_button: 1,
            wheel_delta: 0,
            _reserved: [0; 3],
            mouse_x: local_x,
            mouse_y: local_y,
            timestamp: crate::interrupts::get_tick(),
            _padding: [0; 8],
        };
        deliver_event_to_pid(owner_pid, ev);

        if local_y >= 0 {
            let mut files = crate::files_app::FILES_INSTANCES.lock();
            if let Some(files_state) = files.get_mut(&wid) {
                files_state.handle_mouse_click(local_x as u32, local_y as u32);
                if let Some(surface) = crate::surface::SURFACE_REGISTRY.read().iter().find(|s| Some(s.surface_id) == target_surf_id || s.owner_pid == owner_pid) {
                    let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + surface.shmem_phys_addr) as *mut u32 };
                    files_state.render_to_surface(surf_ptr, crate::files_app::FILES_WIDTH, crate::files_app::FILES_HEIGHT);
                }
                drop(files);
            } else {
                drop(files);
                let (is_editor, should_close) = {
                    let mut editors = crate::editor_app::EDITOR_INSTANCES.lock();
                    if let Some(editor_state) = editors.get_mut(&wid) {
                        editor_state.handle_mouse_click(local_x as u32, local_y as u32);
                        if let Some(surface) = crate::surface::SURFACE_REGISTRY.read().iter().find(|s| Some(s.surface_id) == target_surf_id || s.owner_pid == owner_pid) {
                            let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + surface.shmem_phys_addr) as *mut u32 };
                            editor_state.render_to_surface(surf_ptr, crate::editor_app::EDITOR_WIDTH, crate::editor_app::EDITOR_HEIGHT);
                        }
                        (true, editor_state.pending_close)
                    } else {
                        (false, false)
                    }
                };
                if is_editor && should_close {
                    let _ = crate::wm::WM.lock().destroy_window(owner_pid, wid);
                }
            }
        }
    } else if let Some(app_id) = app_to_spawn {
        let _ = crate::app_registry::spawn_registered_app(app_id);
    } else if crate::crash_reporter::CRASH_REPORTER.lock().active_crash.is_some() {
        let screen_w = unsafe { crate::gui::VESA.width as i32 };
        let screen_h = unsafe { crate::gui::VESA.height as i32 };
        let mw = 260;
        let mh = 160;
        let mx = (screen_w - mw) / 2;
        let my = (screen_h - mh) / 2;
        if cx >= mx + 70 && cx <= mx + 190 && cy >= my + 118 && cy <= my + 142 {
            crate::crash_reporter::CRASH_REPORTER.lock().dismiss_active_crash();
        }
    } else if let Some(action) = crate::desktop::DESKTOP_ENV.lock().handle_mouse_click(cx as u16, cy as u16, crate::interrupts::get_tick()) {
        let (res, app_name) = match action {
            crate::desktop::DesktopIconAction::OpenHome => {
                (crate::files_app::spawn_files_app("files.app"), "files.app")
            }
            crate::desktop::DesktopIconAction::OpenTerminal => {
                (crate::terminal_app::spawn_terminal_app("terminal.app"), "terminal.app")
            }
            crate::desktop::DesktopIconAction::OpenEditor => {
                (crate::editor_app::spawn_editor_app("editor.app", None), "editor.app")
            }
            crate::desktop::DesktopIconAction::OpenTaskMgr => {
                (crate::taskmgr_app::spawn_taskmgr_app("taskmgr.app"), "taskmgr.app")
            }
            crate::desktop::DesktopIconAction::OpenSettings => {
                (crate::settings_app::spawn_settings_app("settings.app"), "settings.app")
            }
            crate::desktop::DesktopIconAction::OpenApplications => {
                crate::wm::WM.lock().launcher_open = true;
                (Ok(0), "launcher")
            }
            crate::desktop::DesktopIconAction::OpenBrowser => {
                (crate::browser_app::spawn_browser_app("browser.app"), "browser.app")
            }
        };
        if let Err(_e) = res {
            crate::crash_reporter::CRASH_REPORTER.lock().report_process_crash(
                0,
                app_name,
                "Failed to allocate memory/process resources",
            );
        }
    } else if cy < 24 && cx > 1000 {
        crate::network_manager::NETWORK_MANAGER.lock().toggle_popup();
    }
}

/// Routes mouse button up: stops dragging and resizing, performs edge snapping, and delivers MouseButtonUp event.
pub fn route_mouse_up(cx: i32, cy: i32) {
    let mut wm = crate::wm::WM.lock();
    wm.mark_damage(cx.saturating_sub(4), cy.saturating_sub(4), 28, 28);
    if let Some((_wid, owner_pid)) = wm.handle_mouse_up() {
        let ev = InputEvent {
            event_type: EventType::MouseButtonUp as u8,
            modifiers: 0,
            key_code: 0,
            mouse_button: 1,
            wheel_delta: 0,
            _reserved: [0; 3],
            mouse_x: 0,
            mouse_y: 0,
            timestamp: crate::interrupts::get_tick(),
            _padding: [0; 8],
        };
        deliver_event_to_pid(owner_pid, ev);
    }
}

/// Routes mouse movement: updates dragging, resizing, hover cursor state, and delivers MouseMove to hovered window client.
pub fn route_mouse_move(cx: i32, cy: i32, last_x: i32, last_y: i32) {
    let mut wm = crate::wm::WM.lock();
    wm.mark_damage(last_x.saturating_sub(4), last_y.saturating_sub(4), 28, 28);
    wm.mark_damage(cx.saturating_sub(4), cy.saturating_sub(4), 28, 28);
    wm.handle_mouse_move(cx, cy);
    if let Some(target_id) = wm.hit_test(cx, cy) {
        if let Some(win) = wm.windows.iter().find(|w| w.window_id == target_id) {
            let local_x = cx - win.x;
            let local_y = cy - (win.y + 20);
            if local_y >= 0 {
                let ev = InputEvent {
                    event_type: EventType::MouseMove as u8,
                    modifiers: 0,
                    key_code: 0,
                    mouse_button: 0,
                    wheel_delta: 0,
                    _reserved: [0; 3],
                    mouse_x: local_x,
                    mouse_y: local_y,
                    timestamp: crate::interrupts::get_tick(),
                    _padding: [0; 8],
                };
                deliver_event_to_pid(win.owner_pid, ev);
            }
        }
    }
}

/// Legacy dispatch helper for direct coordinate injection.
pub fn dispatch_mouse_event(global_x: i32, global_y: i32, button: u8, pressed: bool) -> Option<u64> {
    if pressed && button > 0 {
        route_mouse_down(global_x, global_y);
    } else if !pressed && button > 0 {
        route_mouse_up(global_x, global_y);
    } else {
        route_mouse_move(global_x, global_y, global_x, global_y);
    }
    let wm = crate::wm::WM.lock();
    wm.hit_test(global_x, global_y).and_then(|wid| wm.windows.iter().find(|w| w.window_id == wid).map(|w| w.owner_pid))
}

/// Klavye olayını işler: Kesinlikle YALNIZCA o an odaklı pencerenin sahibine iletir (Focus-Gated Routing).
pub fn dispatch_keyboard_event(key_code: u8, pressed: bool, modifiers: u8) -> Option<u64> {
    let is_ctrl = (modifiers & 0x01 != 0) || crate::keyboard::is_ctrl_pressed();
    let is_alt = (modifiers & 0x08 != 0) || crate::keyboard::is_alt_pressed();
    let is_shift = (modifiers & 0x04 != 0) || crate::keyboard::is_shift_pressed();

    // 1. Alt Release commit for Alt+Tab Switcher
    if !is_alt || (!pressed && key_code == 0x38) {
        let mut wm = crate::wm::WM.lock();
        if wm.alt_tab.active {
            let res = wm.alt_tab_commit();
            let owner = res.and_then(|fid| wm.windows.iter().find(|w| w.window_id == fid).map(|w| w.owner_pid));
            return owner;
        }
    }

    // 2. Escape: cancel Alt+Tab Switcher or close Launcher if active
    if pressed && key_code == 0x01 {
        let mut wm = crate::wm::WM.lock();
        if wm.alt_tab.active {
            wm.alt_tab_cancel();
            return None;
        }
        if wm.launcher_open {
            wm.launcher_open = false;
            let screen_h = unsafe { crate::gui::VESA.height as i32 };
            let dock_y = screen_h.saturating_sub(crate::wm::DOCK_HEIGHT as i32);
            wm.mark_damage(4, (dock_y - 250).max(0), 164, 260);
            return None;
        }
    }

    // 2b. Launcher Keyboard Navigation (Up/Down/Enter)
    if pressed {
        let mut wm = crate::wm::WM.lock();
        if wm.launcher_open {
            if key_code == 0x48 { // Up Arrow
                wm.launcher_nav_up();
                return None;
            } else if key_code == 0x50 { // Down Arrow
                wm.launcher_nav_down();
                return None;
            } else if key_code == 0x1C { // Enter
                let selected_app = wm.launcher_get_selected_app();
                wm.launcher_open = false;
                let screen_h = unsafe { crate::gui::VESA.height as i32 };
                let dock_y = screen_h.saturating_sub(crate::wm::DOCK_HEIGHT as i32);
                wm.mark_damage(4, (dock_y - 250).max(0), 164, 260);
                drop(wm);

                if let Some(app_id) = selected_app {
                    let _ = crate::app_registry::spawn_registered_app(app_id);
                }
                return None;
            }
        }
    }

    // 3. Alt-Tab / Alt-Shift-Tab Window Switcher Shortcut
    if pressed && key_code == 0x0F && is_alt {
        let mut wm = crate::wm::WM.lock();
        wm.alt_tab_press(is_shift);
        return None;
    }

    // 4. Global Shortcut: Ctrl+Alt+T or F1 -> Open/Spawn Terminal
    if pressed && ((key_code == 0x14 && is_ctrl && is_alt) || key_code == 0x3B) {
        let res = crate::terminal_app::spawn_terminal_app("terminal.app");
        match res {
            Ok(pid) => {
                crate::serial_println!("[INPUT] Successfully spawned Terminal PID {}", pid);
            }
            Err(e) => {
                crate::serial_println!("[INPUT] Failed to spawn Terminal: {:?}", e);
                crate::crash_reporter::CRASH_REPORTER.lock().report_process_crash(
                    0,
                    "terminal.app",
                    "Spawning Failed (Memory/Process Limit)",
                );
            }
        }
        return res.ok();
    }

    // 5. Global Shortcut: Ctrl+Escape -> Toggle Application Launcher Menu
    if pressed && key_code == 0x01 && is_ctrl {
        let mut wm = crate::wm::WM.lock();
        wm.launcher_open = !wm.launcher_open;
        let screen_h = unsafe { crate::gui::VESA.height as i32 };
        let dock_y = screen_h.saturating_sub(crate::wm::DOCK_HEIGHT as i32);
        wm.mark_damage(4, (dock_y - 250).max(0), 160, 260);
        wm.mark_damage(0, dock_y, unsafe { crate::gui::VESA.width as u32 }, crate::wm::DOCK_HEIGHT as u32);
        return None;
    }

    let (focused_pid, focused_win_id, focused_surf_id) = {
        let wm = crate::wm::WM.lock();
        let fid = wm.focused_window?;
        let win = wm.windows.iter().find(|w| w.window_id == fid)?;
        (win.owner_pid, fid, win.surface_id)
    };

    // 2. Check Alt-F4 Window Close Shortcut
    if pressed && key_code == 0x3E && (modifiers & 0x08 != 0 || crate::keyboard::is_alt_pressed()) {
        let is_dirty_editor = {
            let mut editors = crate::editor_app::EDITOR_INSTANCES.lock();
            if let Some(editor_state) = editors.get_mut(&focused_win_id) {
                if editor_state.is_dirty && !editor_state.show_unsaved_dialog {
                    editor_state.show_unsaved_dialog = true;
                    if let Some(surface) = crate::surface::SURFACE_REGISTRY.read().iter().find(|s| s.surface_id == focused_surf_id || s.owner_pid == focused_pid) {
                        let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + surface.shmem_phys_addr) as *mut u32 };
                        editor_state.render_to_surface(surf_ptr, crate::editor_app::EDITOR_WIDTH, crate::editor_app::EDITOR_HEIGHT);
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };

        if !is_dirty_editor {
            let _ = crate::wm::WM.lock().destroy_window(focused_pid, focused_win_id);
        }
        return Some(focused_pid);
    }

    let ev_type = if pressed { EventType::KeyDown } else { EventType::KeyUp };
    let ev = InputEvent {
        event_type: ev_type as u8,
        modifiers,
        key_code,
        mouse_button: 0,
        wheel_delta: 0,
        _reserved: [0; 3],
        mouse_x: 0,
        mouse_y: 0,
        timestamp: crate::interrupts::get_tick(),
        _padding: [0; 8],
    };

    deliver_event_to_pid(focused_pid, ev);

    if pressed {
        let (is_term, should_close) = {
            let mut instances = crate::terminal_app::TERMINAL_INSTANCES.lock();
            if let Some(term_state) = instances.get_mut(&focused_win_id) {
                term_state.handle_key_input(key_code, pressed);
                (true, term_state.pending_close)
            } else {
                (false, false)
            }
        };

        if is_term {
            let mut wm = crate::wm::WM.lock();
            if should_close {
                let _ = wm.destroy_window(focused_pid, focused_win_id);
            } else {
                let geom = wm.windows.iter().find(|w| w.window_id == focused_win_id).map(|w| (w.x, w.y, w.width, w.height));
                if let Some((x, y, w, h)) = geom {
                    wm.mark_window_damage(x, y, w, h);
                }
            }
        } else {
            let (is_editor, should_close) = {
                let mut editors = crate::editor_app::EDITOR_INSTANCES.lock();
                if let Some(editor_state) = editors.get_mut(&focused_win_id) {
                    editor_state.handle_key_input(key_code, pressed);
                    if let Some(surface) = crate::surface::SURFACE_REGISTRY.read().iter().find(|s| s.surface_id == focused_surf_id || s.owner_pid == focused_pid) {
                        let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + surface.shmem_phys_addr) as *mut u32 };
                        editor_state.render_to_surface(surf_ptr, crate::editor_app::EDITOR_WIDTH, crate::editor_app::EDITOR_HEIGHT);
                    }
                    (true, editor_state.pending_close)
                } else {
                    (false, false)
                }
            };

            if is_editor {
                let mut wm = crate::wm::WM.lock();
                if should_close {
                    let _ = wm.destroy_window(focused_pid, focused_win_id);
                } else {
                    let geom = wm.windows.iter().find(|w| w.window_id == focused_win_id).map(|w| (w.x, w.y, w.width, w.height));
                    if let Some((x, y, w, h)) = geom {
                        wm.mark_window_damage(x, y, w, h);
                    }
                }
            } else {
                let mut files = crate::files_app::FILES_INSTANCES.lock();
                if let Some(files_state) = files.get_mut(&focused_win_id) {
                    files_state.handle_key_input(key_code, pressed);
                    if let Some(surface) = crate::surface::SURFACE_REGISTRY.read().iter().find(|s| s.surface_id == focused_surf_id || s.owner_pid == focused_pid) {
                        let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + surface.shmem_phys_addr) as *mut u32 };
                        files_state.render_to_surface(surf_ptr, crate::files_app::FILES_WIDTH, crate::files_app::FILES_HEIGHT);
                    }
                    drop(files);
                    let mut wm = crate::wm::WM.lock();
                    let geom = wm.windows.iter().find(|w| w.window_id == focused_win_id).map(|w| (w.x, w.y, w.width, w.height));
                    if let Some((x, y, w, h)) = geom {
                        wm.mark_window_damage(x, y, w, h);
                    }
                }
            }
        }
    }

    Some(focused_pid)
}

/// Pencere odaklanma veya yeniden boyutlandırma olayını iletir
pub fn notify_window_event(pid: u64, event_type: EventType, width: u32, height: u32) {
    let ev = InputEvent {
        event_type: event_type as u8,
        modifiers: 0,
        key_code: 0,
        mouse_button: 0,
        wheel_delta: 0,
        _reserved: [0; 3],
        mouse_x: width as i32,
        mouse_y: height as i32,
        timestamp: crate::interrupts::get_tick(),
        _padding: [0; 8],
    };
    deliver_event_to_pid(pid, ev);
}
