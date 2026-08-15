//! Display / framebuffer sürücü sarmalayıcı (L7).
//!
//! `crate::gui` (VBE/VESA framebuffer, backbuffer, pencere sistemi) mevcut ve
//! çalışıyor; bu modül ONU DEĞİŞTİRMEDEN üzerine inşa eder:
//!   - `DisplayInfo` / `DisplayMode` yapıları (CRT bilgisi)
//!   - `gui::VESA` durumunun okunması (çözünürlük, framebuffer adresleri)
//!   - VBE (Bochs) index/data portları üzerinden resolution değiştirme iskeleti
//!   - `swap_buffers`, `flush_rect`, temizleme gibi gui çağrılarına kolay sarmalayıcılar
//!
//! NOT: Çözünürlük değişikliği gerçek bir masaüstünde backbuffer tahsisini ve
//! GUI pencere boyutlarını da güncellemeyi gerektirir; burada yalnızca VBE
//! donanım adımı ve VESA bilgisinin güncellenmesi yapılır (iskelet).

use core::sync::atomic::Ordering;

use alloc::string::String;
use alloc::format;

use crate::gui;
use crate::vga_buffer::GUI_MODE;

/// Varsayılan (boot) çözünürlük — gui.rs'in `init`'te kurduğu değerler.
pub const DEFAULT_WIDTH: u16 = 640;
pub const DEFAULT_HEIGHT: u16 = 360;
pub const DEFAULT_BPP: u8 = 32;

/// VBE (Bochs) index/data port çifti.
const VBE_DISPI_INDEX_PORT: u16 = 0x01CE;
const VBE_DISPI_DATA_PORT: u16 = 0x01CF;
/// VBE enabler: Enable (0x01) + LFB (0x40).
const VBE_ENABLED_LFB: u16 = 0x01 | 0x40;
/// QEMU/Bochs LFB fiziksel adresi.
const VBE_LFB_PHYS: u64 = 0xFD00_0000;

/// Bir ekran modu (çözünürlük + renk derinliği).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayMode {
    pub width: u16,
    pub height: u16,
    pub bpp: u8,
}

impl DisplayMode {
    pub const fn new(width: u16, height: u16, bpp: u8) -> Self {
        DisplayMode { width, height, bpp }
    }

    /// Piksel başına byte (32bpp için 4).
    pub const fn bytes_per_pixel(&self) -> u8 {
        (self.bpp + 7) / 8
    }
}

/// Mevcut ekran durumunun okunabilir özeti (CRT/info).
#[derive(Clone, Copy, Debug)]
pub struct DisplayInfo {
    pub width: u16,
    pub height: u16,
    pub bpp: u8,
    /// Framebuffer'ın sanal adresi (kernel identity/offset ile haritalanmış).
    pub framebuffer: *mut u32,
    /// Framebuffer'ın fiziksel adresi (`framebuffer - PHYS_OFFSET`).
    pub phys_framebuffer: u64,
    /// Çift tampon (backbuffer) varsa sanal adresi.
    pub backbuffer: *mut u32,
    /// Çift tamponlama aktif mi?
    pub double_buffered: bool,
    /// Satır içi (stride/pitch) byte cinsinden.
    pub stride: usize,
}

impl DisplayInfo {
    /// Toplam framebuffer boyutu (byte).
    pub fn framebuffer_size(&self) -> usize {
        self.stride * self.height as usize
    }

    /// Kısa tanım: `1920x1080 @32bpp stride=7680 double_buffered=true`.
    pub fn describe(&self) -> String {
        format!(
            "{}x{} @{}bpp stride={} fb={:#x} dblbuf={}",
            self.width,
            self.height,
            self.bpp,
            self.stride,
            self.framebuffer as usize,
            self.double_buffered
        )
    }
}

/// Mevcut `gui::VESA` durumundan `DisplayInfo` üretir.
pub fn current_info() -> DisplayInfo {
    unsafe {
        let fb = gui::VESA.framebuffer;
        DisplayInfo {
            width: gui::VESA.width,
            height: gui::VESA.height,
            bpp: DEFAULT_BPP,
            framebuffer: fb,
            phys_framebuffer: if fb.is_null() {
                0
            } else {
                (fb as u64).wrapping_sub(gui::PHYS_OFFSET)
            },
            backbuffer: gui::BACKBUFFER,
            double_buffered: !gui::BACKBUFFER.is_null(),
            stride: gui::VESA.width as usize * 4,
        }
    }
}

/// GUI modu aktif mi? (`vga_buffer::GUI_MODE`)
pub fn is_gui_active() -> bool {
    GUI_MODE.load(Ordering::Relaxed)
}

/// VBE register'ları üzerinden çözünürlük/renk derinliği kurar.
///
/// Bu fonksiyon yalnızca VBE donanım adımını yapar ve `gui::VESA` bilgisini
/// günceller. Backbuffer boyutu / pencere geometrisi güncellenmesi üst katmanın
/// işidir (genelde masaüstünü sıfırdan çizmek gerekir).
pub fn set_mode(mode: DisplayMode) -> Result<(), &'static str> {
    if mode.width == 0 || mode.height == 0 || mode.bpp < 8 {
        return Err("invalid display mode");
    }

    unsafe {
        use x86_64::instructions::port::Port;
        let mut idx: Port<u16> = Port::new(VBE_DISPI_INDEX_PORT);
        let mut dat: Port<u16> = Port::new(VBE_DISPI_DATA_PORT);

        // VBE'yi devre dışı bırak (güvenli geçiş)
        idx.write(4);
        dat.write(0);

        // Genişlik / Yükseklik / Renk derinliği
        idx.write(1);
        dat.write(mode.width);
        idx.write(2);
        dat.write(mode.height);
        idx.write(3);
        dat.write(mode.bpp as u16);

        // Enable + Linear Framebuffer
        idx.write(4);
        dat.write(VBE_ENABLED_LFB);

        // QEMU (Bochs) VBE LFB adresi sabittir; PHYS_OFFSET ile sanala çevrilir.
        gui::VESA.width = mode.width;
        gui::VESA.height = mode.height;
        gui::VESA.framebuffer = (gui::PHYS_OFFSET + VBE_LFB_PHYS) as *mut u32;
    }
    Ok(())
}

/// Geçerli modu (gui::VESA'dan) döndürür.
pub fn current_mode() -> DisplayMode {
    unsafe {
        DisplayMode::new(gui::VESA.width, gui::VESA.height, DEFAULT_BPP)
    }
}

// ---------------------------------------------------------------------------
// gui.rs sarmalayıcıları
// ---------------------------------------------------------------------------

/// Backbuffer'ı ekrana kopyalar (çift tampon swap).
pub fn swap_buffers() {
    gui::swap_buffers();
}

/// Belirli bir dikdörtgeni backbuffer'dan ekrana kopyalar.
pub fn flush_rect(x: u16, y: u16, w: u16, h: u16) {
    gui::flush_rect(x, y, w, h);
}

/// Ekranı düz renkle temizler ve flush eder.
pub fn clear_screen(color: u32) {
    let info = current_info();
    gui::draw_rect(0, 0, info.width, info.height, color);
    gui::flush_rect(0, 0, info.width, info.height);
}

/// Backbuffer'a tek piksel yazar (clip uygulanır).
pub fn put_pixel(x: u16, y: u16, color: u32) {
    gui::draw_rect(x, y, 1, 1, color);
}

/// Sıradan bir dikdörtgen çizer.
pub fn fill_rect(x: u16, y: u16, w: u16, h: u16, color: u32) {
    gui::draw_rect(x, y, w, h, color);
}

// ---------------------------------------------------------------------------
// Üst düzey Display tutamacı
// ---------------------------------------------------------------------------

/// Görüntüleme donanımının durumunu tutan üst düzey sarmalayıcı.
pub struct Display {
    pub info: DisplayInfo,
}

impl Display {
    pub fn new() -> Self {
        Display {
            info: current_info(),
        }
    }

    /// Donanım durumunu yeniden okur (çözünürlük değişmiş olabilir).
    pub fn refresh(&mut self) {
        self.info = current_info();
    }

    /// Backbuffer'ı temizleyip ekrana taşır.
    pub fn clear(&self, color: u32) {
        clear_screen(color);
    }

    /// Çözünürlük değiştirir ve iç durumu tazeler.
    pub fn set_resolution(&mut self, mode: DisplayMode) -> Result<(), &'static str> {
        set_mode(mode)?;
        self.refresh();
        Ok(())
    }
}

impl core::fmt::Display for DisplayMode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}x{} @{}bpp", self.width, self.height, self.bpp)
    }
}
