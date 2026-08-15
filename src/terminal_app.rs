//! SparkOS Desktop V1.16 — Ring-3 Terminal Application (`terminal.app`)
//!
//! Provides a dedicated Ring-3 process with its own CR3, Surface buffer, Window,
//! event processing, scrolling line buffer, command history, and decoupled Shell Service integration.

use alloc::string::String;
use alloc::vec::Vec;
use crate::font::FONT;

pub const TERM_WIDTH: u32 = 380;
pub const TERM_HEIGHT: u32 = 140;
pub const BG_COLOR: u32 = 0x000F172A; // Navy Slate
pub const FG_PROMPT: u32 = 0x0034D399; // Emerald Green
pub const FG_TEXT: u32 = 0x00E2E8F0;   // Crisp Silver
pub const FG_CURSOR: u32 = 0x0038BDF8; // Sky Blue
pub const MAX_HISTORY: usize = 32;
pub const MAX_BUFFER_LINES: usize = 128;

#[derive(Debug, Clone)]
pub struct TerminalState {
    pub lines: Vec<String>,
    pub current_input: String,
    pub history: Vec<String>,
    pub history_cursor: Option<usize>,
    pub cursor_visible: bool,
    pub cursor_blink_ticks: u64,
    pub scroll_offset: usize,
}

impl TerminalState {
    pub fn new() -> Self {
        let mut state = Self {
            lines: Vec::new(),
            current_input: String::new(),
            history: Vec::new(),
            history_cursor: None,
            cursor_visible: true,
            cursor_blink_ticks: 0,
            scroll_offset: 0,
        };
        state.lines.push(String::from("SparkOS Terminal v1.16"));
        state.lines.push(String::from("Type 'help' for available commands."));
        state.lines.push(String::from(""));
        state
    }

    pub fn push_line(&mut self, text: &str) {
        for line in text.split('\n') {
            if self.lines.len() >= MAX_BUFFER_LINES {
                self.lines.remove(0);
            }
            self.lines.push(String::from(line));
        }
    }

    pub fn execute_command(&mut self) {
        let input = self.current_input.trim();
        if !input.is_empty() {
            if self.history.len() >= MAX_HISTORY {
                self.history.remove(0);
            }
            self.history.push(String::from(input));
        }
        self.history_cursor = None;

        let prompt_line = alloc::format!("sparkos /> {}", self.current_input);
        self.push_line(&prompt_line);

        let resp = crate::shell_service::execute_command(&self.current_input);
        if resp.should_clear {
            self.lines.clear();
        } else if resp.output_len > 0 {
            if let Ok(out_str) = core::str::from_utf8(&resp.output[..resp.output_len]) {
                self.push_line(out_str);
            }
        }
        self.current_input.clear();
    }

    pub fn history_up(&mut self) {
        if self.history.is_empty() { return; }
        let next_idx = match self.history_cursor {
            Some(i) if i > 0 => i - 1,
            Some(_) => 0,
            None => self.history.len() - 1,
        };
        self.history_cursor = Some(next_idx);
        self.current_input = self.history[next_idx].clone();
    }

    pub fn history_down(&mut self) {
        if let Some(i) = self.history_cursor {
            if i + 1 < self.history.len() {
                let next_idx = i + 1;
                self.history_cursor = Some(next_idx);
                self.current_input = self.history[next_idx].clone();
            } else {
                self.history_cursor = None;
                self.current_input.clear();
            }
        }
    }

    pub fn render_to_surface(&mut self, surface_ptr: *mut u32, w: u32, h: u32) {
        if surface_ptr.is_null() { return; }
        clear_surface(surface_ptr, w, h, BG_COLOR);

        let visible_lines = (h / 12) as usize;
        let total_lines = self.lines.len() + 1; // +1 for current prompt line
        let start_line = total_lines.saturating_sub(visible_lines);

        let mut y = 6u32;
        for i in start_line..self.lines.len() {
            if y + 10 > h { break; }
            crate::font::draw_text(surface_ptr, w, h, 8, y, &self.lines[i], FG_TEXT, BG_COLOR);
            y += 12;
        }

        // Draw active prompt line
        if y + 10 <= h {
            crate::font::draw_text(surface_ptr, w, h, 8, y, "sparkos /> ", FG_PROMPT, BG_COLOR);
            let input_x = 8 + 11 * 8;
            crate::font::draw_text(surface_ptr, w, h, input_x, y, &self.current_input, FG_TEXT, BG_COLOR);

            // Blinking cursor
            let cursor_x = input_x + (self.current_input.len() as u32 * 8);
            if self.cursor_visible && cursor_x + 8 < w {
                render_glyph_to_surface(surface_ptr, w, h, cursor_x, y, '_', FG_CURSOR, BG_COLOR);
            }
        }
    }
}

/// Renders a single 8x8 character into the surface memory buffer.
pub fn render_glyph_to_surface(surface_ptr: *mut u32, surf_w: u32, surf_h: u32, x: u32, y: u32, c: char, fg: u32, bg: u32) {
    if surface_ptr.is_null() { return; }
    let ascii = (c as u32).min(127) as usize;
    let bitmap = &FONT[ascii];

    for row in 0..8u32 {
        let py = y + row;
        if py >= surf_h { break; }
        let b = bitmap[row as usize];

        for col in 0..8u32 {
            let px = x + col;
            if px >= surf_w { break; }

            let col_val = if (b & (1 << (7 - col))) != 0 { fg } else { bg };
            let offset = (py as usize) * (surf_w as usize) + (px as usize);
            unsafe {
                core::ptr::write_volatile(surface_ptr.add(offset), col_val);
            }
        }
    }
}

/// Clears the entire surface with background color.
pub fn clear_surface(surface_ptr: *mut u32, surf_w: u32, surf_h: u32, bg: u32) {
    if surface_ptr.is_null() { return; }
    let total = (surf_w * surf_h) as usize;
    unsafe {
        for i in 0..total {
            core::ptr::write_volatile(surface_ptr.add(i), bg);
        }
    }
}

/// Emits x86-64 machine code for `terminal.app` running in Ring-3.
pub fn terminal_machine_code() -> Vec<u8> {
    let mut c: Vec<u8> = Vec::new();

    let loop_start = c.len();
    // 1. sys_poll_event(data_slot = 0x402000) -> syscall 39
    c.push(0xBF);
    c.extend_from_slice(&0x00402000u32.to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&39u32.to_le_bytes());
    c.push(0xCD);
    c.push(0x80);

    // 2. sys_yield() -> syscall 9
    c.push(0xB8);
    c.extend_from_slice(&9u32.to_le_bytes());
    c.push(0xCD);
    c.push(0x80);

    // 3. jmp loop_start (Persistent Ring-3 loop)
    let rel = (loop_start as i32 - (c.len() as i32 + 2)) as i8 as u8;
    c.push(0xEB);
    c.push(rel);

    c
}

/// Spawns a dedicated Ring-3 `terminal.app` process with its own CR3, Surface, and Window.
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
    let _win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, 40, 45, TERM_WIDTH, TERM_HEIGHT)
        .map_err(|_| "window creation failed")?;

    if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.surface_id == surf_id) {
        let phys_addr = surface.shmem_phys_addr;
        let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *mut u32 };
        let mut state = TerminalState::new();
        state.render_to_surface(surf_ptr, TERM_WIDTH, TERM_HEIGHT);
    }

    let _ = crate::surface::present_surface(surf_id, 0, 0, TERM_WIDTH, TERM_HEIGHT);
    crate::serial_println!("[APP-REGISTRY] Successfully launched '{}' (PID {}, Entry 0x{:x}, Surface {}, Window)",
        name, pid, code_base, surf_id);

    Ok(pid)
}
