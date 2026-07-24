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

pub fn init() {
    let mut index_port: Port<u16> = Port::new(0x01CE);
    let mut data_port: Port<u16> = Port::new(0x01CF);
    
    unsafe {
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
    }
}

pub fn draw_rect(x: u16, y: u16, w: u16, h: u16, color: u32) {
    unsafe {
        if VESA.framebuffer.is_null() { return; }
        for i in y..(y + h) {
            for j in x..(x + w) {
                if i >= VESA.height || j >= VESA.width { continue; }
                let offset = (i as usize) * (VESA.width as usize) + (j as usize);
                core::ptr::write_volatile(VESA.framebuffer.add(offset), color);
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
    // 3D Dis Kenarlik (Gri)
    draw_3d_rect(x, y, w, h, 0x00C0C0C0, false);
    
    // Baslik cubugu (Lacivert / Modern Blue Gradient-ish)
    draw_rect(x + 4, y + 4, w - 8, 22, 0x000000AA);
    
    // Baslik metni (Ortalanmis veya soldan)
    let mut px = x + 8;
    for c in title.chars() {
        draw_char(px, y + 11, c, 0x00FFFFFF, 0x000000AA);
        px += 8;
    }
    
    // Window Butonlari (X, O, _) 3D efektli
    // Minimize (_)
    draw_3d_rect(x + w - 64, y + 6, 16, 16, 0x00C0C0C0, false);
    draw_char(x + w - 60, y + 10, '_', 0x00000000, 0x00C0C0C0);
    
    // Maximize (O)
    draw_3d_rect(x + w - 44, y + 6, 16, 16, 0x00C0C0C0, false);
    draw_char(x + w - 40, y + 10, 'O', 0x00000000, 0x00C0C0C0);
    
    // Close (X)
    draw_3d_rect(x + w - 24, y + 6, 16, 16, 0x00C0C0C0, false);
    draw_char(x + w - 20, y + 10, 'X', 0x00000000, 0x00C0C0C0);
    
    // Icerik alani: Ubuntu Tarzi Koyu Mor Veya Modern Koyu Gri
    // Renk: 0x001E1E1E (Koyu Gri - VSCode tarzi)
    draw_rect(x + 4, y + 30, w - 8, h - 34, 0x001E1E1E);
}

pub fn draw_icon(x: u16, y: u16, text: &str) {
    // Terminal Icon arkaplani (Beyaz Kutu)
    draw_3d_rect(x, y, 40, 40, 0x00FFFFFF, false);
    // Ust Baslik (Koyu Mavi)
    draw_rect(x + 2, y + 2, 36, 8, 0x000000AA);
    // Ic Siyah Alan
    draw_rect(x + 4, y + 12, 32, 24, 0x001E1E1E);
    // Yesil >
    draw_char(x + 6, y + 16, '>', 0x0000FF00, 0x001E1E1E);
    
    // Altina Isim Yazisi
    let mut px = x as i32 + 20 - ((text.len() * 8) as i32) / 2;
    for c in text.chars() {
        if px >= 0 {
            draw_char(px as u16, y + 44, c, 0x00FFFFFF, 0x00008080);
        }
        px += 8;
    }
}

pub fn draw_desktop() {
    // Windows 95 klasigi "Teal" Arka Plan
    draw_background(0x00008080);
    
    // Masaustu Ikonlari
    draw_icon(20, 20, "Terminal");
    
    // Alt Gorev Cubugu (Taskbar) 3D efektli
    draw_3d_rect(0, 720 - 34, 1280, 34, 0x00C0C0C0, false);
    
    // Start Butonu (3D Efektli)
    draw_3d_rect(4, 720 - 30, 70, 26, 0x00C0C0C0, false);
    let start_text = "Start";
    let mut px = 20;
    for c in start_text.chars() {
        draw_char(px, 720 - 20, c, 0x00000000, 0x00C0C0C0);
        px += 8;
    }
    
    // Sag alt koseye Saat / Logo alani (3D iceri cokuk efekt)
    draw_3d_rect(1280 - 100, 720 - 30, 96, 26, 0x00C0C0C0, true);
    let logo_text = "SparkOS";
    let mut px = 1280 - 85;
    for c in logo_text.chars() {
        draw_char(px, 720 - 20, c, 0x00000000, 0x00C0C0C0);
        px += 8;
    }
}

pub fn draw_desktop_and_window(win_x: u16, win_y: u16, win_w: u16, win_h: u16, visible: bool) {
    draw_desktop();
    if visible {
        draw_window(win_x, win_y, win_w, win_h, "SparkOS Terminal");
    }
}

pub fn draw_background(color: u32) {
    unsafe {
        if VESA.framebuffer.is_null() { return; }
        for i in 0..(VESA.width as usize * VESA.height as usize) {
            core::ptr::write_volatile(VESA.framebuffer.add(i), color);
        }
    }
}

pub fn draw_char(x: u16, y: u16, c: char, fg: u32, bg: u32) {
    if c as usize >= 128 { return; }
    let glyph = crate::font::FONT[c as usize];
    
    unsafe {
        if VESA.framebuffer.is_null() { return; }
        for (row_idx, &row) in glyph.iter().enumerate() {
            let py = y + row_idx as u16;
            for col_idx in 0..8 {
                let px = x + col_idx as u16;
                if py >= VESA.height || px >= VESA.width { continue; }
                
                let offset = (py as usize) * (VESA.width as usize) + (px as usize);
                let bit_set = (row & (1 << col_idx)) != 0;
                let color = if bit_set { fg } else { bg };
                
                if bit_set || bg != 0x00000000 { // Don't draw background if it's transparent (hacky)
                    core::ptr::write_volatile(VESA.framebuffer.add(offset), color);
                }
            }
        }
    }
}

pub struct GuiWriter {
    pub win_x: u16,
    pub win_y: u16,
    pub win_w: u16,
    pub win_h: u16,
    pub visible: bool,
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
        let content_y = self.win_y + 32;
        let content_w = self.win_w - 8;
        let content_h = self.win_h - 36;
        
        unsafe {
            for i in 8..content_h {
                for j in 0..content_w {
                    let src_offset = ((content_y + i) as usize) * (VESA.width as usize) + ((self.win_x + 4 + j) as usize);
                    let dst_offset = ((content_y + i - 8) as usize) * (VESA.width as usize) + ((self.win_x + 4 + j) as usize);
                    let px = core::ptr::read_volatile(VESA.framebuffer.add(src_offset));
                    core::ptr::write_volatile(VESA.framebuffer.add(dst_offset), px);
                }
            }
        }
        
        // En alt satiri terminal arkaplan rengiyle temizle
        draw_rect(self.win_x + 4, content_y + content_h - 8, content_w, 8, self.bg_color);
        
        self.row -= 8;
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
            } else {
                let px = self.win_x + 8 + self.col; // 8 margin left
                let py = self.win_y + 34 + self.row; // 34 margin top
                draw_char(px, py, c, self.fg_color, self.bg_color);
                self.col += 8;
                if self.col >= self.win_w - 24 { // margin right
                    self.col = 0;
                    self.row += 8;
                }
            }
            
            // Scroll logic (h-34 icerik alani y - 8 margin bottom = 42 margin)
            if self.row >= self.win_h - 42 {
                self.scroll();
            }
        }
        Ok(())
    }
}

use spin::Mutex;

pub static WRITER: Mutex<GuiWriter> = Mutex::new(GuiWriter {
    win_x: 100,
    win_y: 100,
    win_w: 800,
    win_h: 500,
    visible: false,
    col: 0,
    row: 0,
    fg_color: 0x0000FF00, // Matrix Yesili veya modern Yesil
    bg_color: 0x001E1E1E, // Terminal arkaplan rengi (Koyu Gri)
});

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
        if VESA.framebuffer.is_null() { return; }
        
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
                CURSOR_BG[idx] = core::ptr::read_volatile(VESA.framebuffer.add(offset));
                idx += 1;
                
                let c = cursor_map[i as usize][j as usize];
                if c == b'*' {
                    core::ptr::write_volatile(VESA.framebuffer.add(offset), 0x00000000); // Siyah dis kenarlik
                } else if c == b'.' {
                    core::ptr::write_volatile(VESA.framebuffer.add(offset), 0x00FFFFFF); // Beyaz ic
                }
            }
        }
    }
}

pub fn erase_cursor(x: u16, y: u16) {
    unsafe {
        if VESA.framebuffer.is_null() { return; }
        
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
                core::ptr::write_volatile(VESA.framebuffer.add(offset), CURSOR_BG[idx]);
                idx += 1;
            }
        }
    }
}

pub fn update_cursor(old_x: u16, old_y: u16, new_x: u16, new_y: u16) {
    erase_cursor(old_x, old_y);
    draw_cursor(new_x, new_y);
}
