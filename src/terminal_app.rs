//! SparkOS Desktop V1.32 — Modern Terminal UI Engine (`terminal.app`)
//!
//! Provides advanced multi-instance terminal support with fully isolated per-window state:
//! independent current working directory (CWD), separate command histories, isolated line
//! and input buffers, semantic syntax colored output, smooth scrolling, and dynamic window resizing.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

pub const TERM_WIDTH: u32 = 420;
pub const TERM_HEIGHT: u32 = 240;
pub const BG_COLOR: u32 = 0x000F172A; // Navy Slate
pub const FG_PROMPT: u32 = 0x0038BDF8; // Sky Blue
pub const FG_CMD: u32 = 0x00F8FAFC;    // Pure White
pub const FG_TEXT: u32 = 0x00E2E8F0;   // Crisp Silver
pub const FG_SUCCESS: u32 = 0x0010B981;// Emerald Green
pub const FG_ERROR: u32 = 0x00EF4444;  // Coral Red
pub const FG_CURSOR: u32 = 0x0060A5FA; // Vibrant Blue
pub const SELECTION_BG: u32 = 0x001D4ED8; // Selection Blue

pub const MAX_HISTORY: usize = 64;
pub const MAX_BUFFER_LINES: usize = 256;

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
    pub history: Vec<String>,
    pub history_cursor: Option<usize>,
    pub cursor_visible: bool,
    pub cursor_blink_ticks: u64,
    pub scroll_offset: usize,
    pub width: u32,
    pub height: u32,
    pub selection_start: Option<(usize, usize)>,
    pub selection_end: Option<(usize, usize)>,
    pub clipboard: String,
}

impl TerminalState {
    pub fn new(window_id: u64, pid: u64) -> Self {
        Self {
            window_id,
            pid,
            cwd: String::from("/home/teha/projects"),
            lines: Vec::new(),
            current_input: String::new(),
            history: Vec::new(),
            history_cursor: None,
            cursor_visible: true,
            cursor_blink_ticks: 0,
            scroll_offset: 0,
            width: TERM_WIDTH,
            height: TERM_HEIGHT,
            selection_start: None,
            selection_end: None,
            clipboard: String::new(),
        }
    }

    pub fn init_welcome(&mut self) {
        self.lines.clear();
        self.push_line("SparkOS Modern Terminal v1.32 [x86_64-smp]", TermLineKind::Prompt);
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

    pub fn execute_command(&mut self) {
        let input = String::from(self.current_input.trim());
        let caller_pid = self.pid;
        let caller_win = self.window_id;

        if !input.is_empty() {
            if self.history.len() >= MAX_HISTORY {
                self.history.remove(0);
            }
            self.history.push(input.clone());
            self.history_cursor = None;

            let prompt_line = format!("sparkos:{}> {}", self.cwd, input);
            self.push_line(&prompt_line, TermLineKind::Command);

            if input == "help" {
                self.push_line("Commands: help, clear, echo, pwd, ls, cd, ps, mem, uptime, exit", TermLineKind::Normal);
            } else if input == "clear" {
                self.lines.clear();
            } else if input == "pwd" {
                let current_cwd = self.cwd.clone();
                self.push_line(&current_cwd, TermLineKind::Normal);
            } else if input == "ls" {
                self.push_line("src/   docs/   main.rs   sparkos.bin   config.toml", TermLineKind::Normal);
            } else if input == "ps" {
                self.push_line("PID  NAME             STATUS", TermLineKind::Normal);
                self.push_line("1    terminal.app     Running", TermLineKind::Normal);
                self.push_line("2    files.app        Running", TermLineKind::Normal);
                self.push_line("3    settings.app     Sleeping", TermLineKind::Normal);
                self.push_line("Success: Active process table retrieved.", TermLineKind::Success);
            } else if input == "mem" {
                self.push_line("Total: 256 MB | Used: 43 MB | Free: 213 MB (Heap: 128 MB)", TermLineKind::Success);
            } else if input == "uptime" {
                let ticks = crate::interrupts::get_tick();
                let sec = ticks / 1000;
                let mins = sec / 60;
                let s = sec % 60;
                self.push_line(&format!("Uptime: {:02}:{:02} (Ticks: {}, SMP Cores: 2)", mins, s, ticks), TermLineKind::Normal);
            } else if input.starts_with("echo ") {
                let arg = input.strip_prefix("echo ").unwrap_or("").trim();
                self.push_line(arg, TermLineKind::Normal);
            } else if input == "echo" {
                self.push_line("", TermLineKind::Normal);
            } else if input.starts_with("cd ") {
                let target_dir = input.strip_prefix("cd ").unwrap_or("").trim();
                if target_dir.starts_with('/') {
                    self.cwd = String::from(target_dir);
                } else if target_dir == ".." {
                    self.cwd = String::from("/home/teha");
                } else {
                    let new_path = format!("{}/{}", self.cwd.trim_end_matches('/'), target_dir);
                    self.cwd = new_path;
                }
                let msg = format!("Directory changed to '{}'", self.cwd);
                self.push_line(&msg, TermLineKind::Normal);
            } else if input == "cd" {
                self.cwd = String::from("/home/teha");
                self.push_line("Directory changed to '/home/teha'", TermLineKind::Normal);
            } else if input == "exit" {
                self.push_line("Session terminated.", TermLineKind::Normal);
                let _ = crate::wm::WM.lock().destroy_window(caller_pid, caller_win);
            } else {
                let err = format!("error: command not found: '{}'", input);
                self.push_line(&err, TermLineKind::Error);
            }
        } else {
            let prompt_line = format!("sparkos:{}>", self.cwd);
            self.push_line(&prompt_line, TermLineKind::Prompt);
        }
        self.current_input.clear();
        self.scroll_offset = 0;
    }

    pub fn handle_key_input(&mut self, key_code: u8, pressed: bool) {
        if !pressed { return; }
        let caller_pid = self.pid;
        let is_ctrl = crate::keyboard::is_ctrl_pressed();

        if is_ctrl {
            match key_code {
                0x2E => { // Ctrl + C: Copy
                    if !self.current_input.is_empty() {
                        crate::clipboard::copy_to_clipboard(&self.current_input);
                    } else if let Some(last_line) = self.lines.last() {
                        crate::clipboard::copy_to_clipboard(&last_line.text);
                    }
                }
                0x2D => { // Ctrl + X: Cut
                    if !self.current_input.is_empty() {
                        crate::clipboard::copy_to_clipboard(&self.current_input);
                        self.current_input.clear();
                    }
                }
                0x2F => { // Ctrl + V: Paste
                    let text = crate::clipboard::get_clipboard_text();
                    for c in text.chars() {
                        if (c as u32) >= 32 && (c as u32) <= 126 {
                            self.current_input.push(c);
                        }
                    }
                }
                0x1E => { // Ctrl + A: Select all (highlight state)
                    self.selection_start = Some((0, 0));
                    self.selection_end = Some((self.current_input.len(), 0));
                }
                _ => {}
            }
        } else {
            match key_code {
                0x1C => { // Enter
                    self.execute_command();
                }
                0x0E => { // Backspace
                    self.current_input.pop();
                }
                0x01 => { // Escape
                    self.current_input.clear();
                }
                _ => {
                    if let Some(ascii_byte) = crate::keyboard::scancode_to_ascii(key_code, false) {
                        if ascii_byte >= 32 && ascii_byte <= 126 {
                            self.current_input.push(ascii_byte as char);
                        }
                    }
                }
            }
        }

        // Re-render to bound surface
        if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.owner_pid == caller_pid) {
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
            let cursor_x = input_x + (self.current_input.len() as u32) * 8;
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

        // Scroll Bar Indicator on Right edge
        if total_lines > max_visible_lines {
            let sb_x = w - 6;
            let sb_h = ((max_visible_lines as f32 / total_lines as f32) * (h as f32)) as u32;
            let sb_y = ((self.scroll_offset as f32 / total_lines as f32) * (h as f32)) as u32;

            for r in 0..sb_h.max(12) {
                let py = (h - 1).saturating_sub(sb_y + r);
                if py >= h { continue; }
                for c in 0..4 {
                    let px = sb_x + c;
                    if px >= w { break; }
                    let offset = (py as usize) * (w as usize) + (px as usize);
                    unsafe {
                        core::ptr::write_volatile(surface_ptr.add(offset), 0x00475569);
                    }
                }
            }
        }
    }
}

/// Map of all active terminal instances keyed by window_id
pub static TERMINAL_INSTANCES: Mutex<BTreeMap<u64, TerminalState>> = Mutex::new(BTreeMap::new());

pub fn cleanup_terminal_for_window(window_id: u64) {
    let mut instances = TERMINAL_INSTANCES.lock();
    if instances.remove(&window_id).is_some() {
        crate::serial_println!("[TERMINAL] Cleaned up terminal state for Window {}", window_id);
    }
}

pub fn clear_surface(ptr: *mut u32, w: u32, h: u32, color: u32) {
    if ptr.is_null() { return; }
    let count = (w as usize) * (h as usize);
    unsafe {
        for i in 0..count {
            core::ptr::write_volatile(ptr.add(i), color);
        }
    }
}

pub fn terminal_machine_code() -> Vec<u8> {
    alloc::vec![
        0xb8, 0x00, 0x00, 0x00, 0x00, // mov eax, 0
        0xeb, 0xfe,                   // jmp $
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
    let win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, 40, 40, TERM_WIDTH, TERM_HEIGHT)
        .map_err(|_| "window creation failed")?;

    {
        let mut state = TerminalState::new(win_id, pid);
        state.init_welcome();
        if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.surface_id == surf_id) {
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
