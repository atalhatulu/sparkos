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

pub static mut CLIP_RECT: Option<(u16, u16, u16, u16)> = None;

pub fn set_clip(rect: Option<(u16, u16, u16, u16)>) {
    unsafe { CLIP_RECT = rect; }
}

pub fn union_rect(x1: u16, y1: u16, w1: u16, h1: u16, x2: u16, y2: u16, w2: u16, h2: u16) -> (u16, u16, u16, u16) {
    let ix1 = x1.min(x2);
    let iy1 = y1.min(y2);
    let ix2 = (x1 + w1).max(x2 + w2);
    let iy2 = (y1 + h1).max(y2 + h2);
    let bx1 = ix1.saturating_sub(10);
    let by1 = iy1.saturating_sub(10);
    let bx2 = ix2 + 15;
    let by2 = iy2 + 15;
    (bx1, by1, bx2 - bx1, by2 - by1)
}

pub fn intersect_rect(x1: u16, y1: u16, w1: u16, h1: u16, x2: u16, y2: u16, w2: u16, h2: u16) -> Option<(u16, u16, u16, u16)> {
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
        if let Some((cx, cy, cw, ch)) = CLIP_RECT {
            if x < cx || x >= cx + cw || y < cy || y >= cy + ch {
                return;
            }
        }
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
        (0x00404040, 0x00FFFFFF)
    } else {
        (0x00FFFFFF, 0x00404040)
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
                let bit_set = (row & (1 << col_idx)) != 0;
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

pub fn draw_icon_glyph(x: u16, y: u16, icon: crate::app_registry::AppIcon, _fg: u32, bg: u32) {
    crate::icons::render_icon_16(x, y, icon, bg);
}
