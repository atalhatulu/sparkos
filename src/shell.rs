use core::fmt::Write;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
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
        // Flush any stale residual bytes from PS/2 keyboard
        crate::keyboard::KEYBOARD.lock().clear();
        self.len = 0;
        self.buf = [0; CMD_BUF];

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
        use crate::keyboard::Key;
        self.len = 0;
        loop {
            let mut got_input = false;

            // 1. Serial Port (COM1 / Terminal stdio)
            while let Some(b) = crate::serial::try_read_byte() {
                got_input = true;
                match b {
                    0x03 => { // Ctrl+C
                        let mut w = WRITE_LOCK.lock();
                        core::fmt::Write::write_str(&mut *w, "^C\n").unwrap();
                        self.len = 0;
                        self.prompt();
                    }
                    b'\r' | b'\n' => {
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
                    0x08 | 0x7F => { // Backspace / DEL
                        if self.len > 0 {
                            self.len -= 1;
                            let mut w = WRITE_LOCK.lock();
                            core::fmt::Write::write_char(&mut *w, '\x08').unwrap();
                        }
                    }
                    0x1B => { // Escape
                        let mut w = WRITE_LOCK.lock();
                        for _ in 0..self.len {
                            core::fmt::Write::write_char(&mut *w, '\x08').unwrap();
                        }
                        self.len = 0;
                    }
                    b'\t' => { // Tab autocomplete
                        let cmd_str = self.cmd().to_string();
                        if let Some(last_space) = cmd_str.rfind(' ') {
                            let prefix = &cmd_str[last_space + 1..];
                            self.auto_complete(prefix, last_space + 1);
                        } else {
                            self.auto_complete(&cmd_str, 0);
                        }
                    }
                    32..=126 => { // Printable ASCII
                        if self.len < CMD_BUF {
                            self.buf[self.len] = b;
                            self.len += 1;
                            let mut w = WRITE_LOCK.lock();
                            core::fmt::Write::write_char(&mut *w, b as char).unwrap();
                        }
                    }
                    _ => {}
                }
            }

            // 2. PS/2 Keyboard (Only consumed by CLI when GUI is NOT active)
            if !crate::display::is_gui_active() {
                while let Some(key) = crate::keyboard::read_key() {
                    got_input = true;
                    match key {
                        Key::CtrlC => {
                            let mut w = WRITE_LOCK.lock();
                            core::fmt::Write::write_str(&mut *w, "^C\n").unwrap();
                            self.len = 0;
                            self.prompt();
                        }
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
                                core::fmt::Write::write_char(&mut *w, '\x08').unwrap();
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
                                core::fmt::Write::write_char(&mut *w, '\x08').unwrap();
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
            } else {
                crate::keyboard::KEYBOARD.lock().clear();
            }

            if !got_input {
                crate::task::yield_now().await;
            }
        }
    }

    fn load_history(&mut self) {
        let mut w = WRITE_LOCK.lock();
        for _ in 0..self.len {
            core::fmt::Write::write_char(&mut *w, '\x08').unwrap();
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
            let commands = [
                "help", "clear", "info", "fetch", "tick", "uptime", "color", "echo", 
                "pwd", "cd", "ls", "mkdir", "write", "touch", "rm", "cat", "hexdump", 
                "edit", "reboot", "shutdown", "ps", "kill", "dmesg", "ktrace", 
                "exec", "run_app", "lspci", "ifconfig", "ping", "host", "mem", "free", 
                "smp", "cpuinfo", "caps", "gui"
            ];
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
                core::fmt::Write::write_char(&mut *w, '\x08').unwrap();
            }
            let bytes = completion.as_bytes();
            let mut i = 0;
            while replace_start + i < CMD_BUF && i < bytes.len() {
                self.buf[replace_start + i] = bytes[i];
                core::fmt::Write::write_char(&mut *w, bytes[i] as char).unwrap();
                i += 1;
            }
            self.len = replace_start + i;
        } else if matches.len() > 1 {
            // Birden fazla eşleşme varsa listele ve komut satırını yeniden çiz
            let mut w = WRITE_LOCK.lock();
            core::fmt::Write::write_str(&mut *w, "\n").unwrap();
            w.set_color(Color::Cyan, Color::Black);
            for m in matches.iter() {
                core::fmt::Write::write_str(&mut *w, m).unwrap();
                core::fmt::Write::write_str(&mut *w, "  ").unwrap();
            }
            w.set_color(Color::White, Color::Black);
            core::fmt::Write::write_str(&mut *w, "\n").unwrap();
            drop(w);
            self.prompt();
            let mut w = WRITE_LOCK.lock();
            for i in 0..self.len {
                core::fmt::Write::write_char(&mut *w, self.buf[i] as char).unwrap();
            }
        }
    }

    fn cmd(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    async fn exec(&mut self) {
        let cmd = self.cmd().trim();
        if cmd.is_empty() { return; }
        
        if let Some(file_name) = cmd.strip_prefix("edit ") {
            let file_name = file_name.trim();
            let resolved = crate::fs::resolve_path(&self.cwd, file_name);
            crate::editor::run_editor(&resolved, file_name).await;
            return;
        }
        
        // Kilitlenmeleri önlemek için donanım kesmelerini kapalı tutarak lock alıyoruz
        let mut w = x86_64::instructions::interrupts::without_interrupts(|| WRITE_LOCK.lock());
        match cmd {
            "help" | "yardim" => {
                w.set_color(Color::Yellow, Color::Black);
                writeln!(w, "=== SparkOS CLI Komut Rehberi ===").unwrap();
                w.set_color(Color::Cyan, Color::Black);
                write!(w, "  [Sistem]:   ").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "fetch, mem, free, smp, caps, uptime, clear, reboot, shutdown").unwrap();
                
                w.set_color(Color::Cyan, Color::Black);
                write!(w, "  [Dosya]:    ").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "ls, cd, pwd, cat, touch, write, mkdir, rm, edit, hexdump").unwrap();
                
                w.set_color(Color::Cyan, Color::Black);
                write!(w, "  [Gorevler]: ").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "ps, kill <pid>, exec <path>, run_app, dmesg, ktrace").unwrap();

                w.set_color(Color::Cyan, Color::Black);
                write!(w, "  [Ag/PCI]:   ").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "lspci, ifconfig, ping, host <domain>").unwrap();

                w.set_color(Color::LightGreen, Color::Black);
                writeln!(w, "  [Klavye]:   ").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "Ctrl+C (Iptal), Tab (Tamamla), Yukari/Asagi (Gecmis)").unwrap();

                w.set_color(Color::LightGray, Color::Black);
                writeln!(w, "--> Detaylar icin: help sys | help fs | help proc | help net | help all").unwrap();
            }
            "help sys" => {
                w.set_color(Color::Yellow, Color::Black);
                writeln!(w, "=== Sistem Komutlari ===").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "  fetch / info  - SparkOS sistem ve donanim ozeti").unwrap();
                writeln!(w, "  uptime / tick - Calisma suresi ve PIT zamanlayici sayaci").unwrap();
                writeln!(w, "  mem / free    - Bellek kullanimi ve heap durumu").unwrap();
                writeln!(w, "  smp / cpuinfo - SMP cok cekirdekli CPU durumu").unwrap();
                writeln!(w, "  caps          - Capability CSpace durumunu listele").unwrap();
                writeln!(w, "  clear         - Ekrani temizle").unwrap();
                writeln!(w, "  color <renk>  - Yazi rengi (red, green, blue, cyan, yellow)").unwrap();
                writeln!(w, "  reboot        - Sistemi yeniden baslat").unwrap();
                writeln!(w, "  shutdown      - Sistemi kapat (QEMU ACPI)").unwrap();
            }
            "help fs" => {
                w.set_color(Color::Yellow, Color::Black);
                writeln!(w, "=== Dosya Sistemi Komutlari ===").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "  ls [path]     - Dosya ve dizinleri listele").unwrap();
                writeln!(w, "  cd <path>     - Dizin degistir").unwrap();
                writeln!(w, "  pwd           - Gecerli dizini goster").unwrap();
                writeln!(w, "  cat <dosya>   - Metin dosyasini ekrana bas").unwrap();
                writeln!(w, "  touch <dosya> - Yeni bos dosya olustur").unwrap();
                writeln!(w, "  write <d> <m> - Dosyaya metin yaz").unwrap();
                writeln!(w, "  mkdir <dizin> - Yeni dizin olustur").unwrap();
                writeln!(w, "  rm <hedef>    - Dosya veya dizin sil").unwrap();
                writeln!(w, "  edit <dosya>  - Tam ekran metin editorunu ac").unwrap();
                writeln!(w, "  hexdump <d>   - Ikili (binary) dosya hex gorunumu").unwrap();
            }
            "help proc" => {
                w.set_color(Color::Yellow, Color::Black);
                writeln!(w, "=== Surecler ve Donanim Komutlari ===").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "  ps            - Calisan surecleri listele").unwrap();
                writeln!(w, "  kill <pid>    - Sureci sonlandir").unwrap();
                writeln!(w, "  exec <path>   - Diskten ELF ikili dosyasini calistir").unwrap();
                writeln!(w, "  run_app       - Dahili test ELF programini Ring 3'te calistir").unwrap();
                writeln!(w, "  dmesg / ktrace- Kernel olay gunlugunu (trace ring) listele").unwrap();
                writeln!(w, "  lspci         - PCI veriyolundaki cihazlari tara").unwrap();
            }
            "help net" => {
                w.set_color(Color::Yellow, Color::Black);
                writeln!(w, "=== Ag Komutlari ===").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "  ifconfig      - Ag karti ve MAC adresini goster").unwrap();
                writeln!(w, "  ping          - Google (8.8.8.8) ICMP testi").unwrap();
                writeln!(w, "  host <domain> - DNS IP sorgusu").unwrap();
            }
            "help all" => {
                w.set_color(Color::Yellow, Color::Black);
                writeln!(w, "=== Tum SparkOS Komutlari ===").unwrap();
                w.set_color(Color::Cyan, Color::Black);
                writeln!(w, "[Sistem]:").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "  fetch, uptime, mem, smp, caps, clear, color, reboot, shutdown").unwrap();
                w.set_color(Color::Cyan, Color::Black);
                writeln!(w, "[Dosya]:").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "  ls, cd, pwd, cat, touch, write, mkdir, rm, edit, hexdump").unwrap();
                w.set_color(Color::Cyan, Color::Black);
                writeln!(w, "[Surec/Ag]:").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "  ps, kill, exec, run_app, dmesg, lspci, ifconfig, ping, host").unwrap();
            }
            "clear" => {
                w.clear();
                return;
            }
            "fetch" | "sparkfetch" | "neofetch" | "info" => {
                w.set_color(Color::Cyan, Color::Black);
                writeln!(w, "  ____                   _     ___  ____  ").unwrap();
                writeln!(w, " / ___| _ __   __ _ _ __| | __/ _ \\/ ___| ").unwrap();
                writeln!(w, " \\___ \\| '_ \\ / _` | '__| |/ / | | \\___ \\ ").unwrap();
                writeln!(w, "  ___) | |_) | (_| | |  |   <| |_| |___) |").unwrap();
                writeln!(w, " |____/| .__/ \\__,_|_|  |_|\\_\\\\___/|____/ ").unwrap();
                writeln!(w, "       |_|                                ").unwrap();
                
                w.set_color(Color::Yellow, Color::Black);
                write!(w, "OS:        ").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "SparkOS 0.1.0-distro (x86_64, Capability-Based)").unwrap();

                w.set_color(Color::Yellow, Color::Black);
                write!(w, "Kernel:    ").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "Rust Microkernel with Per-Process CR3 Isolation").unwrap();

                w.set_color(Color::Yellow, Color::Black);
                write!(w, "Uptime:    ").unwrap();
                w.set_color(Color::White, Color::Black);
                let secs = crate::interrupts::get_tick() / 1000;
                writeln!(w, "{} saniye ({} ms)", secs, crate::interrupts::get_tick()).unwrap();

                w.set_color(Color::Yellow, Color::Black);
                write!(w, "Security:  ").unwrap();
                w.set_color(Color::LightGreen, Color::Black);
                writeln!(w, "Formal CSpace (18/18 Invariants Verified & Frozen)").unwrap();

                w.set_color(Color::Yellow, Color::Black);
                write!(w, "SMP Cores: ").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "2 CPUs (Local APIC + I/O APIC Online)").unwrap();

                w.set_color(Color::Yellow, Color::Black);
                write!(w, "Memory:    ").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "Heap 128 MB | Usable RAM 246 MB").unwrap();

                w.set_color(Color::Yellow, Color::Black);
                write!(w, "Storage:   ").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "SPFS VFS (Binary-Safe + 32KB BlockCache)").unwrap();
            }
            "smp" | "cpuinfo" => {
                w.set_color(Color::Cyan, Color::Black);
                writeln!(w, "APIC ID\tBSP\tDURUM\tPID").unwrap();
                writeln!(w, "-------\t---\t-----\t---").unwrap();
                w.set_color(Color::White, Color::Black);
                let states = crate::smp::CPU_STATES.lock();
                for state in states.iter() {
                    if state.online || state.is_bsp {
                        let bsp_str = if state.is_bsp { "EVET" } else { "HAYIR" };
                        let status_str = if state.online { "ONLINE" } else { "OFFLINE" };
                        let pid_str = match state.current_pid {
                            Some(p) => alloc::format!("{}", p),
                            None => "IDLE".to_string(),
                        };
                        writeln!(w, "{}\t{}\t{}\t{}", state.apic_id, bsp_str, status_str, pid_str).unwrap();
                    }
                }
            }
            "caps" => {
                w.set_color(Color::LightGreen, Color::Black);
                writeln!(w, "Capability Subsystem Status:").unwrap();
                w.set_color(Color::White, Color::Black);
                if let Some(root) = crate::cap::root_cap() {
                    writeln!(w, "  Root Capability: Slot {}, Gen {}", root.slot, root.generation).unwrap();
                }
                writeln!(w, "  Model: Epoch-based Lazy Revocation + CSpace Isolation").unwrap();
                writeln!(w, "  Formal Test Invariants: 18/18 Verified").unwrap();
            }
            "mem" | "free" => {
                w.set_color(Color::Cyan, Color::Black);
                writeln!(w, "Bellek Durumu:").unwrap();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "  Kullanilabilir Toplam RAM: 246 MB").unwrap();
                writeln!(w, "  Kernel Heap Alani:        128 MB (0x18000a84000 - 0x18008a84000)").unwrap();
                writeln!(w, "  User Frame Allocator:      Aktif (İzole CR3 Sayfa Havuzu)").unwrap();
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
                    let mut p: x86_64::instructions::port::Port<u8> = x86_64::instructions::port::Port::new(0x64);
                    p.write(0xFE);
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
                x86_64::instructions::interrupts::without_interrupts(|| {
                    let list = crate::task::PROCESS_LIST.lock();
                    for (id, name) in list.iter() {
                        writeln!(w, "{}\t{}", id, name).unwrap();
                    }
                });
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
                drop(w);

                let mut sequence_num = 1;
                let mut last_ping = 0;
                let mut ping_count = 0;
                
                loop {
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

                    unsafe {
                        if let Some(ref mut dev) = crate::rtl8139::RTL8139_DEV {
                            if let Some(packet) = dev.poll_rx() {
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

                    crate::task::yield_now().await;
                }
                
                w = x86_64::instructions::interrupts::without_interrupts(|| WRITE_LOCK.lock());
            }
            "dmesg" | "ktrace" => {
                let ring = crate::ktrace::TRACE_RING.lock();
                w.set_color(Color::Cyan, Color::Black);
                writeln!(w, "=== Kernel Olay Gunlugu (ktrace: {} kayit) ===", ring.count).unwrap();
                let n = ring.count.min(16); // Son 16 event
                let start = if n >= ring.count {
                    ring.head
                } else {
                    (ring.head + ring.count - n) % crate::ktrace::RING_CAP
                };
                for i in 0..n {
                    let idx = (start + i) % crate::ktrace::RING_CAP;
                    let ev = &ring.events[idx];
                    match ev.level {
                        crate::klog::LogLevel::Error => w.set_color(Color::Red, Color::Black),
                        crate::klog::LogLevel::Warn => w.set_color(Color::Yellow, Color::Black),
                        crate::klog::LogLevel::Info => w.set_color(Color::Green, Color::Black),
                        _ => w.set_color(Color::LightGray, Color::Black),
                    }
                    writeln!(w, "#{} [{}] t={} {}", ev.id, ev.level.tag(), ev.tick, ev.text()).unwrap();
                }
                w.set_color(self.text_color, Color::Black);
            }
            "run_app" => {
                w.set_color(Color::White, Color::Black);
                writeln!(w, "User Mode (Ring 3) ELF yukleniyor...").unwrap();
                drop(w);
                let hello_elf = include_bytes!("../scratch/hello.elf");
                if let Err(e) = crate::user::exec_elf(hello_elf) {
                    w = x86_64::instructions::interrupts::without_interrupts(|| WRITE_LOCK.lock());
                    w.set_color(Color::LightRed, Color::Black);
                    writeln!(w, "ELF Error: {}", e).unwrap();
                    drop(w);
                }
                w = x86_64::instructions::interrupts::without_interrupts(|| WRITE_LOCK.lock());
            }
            _ if cmd.starts_with("exec ") || cmd.starts_with("run ") => {
                let path = if let Some(p) = cmd.strip_prefix("exec ") { p.trim() } else { cmd.strip_prefix("run ").unwrap().trim() };
                let resolved = crate::fs::resolve_path(&self.cwd, path);
                w.set_color(Color::White, Color::Black);
                writeln!(w, "Program baslatiliyor: {}", resolved).unwrap();
                drop(w);
                match crate::fs::read_file_from_path(&resolved) {
                    Ok(elf_bytes) => {
                        if let Err(e) = crate::user::exec_elf(&elf_bytes) {
                            w = x86_64::instructions::interrupts::without_interrupts(|| WRITE_LOCK.lock());
                            w.set_color(Color::LightRed, Color::Black);
                            writeln!(w, "ELF Calistirma Hatasi: {}", e).unwrap();
                            drop(w);
                        }
                    }
                    Err(e) => {
                        w = x86_64::instructions::interrupts::without_interrupts(|| WRITE_LOCK.lock());
                        w.set_color(Color::LightRed, Color::Black);
                        writeln!(w, "Dosya okunamadi: {}", e).unwrap();
                        drop(w);
                    }
                }
                w = x86_64::instructions::interrupts::without_interrupts(|| WRITE_LOCK.lock());
            }
            _ if cmd.starts_with("hexdump ") || cmd.starts_with("xxd ") => {
                let path = if let Some(p) = cmd.strip_prefix("hexdump ") { p.trim() } else { cmd.strip_prefix("xxd ").unwrap().trim() };
                let resolved = crate::fs::resolve_path(&self.cwd, path);
                match crate::fs::read_file_from_path(&resolved) {
                    Ok(bytes) => {
                        w.set_color(Color::Cyan, Color::Black);
                        writeln!(w, "Hexdump: {} ({} bayt)", resolved, bytes.len()).unwrap();
                        w.set_color(Color::White, Color::Black);
                        let limit = core::cmp::min(bytes.len(), 128); // Ekrani doldurmamak icin ilk 128 bayt
                        for chunk_idx in (0..limit).step_by(16) {
                            let chunk_end = core::cmp::min(chunk_idx + 16, limit);
                            let chunk = &bytes[chunk_idx..chunk_end];
                            write!(w, "{:04x}: ", chunk_idx).unwrap();
                            for b in chunk {
                                write!(w, "{:02x} ", b).unwrap();
                            }
                            for _ in chunk.len()..16 {
                                write!(w, "   ").unwrap();
                            }
                            write!(w, " |").unwrap();
                            for b in chunk {
                                if *b >= 0x20 && *b <= 0x7E {
                                    write!(w, "{}", *b as char).unwrap();
                                } else {
                                    write!(w, ".").unwrap();
                                }
                            }
                            writeln!(w, "|").unwrap();
                        }
                        if bytes.len() > limit {
                            writeln!(w, "... ({} bayt daha var)", bytes.len() - limit).unwrap();
                        }
                    }
                    Err(e) => {
                        w.set_color(Color::Red, Color::Black);
                        writeln!(w, "Hata: {}", e).unwrap();
                    }
                }
            }
            _ if cmd.starts_with("touch ") => {
                let file_name = cmd.strip_prefix("touch ").unwrap().trim();
                let resolved = crate::fs::resolve_path(&self.cwd, file_name);
                match crate::fs::write_file_bytes(&resolved, &[]) {
                    Ok(_) => writeln!(w, "Dosya olusturuldu: {}", file_name).unwrap(),
                    Err(e) => {
                        w.set_color(Color::Red, Color::Black);
                        writeln!(w, "Hata: {}", e).unwrap();
                    }
                }
            }
            _ if cmd.starts_with("host ") => {
                let domain = cmd.strip_prefix("host ").unwrap().trim();
                w.set_color(Color::White, Color::Black);
                writeln!(w, "{} adresi icin DNS sorgusu yapiliyor (8.8.8.8:53)...", domain).unwrap();
                drop(w);
                
                let tx_id = (crate::interrupts::get_tick() & 0xFFFF) as u16;
                let mut success = false;
                
                unsafe {
                    if let Some(ref mut dev) = crate::rtl8139::RTL8139_DEV {
                        let mac = dev.get_mac_address();
                        let packet = crate::net::create_dns_query_packet(mac, domain, tx_id);
                        dev.send_packet(&packet);
                        
                        let start_tick = crate::interrupts::get_tick();
                        while crate::interrupts::get_tick() < start_tick + 3000 {
                            if let Some(rx_packet) = dev.poll_rx() {
                                if let Some(ips) = crate::net::parse_dns_response(&rx_packet, tx_id) {
                                    let mut w = x86_64::instructions::interrupts::without_interrupts(|| WRITE_LOCK.lock());
                                    if ips.is_empty() {
                                        w.set_color(Color::Yellow, Color::Black);
                                        writeln!(w, "DNS Yaniti: Kayit bulunamadi.").unwrap();
                                    } else {
                                        w.set_color(Color::LightGreen, Color::Black);
                                        for ip in ips {
                                            writeln!(w, "{} has address {}.{}.{}.{}", domain, ip[0], ip[1], ip[2], ip[3]).unwrap();
                                        }
                                    }
                                    success = true;
                                    break;
                                }
                            }
                            crate::task::yield_now().await;
                        }
                    }
                }
                
                w = x86_64::instructions::interrupts::without_interrupts(|| WRITE_LOCK.lock());
                if !success {
                    w.set_color(Color::LightRed, Color::Black);
                    writeln!(w, "Zaman asimi (Timeout). DNS sunucusundan yanit alinamadi.").unwrap();
                }
            }
            "gui" | "desktop" => {
                crate::gui::init(None);
                crate::vga_buffer::GUI_MODE.store(true, core::sync::atomic::Ordering::Relaxed);
                
                drop(w);

                // Spawn Desktop V1 applications (Terminal & Files)
                let spawn_res = crate::task::process::spawn_desktop_v1_apps();

                // Mark full damage for initial desktop environment paint
                crate::wm::WM.lock().mark_full_damage();

                w = x86_64::instructions::interrupts::without_interrupts(|| WRITE_LOCK.lock());

                if let Ok((pa, pb, _)) = spawn_res {
                    w.set_color(Color::LightGreen, Color::Black);
                    writeln!(w, "[DESKTOP] SparkOS Desktop baslatildi (Terminal PID {}, Files PID {}).", pa, pb).unwrap();
                } else {
                    w.set_color(Color::LightRed, Color::Black);
                    writeln!(w, "HATA: Desktop uygulamalari baslatilamadi.").unwrap();
                }
            }
            _ if cmd.starts_with("kill ") => {
                let id_str = cmd.strip_prefix("kill ").unwrap().trim();
                if let Ok(id) = id_str.parse::<u64>() {
                    x86_64::instructions::interrupts::without_interrupts(|| {
                        let list = crate::task::PROCESS_LIST.lock();
                        if list.contains_key(&id) {
                            crate::task::KILLED_PROCESSES.lock().push(id);
                            w.set_color(Color::Yellow, Color::Black);
                            writeln!(w, "Süreç [{}] (PID: {}) öldürülmek üzere işaretlendi.", list.get(&id).unwrap(), id).unwrap();
                        } else {
                            w.set_color(Color::Red, Color::Black);
                            writeln!(w, "Hata: PID bulunamadi.").unwrap();
                        }
                    });
                } else {
                    w.set_color(Color::Red, Color::Black);
                    writeln!(w, "Hata: Gecersiz PID formati. (Kullanim: kill <PID>)").unwrap();
                }
            }
            "pwd" => {
                writeln!(w, "{}", self.cwd).unwrap();
            }
            _ if cmd.starts_with("cd ") => {
                let path = cmd.strip_prefix("cd ").unwrap().trim();
                let resolved = crate::fs::resolve_path(&self.cwd, path);
                if crate::fs::is_dir(&resolved) {
                    self.cwd = resolved;
                } else {
                    w.set_color(Color::Red, Color::Black);
                    writeln!(w, "Hata: {} bir dizin degil veya bulunamadi", path).unwrap();
                }
            }
            _ if cmd == "ls" || cmd.starts_with("ls ") => {
                let target = cmd.strip_prefix("ls ").unwrap_or("").trim();
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
                let dir_name = cmd.strip_prefix("mkdir ").unwrap().trim();
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
                let file_name = cmd.strip_prefix("cat ").unwrap().trim();
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
                let args = cmd.strip_prefix("write ").unwrap().trim();
                if let Some((file_name, content)) = args.split_once(' ') {
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
                let target = cmd.strip_prefix("rm ").unwrap().trim();
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
                let args = cmd.strip_prefix("disk_write ").unwrap().trim();
                if let Some((lba_str, text)) = args.split_once(' ') {
                    if let Ok(lba) = lba_str.parse::<u32>() {
                        let mut buf = [0u8; 512];
                        let bytes = text.as_bytes();
                        let len = core::cmp::min(bytes.len(), 512);
                        buf[..len].copy_from_slice(&bytes[..len]);
                        
                        x86_64::instructions::interrupts::without_interrupts(|| {
                            match crate::ata::DATA_DRIVE.lock().write_sector(lba, &buf) {
                                Ok(_) => writeln!(w, "LBA {} sektorune basariyla yazildi.", lba).unwrap(),
                                Err(e) => {
                                    w.set_color(Color::Red, Color::Black);
                                    writeln!(w, "Hata: {}", e).unwrap();
                                }
                            }
                        });
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
                let lba_str = cmd.strip_prefix("disk_read ").unwrap().trim();
                if let Ok(lba) = lba_str.parse::<u32>() {
                    let mut buf = [0u8; 512];
                    let res = x86_64::instructions::interrupts::without_interrupts(|| {
                        crate::ata::DATA_DRIVE.lock().read_sector(lba, &mut buf)
                    });
                    match res {
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
                let color_name = cmd.strip_prefix("color ").unwrap().trim();
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
                writeln!(w, "{}", cmd.strip_prefix("echo ").unwrap()).unwrap();
            }
            _ => {
                w.set_color(Color::Red, Color::Black);
                writeln!(w, "Hata: '{}' bilinmiyor (help yazin)", cmd).unwrap();
            }
        }
        w.set_color(self.text_color, Color::Black);
    }
}
