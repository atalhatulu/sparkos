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
    history: Vec<String>,
    history_idx: usize,
}

impl Shell {
    pub fn new() -> Self {
        Shell { 
            buf: [0; CMD_BUF], 
            len: 0, 
            text_color: Color::White, 
            cwd: "/".to_string(),
            history: Vec::new(),
            history_idx: 0,
        }
    }

    pub async fn run(&mut self) {
        // Ekranı temizle
        WRITE_LOCK.lock().clear();
        writeln!(WRITE_LOCK.lock(), "SparkOS CLI Modu Baslatildi.").unwrap();
        
        loop {
            self.prompt();
            self.read_line().await;
            self.exec().await;
        }
    }

    pub fn prompt(&self) {
        let mut w = WRITE_LOCK.lock();
        w.set_color(Color::Green, Color::Black);
        core::fmt::Write::write_str(&mut *w, "sparkos ").unwrap();
        w.set_color(Color::Cyan, Color::Black);
        core::fmt::Write::write_str(&mut *w, &self.cwd).unwrap();
        w.set_color(self.text_color, Color::Black);
        core::fmt::Write::write_str(&mut *w, " > ").unwrap();
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
                        
                        let cmd_str = self.cmd().to_string();
                        if !cmd_str.trim().is_empty() {
                            if self.history.last() != Some(&cmd_str) {
                                self.history.push(cmd_str);
                                if self.history.len() > 20 {
                                    self.history.remove(0);
                                }
                            }
                        }
                        self.history_idx = self.history.len();
                        return;
                    }
                    Key::Escape => {
                        let mut w = WRITE_LOCK.lock();
                        for _ in 0..self.len {
                            w.write_byte(b'\x08');
                        }
                        self.len = 0;
                    }
                    Key::Up => {
                        if self.history_idx > 0 {
                            self.history_idx -= 1;
                            self.load_history();
                        }
                    }
                    Key::Down => {
                        if self.history_idx < self.history.len() {
                            self.history_idx += 1;
                            self.load_history();
                        }
                    }
                    Key::Tab => {
                        let cmd_str = self.cmd().to_string();
                        if let Some(last_space) = cmd_str.rfind(' ') {
                            let prefix = &cmd_str[last_space + 1..];
                            self.auto_complete(prefix, last_space + 1);
                        } else {
                            self.auto_complete(&cmd_str, 0);
                        }
                    }
                    _ => {}
                }
            }
            let scancode = crate::task::keyboard::read_scancode().await;
            crate::keyboard::KEYBOARD.lock().handle_scancode(scancode);
        }
    }

    fn load_history(&mut self) {
        let mut w = WRITE_LOCK.lock();
        for _ in 0..self.len {
            w.write_byte(b'\x08');
        }
        
        if self.history_idx < self.history.len() {
            let cmd = &self.history[self.history_idx];
            let bytes = cmd.as_bytes();
            self.len = core::cmp::min(bytes.len(), CMD_BUF);
            self.buf[..self.len].copy_from_slice(&bytes[..self.len]);
        } else {
            self.len = 0;
        }
        
        for i in 0..self.len {
            core::fmt::Write::write_char(&mut *w, self.buf[i] as char).unwrap();
        }
    }

    fn auto_complete(&mut self, prefix: &str, replace_start: usize) {
        let mut matches = Vec::new();
        if replace_start == 0 {
            let commands = ["help", "clear", "info", "tick", "uptime", "color", "echo", "pwd", "cd", "ls", "mkdir", "write", "rm", "cat", "reboot", "shutdown", "edit", "ps", "kill", "lspci", "ifconfig", "ping"];
            for c in commands.iter() {
                if c.starts_with(prefix) {
                    matches.push(c.to_string());
                }
            }
        }
        
        if let Ok(items) = crate::fs::list_dir(&self.cwd) {
            for (name, _is_dir) in items {
                if name.starts_with(prefix) {
                    matches.push(name);
                }
            }
        }
        
        if matches.len() == 1 {
            let completion = &matches[0];
            let mut w = WRITE_LOCK.lock();
            for _ in 0..prefix.len() {
                w.write_byte(b'\x08');
            }
            let bytes = completion.as_bytes();
            let mut i = 0;
            while replace_start + i < CMD_BUF && i < bytes.len() {
                self.buf[replace_start + i] = bytes[i];
                core::fmt::Write::write_char(&mut *w, bytes[i] as char).unwrap();
                i += 1;
            }
            self.len = replace_start + i;
        }
    }

    fn cmd(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    async fn exec(&mut self) {
        let cmd = self.cmd().trim();
        // Bazı asenkron komutlar için lock'ı geçici almalıyız, o yüzden w'yi match içine taşıyacağız
        if cmd == "" { return; }
        
        if cmd.starts_with("edit ") {
            let file_name = &cmd[5..].trim();
            let resolved = crate::fs::resolve_path(&self.cwd, file_name);
            crate::editor::run_editor(&resolved, file_name).await;
            return;
        }
        
        let mut w = WRITE_LOCK.lock();
        match cmd {
            "help" | "yardim" => {
                w.set_color(Color::Yellow, Color::Black);
                writeln!(w, "Komutlar:").unwrap();
                writeln!(w, "  help       - bu liste").unwrap();
                writeln!(w, "  clear      - ekrani temizle").unwrap();
                writeln!(w, "  info       - sistem bilgisi").unwrap();
                writeln!(w, "  tick       - timer sayaci").unwrap();
                writeln!(w, "  uptime     - sistemin calisma suresi").unwrap();
                writeln!(w, "  color      - yazi rengini degistir").unwrap();
                writeln!(w, "  echo       - mesaj bas").unwrap();
                writeln!(w, "  pwd        - gecerli dizini goster").unwrap();
                writeln!(w, "  cd         - dizin degistir").unwrap();
                writeln!(w, "  ls         - dosyalari ve dizinleri listele").unwrap();
                writeln!(w, "  mkdir      - yeni bir dizin olustur").unwrap();
                writeln!(w, "  write      - yeni bir dosya olustur veya icerigini degistir").unwrap();
                writeln!(w, "  rm         - dosya veya dizin sil").unwrap();
                writeln!(w, "  cat        - dosyanin icerigini oku").unwrap();
                writeln!(w, "  edit       - nano benzeri tam ekran metin editorunu ac").unwrap();
                writeln!(w, "  ps         - calisan surecleri (görevleri) listele").unwrap();
                writeln!(w, "  kill <pid> - belirtilen sureci (PID) sonlandir").unwrap();
                writeln!(w, "  lspci      - PCI donanimlarini tara ve listele").unwrap();
                writeln!(w, "  ifconfig   - Ag karti ve MAC adresini goster").unwrap();
                writeln!(w, "  ping       - Google (8.8.8.8) adresine ICMP paketi yolla").unwrap();
                writeln!(w, "  disk_write - diskin belirli sektorune yaz").unwrap();
                writeln!(w, "  disk_read  - diskin belirli sektorunu oku").unwrap();
                writeln!(w, "  reboot     - sistemi yeniden baslat").unwrap();
                writeln!(w, "  shutdown   - sistemi kapat (QEMU)").unwrap();
                writeln!(w, "  panic      - kernel panic testi (sistemi dondurur)").unwrap();
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
            "shutdown" => {
                w.set_color(Color::Red, Color::Black);
                writeln!(w, "Sistem kapatiliyor...").unwrap();
                let mut port = x86_64::instructions::port::Port::new(0x604);
                unsafe { port.write(0x2000u16); }
            }
            "ps" => {
                w.set_color(Color::Cyan, Color::Black);
                writeln!(w, "PID\tISIM").unwrap();
                writeln!(w, "---\t----").unwrap();
                w.set_color(Color::White, Color::Black);
                let list = crate::task::PROCESS_LIST.lock();
                for (id, name) in list.iter() {
                    writeln!(w, "{}\t{}", id, name).unwrap();
                }
            }
            "lspci" => {
                w.set_color(Color::LightBlue, Color::Black);
                writeln!(w, "Bus\tSlot\tFunc\tCihaz Bilgisi").unwrap();
                writeln!(w, "---\t----\t----\t-------------").unwrap();
                w.set_color(Color::White, Color::Black);
                let devices = crate::pci::scan_pci();
                if devices.is_empty() {
                    writeln!(w, "Hicbir PCI donanimi bulunamadi!").unwrap();
                } else {
                    for dev in devices {
                        writeln!(w, "{:02X}\t{:02X}\t{:02X}\t{}", dev.bus, dev.slot, dev.func, dev.get_name()).unwrap();
                    }
                }
            }
            "ifconfig" => {
                w.set_color(Color::LightCyan, Color::Black);
                unsafe {
                    if let Some(ref dev) = crate::rtl8139::RTL8139_DEV {
                        let mac = dev.get_mac_address();
                        writeln!(w, "eth0: Realtek RTL8139 Fast Ethernet").unwrap();
                        writeln!(w, "      MAC Adresi: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}", 
                                 mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]).unwrap();
                        writeln!(w, "      Durum: AKTIF, Baglanti Bekleniyor...").unwrap();
                    } else {
                        w.set_color(Color::LightRed, Color::Black);
                        writeln!(w, "Sistemde tanimli bir Ag Karti (NIC) bulunamadi!").unwrap();
                    }
                }
            }
            "ping" => {
                w.set_color(Color::White, Color::Black);
                writeln!(w, "PING 8.8.8.8 (Google) 32 bytes of data. Durdurmak icin 'ESC' tusuna basin...").unwrap();
                // VGA kilidini bırak
                drop(w);

                let mut sequence_num = 1;
                let mut last_ping = 0;
                let mut ping_count = 0;
                
                loop {
                    // ESC tusuna basilip basilmadigini kontrol et
                    if let Some(queue) = crate::task::keyboard::SCANCODE_QUEUE.get() {
                        if let Some(scancode) = queue.pop() {
                            if scancode == 0x01 { // ESC Key
                                let mut w = WRITE_LOCK.lock();
                                w.set_color(Color::Yellow, Color::Black);
                                writeln!(w, "--- 8.8.8.8 ping statistics ---").unwrap();
                                writeln!(w, "{} packets transmitted. Ping durduruldu.", ping_count).unwrap();
                                break;
                            }
                        }
                    }

                    // Saniyede 1 kez gonder (1000 tick)
                    let current_tick = crate::interrupts::get_tick();
                    if current_tick >= last_ping + 1000 {
                        unsafe {
                            if let Some(ref mut dev) = crate::rtl8139::RTL8139_DEV {
                                let mac = dev.get_mac_address();
                                let packet = crate::net::create_ping_packet(mac, sequence_num);
                                dev.send_packet(&packet);
                                ping_count += 1;
                            } else {
                                let mut w = WRITE_LOCK.lock();
                                w.set_color(Color::LightRed, Color::Black);
                                writeln!(w, "Ag Karti bulunamadi. PING atilamiyor!").unwrap();
                                break;
                            }
                        }
                        last_ping = current_tick;
                        sequence_num += 1;
                    }

                    // Rx'i dinle (Gelen cevap var mi?)
                    unsafe {
                        if let Some(ref mut dev) = crate::rtl8139::RTL8139_DEV {
                            if let Some(packet) = dev.poll_rx() {
                                // ICMP Echo Reply kontrolu
                                // Ethernet (14) + IP (20) = ICMP baslangici (34. index)
                                if packet.len() >= 42 && packet[14] == 0x45 && packet[23] == 0x01 && packet[34] == 0x00 {
                                    let icmp_seq = ((packet[38] as u16) << 8) | (packet[39] as u16);
                                    let ttl = packet[22];
                                    let mut w = WRITE_LOCK.lock();
                                    w.set_color(Color::LightGreen, Color::Black);
                                    writeln!(w, "64 bytes from 8.8.8.8: icmp_seq={} ttl={}", icmp_seq, ttl).unwrap();
                                }
                            }
                        }
                    }

                    // Sistemin kilitlenmemesi ve arka plan gorevlerinin calismasi icin Yield
                    crate::task::yield_now().await;
                }
                
                // Rust Borrow Checker icin w kilidini geri al (cunku scope sonunda kullaniliyor)
                w = WRITE_LOCK.lock();
            }
            _ if cmd.starts_with("kill ") => {
                let id_str = &cmd[5..].trim();
                if let Ok(id) = id_str.parse::<u64>() {
                    let mut list = crate::task::PROCESS_LIST.lock();
                    if list.contains_key(&id) {
                        crate::task::KILLED_PROCESSES.lock().push(id);
                        w.set_color(Color::Yellow, Color::Black);
                        writeln!(w, "Süreç [{}] (PID: {}) öldürülmek üzere işaretlendi.", list.get(&id).unwrap(), id).unwrap();
                    } else {
                        w.set_color(Color::Red, Color::Black);
                        writeln!(w, "Hata: PID bulunamadi.").unwrap();
                    }
                } else {
                    w.set_color(Color::Red, Color::Black);
                    writeln!(w, "Hata: Gecersiz PID formati. (Kullanim: kill <PID>)").unwrap();
                }
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
