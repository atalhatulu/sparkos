use core::fmt::Write;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use crate::fs;
use crate::task::keyboard;

pub struct Color;
impl Color {
    pub const Black: u32 = 0x00000000;
    pub const White: u32 = 0x00FFFFFF;
    pub const Red: u32 = 0x00FF0000;
    pub const Green: u32 = 0x0000FF00;
    pub const LightBlue: u32 = 0x0000AAFF;
    pub const Yellow: u32 = 0x00FFFF00;
    pub const Cyan: u32 = 0x0000FFFF;
    pub const Magenta: u32 = 0x00FF00FF;
}

const CMD_BUF: usize = 256;

pub struct Shell {
    buf: [u8; CMD_BUF],
    len: usize,
    text_color: u32,
    cwd: String,
}

impl Shell {
    pub fn new() -> Self {
        Shell { buf: [0; CMD_BUF], len: 0, text_color: Color::White, cwd: "/".to_string() }
    }

    pub async fn run(&mut self) {
        loop {
            self.prompt();
            self.read_line().await;
            self.exec();
        }
    }

    pub fn prompt(&self) {
        let mut w = crate::gui::WRITER.lock();
        w.set_color(Color::Green, Color::Black);
        core::fmt::Write::write_str(&mut *w, "sparkos ").unwrap();
        w.set_color(self.text_color, Color::Black);
        core::fmt::Write::write_str(&mut *w, "> ").unwrap();
    }

    async fn read_line(&mut self) {
        self.len = 0;
        loop {
            use crate::keyboard::Key;
            while let Some(key) = crate::keyboard::read_key() {
                match key {
                    Key::Ascii(c) => {
                        if self.len < CMD_BUF {
                            self.buf[self.len] = c as u8;
                            self.len += 1;
                            let mut w = crate::gui::WRITER.lock();
                            core::fmt::Write::write_char(&mut *w, c as char).unwrap();
                        }
                    }
                    Key::Backspace => {
                        if self.len > 0 {
                            self.len -= 1;
                            let mut w = crate::gui::WRITER.lock();
                            if w.col >= 8 {
                                w.col -= 8;
                                let px = w.win_x + 4 + w.col;
                                let py = w.win_y + 28 + w.row;
                                crate::gui::draw_rect(px, py, 8, 8, w.bg_color); // Tamamen sil
                            } else if w.row >= 8 {
                                w.row -= 8;
                                w.col = w.win_w - 16;
                                let px = w.win_x + 4 + w.col;
                                let py = w.win_y + 28 + w.row;
                                crate::gui::draw_rect(px, py, 8, 8, w.bg_color); // Tamamen sil
                            }
                        }
                    }
                    Key::Enter => {
                        let mut w = crate::gui::WRITER.lock();
                        core::fmt::Write::write_str(&mut *w, "\n").unwrap();
                        return;
                    }
                    Key::Delete => {}
                    Key::Left | Key::Right | Key::Home | Key::End => {}
                    Key::Escape => {
                        let mut w = crate::gui::WRITER.lock();
                        for _ in 0..self.len {
                            if w.col >= 8 {
                                w.col -= 8;
                                crate::gui::draw_char(w.col, w.row, ' ', w.fg_color, w.bg_color);
                            }
                        }
                        self.len = 0;
                    }
                    _ => {}
                }
            }
            let scancode = crate::task::keyboard::read_scancode().await;
            crate::keyboard::KEYBOARD.lock().handle_scancode(scancode);
        }
    }

    fn cmd(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    fn exec(&mut self) {
        let cmd = self.cmd().trim();
        let mut w = crate::gui::WRITER.lock();
        match cmd {
            "" => {}
            "help" | "yardim" => {
                w.set_color(Color::Yellow, Color::Black);
                writeln!(w, "Komutlar:").unwrap();
                writeln!(w, "  help     - bu liste").unwrap();
                writeln!(w, "  clear    - ekrani temizle").unwrap();
                writeln!(w, "  info     - sistem bilgisi").unwrap();
                writeln!(w, "  tick     - timer sayaci").unwrap();
                writeln!(w, "  uptime   - sistemin calisma suresi").unwrap();
                writeln!(w, "  color    - yazi rengini degistir").unwrap();
                writeln!(w, "  echo     - mesaj bas").unwrap();
                writeln!(w, "  pwd      - gecerli dizini goster").unwrap();
                writeln!(w, "  cd       - dizin degistir").unwrap();
                writeln!(w, "  ls       - dosyalari ve dizinleri listele").unwrap();
                writeln!(w, "  mkdir    - yeni bir dizin olustur").unwrap();
                writeln!(w, "  write    - yeni bir dosya olustur veya icerigini degistir").unwrap();
                writeln!(w, "  cat      - dosyanin icerigini oku").unwrap();
                writeln!(w, "  disk_write - diskin belirli sektorune yaz").unwrap();
                writeln!(w, "  disk_read  - diskin belirli sektorunu oku").unwrap();
                writeln!(w, "  gui      - piksellerle masaustu (GUI) moduna gec").unwrap();
                writeln!(w, "  reboot   - sistemi yeniden baslat").unwrap();
                writeln!(w, "  shutdown - sistemi kapat (QEMU)").unwrap();
                writeln!(w, "  panic    - kernel panic testi (sistemi dondurur)").unwrap();
            }
            "clear" => {
                crate::gui::draw_rect(w.win_x + 4, w.win_y + 28, w.win_w - 8, w.win_h - 32, w.bg_color);
                w.col = 0;
                w.row = 0;
                return;
            }
            "info" => {
                writeln!(w, "SparkOS v0.1 - Rust x86_64").unwrap();
                writeln!(w, "Bellek: 249 MB").unwrap();
                writeln!(w, "Timer: 1000 Hz").unwrap();
            }
            "tick" => {
                writeln!(w, "Tick: {}", crate::interrupts::get_tick()).unwrap();
            }
            "uptime" => {
                let ticks = crate::interrupts::get_tick();
                let seconds = ticks / 1000;
                writeln!(w, "Uptime: {} saniye ({} ms)", seconds, ticks).unwrap();
            }
            "panic" => {
                drop(w);
                panic!("Kullanici panik testi tetikledi!");
            }
            "reboot" => {
                core::fmt::Write::write_str(&mut *w, "Yeniden baslatiliyor...\n").unwrap();
                unsafe {
                    let mut p: x86_64::instructions::port::Port<u8> = x86_64::instructions::port::Port::new(0x2000u16);
                    p.write(0x04u8);
                }
            }
            "shutdown" | "kapat" => {
                writeln!(w, "Sistem kapatiliyor...").unwrap();
                crate::serial_println!("[shell] shutdown");
                unsafe {
                    // QEMU/Bochs ACPI kapatma portu
                    let mut p: x86_64::instructions::port::Port<u16> =
                        x86_64::instructions::port::Port::new(0xB004);
                    p.write(0x2000);
                }
                // Port kapanmazsa diye bekle
                loop { x86_64::instructions::hlt(); }
            }
            "pwd" => {
                writeln!(w, "{}", self.cwd).unwrap();
            }
            _ if cmd.starts_with("cd ") => {
                let path = &cmd[3..].trim();
                let resolved = crate::fs::resolve_path(&self.cwd, path);
                if crate::fs::is_dir(&resolved) {
                    self.cwd = resolved;
                } else {
                    w.set_color(Color::Red, Color::Black);
                    writeln!(w, "Hata: {} bir dizin degil veya bulunamadi", path).unwrap();
                    w.set_color(self.text_color, Color::Black);
                }
            }
            _ if cmd == "ls" || cmd.starts_with("ls ") => {
                let target = if cmd.len() > 2 { &cmd[3..].trim() } else { "" };
                let resolved = crate::fs::resolve_path(&self.cwd, target);
                match crate::fs::list_dir(&resolved) {
                    Ok(items) => {
                        if items.is_empty() {
                            writeln!(w, "(Bos)").unwrap();
                        } else {
                            for (name, is_dir) in items {
                                if is_dir {
                                    w.set_color(Color::LightBlue, Color::Black);
                                    write!(w, "{}/  ", name).unwrap();
                                } else {
                                    w.set_color(Color::White, Color::Black);
                                    write!(w, "{}  ", name).unwrap();
                                }
                            }
                            writeln!(w).unwrap();
                            w.set_color(self.text_color, Color::Black);
                        }
                    }
                    Err(e) => {
                        w.set_color(Color::Red, Color::Black);
                        writeln!(w, "Hata: {}", e).unwrap();
                        w.set_color(self.text_color, Color::Black);
                    }
                }
            }
            _ if cmd.starts_with("mkdir ") => {
                let dir_name = &cmd[6..].trim();
                let resolved = crate::fs::resolve_path(&self.cwd, dir_name);
                match crate::fs::mkdir(&resolved) {
                    Ok(_) => writeln!(w, "Dizin olusturuldu: {}", dir_name).unwrap(),
                    Err(e) => {
                        w.set_color(Color::Red, Color::Black);
                        writeln!(w, "Hata: {}", e).unwrap();
                        w.set_color(self.text_color, Color::Black);
                    }
                }
            }
            _ if cmd.starts_with("cat ") => {
                let file_name = &cmd[4..].trim();
                let resolved = crate::fs::resolve_path(&self.cwd, file_name);
                match crate::fs::read_file(&resolved) {
                    Ok(content) => writeln!(w, "{}", content).unwrap(),
                    Err(e) => {
                        w.set_color(Color::Red, Color::Black);
                        writeln!(w, "Hata: {}", e).unwrap();
                        w.set_color(self.text_color, Color::Black);
                    }
                }
            }
            _ if cmd.starts_with("write ") => {
                let args = &cmd[6..].trim();
                if let Some(space_idx) = args.find(' ') {
                    let file_name = &args[..space_idx];
                    let content = &args[space_idx + 1..];
                    let resolved = crate::fs::resolve_path(&self.cwd, file_name);
                    match crate::fs::write_file(&resolved, content) {
                        Ok(_) => writeln!(w, "Dosyaya yazildi: {}", file_name).unwrap(),
                        Err(e) => {
                            w.set_color(Color::Red, Color::Black);
                            writeln!(w, "Hata: {}", e).unwrap();
                            w.set_color(self.text_color, Color::Black);
                        }
                    }
                } else {
                    w.set_color(Color::Red, Color::Black);
                    writeln!(w, "Kullanim: write <dosya_adi> <icerik>").unwrap();
                    w.set_color(self.text_color, Color::Black);
                }
            }
            _ if cmd.starts_with("disk_write ") => {
                let args = &cmd[11..].trim();
                if let Some(space_idx) = args.find(' ') {
                    let lba_str = &args[..space_idx];
                    let text = &args[space_idx + 1..];
                    if let Ok(lba) = lba_str.parse::<u32>() {
                        let mut buf = [0u8; 512];
                        let bytes = text.as_bytes();
                        let len = core::cmp::min(bytes.len(), 512);
                        buf[..len].copy_from_slice(&bytes[..len]);
                        
                        match crate::ata::DATA_DRIVE.lock().write_sector(lba, &buf) {
                            Ok(_) => writeln!(w, "LBA {} sektorune basariyla yazildi.", lba).unwrap(),
                            Err(e) => {
                                w.set_color(Color::Red, Color::Black);
                                writeln!(w, "Hata: {}", e).unwrap();
                                w.set_color(self.text_color, Color::Black);
                            }
                        }
                    } else {
                        w.set_color(Color::Red, Color::Black);
                        writeln!(w, "Hata: Gecersiz LBA (Sektor) numarasi").unwrap();
                        w.set_color(self.text_color, Color::Black);
                    }
                } else {
                    w.set_color(Color::Red, Color::Black);
                    writeln!(w, "Kullanim: disk_write <sektor_no> <metin>").unwrap();
                    w.set_color(self.text_color, Color::Black);
                }
            }
            _ if cmd.starts_with("disk_read ") => {
                let lba_str = &cmd[10..].trim();
                if let Ok(lba) = lba_str.parse::<u32>() {
                    let mut buf = [0u8; 512];
                    match crate::ata::DATA_DRIVE.lock().read_sector(lba, &mut buf) {
                        Ok(_) => {
                            // Trim trailing null bytes for display
                            let mut end = 512;
                            while end > 0 && buf[end - 1] == 0 { end -= 1; }
                            let s = core::str::from_utf8(&buf[..end]).unwrap_or("<Gecersiz UTF-8 verisi>");
                            w.set_color(Color::Cyan, Color::Black);
                            writeln!(w, "LBA {} Icerigi: {}", lba, s).unwrap();
                            w.set_color(self.text_color, Color::Black);
                        }
                        Err(e) => {
                            w.set_color(Color::Red, Color::Black);
                            writeln!(w, "Hata: {}", e).unwrap();
                            w.set_color(self.text_color, Color::Black);
                        }
                    }
                } else {
                    w.set_color(Color::Red, Color::Black);
                    writeln!(w, "Hata: Gecersiz LBA (Sektor) numarasi").unwrap();
                    w.set_color(self.text_color, Color::Black);
                }
            }
            "gui" => {
                core::fmt::Write::write_str(&mut *w, "Zaten GUI modundayiz!\n").unwrap();
            }
            _ if cmd.starts_with("color ") => {
                let color_name = &cmd[6..];
                let color = match color_name {
                    "red" | "kirmizi" => Color::Red,
                    "green" | "yesil" => Color::Green,
                    "blue" | "mavi" => Color::LightBlue,
                    "yellow" | "sari" => Color::Yellow,
                    "cyan" => Color::Cyan,
                    "magenta" => Color::Magenta,
                    "white" | "beyaz" | "reset" => Color::White,
                    _ => {
                        writeln!(w, "Bilinmeyen renk! Gecerli: red, green, blue, yellow, cyan, magenta, white").unwrap();
                        self.text_color
                    }
                };
                self.text_color = color;
            }
            _ if cmd.starts_with("echo ") => {
                writeln!(w, "{}", &cmd[5..]).unwrap();
            }
            _ => {
                w.set_color(Color::Red, Color::Black);
                writeln!(w, "Hata: '{}' bilinmiyor (help yazin)", cmd).unwrap();
            }
        }
        w.set_color(self.text_color, Color::Black);
    }
}
