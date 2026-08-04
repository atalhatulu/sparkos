import re

with open("src/gui.rs", "r") as f:
    gui = f.read()

# Replace draw_desktop_and_window and draw_desktop
gui = re.sub(r'pub fn draw_desktop_and_window.*?}\n}', '''pub fn redraw_all() {
    let writers = WRITERS.lock();
    let z = Z_ORDER.lock();
    
    draw_background(0x001A2421);
    draw_icon(20, 20, "Terminal");
    draw_icon(20, 80, "Files");
    draw_icon(20, 140, "Notepad");
    draw_icon(20, 200, "TaskMgr");
    
    draw_rect(0, 1080 - 34, 1920, 34, 0x002D2D2D);
    draw_rect(0, 1080 - 34, 1920, 1, 0x004A4A4A);
    draw_rect(4, 1080 - 30, 70, 26, 0x003A3A3A);
    let mut px = 20; for c in "Start".chars() { draw_char(px, 1080 - 21, c, 0x00E0E0E0, 0x003A3A3A); px += 8; }
    
    let mut taskbar_x = 78;
    for &id in z.iter() {
        let w = &writers[id];
        if w.visible {
            let btn_color = if w.minimized { 0x003A3A3A } else { 0x005A5A5A };
            draw_rect(taskbar_x, 1080 - 30, 100, 26, btn_color);
            let title = match w.app_id { 1 => "Files", 2 => "Notepad", 3 => "TaskMgr", _ => "Terminal" };
            let mut tpx = taskbar_x + 8;
            for c in title.chars() { draw_char(tpx, 1080 - 21, c, 0x00E0E0E0, btn_color); tpx += 8; }
            taskbar_x += 110;
        }
    }
    
    for &id in z.iter() {
        let w = &writers[id];
        if w.visible && !w.minimized {
            let title = match w.app_id { 1 => "SparkOS Files", 2 => "SparkOS Notepad", 3 => "SparkOS Task Manager", _ => "SparkOS Terminal" };
            draw_window(w.win_x, w.win_y, w.win_w, w.win_h, title);
            
            if w.app_id == 0 {
                restore_window_content(0, w.win_x, w.win_y, w.win_w, w.win_h, w.win_w, w.win_h);
            } else if w.app_id == 1 { draw_files_ui(w.win_x, w.win_y, w.win_w, w.win_h); }
            else if w.app_id == 2 { draw_notepad_ui(w.win_x, w.win_y, w.win_w, w.win_h); }
            else if w.app_id == 3 { draw_taskmgr_ui(w.win_x, w.win_y, w.win_w, w.win_h); }
        }
    }
    
    if START_MENU_OPEN.load(core::sync::atomic::Ordering::Relaxed) {
        draw_start_menu();
    }
}''', gui, flags=re.DOTALL)

with open("src/gui.rs", "w") as f:
    f.write(gui)
