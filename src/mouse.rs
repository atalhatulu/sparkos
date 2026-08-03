use x86_64::instructions::port::Port;
use spin::Mutex;
use core::sync::atomic::{AtomicU16, AtomicBool, Ordering};

pub static MOUSE_X: AtomicU16 = AtomicU16::new(400);
pub static MOUSE_Y: AtomicU16 = AtomicU16::new(300);
pub static MOUSE_LEFT_CLICK: AtomicBool = AtomicBool::new(false);

pub struct DragState {
    pub mode: u8, // 0=None, 1=Move, 2=Right, 3=Bottom, 4=BottomRight
    pub start_x: u16,
    pub start_y: u16,
    pub win_start_w: u16,
    pub win_start_h: u16,
}
pub static DRAG_STATE: Mutex<DragState> = Mutex::new(DragState { mode: 0, start_x: 0, start_y: 0, win_start_w: 0, win_start_h: 0 });

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
        
        if click && !last_click {
            let mut w = crate::gui::WRITER.lock();
            let mut drag = DRAG_STATE.lock();
            let c_x = cx;
            let c_y = cy;
            
            // Start Menu Kontrolu
            if crate::gui::START_MENU_OPEN.load(Ordering::Relaxed) {
                crate::gui::START_MENU_OPEN.store(false, Ordering::Relaxed);
                
                // Restart Tıklandı Mı? (x: 4-154, y: 970-1010)
                if c_x >= 4 && c_x <= 154 && c_y >= 970 && c_y <= 1010 {
                    crate::gui::erase_cursor(last_x, last_y);
                    unsafe {
                        let mut p: x86_64::instructions::port::Port<u8> = x86_64::instructions::port::Port::new(0x2000u16);
                        p.write(0x04u8);
                    }
                }
                // Shutdown Tıklandı Mı? (x: 4-154, y: 1010-1050)
                else if c_x >= 4 && c_x <= 154 && c_y >= 1010 && c_y <= 1050 {
                    crate::gui::erase_cursor(last_x, last_y);
                    unsafe {
                        let mut p: x86_64::instructions::port::Port<u16> = x86_64::instructions::port::Port::new(0xB004);
                        p.write(0x2000);
                    }
                    loop { x86_64::instructions::hlt(); }
                }
                
                // Menüyü kapattığımız için masaüstünü yeniden çiz
                crate::gui::draw_desktop_and_window(w.win_x, w.win_y, w.win_w, w.win_h, w.visible);
                    crate::gui::swap_buffers();
                
                drop(drag);
                drop(w);
                last_click = click;
                crate::task::yield_now().await;
                continue;
            }
            
            // Start Butonu Tıklama (x: 4-74, y: 1050-716)
            if c_x >= 4 && c_x <= 74 && c_y >= 1050 && c_y <= 716 {
                crate::gui::START_MENU_OPEN.store(true, Ordering::Relaxed);
                crate::gui::draw_desktop_and_window(w.win_x, w.win_y, w.win_w, w.win_h, w.visible);
                    crate::gui::swap_buffers();
                drop(drag);
                drop(w);
                last_click = click;
                crate::task::yield_now().await;
                continue;
            }

            // Masaustu Ikonlari (x: 20-60)
            if c_x >= 20 && c_x <= 60 {
                let mut app_name = "";
                if c_y >= 20 && c_y <= 60 { app_name = "Terminal"; }
                else if c_y >= 80 && c_y <= 120 { app_name = "Files"; }
                else if c_y >= 140 && c_y <= 180 { app_name = "Notepad"; }
                else if c_y >= 200 && c_y <= 240 { app_name = "TaskMgr"; }
                
                if !app_name.is_empty() {
                    if !w.visible {
                        w.visible = true;
                        crate::gui::draw_desktop_and_window(w.win_x, w.win_y, w.win_w, w.win_h, w.visible);
                    crate::gui::swap_buffers();
                    }
                    use core::fmt::Write;
                    if app_name == "Terminal" && w.row == 0 && w.col == 0 {
                        let _ = write!(w, "Welcome to SparkOS!\nsparkos > ");
                    } else {
                        let _ = writeln!(w, "\n[Sistem] {} uygulamasi aciliyor...", app_name);
                        w.set_color(0x00FFFFFF, 0x001E1E1E);
                        let _ = write!(w, "sparkos > ");
                    }
                }
            }
            
            // Files (Dolphin) Butonu (x=78, y=1050, w=70, h=26)
            if c_x >= 78 && c_x <= 148 && c_y >= 1050 && c_y <= 716 {
                if !w.visible {
                    w.visible = true;
                    crate::gui::draw_desktop_and_window(w.win_x, w.win_y, w.win_w, w.win_h, w.visible);
                    crate::gui::swap_buffers();
                }
                use core::fmt::Write;
                let _ = writeln!(w, "\n[Dosya Yöneticisi] Disk Durumu (/)");
                match crate::fs::list_dir("/") {
                    Ok(items) => {
                        if items.is_empty() {
                            let _ = writeln!(w, "(Bos)");
                        } else {
                            for (name, is_dir) in items {
                                if is_dir {
                                    w.set_color(0x0000AAFF, 0x001E1E1E); // LightBlue
                                    let _ = write!(w, "{}/  ", name);
                                } else {
                                    w.set_color(0x00FFFFFF, 0x001E1E1E); // White
                                    let _ = write!(w, "{}  ", name);
                                }
                            }
                            let _ = writeln!(w);
                        }
                    }
                    Err(e) => {
                        w.set_color(0x00FF0000, 0x001E1E1E); // Red
                        let _ = writeln!(w, "Hata: {}", e);
                    }
                }
                w.set_color(0x00FFFFFF, 0x001E1E1E);
                let _ = write!(w, "sparkos > ");
            }
            
            // Eger pencere kapaliysa diger butonlara bakma
            if !w.visible {
                drop(drag);
                drop(w);
                last_click = click;
                crate::task::yield_now().await;
                continue;
            }
            
            // Kapat Butonu (X)
            if c_x >= w.win_x + w.win_w - 24 && c_x <= w.win_x + w.win_w - 8 && c_y >= w.win_y + 6 && c_y <= w.win_y + 22 {
                w.visible = false;
                crate::gui::draw_desktop();
                crate::gui::swap_buffers();
            }
            // Buyut / Kucult Butonu (O)
            else if c_x >= w.win_x + w.win_w - 44 && c_x <= w.win_x + w.win_w - 28 && c_y >= w.win_y + 6 && c_y <= w.win_y + 22 {
                crate::gui::erase_cursor(last_x, last_y); // cursoru temizle once
                
                if w.win_w < 1920 {
                    // Maximize
                    w.win_x = 0;
                    w.win_y = 0;
                    w.win_w = 1920;
                    w.win_h = 1050; // Taskbar payi (1080-30)
                    crate::gui::draw_desktop_and_window(w.win_x, w.win_y, w.win_w, w.win_h, w.visible);
                    crate::gui::swap_buffers();
                } else {
                    // Restore
                    w.win_x = 100;
                    w.win_y = 100;
                    w.win_w = 800;
                    w.win_h = 500;
                    crate::gui::draw_desktop_and_window(w.win_x, w.win_y, w.win_w, w.win_h, w.visible);
                    crate::gui::swap_buffers();
                }
                crate::gui::draw_cursor(last_x, last_y); // cursoru tekrar ciz
            }
            // Yeniden Boyutlandirma: Sag Alt Kose
            else if c_x >= w.win_x + w.win_w - 8 && c_x <= w.win_x + w.win_w && c_y >= w.win_y + w.win_h - 8 && c_y <= w.win_y + w.win_h {
                drag.mode = 4;
                drag.start_x = cx;
                drag.start_y = cy;
                drag.win_start_w = w.win_w;
                drag.win_start_h = w.win_h;
            }
            // Yeniden Boyutlandirma: Sag Kenar
            else if c_x >= w.win_x + w.win_w - 8 && c_x <= w.win_x + w.win_w && c_y >= w.win_y && c_y <= w.win_y + w.win_h {
                drag.mode = 2;
                drag.start_x = cx;
                drag.win_start_w = w.win_w;
            }
            // Yeniden Boyutlandirma: Alt Kenar
            else if c_y >= w.win_y + w.win_h - 8 && c_y <= w.win_y + w.win_h && c_x >= w.win_x && c_x <= w.win_x + w.win_w {
                drag.mode = 3;
                drag.start_y = cy;
                drag.win_start_h = w.win_h;
            }
            // Tasi (Title Bar)
            else if c_x >= w.win_x && c_x <= w.win_x + w.win_w && c_y >= w.win_y && c_y <= w.win_y + 24 {
                drag.mode = 1;
                drag.start_x = cx.saturating_sub(w.win_x);
                drag.start_y = cy.saturating_sub(w.win_y);
            }
        } 
        else if !click && last_click {
            // Mouse UP
            let mut drag = DRAG_STATE.lock();
            drag.mode = 0;
        } 
        else if click && last_click && moved {
            let drag = DRAG_STATE.lock();
            if drag.mode != 0 {
                crate::gui::erase_cursor(last_x, last_y);
                let mut w = crate::gui::WRITER.lock();
                
                let old_w = w.win_w;
                let old_h = w.win_h;
                
                // Icerigi off-screen buffera (1920x1080 otesine) kopyala
                unsafe {
                    let offscreen = crate::gui::VESA.framebuffer.add(1920 * 1080);
                    for i in 0..(w.win_h - 32) {
                        for j in 0..(w.win_w - 8) {
                            let src_offset = ((w.win_y + 28 + i) as usize) * 1920 + ((w.win_x + 4 + j) as usize);
                            let dst_offset = (i as usize) * 1920 + (j as usize);
                            let px = core::ptr::read_volatile(crate::gui::VESA.framebuffer.add(src_offset));
                            core::ptr::write_volatile(offscreen.add(dst_offset), px);
                        }
                    }
                }
                
                if drag.mode == 1 {
                    w.win_x = cx.saturating_sub(drag.start_x);
                    w.win_y = cy.saturating_sub(drag.start_y);
                } else if drag.mode == 2 {
                    let diff = cx as i32 - drag.start_x as i32;
                    w.win_w = (drag.win_start_w as i32 + diff).max(300).min(1920) as u16;
                } else if drag.mode == 3 {
                    let diff = cy as i32 - drag.start_y as i32;
                    w.win_h = (drag.win_start_h as i32 + diff).max(200).min(1050) as u16;
                } else if drag.mode == 4 {
                    let diff_x = cx as i32 - drag.start_x as i32;
                    let diff_y = cy as i32 - drag.start_y as i32;
                    w.win_w = (drag.win_start_w as i32 + diff_x).max(300).min(1920) as u16;
                    w.win_h = (drag.win_start_h as i32 + diff_y).max(200).min(1050) as u16;
                }
                
                // Desktop'i ve yeni pencereyi ciz
                crate::gui::draw_desktop_and_window(w.win_x, w.win_y, w.win_w, w.win_h, w.visible);
                    crate::gui::swap_buffers();
                
                // Off-screen'den icerigi geri kopyala (Kuculen veya buyuyen sinirlara dikkat et)
                let copy_w = core::cmp::min(old_w, w.win_w) - 8;
                let copy_h = core::cmp::min(old_h, w.win_h) - 32;
                unsafe {
                    let offscreen = crate::gui::VESA.framebuffer.add(1920 * 1080);
                    for i in 0..copy_h {
                        for j in 0..copy_w {
                            let src_offset = (i as usize) * 1920 + (j as usize);
                            let dst_offset = ((w.win_y + 28 + i) as usize) * 1920 + ((w.win_x + 4 + j) as usize);
                            let px = core::ptr::read_volatile(offscreen.add(src_offset));
                            core::ptr::write_volatile(crate::gui::VESA.framebuffer.add(dst_offset), px);
                        }
                    }
                }
                
                crate::gui::draw_cursor(cx, cy);
            }
        }
        last_click = click;
        
        // Asenkron gorev oldugu icin beklerken sistemi kilitleme
        crate::task::yield_now().await;
    }
}
