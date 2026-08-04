use x86_64::instructions::port::Port;

pub struct Vesa {
    pub width: u16,
    pub height: u16,
    pub framebuffer: *mut u32,
}

pub static mut VESA: Vesa = Vesa {
    width: 1920,
    height: 1080,
    framebuffer: core::ptr::null_mut(),
};

pub static mut PHYS_OFFSET: u64 = 0;
pub static mut BACKBUFFER: *mut u32 = core::ptr::null_mut();
pub static ACTIVE_APP: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
pub static START_MENU_OPEN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub fn init() {
    let mut index_port: Port<u16> = Port::new(0x01CE);
    let mut data_port: Port<u16> = Port::new(0x01CF);
    
    unsafe {
        // VBE'yi devre dışı bırak
        index_port.write(4);
        data_port.write(0);
        
        // Genişlik = 1280
        index_port.write(1);
        data_port.write(1920);
        
        // Yükseklik = 720
        index_port.write(2);
        data_port.write(1080);
        
        // Renk Derinliği = 32 BPP (Bits Per Pixel)
        index_port.write(3);
        data_port.write(32);
        
        // VBE'yi Etkinleştir (0x01) ve Linear Framebuffer'ı (0x40) aç
        index_port.write(4);
        data_port.write(0x01 | 0x40);
        
        // QEMU (Bochs) VBE LFB adresi genelde 0xFD000000'dır.
        VESA.framebuffer = (PHYS_OFFSET + 0xFD000000) as *mut u32;
        // BACKBUFFER icin 2 ekranlik (16 MB) yer ayiriyoruz (ilk yarisi cizim, ikinci yarisi off-screen cache)
        let mut buf = alloc::vec::Vec::<u32>::with_capacity(1920 * 1080 * 2);
        buf.resize(1920 * 1080 * 2, 0);
        BACKBUFFER = buf.as_mut_ptr();
        core::mem::forget(buf);
    }
}

pub fn swap_buffers() {
    unsafe {
        if BACKBUFFER.is_null() || VESA.framebuffer.is_null() { return; }
        core::ptr::copy_nonoverlapping(BACKBUFFER, VESA.framebuffer, 1920 * 1080);
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


pub fn draw_rect(x: u16, y: u16, w: u16, h: u16, color: u32) {
    unsafe {
        if BACKBUFFER.is_null() { return; }
        
        let start_y = core::cmp::min(y, VESA.height);
        let end_y = core::cmp::min(y + h, VESA.height);
        let start_x = core::cmp::min(x, VESA.width);
        let copy_width = core::cmp::min(w, VESA.width - start_x);
        
        if copy_width == 0 { return; }
        
        for row in start_y..end_y {
            let offset = (row as usize) * (VESA.width as usize) + (start_x as usize);
            let slice = core::slice::from_raw_parts_mut(BACKBUFFER.add(offset), copy_width as usize);
            slice.fill(color);
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
    // Window Gölgesi (Basit bir siyah katman sağ alta)
    draw_rect(x + 5, y + 5, w, h, 0x000A0A0A);
    
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

pub fn draw_desktop(terminal_visible: bool, terminal_minimized: bool) {
    // Premium Koyu Okyanus Arka Plan
    draw_background(0x001A2421); // Koyu yesil/lacivert karisimi
    
    // Masaustu Ikonlari
    draw_icon(20, 20, "Terminal");
    draw_icon(20, 80, "Files");
    draw_icon(20, 140, "Notepad");
    draw_icon(20, 200, "TaskMgr");
    
    // Alt Gorev Cubugu (Taskbar) Modern Koyu Gri
    draw_rect(0, 1080 - 34, 1920, 34, 0x002D2D2D);
    // Taskbar Ust Ince Cizgisi
    draw_rect(0, 1080 - 34, 1920, 1, 0x004A4A4A);
    
    // Start Butonu (Modern Kutu)
    draw_rect(4, 1080 - 30, 70, 26, 0x003A3A3A);
    let start_text = "Start";
    let mut px = 20;
    for c in start_text.chars() {
        draw_char(px, 1080 - 21, c, 0x00E0E0E0, 0x003A3A3A);
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
    // Menu Kutusu: Modern Koyu Gri
    draw_rect(4, 970, 150, 76, 0x00333333);
    draw_rect(4, 970, 150, 1, 0x00555555); // border top
    draw_rect(4, 970, 1, 76, 0x00555555); // border left
    
    // Restart Seçeneği
    let mut px = 12;
    for c in "Restart".chars() {
        draw_char(px, 985, c, 0x00E0E0E0, 0x00333333);
        px += 8;
    }
    
    // Shutdown Seçeneği
    let mut px = 12;
    for c in "Shutdown".chars() {
        draw_char(px, 1025, c, 0x00E0E0E0, 0x00333333);
        px += 8;
    }
}


pub fn draw_files_ui(x: u16, y: u16, w: u16, h: u16) {
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
    draw_rect(x + 2, y + 30, w.saturating_sub(4), 24, 0x00E0E0E0);
    let mut px = x + 10;
    for c in "File   Edit   View".chars() {
        draw_char(px, y + 38, c, 0x00000000, 0x00E0E0E0);
        px += 8;
    }
    // Some text
    let text = "Hello from SparkOS Notepad!";
    px = x + 10;
    for c in text.chars() {
        draw_char(px, y + 70, c, 0x00000000, 0x00F0F0F0);
        px += 8;
    }
}

pub fn draw_taskmgr_ui(x: u16, y: u16, w: u16, h: u16) {
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

pub fn redraw_all() {
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
    
    flush_rect(0, 0, 1920, 1080);
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

pub fn draw_background(color: u32) {
    unsafe {
        if BACKBUFFER.is_null() { return; }
        let slice = core::slice::from_raw_parts_mut(BACKBUFFER, (VESA.width as usize) * (VESA.height as usize));
        slice.fill(color);
    }
}

pub fn draw_char(x: u16, y: u16, c: char, fg: u32, bg: u32) {
    if c as usize >= 128 { return; }
    let glyph = crate::font::FONT[c as usize];
    
    unsafe {
        if BACKBUFFER.is_null() { return; }
        for (row_idx, &row) in glyph.iter().enumerate() {
            let py = y + row_idx as u16;
            for col_idx in 0..8 {
                let px = x + col_idx as u16;
                if py >= VESA.height || px >= VESA.width { continue; }
                
                let offset = (py as usize) * (VESA.width as usize) + (px as usize);
                let bit_set = (row & (1 << col_idx)) != 0;
                let color = if bit_set { fg } else { bg };
                
                if bit_set || bg != 0x00000000 { // Don't draw background if it's transparent (hacky)
                    core::ptr::write_volatile(BACKBUFFER.add(offset), color);
                }
            }
        }
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
    GuiWriter { app_id: 0, offscreen_offset: 0, win_x: 150, win_y: 150, win_w: 900, win_h: 600, visible: false, minimized: false, col: 0, row: 0, fg_color: 0x00E0E0E0, bg_color: 0x00141414 },
    GuiWriter { app_id: 1, offscreen_offset: 0, win_x: 200, win_y: 200, win_w: 800, win_h: 500, visible: false, minimized: false, col: 0, row: 0, fg_color: 0x00E0E0E0, bg_color: 0x00141414 },
    GuiWriter { app_id: 2, offscreen_offset: 0, win_x: 250, win_y: 250, win_w: 800, win_h: 600, visible: false, minimized: false, col: 0, row: 0, fg_color: 0x00E0E0E0, bg_color: 0x00141414 },
    GuiWriter { app_id: 3, offscreen_offset: 0, win_x: 300, win_y: 300, win_w: 700, win_h: 500, visible: false, minimized: false, col: 0, row: 0, fg_color: 0x00E0E0E0, bg_color: 0x00141414 },
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
