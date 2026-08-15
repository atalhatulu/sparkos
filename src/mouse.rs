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
            
            if x < 0 { x = 0; }
            if y < 0 { y = 0; }
            if x >= 640 { x = 639; }
            if y >= 360 { y = 359; }
            
            MOUSE_X.store(x as u16, Ordering::Relaxed);
            MOUSE_Y.store(y as u16, Ordering::Relaxed);
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
                if let Some((wid, owner_pid)) = wm.handle_mouse_down(cx as i32, cy as i32) {
                    if let Some(win) = wm.windows.iter().find(|w| w.window_id == wid) {
                        let local_x = (cx as i32) - win.x;
                        let local_y = (cy as i32) - (win.y + 24);
                        let ev = crate::input::InputEvent {
                            event_type: crate::input::EventType::MouseDown as u8,
                            modifiers: 0,
                            key_code: 0,
                            mouse_button: 1,
                            wheel_delta: 0,
                            _reserved: [0; 3],
                            mouse_x: local_x,
                            mouse_y: local_y,
                            timestamp: 1000,
                            _padding: [0; 8],
                        };
                        crate::input::deliver_event_to_pid(owner_pid, ev);
                    }
                }
            } else if !click && last_click {
                let mut wm = crate::wm::WM.lock();
                if let Some((_wid, owner_pid)) = wm.handle_mouse_up() {
                    let ev = crate::input::InputEvent {
                        event_type: crate::input::EventType::MouseUp as u8,
                        modifiers: 0,
                        key_code: 0,
                        mouse_button: 1,
                        wheel_delta: 0,
                        _reserved: [0; 3],
                        mouse_x: 0,
                        mouse_y: 0,
                        timestamp: 1000,
                        _padding: [0; 8],
                    };
                    crate::input::deliver_event_to_pid(owner_pid, ev);
                }
            } else if moved {
                let mut wm = crate::wm::WM.lock();
                wm.handle_mouse_move(cx as i32, cy as i32);
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
