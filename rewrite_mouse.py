with open("src/mouse.rs", "w") as f:
    f.write("""
use crate::ata::AtaDrive;
use spin::Mutex;
use core::sync::atomic::Ordering;

#[derive(Clone, Copy)]
pub struct MouseState {
    pub x: u16,
    pub y: u16,
    pub left_pressed: bool,
    pub right_pressed: bool,
}

pub struct DragState {
    pub mode: u8, // 0: None, 1: Move, 2: Resize R, 3: Resize B, 4: Resize RB
    pub start_x: u16,
    pub start_y: u16,
    pub win_start_w: u16,
    pub win_start_h: u16,
    pub app_id: u8,
}

pub static MOUSE: Mutex<MouseState> = Mutex::new(MouseState {
    x: 960,
    y: 540,
    left_pressed: false,
    right_pressed: false,
});

pub static DRAG_STATE: Mutex<DragState> = Mutex::new(DragState {
    mode: 0,
    start_x: 0,
    start_y: 0,
    win_start_w: 0,
    win_start_h: 0,
    app_id: 255,
});

pub fn update_mouse(dx: i16, dy: i16, left: bool, right: bool) {
    let mut state = MOUSE.lock();
    let old_x = state.x;
    let old_y = state.y;

    let new_x = (old_x as i32 + dx as i32).clamp(0, 1919) as u16;
    let new_y = (old_y as i32 - dy as i32).clamp(0, 1079) as u16;

    state.x = new_x;
    state.y = new_y;
    state.left_pressed = left;
    state.right_pressed = right;

    crate::gui::erase_cursor(old_x, old_y);
    crate::gui::draw_cursor(new_x, new_y);
}

pub async fn run_mouse() {
    let mut last_click = false;
    let mut last_x = 960;
    let mut last_y = 540;

    loop {
        let (cx, cy, click) = {
            let m = MOUSE.lock();
            (m.x, m.y, m.left_pressed)
        };

        let moved = cx != last_x || cy != last_y;
        if moved {
            last_x = cx;
            last_y = cy;
        }

        if click && !last_click {
            // MOUSE DOWN
            let mut drag = DRAG_STATE.lock();
            let mut writers = crate::gui::WRITERS.lock();
            let mut z_order = crate::gui::Z_ORDER.lock();

            // Taskbar Start Menu Click
            if cx <= 74 && cy >= 1046 {
                let is_open = crate::gui::START_MENU_OPEN.load(Ordering::Relaxed);
                crate::gui::START_MENU_OPEN.store(!is_open, Ordering::Relaxed);
                crate::gui::redraw_all();
                crate::gui::draw_cursor(cx, cy);
                last_click = click;
                crate::task::yield_now().await;
                continue;
            }

            // Start Menu Buttons
            if crate::gui::START_MENU_OPEN.load(Ordering::Relaxed) {
                if cx >= 4 && cx <= 204 {
                    if cy >= 1000 && cy <= 1040 {
                        unsafe { crate::pci::outb(0x64, 0xFE); } // Reboot
                    }
                    if cy >= 950 && cy <= 990 {
                        unsafe { crate::pci::outw(0x604, 0x2000); } // Shutdown
                    }
                }
                crate::gui::START_MENU_OPEN.store(false, Ordering::Relaxed);
                crate::gui::redraw_all();
                crate::gui::draw_cursor(cx, cy);
            }

            let mut hit_app = 255;
            
            // 1. Check windows from Top to Bottom
            for i in (0..4).rev() {
                let app_id = z_order[i] as u8;
                let w = &writers[app_id as usize];
                if w.visible && !w.minimized {
                    if cx >= w.win_x && cx <= w.win_x + w.win_w && cy >= w.win_y && cy <= w.win_y + w.win_h {
                        hit_app = app_id;
                        break;
                    }
                }
            }

            if hit_app != 255 {
                // Clicked inside a window!
                crate::gui::ACTIVE_APP.store(hit_app, Ordering::Relaxed);
                let w = &mut writers[hit_app as usize];
                
                // Bring to front logic (update z_order array manually here since we have the lock)
                let mut pos = 0;
                for i in 0..4 { if z_order[i] == hit_app as usize { pos = i; break; } }
                for i in pos..3 { z_order[i] = z_order[i + 1]; }
                z_order[3] = hit_app as usize;
                
                // Check Window Buttons
                // Close
                if cx >= w.win_x + w.win_w - 26 && cx <= w.win_x + w.win_w - 6 && cy >= w.win_y + 6 && cy <= w.win_y + 26 {
                    w.visible = false;
                    drop(writers);
                    drop(z_order);
                    crate::gui::redraw_all();
                    crate::gui::draw_cursor(cx, cy);
                }
                // Minimize
                else if cx >= w.win_x + w.win_w - 74 && cx <= w.win_x + w.win_w - 54 && cy >= w.win_y + 6 && cy <= w.win_y + 26 {
                    crate::gui::backup_window_content(w.app_id, w.win_x, w.win_y, w.win_w, w.win_h);
                    w.minimized = true;
                    drop(writers);
                    drop(z_order);
                    crate::gui::redraw_all();
                    crate::gui::draw_cursor(cx, cy);
                }
                // Maximize/Restore
                else if cx >= w.win_x + w.win_w - 50 && cx <= w.win_x + w.win_w - 30 && cy >= w.win_y + 6 && cy <= w.win_y + 26 {
                    crate::gui::backup_window_content(w.app_id, w.win_x, w.win_y, w.win_w, w.win_h);
                    if w.win_w < 1920 {
                        w.win_x = 0; w.win_y = 0; w.win_w = 1920; w.win_h = 1046;
                    } else {
                        w.win_x = 100; w.win_y = 100; w.win_w = 800; w.win_h = 500;
                    }
                    drop(writers);
                    drop(z_order);
                    crate::gui::redraw_all();
                    crate::gui::draw_cursor(cx, cy);
                }
                // Resize RB
                else if cx >= w.win_x + w.win_w - 8 && cx <= w.win_x + w.win_w && cy >= w.win_y + w.win_h - 8 && cy <= w.win_y + w.win_h {
                    drag.mode = 4; drag.start_x = cx; drag.start_y = cy; drag.win_start_w = w.win_w; drag.win_start_h = w.win_h; drag.app_id = hit_app;
                }
                // Resize R
                else if cx >= w.win_x + w.win_w - 8 && cx <= w.win_x + w.win_w && cy >= w.win_y && cy <= w.win_y + w.win_h {
                    drag.mode = 2; drag.start_x = cx; drag.win_start_w = w.win_w; drag.app_id = hit_app;
                }
                // Resize B
                else if cy >= w.win_y + w.win_h - 8 && cy <= w.win_y + w.win_h && cx >= w.win_x && cx <= w.win_x + w.win_w {
                    drag.mode = 3; drag.start_y = cy; drag.win_start_h = w.win_h; drag.app_id = hit_app;
                }
                // Move (Title Bar)
                else if cx >= w.win_x && cx <= w.win_x + w.win_w && cy >= w.win_y && cy <= w.win_y + 24 {
                    drag.mode = 1; drag.start_x = cx.saturating_sub(w.win_x); drag.start_y = cy.saturating_sub(w.win_y); drag.app_id = hit_app;
                } else {
                    // Clicked inside content, just bring to front
                    drop(writers);
                    drop(z_order);
                    crate::gui::redraw_all();
                    crate::gui::draw_cursor(cx, cy);
                }
            } else {
                // 2. Check Desktop Icons
                if cx >= 20 && cx <= 60 {
                    let mut app_id = 255;
                    if cy >= 20 && cy <= 60 { app_id = 0; }
                    else if cy >= 80 && cy <= 120 { app_id = 1; }
                    else if cy >= 140 && cy <= 180 { app_id = 2; }
                    else if cy >= 200 && cy <= 240 { app_id = 3; }
                    
                    if app_id != 255 {
                        crate::gui::ACTIVE_APP.store(app_id, Ordering::Relaxed);
                        writers[app_id as usize].visible = true;
                        writers[app_id as usize].minimized = false;
                        
                        let mut pos = 0;
                        for i in 0..4 { if z_order[i] == app_id as usize { pos = i; break; } }
                        for i in pos..3 { z_order[i] = z_order[i + 1]; }
                        z_order[3] = app_id as usize;
                        
                        drop(writers);
                        drop(z_order);
                        crate::gui::redraw_all();
                        crate::gui::draw_cursor(cx, cy);
                    }
                }
                
                // 3. Check Taskbar Buttons
                if cy >= 1046 && cx > 74 {
                    let mut taskbar_x = 78;
                    let mut clicked_app = 255;
                    for &id in z_order.iter() {
                        if writers[id].visible {
                            if cx >= taskbar_x && cx <= taskbar_x + 100 {
                                clicked_app = id as u8;
                                break;
                            }
                            taskbar_x += 110;
                        }
                    }
                    if clicked_app != 255 {
                        let w = &mut writers[clicked_app as usize];
                        if w.minimized || z_order[3] != clicked_app as usize {
                            w.minimized = false;
                            
                            let mut pos = 0;
                            for i in 0..4 { if z_order[i] == clicked_app as usize { pos = i; break; } }
                            for i in pos..3 { z_order[i] = z_order[i + 1]; }
                            z_order[3] = clicked_app as usize;
                        } else {
                            w.minimized = true;
                        }
                        crate::gui::ACTIVE_APP.store(z_order[3] as u8, Ordering::Relaxed);
                        drop(writers);
                        drop(z_order);
                        crate::gui::redraw_all();
                        crate::gui::draw_cursor(cx, cy);
                    }
                }
            }
        } else if !click && last_click {
            // Mouse UP
            let mut drag = DRAG_STATE.lock();
            drag.mode = 0;
        } else if click && last_click && moved {
            let drag = DRAG_STATE.lock();
            if drag.mode != 0 {
                let mut writers = crate::gui::WRITERS.lock();
                let w = &mut writers[drag.app_id as usize];
                
                crate::gui::backup_window_content(w.app_id, w.win_x, w.win_y, w.win_w, w.win_h);
                
                if drag.mode == 1 {
                    w.win_x = cx.saturating_sub(drag.start_x);
                    w.win_y = cy.saturating_sub(drag.start_y);
                } else if drag.mode == 2 {
                    let diff = cx as i32 - drag.start_x as i32;
                    w.win_w = (drag.win_start_w as i32 + diff).max(300).min(1920) as u16;
                } else if drag.mode == 3 {
                    let diff = cy as i32 - drag.start_y as i32;
                    w.win_h = (drag.win_start_h as i32 + diff).max(200).min(1046) as u16;
                } else if drag.mode == 4 {
                    let diff_x = cx as i32 - drag.start_x as i32;
                    let diff_y = cy as i32 - drag.start_y as i32;
                    w.win_w = (drag.win_start_w as i32 + diff_x).max(300).min(1920) as u16;
                    w.win_h = (drag.win_start_h as i32 + diff_y).max(200).min(1046) as u16;
                }
                
                if w.win_x + w.win_w > 1920 { w.win_x = 1920 - w.win_w; }
                if w.win_y + w.win_h > 1046 { w.win_y = 1046 - w.win_h; }
                
                drop(writers);
                crate::gui::redraw_all();
                crate::gui::draw_cursor(cx, cy);
            }
        }
        
        last_click = click;
        crate::task::yield_now().await;
    }
}
""")
