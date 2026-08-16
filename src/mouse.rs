use x86_64::instructions::port::Port;
use spin::Mutex;
use core::sync::atomic::{AtomicU16, AtomicBool, Ordering};

pub static MOUSE_X: AtomicU16 = AtomicU16::new(320);
pub static MOUSE_Y: AtomicU16 = AtomicU16::new(180);
pub static MOUSE_LEFT_CLICK: AtomicBool = AtomicBool::new(false);


#[derive(Debug)]
pub struct MouseState {
    cycle: u8,
    packet: [u8; 3],
}

pub static MOUSE: Mutex<MouseState> = Mutex::new(MouseState {
    cycle: 0,
    packet: [0; 3],
});

fn mouse_wait(type_val: u8) {
    let mut port64: Port<u8> = Port::new(0x64);
    for _ in 0..100000 {
        let status = unsafe { port64.read() };
        if type_val == 0 {
            if (status & 1) == 1 { return; } // Data available
        } else {
            if (status & 2) == 0 { return; } // Command port ready
        }
    }
}

fn mouse_write(data: u8) {
    let mut port64: Port<u8> = Port::new(0x64);
    let mut port60: Port<u8> = Port::new(0x60);
    mouse_wait(1);
    unsafe { port64.write(0xD4); }
    mouse_wait(1);
    unsafe { port60.write(data); }
}

fn mouse_read() -> u8 {
    let mut port60: Port<u8> = Port::new(0x60);
    mouse_wait(0);
    unsafe { port60.read() }
}

pub fn init() {
    let mut port64: Port<u8> = Port::new(0x64);
    let mut port60: Port<u8> = Port::new(0x60);
    
    unsafe {
        // Enable auxiliary mouse device
        mouse_wait(1);
        port64.write(0xA8);
        
        // Read configuration byte
        mouse_wait(1);
        port64.write(0x20);
        let mut status = mouse_read();
        
        // Enable IRQ12 (bit 1)
        status |= 1 << 1;
        
        // Write configuration byte
        mouse_wait(1);
        port64.write(0x60);
        mouse_wait(1);
        port60.write(status);
        
        // Set defaults
        mouse_write(0xF6);
        mouse_read(); // ACK
        
        // Set resolution (8 counts/mm)
        mouse_write(0xE8);
        mouse_read();
        mouse_write(0x03);
        mouse_read();

        // Set sample rate (200 Hz)
        mouse_write(0xF3);
        mouse_read();
        mouse_write(200);
        mouse_read();

        // Enable packet streaming
        mouse_write(0xF4);
        mouse_read(); // ACK
    }
}

pub fn handle_interrupt() {
    let mut port60: Port<u8> = Port::new(0x60);
    let data = unsafe { port60.read() };
    
    let mut state = MOUSE.lock();
    match state.cycle {
        0 => {
            // First byte must have bit 3 set, and overflow bits (6 and 7) should usually be 0 
            // to help prevent false sync with movement bytes.
            if (data & 0x08) != 0 && (data & 0xC0) == 0 {
                state.packet[0] = data;
                state.cycle = 1;
            }
        }
        1 => {
            state.packet[1] = data;
            state.cycle = 2;
        }
        2 => {
            state.packet[2] = data;
            state.cycle = 0;
            
            // Y ekseni değişimi (Yukari pozitif olduğu için ters ceviriyoruz)
            let mut dy = state.packet[2] as i16;
            if (state.packet[0] & 0x20) != 0 { dy -= 256; }
            dy = -dy;
            
            // X ekseni değişimi
            let mut dx = state.packet[1] as i16;
            if (state.packet[0] & 0x10) != 0 { dx -= 256; }
            
            // 2x sensitivity multiplier for full screen reach in VNC
            dx *= 2;
            dy *= 2;

            let left_click = (state.packet[0] & 1) != 0;
            MOUSE_LEFT_CLICK.store(left_click, Ordering::Relaxed);
            
            let mut x = MOUSE_X.load(Ordering::Relaxed) as i16 + dx;
            let mut y = MOUSE_Y.load(Ordering::Relaxed) as i16 + dy;
            
            let max_w = unsafe { crate::gui::VESA.width };
            let max_h = unsafe { crate::gui::VESA.height };
            crate::cursor::update_mouse_input(x, y, left_click, max_w, max_h);
            let cstate = crate::cursor::get_cursor_state();
            
            MOUSE_X.store(cstate.x, Ordering::Relaxed);
            MOUSE_Y.store(cstate.y, Ordering::Relaxed);
        }
        _ => state.cycle = 0,
    }
}


pub async fn mouse_task() {
    let mut last_x = 320;
    let mut last_y = 180;
    let mut last_click = false;
    
    loop {
        let cx = MOUSE_X.load(Ordering::Relaxed);
        let cy = MOUSE_Y.load(Ordering::Relaxed);
        let click = MOUSE_LEFT_CLICK.load(Ordering::Relaxed);
        
        let moved = cx != last_x || cy != last_y;
        
        if crate::vga_buffer::GUI_MODE.load(Ordering::Relaxed) {
            if click && !last_click {
                let mut wm = crate::wm::WM.lock();
                let event_target = wm.handle_mouse_down(cx as i32, cy as i32);
                let app_to_spawn = wm.pending_spawn_app.take();
                drop(wm);

                if let Some((wid, owner_pid)) = event_target {
                    let (local_x, local_y) = {
                        let wm = crate::wm::WM.lock();
                        if let Some(w) = wm.windows.iter().find(|w| w.window_id == wid) {
                            let surf_reg = crate::surface::SURFACE_REGISTRY.lock();
                            let (surf_w, surf_h) = if let Some(surf) = surf_reg.iter().find(|s| s.surface_id == w.surface_id) {
                                (surf.width as i32, surf.height as i32)
                            } else {
                                (w.width as i32, w.height as i32)
                            };
                            let lx = if w.width > 0 { (((cx as i32) - w.x) * surf_w) / (w.width as i32) } else { (cx as i32) - w.x };
                            let ly = if w.height > 0 { (((cy as i32) - (w.y + 20)) * surf_h) / (w.height as i32) } else { (cy as i32) - (w.y + 20) };
                            (lx, ly)
                        } else {
                            ((cx as i32), (cy as i32))
                        }
                    };

                    let ev = crate::input::InputEvent {
                        event_type: crate::input::EventType::MouseButtonDown as u8,
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
                    crate::input::deliver_event_to_pid(owner_pid, ev);

                    if local_y >= 0 {
                        let mut files = crate::files_app::FILES_INSTANCES.lock();
                        if let Some(files_state) = files.get_mut(&wid) {
                            files_state.handle_mouse_click(local_x as u32, local_y as u32);
                            if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.owner_pid == owner_pid) {
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
                                    if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.owner_pid == owner_pid) {
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
                    if (cx as i32) >= mx + 70 && (cx as i32) <= mx + 190 && (cy as i32) >= my + 118 && (cy as i32) <= my + 142 {
                        crate::crash_reporter::CRASH_REPORTER.lock().dismiss_active_crash();
                    }
                } else if let Some(action) = crate::desktop::DESKTOP_ENV.lock().handle_mouse_click(cx, cy, crate::interrupts::get_tick()) {
                    match action {
                        crate::desktop::DesktopIconAction::OpenHome | crate::desktop::DesktopIconAction::OpenComputer => {
                            let _ = crate::files_app::spawn_files_app("files.app");
                        }
                        crate::desktop::DesktopIconAction::OpenApplications => {
                            let _ = crate::settings_app::spawn_settings_app("settings.app");
                        }
                        crate::desktop::DesktopIconAction::OpenTrash => {
                            let _ = crate::files_app::spawn_files_app("trash.app");
                        }
                    }
                } else if cy < 24 && cx > 1000 {
                    crate::network_manager::NETWORK_MANAGER.lock().toggle_popup();
                }
                crate::wm::WM.lock().composite_desktop(cx as i32, cy as i32);
            } else if !click && last_click {
                let mut wm = crate::wm::WM.lock();
                if let Some((_wid, owner_pid)) = wm.handle_mouse_up() {
                    let ev = crate::input::InputEvent {
                        event_type: crate::input::EventType::MouseButtonUp as u8,
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
                    crate::input::deliver_event_to_pid(owner_pid, ev);
                }
                drop(wm);
                crate::wm::WM.lock().composite_desktop(cx as i32, cy as i32);
            } else if moved {
                let mut wm = crate::wm::WM.lock();
                wm.handle_mouse_move(cx as i32, cy as i32);
                if let Some(target_id) = wm.hit_test(cx as i32, cy as i32) {
                    if let Some(win) = wm.windows.iter().find(|w| w.window_id == target_id) {
                        let local_x = (cx as i32) - win.x;
                        let local_y = (cy as i32) - (win.y + 20);
                        if local_y >= 0 {
                            let ev = crate::input::InputEvent {
                                event_type: crate::input::EventType::MouseMove as u8,
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
                            crate::input::deliver_event_to_pid(win.owner_pid, ev);
                        }
                    }
                }
            }

            if moved || click != last_click {
                crate::wm::WM.lock().composite_desktop(cx as i32, cy as i32);
            }
        }
        
        last_x = cx;
        last_y = cy;
        last_click = click;
        crate::task::yield_now().await;
    }
}
