use core::fmt::Write;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use crate::fs;
use crate::task::keyboard;
use crate::vga_buffer::{Color, WRITE_LOCK};

const CMD_BUF: usize = 256;

pub struct Shell {
    buf: [u8; CMD_BUF],
    len: usize,
    text_color: Color,
    cwd: String,
}

impl Shell {
    pub fn new() -> Self {
        Shell { buf: [0; CMD_BUF], len: 0, text_color: Color::White, cwd: "/".to_string() }
    }

    pub async fn run(&mut self) {
        // Ekranı temizle
        WRITE_LOCK.lock().clear();
        writeln!(WRITE_LOCK.lock(), "SparkOS CLI Modu Baslatildi.").unwrap();
        
        loop {
            self.prompt();
            self.read_line().await;
            self.exec();
        }
    }

    pub fn prompt(&self) {
        let mut w = WRITE_LOCK.lock();
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
                            let mut w = WRITE_LOCK.lock();
                            core::fmt::Write::write_char(&mut *w, c as char).unwrap();
                        }
                    }
                    Key::Backspace => {
                        if self.len > 0 {
                            self.len -= 1;
                            let mut w = WRITE_LOCK.lock();
                            w.write_byte(b'\x08'); // vga_buffer backspace
                        }
                    }
                    Key::Enter => {
                        let mut w = WRITE_LOCK.lock();
                        core::fmt::Write::write_str(&mut *w, "\n").unwrap();
                        return;
                    }
                    Key::Escape => {
                        let mut w = WRITE_LOCK.lock();
                        for _ in 0..self.len {
                            w.write_byte(b'\x08');
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
        let mut w = WRITE_LOCK.lock();
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
                writeln!(w, "  rm       - dosya veya dizin sil").unwrap();
                writeln!(w, "  cat      - dosyanin icerigini oku").unwrap();
                writeln!(w, "  disk_write - diskin belirli sektorune yaz").unwrap();
                writeln!(w, "  disk_read  - diskin belirli sektorunu oku").unwrap();
                writeln!(w, "  reboot   - sistemi yeniden baslat").unwrap();
                writeln!(w, "  shutdown - sistemi kapat (QEMU)").unwrap();
                writeln!(w, "  panic    - kernel panic testi (sistemi dondurur)").unwrap();
            }
            "clear" => {
                w.clear();
                return;
            }
            "info" => {
                writeln!(w, "SparkOS v0.1 - Rust x86_64").unwrap();
                writeln!(w, "Timer: 1000 Hz").unwrap();
                writeln!(w, "Mod: CLI (VGA Text Mode)").unwrap();
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
                    // QEMU ACPI kapatma portu
                    let mut p: x86_64::instructions::port::Port<u16> =
                        x86_64::instructions::port::Port::new(0x604);
                    p.write(0x2000);
                }
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
                        }
                    }
                    Err(e) => {
                        w.set_color(Color::Red, Color::Black);
                        writeln!(w, "Hata: {}", e).unwrap();
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
                        }
                    }
                } else {
                    w.set_color(Color::Red, Color::Black);
                    writeln!(w, "Kullanim: write <dosya_adi> <icerik>").unwrap();
                }
            }
            _ if cmd.starts_with("rm ") => {
                let target = &cmd[3..].trim();
                let resolved = crate::fs::resolve_path(&self.cwd, target);
                match crate::fs::remove(&resolved) {
                    Ok(_) => writeln!(w, "Silindi: {}", target).unwrap(),
                    Err(e) => {
                        w.set_color(Color::Red, Color::Black);
                        writeln!(w, "Hata: {}", e).unwrap();
                    }
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
                            }
                        }
                    } else {
                        w.set_color(Color::Red, Color::Black);
                        writeln!(w, "Hata: Gecersiz LBA (Sektor) numarasi").unwrap();
                    }
                } else {
                    w.set_color(Color::Red, Color::Black);
                    writeln!(w, "Kullanim: disk_write <sektor_no> <metin>").unwrap();
                }
            }
            _ if cmd.starts_with("disk_read ") => {
                let lba_str = &cmd[10..].trim();
                if let Ok(lba) = lba_str.parse::<u32>() {
                    let mut buf = [0u8; 512];
                    match crate::ata::DATA_DRIVE.lock().read_sector(lba, &mut buf) {
                        Ok(_) => {
                            let mut end = 512;
                            while end > 0 && buf[end - 1] == 0 { end -= 1; }
                            let s = core::str::from_utf8(&buf[..end]).unwrap_or("<Gecersiz UTF-8 verisi>");
                            w.set_color(Color::Cyan, Color::Black);
                            writeln!(w, "LBA {} Icerigi: {}", lba, s).unwrap();
                        }
                        Err(e) => {
                            w.set_color(Color::Red, Color::Black);
                            writeln!(w, "Hata: {}", e).unwrap();
                        }
                    }
                } else {
                    w.set_color(Color::Red, Color::Black);
                    writeln!(w, "Hata: Gecersiz LBA (Sektor) numarasi").unwrap();
                }
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
