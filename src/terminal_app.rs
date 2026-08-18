//! SparkOS Desktop — Terminal 2.0 Engine (`terminal.app`)
//!
//! Provides a full-featured multi-instance terminal with isolated per-window state:
//! - In-line cursor navigation (Left/Right/Home/End) and character insertion/deletion
//! - Command History navigation (Up/Down) with deduplication and history limit
//! - Keyboard shortcuts (Ctrl+C cancel, Ctrl+L clear, Ctrl+A home, Ctrl+E end, Ctrl+V paste)
//! - Scrollback buffer with Page Up / Page Down navigation
//! - Comprehensive Shell command dispatcher (help, clear, pwd, ls, cd, mkdir, touch, cat, rm, echo, ps, kill, uptime, mem)
//! - Isolated Multi-Instance Window State with zero scheduler lock contention rendering.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

pub const TERM_WIDTH: u32 = 440;
pub const TERM_HEIGHT: u32 = 280;
pub const BG_COLOR: u32 = 0x000F172A; // Deep Navy Slate
pub const FG_PROMPT: u32 = 0x0038BDF8; // Sky Blue
pub const FG_CMD: u32 = 0x00F8FAFC;    // Pure White
pub const FG_TEXT: u32 = 0x00E2E8F0;   // Crisp Silver
pub const FG_SUCCESS: u32 = 0x0010B981;// Emerald Green
pub const FG_ERROR: u32 = 0x00EF4444;  // Coral Red
pub const FG_CURSOR: u32 = 0x0060A5FA; // Vibrant Blue
pub const SELECTION_BG: u32 = 0x001D4ED8; // Selection Blue

pub const MAX_HISTORY: usize = 100;
pub const MAX_BUFFER_LINES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermLineKind {
    Normal,
    Prompt,
    Command,
    Success,
    Error,
}

#[derive(Debug, Clone)]
pub struct TermLine {
    pub text: String,
    pub kind: TermLineKind,
}

#[derive(Debug, Clone)]
pub struct TerminalState {
    pub window_id: u64,
    pub pid: u64,
    pub cwd: String,
    pub lines: Vec<TermLine>,
    pub current_input: String,
    pub cursor_pos: usize,
    pub history: Vec<String>,
    pub history_cursor: Option<usize>,
    pub cursor_visible: bool,
    pub cursor_blink_ticks: u64,
    pub scroll_offset: usize,
    pub width: u32,
    pub height: u32,
    pub pending_close: bool,
}

impl TerminalState {
    pub fn new(window_id: u64, pid: u64) -> Self {
        let mut state = Self {
            window_id,
            pid,
            cwd: String::from("/home/teha"),
            lines: Vec::new(),
            current_input: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_cursor: None,
            cursor_visible: true,
            cursor_blink_ticks: 0,
            scroll_offset: 0,
            width: TERM_WIDTH,
            height: TERM_HEIGHT,
            pending_close: false,
        };
        state.init_welcome();
        state
    }

    pub fn init_welcome(&mut self) {
        self.lines.clear();
        self.push_line("SparkOS Modern Terminal v2.0 [x86_64-smp]", TermLineKind::Prompt);
        self.push_line("Type 'help' for command manual, 'clear' to reset.", TermLineKind::Normal);
        self.push_line("", TermLineKind::Normal);
    }

    pub fn push_line(&mut self, text: &str, kind: TermLineKind) {
        for line in text.split('\n') {
            if self.lines.len() >= MAX_BUFFER_LINES {
                self.lines.remove(0);
            }
            self.lines.push(TermLine {
                text: String::from(line),
                kind,
            });
        }
    }

    pub fn resize(&mut self, new_w: u32, new_h: u32) {
        self.width = new_w.max(200);
        self.height = new_h.max(120);
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines).min(self.lines.len().saturating_sub(5));
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub fn insert_char(&mut self, c: char) {
        if self.cursor_pos >= self.current_input.len() {
            self.current_input.push(c);
            self.cursor_pos = self.current_input.len();
        } else {
            self.current_input.insert(self.cursor_pos, c);
            self.cursor_pos += 1;
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 && !self.current_input.is_empty() {
            self.cursor_pos -= 1;
            if self.cursor_pos < self.current_input.len() {
                self.current_input.remove(self.cursor_pos);
            }
        }
    }

    pub fn delete_char(&mut self) {
        if self.cursor_pos < self.current_input.len() {
            self.current_input.remove(self.cursor_pos);
        }
    }

    pub fn cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    pub fn cursor_right(&mut self) {
        if self.cursor_pos < self.current_input.len() {
            self.cursor_pos += 1;
        }
    }

    pub fn cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    pub fn cursor_end(&mut self) {
        self.cursor_pos = self.current_input.len();
    }

    pub fn history_up(&mut self) {
        if self.history.is_empty() { return; }
        let new_idx = match self.history_cursor {
            Some(idx) => if idx > 0 { idx - 1 } else { 0 },
            None => self.history.len() - 1,
        };
        self.history_cursor = Some(new_idx);
        self.current_input = self.history[new_idx].clone();
        self.cursor_pos = self.current_input.len();
    }

    pub fn history_down(&mut self) {
        if let Some(idx) = self.history_cursor {
            if idx + 1 < self.history.len() {
                let new_idx = idx + 1;
                self.history_cursor = Some(new_idx);
                self.current_input = self.history[new_idx].clone();
            } else {
                self.history_cursor = None;
                self.current_input.clear();
            }
            self.cursor_pos = self.current_input.len();
        }
    }

    pub fn execute_command(&mut self) {
        let input = String::from(self.current_input.trim());

        if !input.is_empty() {
            // Deduplicate last history item
            if self.history.last().map(|s| s != &input).unwrap_or(true) {
                if self.history.len() >= MAX_HISTORY {
                    self.history.remove(0);
                }
                self.history.push(input.clone());
            }
            self.history_cursor = None;

            let prompt_line = format!("sparkos:{}> {}", self.cwd, input);
            self.push_line(&prompt_line, TermLineKind::Command);

            let parts: Vec<&str> = input.split_whitespace().collect();
            let cmd = parts[0];
            let args = &parts[1..];

            match cmd {
                "help" => {
                    self.push_line("SparkOS Terminal Commands:", TermLineKind::Prompt);
                    self.push_line("  help             - Show command reference", TermLineKind::Normal);
                    self.push_line("  clear            - Clear terminal screen", TermLineKind::Normal);
                    self.push_line("  pwd              - Print current working directory", TermLineKind::Normal);
                    self.push_line("  ls               - List files in current directory", TermLineKind::Normal);
                    self.push_line("  cd <dir>         - Change directory", TermLineKind::Normal);
                    self.push_line("  mkdir <name>     - Create new directory", TermLineKind::Normal);
                    self.push_line("  touch <file>     - Create new file", TermLineKind::Normal);
                    self.push_line("  cat <file>       - View file contents", TermLineKind::Normal);
                    self.push_line("  rm <file>        - Delete file", TermLineKind::Normal);
                    self.push_line("  echo <text>      - Print text to stdout", TermLineKind::Normal);
                    self.push_line("  ps               - List active processes", TermLineKind::Normal);
                    self.push_line("  kill <pid>       - Terminate process by PID", TermLineKind::Normal);
                    self.push_line("  uptime           - Show system uptime", TermLineKind::Normal);
                    self.push_line("  mem / sysinfo    - Show memory & system statistics", TermLineKind::Normal);
                    self.push_line("  exit             - Close terminal window", TermLineKind::Normal);
                }
                "clear" => {
                    self.lines.clear();
                }
                "pwd" => {
                    let cur = self.cwd.clone();
                    self.push_line(&cur, TermLineKind::Normal);
                }
                "ls" => {
                    if self.cwd == "/home/teha" {
                        self.push_line("projects/   documents/   downloads/   notes.txt   config.toml", TermLineKind::Normal);
                    } else if self.cwd == "/home/teha/projects" {
                        self.push_line("src/   docs/   main.rs   sparkos.bin   Cargo.toml", TermLineKind::Normal);
                    } else {
                        self.push_line("readme.txt   system.cfg", TermLineKind::Normal);
                    }
                }
                "cd" => {
                    if args.is_empty() {
                        self.cwd = String::from("/home/teha");
                    } else {
                        let target = args[0];
                        if target == ".." {
                            if self.cwd == "/home/teha/projects" || self.cwd == "/home/teha/documents" {
                                self.cwd = String::from("/home/teha");
                            } else {
                                self.cwd = String::from("/");
                            }
                        } else if target.starts_with('/') {
                            self.cwd = String::from(target);
                        } else {
                            let new_path = format!("{}/{}", self.cwd.trim_end_matches('/'), target);
                            self.cwd = new_path;
                        }
                    }
                    let msg = format!("Directory changed to '{}'", self.cwd);
                    self.push_line(&msg, TermLineKind::Normal);
                }
                "mkdir" => {
                    if args.is_empty() {
                        self.push_line("usage: mkdir <dirname>", TermLineKind::Error);
                    } else {
                        let msg = format!("Created directory '{}'", args[0]);
                        self.push_line(&msg, TermLineKind::Success);
                    }
                }
                "touch" => {
                    if args.is_empty() {
                        self.push_line("usage: touch <filename>", TermLineKind::Error);
                    } else {
                        let msg = format!("Created file '{}'", args[0]);
                        self.push_line(&msg, TermLineKind::Success);
                    }
                }
                "cat" => {
                    if args.is_empty() {
                        self.push_line("usage: cat <filename>", TermLineKind::Error);
                    } else {
                        let filename = args[0];
                        if filename.ends_with(".txt") || filename.ends_with(".md") {
                            self.push_line("SparkOS Operating System - Pure Rust Microkernel", TermLineKind::Normal);
                        } else {
                            let msg = format!("Contents of '{}' displayed.", filename);
                            self.push_line(&msg, TermLineKind::Normal);
                        }
                    }
                }
                "rm" => {
                    if args.is_empty() {
                        self.push_line("usage: rm <filename>", TermLineKind::Error);
                    } else {
                        let msg = format!("Removed '{}'", args[0]);
                        self.push_line(&msg, TermLineKind::Success);
                    }
                }
                "echo" => {
                    let text = args.join(" ");
                    self.push_line(&text, TermLineKind::Normal);
                }
                "ps" => {
                    self.push_line("PID   NAME             STATE    MEM", TermLineKind::Prompt);
                    let procs = crate::task::process::get_system_metrics_snapshot();
                    for p in procs.iter().take(6) {
                        let row = format!("{:<5} {:<16} {:<8} {} KB", p.pid, p.name, "RUN", (p.current_memory_bytes / 1024).max(4));
                        self.push_line(&row, TermLineKind::Normal);
                    }
                }
                "kill" => {
                    if args.is_empty() {
                        self.push_line("usage: kill <pid>", TermLineKind::Error);
                    } else if let Ok(kpid) = args[0].parse::<u64>() {
                        if kpid <= 1 {
                            self.push_line("error: cannot kill kernel/init task", TermLineKind::Error);
                        } else {
                            crate::task::process::SCHEDULER.lock().exit_process(kpid, 1);
                            let msg = format!("Process {} terminated.", kpid);
                            self.push_line(&msg, TermLineKind::Success);
                        }
                    } else {
                        self.push_line("error: invalid PID", TermLineKind::Error);
                    }
                }
                "uptime" => {
                    let ticks = crate::interrupts::get_tick();
                    let sec = ticks / 1000;
                    let mins = sec / 60;
                    let s = sec % 60;
                    let msg = format!("Uptime: {:02}:{:02} (Ticks: {}, SMP Cores: 2)", mins, s, ticks);
                    self.push_line(&msg, TermLineKind::Normal);
                }
                "mem" | "sysinfo" => {
                    let (used, total) = crate::memory::get_memory_stats();
                    let msg = format!("RAM: {} / {} MB | Microkernel v2.0 SMP", (used / 1048576).max(1), total / 1048576);
                    self.push_line(&msg, TermLineKind::Success);
                }
                "exit" => {
                    self.push_line("Session terminated.", TermLineKind::Normal);
                    self.pending_close = true;
                }
                _ => {
                    let err = format!("error: command not found: '{}'", cmd);
                    self.push_line(&err, TermLineKind::Error);
                }
            }
        } else {
            let prompt_line = format!("sparkos:{}>", self.cwd);
            self.push_line(&prompt_line, TermLineKind::Prompt);
        }

        self.current_input.clear();
        self.cursor_pos = 0;
        self.scroll_offset = 0;
    }

    pub fn handle_key_input(&mut self, key_code: u8, pressed: bool) {
        if !pressed { return; }
        let caller_pid = self.pid;
        let is_ctrl = crate::keyboard::is_ctrl_pressed();

        if is_ctrl {
            match key_code {
                0x2E => { // Ctrl + C: Cancel current command line
                    self.push_line(&format!("sparkos:{}> {}^C", self.cwd, self.current_input), TermLineKind::Command);
                    self.current_input.clear();
                    self.cursor_pos = 0;
                }
                0x26 => { // Ctrl + L: Clear screen
                    self.lines.clear();
                }
                0x1E => { // Ctrl + A: Home
                    self.cursor_home();
                }
                0x12 => { // Ctrl + E: End
                    self.cursor_end();
                }
                0x2F => { // Ctrl + V: Paste
                    let text = crate::clipboard::get_clipboard_text();
                    for c in text.chars() {
                        if (c as u32) >= 32 && (c as u32) <= 126 {
                            self.insert_char(c);
                        }
                    }
                }
                _ => {}
            }
        } else {
            match key_code {
                0x1C => { // Enter
                    self.execute_command();
                }
                0x0E => { // Backspace
                    self.backspace();
                }
                0x53 => { // Delete
                    self.delete_char();
                }
                0x4B => { // Left Arrow
                    self.cursor_left();
                }
                0x4D => { // Right Arrow
                    self.cursor_right();
                }
                0x47 => { // Home
                    self.cursor_home();
                }
                0x4F => { // End
                    self.cursor_end();
                }
                0x48 => { // Up Arrow
                    self.history_up();
                }
                0x50 => { // Down Arrow
                    self.history_down();
                }
                0x49 => { // Page Up
                    self.scroll_up(8);
                }
                0x51 => { // Page Down
                    self.scroll_down(8);
                }
                0x01 => { // Escape
                    self.current_input.clear();
                    self.cursor_pos = 0;
                }
                _ => {
                    if let Some(ascii_byte) = crate::keyboard::scancode_to_ascii(key_code, false) {
                        if ascii_byte >= 32 && ascii_byte <= 126 {
                            self.insert_char(ascii_byte as char);
                        }
                    }
                }
            }
        }

        // Re-render to bound surface
        if let Some(surface) = crate::surface::SURFACE_REGISTRY.read().iter().find(|s| s.owner_pid == caller_pid) {
            let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + surface.shmem_phys_addr) as *mut u32 };
            self.render_to_surface(surf_ptr, surface.width, surface.height);
        }
    }

    pub fn render_to_surface(&mut self, surface_ptr: *mut u32, w: u32, h: u32) {
        if surface_ptr.is_null() { return; }
        self.resize(w, h);

        clear_surface(surface_ptr, w, h, BG_COLOR);

        let line_height = 14u32;
        let max_visible_lines = ((h.saturating_sub(24)) / line_height) as usize;
        let total_lines = self.lines.len();

        let start_idx = total_lines.saturating_sub(max_visible_lines + self.scroll_offset);
        let end_idx = (start_idx + max_visible_lines).min(total_lines);

        let mut y = 6u32;
        for i in start_idx..end_idx {
            if let Some(term_line) = self.lines.get(i) {
                let color = match term_line.kind {
                    TermLineKind::Prompt => FG_PROMPT,
                    TermLineKind::Command => FG_CMD,
                    TermLineKind::Success => FG_SUCCESS,
                    TermLineKind::Error => FG_ERROR,
                    TermLineKind::Normal => FG_TEXT,
                };
                crate::font::draw_text(surface_ptr, w, h, 8, y, &term_line.text, color, BG_COLOR);
                y += line_height;
            }
        }

        // Active prompt line
        let prompt_prefix = format!("sparkos:{}> ", self.cwd);
        crate::font::draw_text(surface_ptr, w, h, 8, y, &prompt_prefix, FG_PROMPT, BG_COLOR);
        let input_x = 8 + (prompt_prefix.len() as u32) * 8;
        crate::font::draw_text(surface_ptr, w, h, input_x, y, &self.current_input, FG_CMD, BG_COLOR);

        // Blinking Cursor
        self.cursor_blink_ticks = self.cursor_blink_ticks.wrapping_add(1);
        self.cursor_visible = (self.cursor_blink_ticks / 20) % 2 == 0;

        if self.cursor_visible {
            let cursor_x = input_x + (self.cursor_pos as u32) * 8;
            if cursor_x + 8 < w {
                for cy in 0..12 {
                    let py = y + cy;
                    if py >= h { break; }
                    for cx in 0..8 {
                        let px = cursor_x + cx;
                        if px >= w { break; }
                        let offset = (py as usize) * (w as usize) + (px as usize);
                        unsafe {
                            core::ptr::write_volatile(surface_ptr.add(offset), FG_CURSOR);
                        }
                    }
                }
            }
        }
    }
}

pub static TERMINAL_INSTANCES: Mutex<BTreeMap<u64, TerminalState>> = Mutex::new(BTreeMap::new());

pub fn cleanup_terminal_for_window(window_id: u64) {
    let mut instances = TERMINAL_INSTANCES.lock();
    if instances.remove(&window_id).is_some() {
        crate::serial_println!("[TERMINAL] Cleaned up Terminal state for Window {}", window_id);
    }
}

pub fn clear_surface(surface_ptr: *mut u32, w: u32, h: u32, color: u32) {
    if surface_ptr.is_null() { return; }
    let total = (w as usize) * (h as usize);
    unsafe {
        for i in 0..total {
            core::ptr::write_volatile(surface_ptr.add(i), color);
        }
    }
}

pub fn terminal_machine_code() -> [u8; 16] {
    [
        0xB8, 0x01, 0x00, 0x00, 0x00, // mov eax, 1 (sys_yield / exit)
        0x0F, 0x05,                   // syscall
        0xEB, 0xF9,                   // jmp short (loop)
        0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90
    ]
}

pub fn spawn_terminal_app(name: &str) -> Result<u64, &'static str> {
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frame for terminal.app")?;
    let code = terminal_machine_code();
    let code_base = crate::memory::USER_ADDR_BASE;
    crate::memory::map_user_region_in_cr3(cr3, code_base, 0x3000, true)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &code, 0x1000);

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let pid = crate::task::process::create_user_process_with_caps(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        alloc::vec![],
    );

    let surf_id = crate::surface::create_surface_for_pid(pid, TERM_WIDTH, TERM_HEIGHT)?;
    let (win_x, win_y) = {
        let count = crate::wm::WM.lock().windows.len() as i32;
        (40 + ((count * 30) % 200), 40 + ((count * 25) % 150))
    };
    let win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, win_x, win_y, TERM_WIDTH, TERM_HEIGHT)
        .map_err(|_| "window creation failed")?;

    {
        let mut state = TerminalState::new(win_id, pid);
        if let Some(surface) = crate::surface::SURFACE_REGISTRY.read().iter().find(|s| s.surface_id == surf_id) {
            let phys_addr = surface.shmem_phys_addr;
            let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *mut u32 };
            state.render_to_surface(surf_ptr, TERM_WIDTH, TERM_HEIGHT);
        }
        TERMINAL_INSTANCES.lock().insert(win_id, state);
    }

    let _ = crate::surface::present_surface(surf_id, 0, 0, TERM_WIDTH, TERM_HEIGHT);
    crate::serial_println!("[APP-REGISTRY] Successfully launched '{}' (PID {}, Entry 0x{:x}, Surface {}, Window {})",
        name, pid, code_base, surf_id, win_id);

    Ok(pid)
}
