use x86_64::instructions::port::Port;
use spin::Mutex;
use core::sync::atomic::{AtomicU16, AtomicBool, Ordering};

pub static MOUSE_X: AtomicU16 = AtomicU16::new(400);
pub static MOUSE_Y: AtomicU16 = AtomicU16::new(300);
pub static MOUSE_LEFT_CLICK: AtomicBool = AtomicBool::new(false);


#[derive(Debug)]
struct MouseState {
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
            
            let left_click = (state.packet[0] & 1) != 0;
            MOUSE_LEFT_CLICK.store(left_click, Ordering::Relaxed);
            
            let mut x = MOUSE_X.load(Ordering::Relaxed) as i16 + dx;
            let mut y = MOUSE_Y.load(Ordering::Relaxed) as i16 + dy;
            
            if x < 0 { x = 0; }
            if y < 0 { y = 0; }
            if x >= 1920 { x = 1919; }
            if y >= 1080 { y = 1079; }
            
            MOUSE_X.store(x as u16, Ordering::Relaxed);
            MOUSE_Y.store(y as u16, Ordering::Relaxed);
        }
        _ => state.cycle = 0,
    }
}


pub async fn mouse_task() {
    let mut last_x = 400;
    let mut last_y = 300;
    let mut last_click = false;
    crate::gui::draw_cursor(last_x, last_y);
    
    loop {
        let cx = MOUSE_X.load(Ordering::Relaxed);
        let cy = MOUSE_Y.load(Ordering::Relaxed);
        let click = MOUSE_LEFT_CLICK.load(Ordering::Relaxed);
        
        let moved = cx != last_x || cy != last_y;
        
        if moved {
            crate::gui::update_cursor(last_x, last_y, cx, cy);
            last_x = cx;
            last_y = cy;
        }
        
        crate::gui::process_mouse_event(cx, cy, click, last_click, moved);
        
        last_click = click;
        crate::task::yield_now().await;
    }
}
