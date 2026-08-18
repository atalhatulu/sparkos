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

/// Fare olayını işler: Hit-test yapar, global koordinatları yerel pencere koordinatına
/// dönüştürür ve yalnızca hedef pencerenin sahibine iletir.
pub fn dispatch_mouse_event(global_x: i32, global_y: i32, button: u8, pressed: bool) -> Option<u64> {
    let mut wm = crate::wm::WM.lock();
    if let Some(win_id) = wm.hit_test(global_x, global_y) {
        if pressed {
            let _ = wm.raise_to_top_internal(win_id);
        }
        if let Some(win) = wm.windows.iter().find(|w| w.window_id == win_id) {
            let owner_pid = win.owner_pid;
            let local_x = global_x - win.x;
            let local_y = global_y - (win.y + 20); // Surface content begins below 20px titlebar

            // Only deliver event to client if within client surface area (below title bar)
            if local_y >= 0 {
                let ev_type = if button > 0 {
                    if pressed { EventType::MouseButtonDown } else { EventType::MouseButtonUp }
                } else {
                    EventType::MouseMove
                };

                let ev = InputEvent {
                    event_type: ev_type as u8,
                    modifiers: 0,
                    key_code: 0,
                    mouse_button: button,
                    wheel_delta: 0,
                    _reserved: [0; 3],
                    mouse_x: local_x,
                    mouse_y: local_y,
                    timestamp: crate::interrupts::get_tick(),
                    _padding: [0; 8],
                };

                drop(wm);
                deliver_event_to_pid(owner_pid, ev);

                if pressed && button > 0 {
                    let mut files = crate::files_app::FILES_INSTANCES.lock();
                    if let Some(files_state) = files.get_mut(&win_id) {
                        files_state.handle_mouse_click(local_x as u32, local_y as u32);
                        if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.owner_pid == owner_pid) {
                            let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + surface.shmem_phys_addr) as *mut u32 };
                            files_state.render_to_surface(surf_ptr, crate::files_app::FILES_WIDTH, crate::files_app::FILES_HEIGHT);
                        }
                        drop(files);
                        crate::wm::WM.lock().composite_desktop(0, 0);
                    } else {
                        drop(files);
                        let mut editors = crate::editor_app::EDITOR_INSTANCES.lock();
                        if let Some(editor_state) = editors.get_mut(&win_id) {
                            editor_state.handle_mouse_click(local_x as u32, local_y as u32);
                            if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.owner_pid == owner_pid) {
                                let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + surface.shmem_phys_addr) as *mut u32 };
                                editor_state.render_to_surface(surf_ptr, crate::editor_app::EDITOR_WIDTH, crate::editor_app::EDITOR_HEIGHT);
                            }
                            drop(editors);
                            crate::wm::WM.lock().composite_desktop(0, 0);
                        }
                    }
                }

                return Some(owner_pid);
            }
        }
    }
    None
}

/// Klavye olayını işler: Kesinlikle YALNIZCA o an odaklı pencerenin sahibine iletir (Focus-Gated Routing).
pub fn dispatch_keyboard_event(key_code: u8, pressed: bool, modifiers: u8) -> Option<u64> {
    // 1. Check Alt-Tab Window Switcher Shortcut
    if pressed && key_code == 0x0F && (modifiers & 0x08 != 0 || crate::keyboard::is_alt_pressed()) {
        let mut wm = crate::wm::WM.lock();
        let new_focused = wm.alt_tab_cycle();
        let owner = new_focused.and_then(|fid| wm.windows.iter().find(|w| w.window_id == fid).map(|w| w.owner_pid));
        drop(wm);
        crate::wm::WM.lock().composite_desktop(0, 0);
        return owner;
    }

    let is_ctrl = (modifiers & 0x01 != 0) || crate::keyboard::is_ctrl_pressed();
    let is_alt = (modifiers & 0x08 != 0) || crate::keyboard::is_alt_pressed();

    if pressed {
        crate::serial_println!("[INPUT] key=0x{:02x} pressed, is_ctrl={}, is_alt={}", key_code, is_ctrl, is_alt);
    }

    // 2. Global Shortcut: Ctrl+Alt+T or F1 -> Open/Spawn Terminal
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
        crate::wm::WM.lock().composite_desktop(0, 0);
        return res.ok();
    }

    // 3. Global Shortcut: Ctrl+Escape -> Toggle Application Launcher Menu
    if pressed && key_code == 0x01 && is_ctrl {
        let mut wm = crate::wm::WM.lock();
        wm.launcher_open = !wm.launcher_open;
        drop(wm);
        crate::wm::WM.lock().composite_desktop(0, 0);
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
                    if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.surface_id == focused_surf_id || s.owner_pid == focused_pid) {
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
        crate::wm::WM.lock().composite_desktop(0, 0);
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
            if should_close {
                let _ = crate::wm::WM.lock().destroy_window(focused_pid, focused_win_id);
            }
            crate::wm::WM.lock().composite_desktop(0, 0);
        } else {
            let (is_editor, should_close) = {
                let mut editors = crate::editor_app::EDITOR_INSTANCES.lock();
                if let Some(editor_state) = editors.get_mut(&focused_win_id) {
                    editor_state.handle_key_input(key_code, pressed);
                    if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.surface_id == focused_surf_id || s.owner_pid == focused_pid) {
                        let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + surface.shmem_phys_addr) as *mut u32 };
                        editor_state.render_to_surface(surf_ptr, crate::editor_app::EDITOR_WIDTH, crate::editor_app::EDITOR_HEIGHT);
                    }
                    (true, editor_state.pending_close)
                } else {
                    (false, false)
                }
            };

            if is_editor {
                if should_close {
                    let _ = crate::wm::WM.lock().destroy_window(focused_pid, focused_win_id);
                }
                crate::wm::WM.lock().composite_desktop(0, 0);
            } else {
                let mut files = crate::files_app::FILES_INSTANCES.lock();
                if let Some(files_state) = files.get_mut(&focused_win_id) {
                    files_state.handle_key_input(key_code, pressed);
                    if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.surface_id == focused_surf_id || s.owner_pid == focused_pid) {
                        let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + surface.shmem_phys_addr) as *mut u32 };
                        files_state.render_to_surface(surf_ptr, crate::files_app::FILES_WIDTH, crate::files_app::FILES_HEIGHT);
                    }
                    drop(files);
                    crate::wm::WM.lock().composite_desktop(0, 0);
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
