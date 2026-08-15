use x86_64::instructions::port::Port;

pub struct Vesa {
    pub width: u16,
    pub height: u16,
    pub framebuffer: *mut u32,
}

pub static mut VESA: Vesa = Vesa {
    width: 1280,
    height: 720,
    framebuffer: core::ptr::null_mut(),
};

pub static mut PHYS_OFFSET: u64 = 0;
pub static mut BACKBUFFER: *mut u32 = core::ptr::null_mut();
pub static ACTIVE_APP: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
pub static START_MENU_OPEN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub static mut CLIP_RECT: Option<(u16, u16, u16, u16)> = None;

pub fn set_clip(rect: Option<(u16, u16, u16, u16)>) {
    unsafe { CLIP_RECT = rect; }
}

fn union_rect(x1: u16, y1: u16, w1: u16, h1: u16, x2: u16, y2: u16, w2: u16, h2: u16) -> (u16, u16, u16, u16) {
    let ix1 = x1.min(x2);
    let iy1 = y1.min(y2);
    let ix2 = (x1 + w1).max(x2 + w2);
    let iy2 = (y1 + h1).max(y2 + h2);
    // Include shadow size + margin
    let bx1 = ix1.saturating_sub(10);
    let by1 = iy1.saturating_sub(10);
    let bx2 = ix2 + 15;
    let by2 = iy2 + 15;
    (bx1, by1, bx2 - bx1, by2 - by1)
}
fn intersect_rect(x1: u16, y1: u16, w1: u16, h1: u16, x2: u16, y2: u16, w2: u16, h2: u16) -> Option<(u16, u16, u16, u16)> {
    let ix1 = x1.max(x2);
    let iy1 = y1.max(y2);
    let ix2 = (x1 + w1).min(x2 + w2);
    let iy2 = (y1 + h1).min(y2 + h2);
    if ix1 < ix2 && iy1 < iy2 {
        Some((ix1, iy1, ix2 - ix1, iy2 - iy1))
    } else {
        None
    }
}


pub fn init(backbuffer_ptr: Option<u64>) {
    let mut index_port: Port<u16> = Port::new(0x01CE);
    let mut data_port: Port<u16> = Port::new(0x01CF);
    
    unsafe {
        VESA.width = 1280;
        VESA.height = 720;

        // VBE'yi devre dışı bırak
        index_port.write(4);
        data_port.write(0);
        
        // Genişlik = 1280
        index_port.write(1);
        data_port.write(1280);
        
        // Yükseklik = 720
        index_port.write(2);
        data_port.write(720);
        
        // Renk Derinliği = 32 BPP (Bits Per Pixel)
        index_port.write(3);
        data_port.write(32);
        
        // VBE'yi Etkinleştir (0x01) ve Linear Framebuffer'ı (0x40) aç
        index_port.write(4);
        data_port.write(0x01 | 0x40);
        
        // QEMU (Bochs) VBE LFB adresi genelde 0xFD000000'dır.
        VESA.framebuffer = (PHYS_OFFSET + 0xFD000000) as *mut u32;
        
        if let Some(ptr) = backbuffer_ptr {
            BACKBUFFER = ptr as *mut u32;
        } else if BACKBUFFER.is_null() {
            let layout = alloc::alloc::Layout::from_size_align(1280 * 720 * 4, 64).unwrap();
            BACKBUFFER = alloc::alloc::alloc_zeroed(layout) as *mut u32;
        }
    }
}

pub fn swap_buffers() {
    unsafe {
        if BACKBUFFER.is_null() || VESA.framebuffer.is_null() { return; }
        let total_pixels = (VESA.width as usize) * (VESA.height as usize);
        core::ptr::copy_nonoverlapping(BACKBUFFER, VESA.framebuffer, total_pixels);
    }
}

pub fn flush_rect(x: u16, y: u16, w: u16, h: u16) {
    unsafe {
        if BACKBUFFER.is_null() || VESA.framebuffer.is_null() { return; }
        
        let start_y = core::cmp::min(y, VESA.height);
        let end_y = core::cmp::min(y + h, VESA.height);
        let start_x = core::cmp::min(x, VESA.width);
        let copy_width = core::cmp::min(w, VESA.width - start_x);
        
        for row in start_y..end_y {
            let offset = (row as usize) * (VESA.width as usize) + (start_x as usize);
            core::ptr::copy_nonoverlapping(
                BACKBUFFER.add(offset),
                VESA.framebuffer.add(offset),
                copy_width as usize
            );
        }
    }
}


pub fn draw_pixel(x: u16, y: u16, color: u32) {
    unsafe {
        if BACKBUFFER.is_null() || x >= VESA.width || y >= VESA.height { return; }
        let offset = (y as usize) * (VESA.width as usize) + (x as usize);
        *BACKBUFFER.add(offset) = color;
    }
}

pub fn draw_rect(x: u16, y: u16, w: u16, h: u16, color: u32) {
    unsafe {
        if BACKBUFFER.is_null() { return; }
        
        let (cx, cy, cw, ch) = match CLIP_RECT {
            Some(r) => match intersect_rect(x, y, w, h, r.0, r.1, r.2, r.3) {
                Some(cr) => cr,
                None => return,
            },
            None => (x, y, w, h),
        };
        
        let start_y = core::cmp::min(cy, VESA.height);
        let end_y = core::cmp::min(cy + ch, VESA.height);
        let start_x = core::cmp::min(cx, VESA.width);
        let copy_width = core::cmp::min(cw, VESA.width - start_x);
        
        if copy_width == 0 { return; }
        
        for row in start_y..end_y {
            let offset = (row as usize) * (VESA.width as usize) + (start_x as usize);
            let slice = core::slice::from_raw_parts_mut(BACKBUFFER.add(offset), copy_width as usize);
            slice.fill(color);
        }
    }
}

pub fn alpha_blend(bg: u32, fg: u32, alpha: u8) -> u32 {
    let a = alpha as u32;
    let inv_a = 255 - a;

    let br = (bg >> 16) & 0xFF;
    let bg_g = (bg >> 8) & 0xFF;
    let bb = bg & 0xFF;

    let fr = (fg >> 16) & 0xFF;
    let fg_g = (fg >> 8) & 0xFF;
    let fb = fg & 0xFF;

    let r = ((fr * a) + (br * inv_a)) / 255;
    let g = ((fg_g * a) + (bg_g * inv_a)) / 255;
    let b = ((fb * a) + (bb * inv_a)) / 255;

    (r << 16) | (g << 8) | b
}

pub fn draw_rect_alpha(x: u16, y: u16, w: u16, h: u16, color: u32, alpha: u8) {
    unsafe {
        if BACKBUFFER.is_null() { return; }
        
        let (cx, cy, cw, ch) = match CLIP_RECT {
            Some(r) => match intersect_rect(x, y, w, h, r.0, r.1, r.2, r.3) {
                Some(cr) => cr,
                None => return,
            },
            None => (x, y, w, h),
        };
        
        let start_y = core::cmp::min(cy, VESA.height);
        let end_y = core::cmp::min(cy + ch, VESA.height);
        let start_x = core::cmp::min(cx, VESA.width);
        let copy_width = core::cmp::min(cw, VESA.width - start_x);
        
        if copy_width == 0 { return; }
        
        for row in start_y..end_y {
            let offset = (row as usize) * (VESA.width as usize) + (start_x as usize);
            let slice = core::slice::from_raw_parts_mut(BACKBUFFER.add(offset), copy_width as usize);
            for pixel in slice.iter_mut() {
                *pixel = alpha_blend(*pixel, color, alpha);
            }
        }
    }
}

pub fn draw_3d_rect(x: u16, y: u16, w: u16, h: u16, bg_color: u32, pushed: bool) {
    draw_rect(x, y, w, h, bg_color);
    let (light, dark) = if pushed {
        (0x00404040, 0x00FFFFFF) // Basiliyken karanlik sol ust
    } else {
        (0x00FFFFFF, 0x00404040) // Normalde aydinlik sol ust
    };
    
    // Top border
    draw_rect(x, y, w, 2, light);
    // Left border
    draw_rect(x, y, 2, h, light);
    // Bottom border
    draw_rect(x, y + h - 2, w, 2, dark);
    // Right border
    draw_rect(x + w - 2, y, 2, h, dark);
}

pub fn draw_window(x: u16, y: u16, w: u16, h: u16, title: &str) {
    // Window Gölgesi (Yumuşak Yarı Saydam Siyah Katman)
    draw_rect_alpha(x + 5, y + 5, w, h, 0x00000000, 100);
    
    // Modern Kenarlık ve Arkaplan (Çok ince)
    draw_rect(x, y, w, h, 0x00333333); // Dış border
    draw_rect(x + 1, y + 1, w - 2, h - 2, 0x00222222); // İçerik arka planı
    
    // Baslik cubugu (Dark gradient tarzı)
    draw_rect(x + 1, y + 1, w - 2, 28, 0x00181818); // Koyu siyahımsı bar
    
    // Baslik metni (Açık Gri)
    let mut px = x + 12;
    for c in title.chars() {
        draw_char(px, y + 9, c, 0x00E0E0E0, 0x00181818);
        px += 8;
    }
    
    // Window Butonlari (Flat Design)
    // Minimize (Gri)
    draw_rect(x + w - 74, y + 6, 20, 20, 0x00444444);
    draw_char(x + w - 68, y + 10, '_', 0x00E0E0E0, 0x00444444);
    
    // Maximize (Mavi/Gri)
    draw_rect(x + w - 50, y + 6, 20, 20, 0x00444444);
    draw_char(x + w - 44, y + 10, 'O', 0x00E0E0E0, 0x00444444);
    
    // Close (Kırmızı)
    draw_rect(x + w - 26, y + 6, 20, 20, 0x00C0392B);
    draw_char(x + w - 20, y + 10, 'X', 0x00FFFFFF, 0x00C0392B);
    
    // Icerik alani: Premium Dark Mode
    draw_rect(x + 2, y + 30, w - 4, h - 32, 0x00141414);
}

pub fn draw_icon(x: u16, y: u16, text: &str) {
    // Modern Ikon (Hafif yuvarlak hissiyatı veren düz renkler)
    draw_rect(x, y, 48, 48, 0x003A4A5A); // Slate Blue
    draw_rect(x + 2, y + 2, 44, 44, 0x004A5A6A); // Inner Slate Blue
    
    if text == "Terminal" {
        draw_rect(x + 8, y + 14, 32, 24, 0x00141414);
        draw_char(x + 10, y + 18, '>', 0x004CAF50, 0x00141414);
    } else if text == "Files" {
        draw_rect(x + 6, y + 14, 36, 24, 0x00F39C12);
        draw_rect(x + 6, y + 10, 16, 6, 0x00E67E22);
    } else {
        draw_rect(x + 12, y + 12, 24, 24, 0x00E0E0E0);
    }
    
    // Altina Isim Yazisi
    let mut px = x as i32 + 24 - ((text.len() * 8) as i32) / 2;
    for c in text.chars() {
        if px >= 0 {
            draw_char(px as u16, y + 54, c, 0x00FFFFFF, 0x001A2421); // Desktop arkaplanı ile eşleşmeli
        }
        px += 8;
    }
}

pub fn draw_bitmap_8x8(x: u16, y: u16, bitmap: &[u8; 8], fg: u32, bg: u32) {
    for row in 0..8 {
        let b = bitmap[row];
        for col in 0..8 {
            let px = x + col as u16;
            let py = y + row as u16;
            let col_val = if (b & (1 << (7 - col))) != 0 { fg } else { bg };
            draw_pixel(px, py, col_val);
        }
    }
}

pub fn draw_icon_glyph(x: u16, y: u16, icon: crate::app_registry::AppIcon, fg: u32, bg: u32) {
    match icon {
        crate::app_registry::AppIcon::Logo => {
            let bitmap: [u8; 8] = [
                0b00111100,
                0b01111110,
                0b11011011,
                0b11111111,
                0b11111111,
                0b11011011,
                0b01111110,
                0b00111100,
            ];
            draw_bitmap_8x8(x, y, &bitmap, 0x0038BDF8 /* Sky Blue */, bg);
        }
        crate::app_registry::AppIcon::Terminal => {
            let bitmap: [u8; 8] = [
                0b00000000,
                0b10000000,
                0b01000000,
                0b00100000,
                0b01000000,
                0b10000000,
                0b00001111,
                0b00000000,
            ];
            draw_bitmap_8x8(x, y, &bitmap, 0x0034D399 /* Emerald Green */, bg);
        }
        crate::app_registry::AppIcon::Demo => {
            let bitmap: [u8; 8] = [
                0b00111100,
                0b00011000,
                0b00011000,
                0b00111100,
                0b01111110,
                0b11100111,
                0b11111111,
                0b01111110,
            ];
            draw_bitmap_8x8(x, y, &bitmap, 0x00F59E0B /* Amber */, bg);
        }
        crate::app_registry::AppIcon::Files => {
            let bitmap: [u8; 8] = [
                0b01110000,
                0b11111110,
                0b10000001,
                0b11111111,
                0b11111111,
                0b11111111,
                0b11111111,
                0b01111110,
            ];
            draw_bitmap_8x8(x, y, &bitmap, 0x0060A5FA /* Blue */, bg);
        }
        _ => {
            let bitmap: [u8; 8] = [
                0b11111111,
                0b10000001,
                0b10000001,
                0b10000001,
                0b10000001,
                0b10000001,
                0b10000001,
                0b11111111,
            ];
            draw_bitmap_8x8(x, y, &bitmap, fg, bg);
        }
    }
}

pub fn draw_desktop(terminal_visible: bool, terminal_minimized: bool) {
    // Premium Koyu Okyanus Arka Plan (Gradient)
    draw_background(0x000F2027, 0x00203A43);
    
    // Masaustu Ikonlari
    draw_icon(20, 20, "Terminal");
    draw_icon(20, 80, "Files");
    draw_icon(20, 140, "Notepad");
    draw_icon(20, 200, "TaskMgr");
    
    // Alt Gorev Cubugu (Taskbar) Modern Koyu Gri Glassmorphism
    draw_rect_alpha(0, 1080 - 34, 1920, 34, 0x001E293B, 200); // 200 alpha
    // Taskbar Ust Ince Cizgisi
    draw_rect_alpha(0, 1080 - 34, 1920, 1, 0x00334155, 200);
    
    // Start Butonu (Modern Kutu, Glassmorphism)
    draw_rect_alpha(4, 1080 - 30, 70, 26, 0x002563EB, 220); // Koyu mavi buton
    let start_text = "Start";
    let mut px = 20;
    for c in start_text.chars() {
        draw_char(px, 1080 - 21, c, 0x00FFFFFF, 0x00000000); // Transparent arka plan
        px += 8;
    }
    
    // Files Butonu
    draw_rect(78, 1080 - 30, 70, 26, 0x003A3A3A);
    let files_text = "Files";
    let mut px = 94;
    for c in files_text.chars() {
        draw_char(px, 1080 - 21, c, 0x00E0E0E0, 0x003A3A3A);
        px += 8;
    }
    
    // Açık olan Terminal Penceresi Butonu (Görev Çubuğunda)
    if terminal_visible {
        let btn_color = if terminal_minimized { 0x003A3A3A } else { 0x005A5A5A }; // Açıkken daha parlak gri
        draw_rect(152, 1080 - 30, 100, 26, btn_color);
        
        let app_id = ACTIVE_APP.load(core::sync::atomic::Ordering::Relaxed);
        let title = match app_id {
            1 => "Files",
            2 => "Notepad",
            3 => "TaskMgr",
            _ => "Terminal",
        };
        
        let mut px = 160;
        for c in title.chars() {
            draw_char(px, 1080 - 21, c, 0x00E0E0E0, btn_color);
            px += 8;
        }
    }
    
    // Sag alt kose Saat / Logo alani (Modern)
    draw_rect(1920 - 100, 1080 - 30, 96, 26, 0x00202020);
    let logo_text = "SparkOS";
    let mut px = 1920 - 85;
    for c in logo_text.chars() {
        draw_char(px, 1080 - 21, c, 0x00808080, 0x00202020); // Silik yazi
        px += 8;
    }
    
    if START_MENU_OPEN.load(core::sync::atomic::Ordering::Relaxed) {
        draw_start_menu();
    }
    
    flush_rect(0, 0, 1920, 1080);
}

pub fn draw_start_menu() {
    // Menu Kutusu: Modern Koyu Gri Glassmorphism
    draw_rect_alpha(4, 970, 150, 76, 0x001E293B, 220);
    draw_rect_alpha(4, 970, 150, 1, 0x00334155, 220); // border top
    draw_rect_alpha(4, 970, 1, 76, 0x00334155, 220); // border left
    
    // Restart Seçeneği
    let mut px = 12;
    for c in "Restart".chars() {
        draw_char(px, 985, c, 0x00E0E0E0, 0x00000000);
        px += 8;
    }
    
    // Shutdown Seçeneği
    let mut px = 12;
    for c in "Shutdown".chars() {
        draw_char(px, 1025, c, 0x00E0E0E0, 0x00000000);
        px += 8;
    }
}


pub fn draw_files_ui(x: u16, y: u16, _w: u16, h: u16) {
    // Sidebar
    draw_rect(x + 2, y + 30, 120, h.saturating_sub(32), 0x001A1A1A);
    // Sidebar items
    let mut py = y + 40;
    for item in ["Root", "System", "Users", "Docs"].iter() {
        let mut px = x + 10;
        for c in item.chars() {
            draw_char(px, py, c, 0x00CCCCCC, 0x001A1A1A);
            px += 8;
        }
        py += 24;
    }
    // Main area icons
    draw_rect(x + 150, y + 50, 48, 48, 0x00FFA500); // Folder icon
    let mut px = x + 152;
    for c in "bin".chars() { draw_char(px, y + 105, c, 0x00E0E0E0, 0x00141414); px += 8; }
    
    draw_rect(x + 230, y + 50, 48, 48, 0x00FFA500); // Folder icon
    let mut px = x + 232;
    for c in "etc".chars() { draw_char(px, y + 105, c, 0x00E0E0E0, 0x00141414); px += 8; }
    
    draw_rect(x + 310, y + 50, 48, 48, 0x00FFA500); // Folder icon
    let mut px = x + 312;
    for c in "home".chars() { draw_char(px, y + 105, c, 0x00E0E0E0, 0x00141414); px += 8; }
}

pub fn draw_notepad_ui(x: u16, y: u16, w: u16, h: u16) {
    // White-ish background for notepad
    draw_rect(x + 2, y + 30, w.saturating_sub(4), h.saturating_sub(32), 0x00F0F0F0);
    // Menu bar
    draw_rect(x + 2, y + 30, w.saturating_sub(4), 30, 0x00E0E0E0);
}

pub fn draw_taskmgr_ui(x: u16, y: u16, w: u16, _h: u16) {
    // Tabs
    draw_rect(x + 2, y + 30, w.saturating_sub(4), 30, 0x00222222);
    let mut px = x + 10;
    for c in "Processes   Performance".chars() {
        draw_char(px, y + 40, c, 0x00E0E0E0, 0x00222222);
        px += 8;
    }
    
    // Process list header
    draw_rect(x + 2, y + 60, w.saturating_sub(4), 20, 0x001A1A1A);
    px = x + 10;
    for c in "Name                PID    CPU    RAM".chars() {
        draw_char(px, y + 66, c, 0x00AAAAAA, 0x001A1A1A);
        px += 8;
    }
    
    // Dummy processes
    px = x + 10;
    for c in "System Idle         0      98%    4 MB".chars() { draw_char(px, y + 90, c, 0x00E0E0E0, 0x00141414); px += 8; }
    px = x + 10;
    for c in "Desktop Window      1      1%     12 MB".chars() { draw_char(px, y + 110, c, 0x00E0E0E0, 0x00141414); px += 8; }
    px = x + 10;
    for c in "SparkOS Kernel      2      1%     24 MB".chars() { draw_char(px, y + 130, c, 0x00E0E0E0, 0x00141414); px += 8; }
}

pub fn redraw_all(clip: Option<(u16, u16, u16, u16)>) {
    set_clip(clip);
    let writers = WRITERS.lock();
    let z = Z_ORDER.lock();
    
    draw_background(0x000F2027, 0x00203A43);
    draw_icon(20, 20, "Terminal");
    draw_icon(20, 80, "Files");
    draw_icon(20, 140, "Notepad");
    draw_icon(20, 200, "TaskMgr");
    
    // Alt Gorev Cubugu (Taskbar) Modern Koyu Gri Glassmorphism
    draw_rect_alpha(0, 1080 - 34, 1920, 34, 0x001E293B, 200); // 200 alpha
    draw_rect_alpha(0, 1080 - 34, 1920, 1, 0x00334155, 200);
    
    draw_rect_alpha(4, 1080 - 30, 70, 26, 0x002563EB, 220); // Koyu mavi buton
    let mut px = 20; for c in "Start".chars() { draw_char(px, 1080 - 21, c, 0x00FFFFFF, 0x00000000); px += 8; }
    
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

            // Draw object-oriented widgets
            for widget in &w.widgets {
                widget.draw(w.win_x, w.win_y);
            }
        }
    }
    
    if START_MENU_OPEN.load(core::sync::atomic::Ordering::Relaxed) {
        draw_start_menu();
    }
    
    if let Some((cx, cy, cw, ch)) = clip {
        flush_rect(cx, cy, cw, ch);
    } else {
        flush_rect(0, 0, 1920, 1080);
    }
    set_clip(None);
}

// Pencere icerigini (yazilari) off-screen buffera kaydet
pub fn backup_window_content(app_id: u8, x: u16, y: u16, w: u16, h: u16) {
    let offset = match app_id {
        0 => 0,
        1 => 900*600,
        2 => 900*600 + 800*500,
        3 => 900*600 + 800*500 + 800*600,
        _ => 0,
    };
    unsafe {
        if BACKBUFFER.is_null() { return; }
        let offscreen = BACKBUFFER.add(1920 * 1080 + offset);
        let content_w = w.saturating_sub(4) as usize;
        let content_h = h.saturating_sub(32) as usize;
        
        for i in 0..content_h {
            let src_offset = ((y + 30 + i as u16) as usize) * 1920 + ((x + 2) as usize);
            let dst_offset = i * 1920;
            core::ptr::copy_nonoverlapping(
                BACKBUFFER.add(src_offset),
                offscreen.add(dst_offset),
                content_w
            );
        }
    }
}

// Pencere icerigini off-screen bufferdan ekrana geri yukle
pub fn restore_window_content(app_id: u8, x: u16, y: u16, w: u16, h: u16, old_w: u16, old_h: u16) {
    let offset = match app_id {
        0 => 0,
        1 => 900*600,
        2 => 900*600 + 800*500,
        3 => 900*600 + 800*500 + 800*600,
        _ => 0,
    };
    unsafe {
        if BACKBUFFER.is_null() { return; }
        let offscreen = BACKBUFFER.add(1920 * 1080 + offset);
        let copy_w = core::cmp::min(w, old_w).saturating_sub(4) as usize;
        let copy_h = core::cmp::min(h, old_h).saturating_sub(32) as usize;
        
        for i in 0..copy_h {
            let src_offset = i * 1920;
            let dst_offset = ((y + 30 + i as u16) as usize) * 1920 + ((x + 2) as usize);
            core::ptr::copy_nonoverlapping(
                offscreen.add(src_offset),
                BACKBUFFER.add(dst_offset),
                copy_w
            );
        }
    }
}

pub fn draw_background(start_color: u32, end_color: u32) {
    unsafe {
        if BACKBUFFER.is_null() { return; }
        
        let (cx, cy, cw, ch) = match CLIP_RECT {
            Some(r) => r,
            None => (0, 0, VESA.width, VESA.height),
        };
        
        let start_y = core::cmp::min(cy, VESA.height) as usize;
        let end_y = core::cmp::min(cy + ch, VESA.height) as usize;
        let start_x = core::cmp::min(cx, VESA.width) as usize;
        let copy_width = core::cmp::min(cw, VESA.width - cx) as usize;
        let width = VESA.width as usize;
        
        let sr = (start_color >> 16) & 0xFF;
        let sg = (start_color >> 8) & 0xFF;
        let sb = start_color & 0xFF;

        let er = (end_color >> 16) & 0xFF;
        let eg = (end_color >> 8) & 0xFF;
        let eb = end_color & 0xFF;

        for y in start_y..end_y {
            let ratio = (y as u32 * 255) / (VESA.height as u32);
            let inv_ratio = 255 - ratio;
            
            let r = ((er * ratio) + (sr * inv_ratio)) / 255;
            let g = ((eg * ratio) + (sg * inv_ratio)) / 255;
            let b = ((eb * ratio) + (sb * inv_ratio)) / 255;
            
            let color = (r << 16) | (g << 8) | b;
            
            let offset = y * width + start_x;
            let slice = core::slice::from_raw_parts_mut(BACKBUFFER.add(offset), copy_width);
            slice.fill(color);
        }
    }
}

pub fn draw_char(x: u16, y: u16, c: char, fg: u32, bg: u32) {
    if c as usize >= 128 { return; }
    let glyph = crate::font::FONT[c as usize];
    
    unsafe {
        if BACKBUFFER.is_null() { return; }
        let (cx, cy, cw, ch) = match CLIP_RECT {
            Some(r) => r,
            None => (0, 0, VESA.width, VESA.height),
        };
        let cx2 = cx + cw;
        let cy2 = cy + ch;
        
        for (row_idx, &row) in glyph.iter().enumerate() {
            let py = y + row_idx as u16;
            if py < cy || py >= cy2 || py >= VESA.height { continue; }
            for col_idx in 0..8 {
                let px = x + col_idx as u16;
                if px < cx || px >= cx2 || px >= VESA.width { continue; }
                
                let offset = (py as usize) * (VESA.width as usize) + (px as usize);
                let bit_set = (row & (1 << (7 - col_idx))) != 0;
                let color = if bit_set { fg } else { bg };
                
                if bit_set || bg != 0x00000000 {
                    core::ptr::write_volatile(BACKBUFFER.add(offset), color);
                }
            }
        }
    }
}

pub fn draw_string(mut x: u16, y: u16, s: &str, fg: u32, bg: u32) {
    for c in s.chars() {
        draw_char(x, y, c, fg, bg);
        x = x.saturating_add(8);
    }
}

pub struct GuiWriter {
    pub app_id: u8,
    pub offscreen_offset: usize,
    pub win_x: u16,
    pub win_y: u16,
    pub win_w: u16,
    pub win_h: u16,
    pub visible: bool,
    pub minimized: bool,
    pub col: u16,
    pub row: u16,
    pub fg_color: u32,
    pub bg_color: u32,
    pub widgets: alloc::vec::Vec<alloc::boxed::Box<dyn crate::ui::Widget>>,
}

impl GuiWriter {
    pub fn set_color(&mut self, fg: u32, bg: u32) {
        self.fg_color = fg;
        self.bg_color = bg;
    }
    
    pub fn scroll(&mut self) {
        let content_y = self.win_y + 30;
        let content_w = self.win_w - 4;
        let content_h = self.win_h - 32;
        
        unsafe {
            for i in 8..content_h {
                let src_offset = ((content_y + i) as usize) * (VESA.width as usize) + ((self.win_x + 2) as usize);
                let dst_offset = ((content_y + i - 8) as usize) * (VESA.width as usize) + ((self.win_x + 2) as usize);
                
                core::ptr::copy(
                    BACKBUFFER.add(src_offset),
                    BACKBUFFER.add(dst_offset),
                    content_w as usize
                );
            }
        }
        
        // En alt satiri terminal arkaplan rengiyle temizle
        draw_rect(self.win_x + 2, content_y + content_h - 8, content_w, 8, self.bg_color);
        
        self.row -= 8;
    }
    
    pub fn clear(&mut self) {
        let content_y = self.win_y + 30;
        let content_w = self.win_w - 4;
        let content_h = self.win_h - 32;
        
        draw_rect(self.win_x + 2, content_y, content_w, content_h, self.bg_color);
        
        self.col = 0;
        self.row = 0;
    }
}

impl core::fmt::Write for GuiWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if !self.visible {
            return Ok(()); // Eger pencere kapaliysa ekrana yazi cizme (arka planda kaybolsun)
        }
        
        for c in s.chars() {
            if c == '\n' {
                self.col = 0;
                self.row += 8;
            } else if c == '\x08' {
                if self.col >= 8 {
                    self.col -= 8;
                    let px = self.win_x + 6 + self.col;
                    let py = self.win_y + 34 + self.row;
                    draw_rect(px, py, 8, 8, self.bg_color);
                }
            } else if c == '\r' {
                self.col = 0;
            } else {
                let px = self.win_x + 6 + self.col; // 2 margin + 4 padding
                let py = self.win_y + 34 + self.row; // 30 margin + 4 padding
                draw_char(px, py, c, self.fg_color, self.bg_color);
                
                self.col += 8;
                if self.col >= self.win_w - 12 { // 4 margin right
                    self.col = 0;
                    self.row += 8;
                }
            }
            
            // Scroll logic
            if self.row >= self.win_h - 40 {
                self.scroll();
            }
        }
        Ok(())
    }
}

use spin::Mutex;

pub static WRITERS: Mutex<[GuiWriter; 4]> = Mutex::new([
    GuiWriter { app_id: 0, offscreen_offset: 0, win_x: 150, win_y: 150, win_w: 900, win_h: 600, visible: false, minimized: false, col: 0, row: 0, fg_color: 0x00E0E0E0, bg_color: 0x00141414, widgets: alloc::vec::Vec::new() },
    GuiWriter { app_id: 1, offscreen_offset: 0, win_x: 200, win_y: 200, win_w: 800, win_h: 500, visible: false, minimized: false, col: 0, row: 0, fg_color: 0x00E0E0E0, bg_color: 0x00141414, widgets: alloc::vec::Vec::new() },
    GuiWriter { app_id: 2, offscreen_offset: 0, win_x: 250, win_y: 250, win_w: 800, win_h: 600, visible: false, minimized: false, col: 0, row: 0, fg_color: 0x00E0E0E0, bg_color: 0x00141414, widgets: alloc::vec::Vec::new() },
    GuiWriter { app_id: 3, offscreen_offset: 0, win_x: 300, win_y: 300, win_w: 700, win_h: 500, visible: false, minimized: false, col: 0, row: 0, fg_color: 0x00E0E0E0, bg_color: 0x00141414, widgets: alloc::vec::Vec::new() },
]);
pub static Z_ORDER: Mutex<[usize; 4]> = Mutex::new([0, 1, 2, 3]);

pub fn bring_to_front(app_id: usize) {
    let mut z = Z_ORDER.lock();
    let mut pos = 0;
    for i in 0..4 { if z[i] == app_id { pos = i; break; } }
    for i in pos..3 { z[i] = z[i + 1]; }
    z[3] = app_id;
}


pub static mut CURSOR_BG: [u32; 12 * 19] = [0; 12 * 19];

pub fn draw_cursor(x: u16, y: u16) {
    let cursor_map = [
        b"*           ",
        b"**          ",
        b"*.*         ",
        b"*..*        ",
        b"*...*       ",
        b"*....*      ",
        b"*.....*     ",
        b"*......*    ",
        b"*.......*   ",
        b"*........*  ",
        b"*.........* ",
        b"*..........*",
        b"*......*****",
        b"*...*..*    ",
        b"*..* *..*   ",
        b"*.*  *..*   ",
        b"**    *..*  ",
        b"*     *..*  ",
        b"       **   ",
    ];

    unsafe {
        if BACKBUFFER.is_null() { return; }
        
        let mut idx = 0;
        for i in 0..19 {
            for j in 0..12 {
                let py = y + i as u16;
                let px = x + j as u16;
                
                if py >= VESA.height || px >= VESA.width { 
                    idx += 1;
                    continue; 
                }
                
                let offset = (py as usize) * (VESA.width as usize) + (px as usize);
                CURSOR_BG[idx] = core::ptr::read_volatile(BACKBUFFER.add(offset));
                idx += 1;
                
                let c = cursor_map[i as usize][j as usize];
                if c == b'*' {
                    core::ptr::write_volatile(BACKBUFFER.add(offset), 0x00000000); // Siyah dis kenarlik
                } else if c == b'.' {
                    core::ptr::write_volatile(BACKBUFFER.add(offset), 0x00FFFFFF); // Beyaz ic
                }
            }
        }
    }
}

pub fn erase_cursor(x: u16, y: u16) {
    unsafe {
        if BACKBUFFER.is_null() { return; }
        
        let mut idx = 0;
        for i in 0..19 {
            for j in 0..12 {
                let py = y + i as u16;
                let px = x + j as u16;
                
                if py >= VESA.height || px >= VESA.width { 
                    idx += 1;
                    continue; 
                }
                
                let offset = (py as usize) * (VESA.width as usize) + (px as usize);
                core::ptr::write_volatile(BACKBUFFER.add(offset), CURSOR_BG[idx]);
                idx += 1;
            }
        }
    }
}

pub fn update_cursor(old_x: u16, old_y: u16, new_x: u16, new_y: u16) {
    erase_cursor(old_x, old_y);
    draw_cursor(new_x, new_y);
    
    // Yalnizca eski ve yeni imlecin kapladigi alani onbellekten asil ekrana gonder
    flush_rect(old_x, old_y, 12, 19);
    flush_rect(new_x, new_y, 12, 19);
}

use core::sync::atomic::Ordering;

pub struct DragState {
    pub mode: u8, // 0=None, 1=Move, 2=Right, 3=Bottom, 4=BottomRight
    pub start_x: u16,
    pub start_y: u16,
    pub win_start_w: u16,
    pub win_start_h: u16,
    pub app_id: u8,
}
pub static DRAG_STATE: Mutex<DragState> = Mutex::new(DragState { mode: 0, start_x: 0, start_y: 0, win_start_w: 0, win_start_h: 0, app_id: 255 });

pub fn process_mouse_event(cx: u16, cy: u16, click: bool, last_click: bool, moved: bool) {
    if click && !last_click {
        // MOUSE DOWN
        let mut drag = DRAG_STATE.lock();
        let mut writers = WRITERS.lock();
        let mut z_order = Z_ORDER.lock();

        // Taskbar Start Menu Click
        if cx <= 74 && cy >= 1046 {
            let is_open = START_MENU_OPEN.load(Ordering::Relaxed);
            START_MENU_OPEN.store(!is_open, Ordering::Relaxed);
            drop(writers);
            drop(z_order);
            drop(drag);
            redraw_all(None);
            draw_cursor(cx, cy);
            return;
        }

        // Start Menu Buttons
        if START_MENU_OPEN.load(Ordering::Relaxed) {
            if cx >= 4 && cx <= 204 {
                if cy >= 1000 && cy <= 1040 {
                    unsafe { x86_64::instructions::port::PortWriteOnly::<u8>::new(0x64).write(0xFE); }
                }
                if cy >= 950 && cy <= 990 {
                    unsafe { x86_64::instructions::port::PortWriteOnly::<u16>::new(0x604).write(0x2000); }
                }
            }
            START_MENU_OPEN.store(false, Ordering::Relaxed);
            drop(writers);
            drop(z_order);
            drop(drag);
            redraw_all(None);
            draw_cursor(cx, cy);
            return;
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
            ACTIVE_APP.store(hit_app, Ordering::Relaxed);
            let w = &mut writers[hit_app as usize];
            
            // Bring to front logic
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
                redraw_all(None);
                draw_cursor(cx, cy);
            }
            // Minimize
            else if cx >= w.win_x + w.win_w - 74 && cx <= w.win_x + w.win_w - 54 && cy >= w.win_y + 6 && cy <= w.win_y + 26 {
                backup_window_content(w.app_id, w.win_x, w.win_y, w.win_w, w.win_h);
                w.minimized = true;
                drop(writers);
                drop(z_order);
                redraw_all(None);
                draw_cursor(cx, cy);
            }
            // Maximize/Restore
            else if cx >= w.win_x + w.win_w - 50 && cx <= w.win_x + w.win_w - 30 && cy >= w.win_y + 6 && cy <= w.win_y + 26 {
                backup_window_content(w.app_id, w.win_x, w.win_y, w.win_w, w.win_h);
                if w.win_w < 1920 {
                    w.win_x = 0; w.win_y = 0; w.win_w = 1920; w.win_h = 1046;
                } else {
                    w.win_x = 100; w.win_y = 100; w.win_w = 800; w.win_h = 500;
                }
                drop(writers);
                drop(z_order);
                redraw_all(None);
                draw_cursor(cx, cy);
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
                let mut handled = false;
                for widget in &mut w.widgets {
                    if widget.handle_event(crate::ui::UiEvent::MouseClick { x: cx, y: cy }, w.win_x, w.win_y) {
                        handled = true;
                        break;
                    }
                }
                
                drop(writers);
                drop(z_order);
                if !handled {
                    redraw_all(None);
                }
                draw_cursor(cx, cy);
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
                    ACTIVE_APP.store(app_id, Ordering::Relaxed);
                    writers[app_id as usize].visible = true;
                    writers[app_id as usize].minimized = false;
                    if app_id != 0 {
                        writers[app_id as usize].col = 0;
                        writers[app_id as usize].row = 0;
                    }
                    
                    let mut pos = 0;
                    for i in 0..4 { if z_order[i] == app_id as usize { pos = i; break; } }
                    for i in pos..3 { z_order[i] = z_order[i + 1]; }
                    z_order[3] = app_id as usize;
                    
                    drop(writers);
                    drop(z_order);
                    redraw_all(None);
                    draw_cursor(cx, cy);
                    return;
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
                    ACTIVE_APP.store(z_order[3] as u8, Ordering::Relaxed);
                    drop(writers);
                    drop(z_order);
                    redraw_all(None);
                    draw_cursor(cx, cy);
                }
            }
        }
    } else if !click && last_click {
        // Mouse UP
        let mut writers = WRITERS.lock();
        let active_app = ACTIVE_APP.load(Ordering::Relaxed);
        if active_app < 4 {
            let w = &mut writers[active_app as usize];
            for widget in &mut w.widgets {
                widget.handle_event(crate::ui::UiEvent::MouseUp { x: cx, y: cy }, w.win_x, w.win_y);
            }
        }
        drop(writers);
        
        let mut drag = DRAG_STATE.lock();
        drag.mode = 0;
    } else if click && last_click && moved {
        let drag = DRAG_STATE.lock();
        if drag.mode != 0 {
            let mut writers = WRITERS.lock();
            let w = &mut writers[drag.app_id as usize];
            
            let old_x = w.win_x;
            let old_y = w.win_y;
            let old_w = w.win_w;
            let old_h = w.win_h;
            
            backup_window_content(w.app_id, w.win_x, w.win_y, w.win_w, w.win_h);
            
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
            
            let dirty = union_rect(old_x, old_y, old_w, old_h, w.win_x, w.win_y, w.win_w, w.win_h);
            
            drop(writers);
            redraw_all(Some(dirty));
            draw_cursor(cx, cy);
        }
    }
}
