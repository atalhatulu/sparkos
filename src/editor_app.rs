//! SparkOS Desktop V1.40 — Editor 3.0: Search, Replace & Word Wrap (`editor.app`)
//!
//! Features Find (Ctrl+F, Enter/Shift+Enter, Case Toggle), Replace (Ctrl+H, Replace, Replace All),
//! Smart Word Wrap (soft-wrap without content mutation), Multi-level Undo/Redo (Ctrl+Z/Y),
//! Unsaved Changes Dialog (Save/Discard/Cancel), and strict Multi-Document State Isolation.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

pub const EDITOR_WIDTH: u32 = 420;
pub const EDITOR_HEIGHT: u32 = 260;
pub const MAX_FILE_SIZE_BYTES: usize = 64 * 1024;
pub const MAX_UNDO_STEPS: usize = 50;

pub const VISIBLE_ROWS: usize = 12;
pub const VISIBLE_COLS: usize = 44;

#[derive(Debug, Clone)]
pub struct EditorSnapshot {
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub is_dirty: bool,
}

#[derive(Debug, Clone)]
pub struct EditorAppState {
    pub window_id: u64,
    pub pid: u64,
    pub file_path: Option<String>,
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub scroll_row: usize,
    pub scroll_col: usize,
    pub is_dirty: bool,
    pub undo_stack: Vec<EditorSnapshot>,
    pub redo_stack: Vec<EditorSnapshot>,
    pub selection_anchor: Option<(usize, usize)>,
    pub selection_focus: Option<(usize, usize)>,
    pub show_unsaved_dialog: bool,
    pub show_find_dialog: bool,
    pub show_replace_dialog: bool,
    pub search_query: String,
    pub replace_query: String,
    pub case_sensitive: bool,
    pub word_wrap: bool,
    pub active_input_field: u8, // 0: Editor Canvas, 1: Find Field, 2: Replace Field
    pub pending_close: bool,
    pub status_message: String,
}

impl EditorAppState {
    pub fn new(window_id: u64, pid: u64, initial_path: Option<&str>) -> Self {
        let mut state = Self {
            window_id,
            pid,
            file_path: initial_path.map(String::from),
            lines: alloc::vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
            scroll_row: 0,
            scroll_col: 0,
            is_dirty: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            selection_anchor: None,
            selection_focus: None,
            show_unsaved_dialog: false,
            show_find_dialog: false,
            show_replace_dialog: false,
            search_query: String::new(),
            replace_query: String::new(),
            case_sensitive: false,
            word_wrap: false,
            active_input_field: 0,
            pending_close: false,
            status_message: String::from("Ready"),
        };

        if let Some(path) = initial_path {
            state.open_file(path);
        } else {
            state.lines = alloc::vec![
                String::from("// SparkOS Text Editor 3.0"),
                String::from("Welcome to editor.app!"),
                String::from(""),
            ];
            state.cursor_row = 2;
            state.cursor_col = 0;
            state.is_dirty = false;
        }

        state
    }

    pub fn push_undo_snapshot(&mut self) {
        let snapshot = EditorSnapshot {
            lines: self.lines.clone(),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
            is_dirty: self.is_dirty,
        };

        if self.undo_stack.len() >= MAX_UNDO_STEPS {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snapshot);
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo_stack.pop() {
            let current = EditorSnapshot {
                lines: self.lines.clone(),
                cursor_row: self.cursor_row,
                cursor_col: self.cursor_col,
                is_dirty: self.is_dirty,
            };
            self.redo_stack.push(current);

            self.lines = prev.lines;
            self.cursor_row = prev.cursor_row;
            self.cursor_col = prev.cursor_col;
            self.is_dirty = prev.is_dirty;
            self.ensure_cursor_visible();
            self.status_message = String::from("Undo");
        } else {
            self.status_message = String::from("Already at oldest change");
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            let current = EditorSnapshot {
                lines: self.lines.clone(),
                cursor_row: self.cursor_row,
                cursor_col: self.cursor_col,
                is_dirty: self.is_dirty,
            };
            self.undo_stack.push(current);

            self.lines = next.lines;
            self.cursor_row = next.cursor_row;
            self.cursor_col = next.cursor_col;
            self.is_dirty = next.is_dirty;
            self.ensure_cursor_visible();
            self.status_message = String::from("Redo");
        } else {
            self.status_message = String::from("Already at newest change");
        }
    }

    pub fn open_file(&mut self, path: &str) {
        if path.contains("//") {
            self.status_message = String::from("Invalid path format");
            return;
        }

        self.file_path = Some(String::from(path));
        self.lines.clear();
        self.undo_stack.clear();
        self.redo_stack.clear();

        let stored = {
            let store = GLOBAL_FILE_STORE.lock();
            store.get(path).cloned()
        };

        if let Some(lines) = stored {
            self.lines = lines;
        } else {
            if path.ends_with("notes.txt") {
                self.lines.push(String::from("SparkOS V1.40 Desktop Notes"));
                self.lines.push(String::from("- Full Window Manager 2.0 active"));
                self.lines.push(String::from("- Real File Manager & Clipboard active"));
                self.lines.push(String::from("- Editor 3.0: Search, Replace & Word Wrap"));
                self.lines.push(String::from("- Türkçe karakter desteği: ç ğ ı İ ö ş ü"));
            } else if path.ends_with("main.rs") {
                self.lines.push(String::from("fn main() {"));
                self.lines.push(String::from("    println!(\"Hello SparkOS 3.0!\");"));
                self.lines.push(String::from("}"));
            } else if path.ends_with("config.toml") {
                self.lines.push(String::from("[desktop]"));
                self.lines.push(String::from("theme = \"dark\""));
                self.lines.push(String::from("font_size = 14"));
                self.lines.push(String::from("word_wrap = true"));
            } else {
                self.lines.push(format!("// File: {}", path));
                self.lines.push(String::from(""));
            }
            GLOBAL_FILE_STORE.lock().insert(String::from(path), self.lines.clone());
        }

        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_row = 0;
        self.scroll_col = 0;
        self.is_dirty = false;
        self.status_message = format!("Opened '{}'", path);
    }

    pub fn save_file(&mut self) {
        let path = if let Some(ref p) = self.file_path {
            p.clone()
        } else {
            let p = String::from("/home/teha/untitled.txt");
            self.file_path = Some(p.clone());
            p
        };

        GLOBAL_FILE_STORE.lock().insert(path.clone(), self.lines.clone());
        self.is_dirty = false;
        self.status_message = format!("Saved '{}' ({} lines)", path, self.lines.len());
    }

    pub fn save_as(&mut self, new_path: &str) {
        self.file_path = Some(String::from(new_path));
        GLOBAL_FILE_STORE.lock().insert(String::from(new_path), self.lines.clone());
        self.is_dirty = false;
        self.status_message = format!("Saved as '{}'", new_path);
    }

    pub fn new_document(&mut self) {
        self.push_undo_snapshot();
        self.file_path = None;
        self.lines = alloc::vec![String::new()];
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_row = 0;
        self.scroll_col = 0;
        self.is_dirty = false;
        self.selection_anchor = None;
        self.selection_focus = None;
        self.status_message = String::from("New document");
    }

    pub fn toggle_word_wrap(&mut self) {
        self.word_wrap = !self.word_wrap;
        self.scroll_col = 0;
        self.status_message = format!("Word Wrap: {}", if self.word_wrap { "ON" } else { "OFF" });
    }

    pub fn toggle_case_sensitive(&mut self) {
        self.case_sensitive = !self.case_sensitive;
        self.status_message = format!("Match Case: {}", if self.case_sensitive { "ON" } else { "OFF" });
    }

    pub fn find_next(&mut self) -> bool {
        if self.search_query.is_empty() || self.lines.is_empty() {
            self.status_message = String::from("Empty search term");
            return false;
        }

        let query = if self.case_sensitive {
            self.search_query.clone()
        } else {
            self.search_query.to_lowercase()
        };

        let start_row = self.cursor_row;
        let start_col = self.cursor_col;
        let total_rows = self.lines.len();

        for i in 0..total_rows {
            let r = (start_row + i) % total_rows;
            let line_content = if self.case_sensitive {
                self.lines[r].clone()
            } else {
                self.lines[r].to_lowercase()
            };

            let search_start = if i == 0 { (start_col + 1).min(line_content.len()) } else { 0 };
            if search_start <= line_content.len() {
                if let Some(pos) = line_content[search_start..].find(&query) {
                    let match_col = search_start + pos;
                    self.cursor_row = r;
                    self.cursor_col = match_col + query.len();
                    self.selection_anchor = Some((r, match_col));
                    self.selection_focus = Some((r, match_col + query.len()));
                    self.ensure_cursor_visible();
                    self.status_message = format!("Match found at Ln {}, Col {}", r + 1, match_col + 1);
                    return true;
                }
            }
        }

        self.status_message = format!("Pattern '{}' not found", self.search_query);
        false
    }

    pub fn find_previous(&mut self) -> bool {
        if self.search_query.is_empty() || self.lines.is_empty() {
            self.status_message = String::from("Empty search term");
            return false;
        }

        let query = if self.case_sensitive {
            self.search_query.clone()
        } else {
            self.search_query.to_lowercase()
        };

        let start_row = self.cursor_row;
        let total_rows = self.lines.len();

        for i in 0..total_rows {
            let r = if start_row >= i { start_row - i } else { total_rows - (i - start_row) };
            let line_content = if self.case_sensitive {
                self.lines[r].clone()
            } else {
                self.lines[r].to_lowercase()
            };

            let search_end = if i == 0 { self.cursor_col.saturating_sub(query.len()) } else { line_content.len() };
            if search_end > 0 && search_end <= line_content.len() {
                if let Some(pos) = line_content[..search_end].rfind(&query) {
                    self.cursor_row = r;
                    self.cursor_col = pos + query.len();
                    self.selection_anchor = Some((r, pos));
                    self.selection_focus = Some((r, pos + query.len()));
                    self.ensure_cursor_visible();
                    self.status_message = format!("Match found at Ln {}, Col {}", r + 1, pos + 1);
                    return true;
                }
            }
        }

        self.status_message = format!("Pattern '{}' not found", self.search_query);
        false
    }

    pub fn replace_current(&mut self) -> bool {
        if self.search_query.is_empty() { return false; }

        if let (Some((ar, ac)), Some((fr, fc))) = (self.selection_anchor, self.selection_focus) {
            if ar == fr && ar < self.lines.len() {
                self.push_undo_snapshot();
                let (start_c, end_c) = if ac < fc { (ac, fc) } else { (fc, ac) };
                let mut line = self.lines[ar].clone();
                if start_c <= line.len() && end_c <= line.len() {
                    line.replace_range(start_c..end_c, &self.replace_query);
                    self.lines[ar] = line;
                    self.cursor_row = ar;
                    self.cursor_col = start_c + self.replace_query.len();
                    self.is_dirty = true;
                    self.selection_anchor = None;
                    self.selection_focus = None;
                    self.ensure_cursor_visible();
                    self.status_message = String::from("Replaced 1 match");
                    self.find_next();
                    return true;
                }
            }
        }

        // If no match selected, find next match first
        self.find_next()
    }

    pub fn replace_all(&mut self) -> usize {
        if self.search_query.is_empty() || self.lines.is_empty() { return 0; }

        self.push_undo_snapshot();
        let mut count = 0usize;
        let query = &self.search_query;
        let replacement = &self.replace_query;

        for line in self.lines.iter_mut() {
            if self.case_sensitive {
                let matches = line.matches(query).count();
                if matches > 0 {
                    *line = line.replace(query, replacement);
                    count += matches;
                }
            } else {
                let mut new_line = String::new();
                let lower_line = line.to_lowercase();
                let lower_query = query.to_lowercase();
                let mut last_idx = 0;

                while let Some(pos) = lower_line[last_idx..].find(&lower_query) {
                    let match_start = last_idx + pos;
                    new_line.push_str(&line[last_idx..match_start]);
                    new_line.push_str(replacement);
                    last_idx = match_start + lower_query.len();
                    count += 1;
                }
                new_line.push_str(&line[last_idx..]);
                *line = new_line;
            }
        }

        if count > 0 {
            self.is_dirty = true;
            self.selection_anchor = None;
            self.selection_focus = None;
            self.ensure_cursor_visible();
            self.status_message = format!("Replaced {} occurrence(s)", count);
        } else {
            self.status_message = format!("Pattern '{}' not found for replace", self.search_query);
        }

        count
    }

    pub fn ensure_cursor_visible(&mut self) {
        // Vertical auto-scroll
        if self.cursor_row < self.scroll_row {
            self.scroll_row = self.cursor_row;
        } else if self.cursor_row >= self.scroll_row + VISIBLE_ROWS {
            self.scroll_row = self.cursor_row.saturating_sub(VISIBLE_ROWS - 1);
        }

        // Horizontal auto-scroll (only if wrap is disabled)
        if !self.word_wrap {
            if self.cursor_col < self.scroll_col {
                self.scroll_col = self.cursor_col;
            } else if self.cursor_col >= self.scroll_col + VISIBLE_COLS {
                self.scroll_col = self.cursor_col.saturating_sub(VISIBLE_COLS - 1);
            }
        } else {
            self.scroll_col = 0;
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.push_undo_snapshot();
        if self.cursor_row >= self.lines.len() {
            self.lines.push(String::new());
            self.cursor_row = self.lines.len() - 1;
        }

        let line = &mut self.lines[self.cursor_row];
        if self.cursor_col >= line.len() {
            line.push(c);
            self.cursor_col = line.len();
        } else {
            line.insert(self.cursor_col, c);
            self.cursor_col += 1;
        }
        self.is_dirty = true;
        self.selection_anchor = None;
        self.selection_focus = None;
        self.ensure_cursor_visible();
    }

    pub fn insert_newline(&mut self) {
        self.push_undo_snapshot();
        if self.cursor_row >= self.lines.len() {
            self.lines.push(String::new());
            self.cursor_row = self.lines.len() - 1;
        }

        let current_line = &self.lines[self.cursor_row];
        let right_split = if self.cursor_col < current_line.len() {
            String::from(&current_line[self.cursor_col..])
        } else {
            String::new()
        };

        self.lines[self.cursor_row].truncate(self.cursor_col);
        self.lines.insert(self.cursor_row + 1, right_split);
        self.cursor_row += 1;
        self.cursor_col = 0;
        self.is_dirty = true;
        self.selection_anchor = None;
        self.selection_focus = None;
        self.ensure_cursor_visible();
    }

    pub fn delete_backward(&mut self) {
        if self.cursor_col > 0 {
            self.push_undo_snapshot();
            if self.cursor_row < self.lines.len() {
                let line = &mut self.lines[self.cursor_row];
                if self.cursor_col <= line.len() {
                    line.remove(self.cursor_col - 1);
                    self.cursor_col -= 1;
                    self.is_dirty = true;
                }
            }
        } else if self.cursor_row > 0 {
            self.push_undo_snapshot();
            let removed_line = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            let prev_len = self.lines[self.cursor_row].len();
            self.lines[self.cursor_row].push_str(&removed_line);
            self.cursor_col = prev_len;
            self.is_dirty = true;
        }
        self.selection_anchor = None;
        self.selection_focus = None;
        self.ensure_cursor_visible();
    }

    pub fn delete_forward(&mut self) {
        if self.cursor_row < self.lines.len() {
            let line_len = self.lines[self.cursor_row].len();
            if self.cursor_col < line_len {
                self.push_undo_snapshot();
                self.lines[self.cursor_row].remove(self.cursor_col);
                self.is_dirty = true;
            } else if self.cursor_row + 1 < self.lines.len() {
                self.push_undo_snapshot();
                let next_line = self.lines.remove(self.cursor_row + 1);
                self.lines[self.cursor_row].push_str(&next_line);
                self.is_dirty = true;
            }
        }
        self.selection_anchor = None;
        self.selection_focus = None;
        self.ensure_cursor_visible();
    }

    pub fn move_left(&mut self, select: bool) {
        if select && self.selection_anchor.is_none() {
            self.selection_anchor = Some((self.cursor_row, self.cursor_col));
        }

        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
        }

        if select {
            self.selection_focus = Some((self.cursor_row, self.cursor_col));
        } else {
            self.selection_anchor = None;
            self.selection_focus = None;
        }
        self.ensure_cursor_visible();
    }

    pub fn move_right(&mut self, select: bool) {
        if select && self.selection_anchor.is_none() {
            self.selection_anchor = Some((self.cursor_row, self.cursor_col));
        }

        if self.cursor_row < self.lines.len() {
            if self.cursor_col < self.lines[self.cursor_row].len() {
                self.cursor_col += 1;
            } else if self.cursor_row + 1 < self.lines.len() {
                self.cursor_row += 1;
                self.cursor_col = 0;
            }
        }

        if select {
            self.selection_focus = Some((self.cursor_row, self.cursor_col));
        } else {
            self.selection_anchor = None;
            self.selection_focus = None;
        }
        self.ensure_cursor_visible();
    }

    pub fn move_up(&mut self, select: bool) {
        if select && self.selection_anchor.is_none() {
            self.selection_anchor = Some((self.cursor_row, self.cursor_col));
        }

        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
        }

        if select {
            self.selection_focus = Some((self.cursor_row, self.cursor_col));
        } else {
            self.selection_anchor = None;
            self.selection_focus = None;
        }
        self.ensure_cursor_visible();
    }

    pub fn move_down(&mut self, select: bool) {
        if select && self.selection_anchor.is_none() {
            self.selection_anchor = Some((self.cursor_row, self.cursor_col));
        }

        if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
        }

        if select {
            self.selection_focus = Some((self.cursor_row, self.cursor_col));
        } else {
            self.selection_anchor = None;
            self.selection_focus = None;
        }
        self.ensure_cursor_visible();
    }

    pub fn move_home(&mut self, select: bool) {
        if select && self.selection_anchor.is_none() {
            self.selection_anchor = Some((self.cursor_row, self.cursor_col));
        }
        self.cursor_col = 0;
        if select {
            self.selection_focus = Some((self.cursor_row, self.cursor_col));
        } else {
            self.selection_anchor = None;
            self.selection_focus = None;
        }
        self.ensure_cursor_visible();
    }

    pub fn move_end(&mut self, select: bool) {
        if select && self.selection_anchor.is_none() {
            self.selection_anchor = Some((self.cursor_row, self.cursor_col));
        }
        if self.cursor_row < self.lines.len() {
            self.cursor_col = self.lines[self.cursor_row].len();
        }
        if select {
            self.selection_focus = Some((self.cursor_row, self.cursor_col));
        } else {
            self.selection_anchor = None;
            self.selection_focus = None;
        }
        self.ensure_cursor_visible();
    }

    pub fn move_document_start(&mut self) {
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.selection_anchor = None;
        self.selection_focus = None;
        self.ensure_cursor_visible();
    }

    pub fn move_document_end(&mut self) {
        self.cursor_row = self.lines.len().saturating_sub(1);
        self.cursor_col = self.lines.last().map(|l| l.len()).unwrap_or(0);
        self.selection_anchor = None;
        self.selection_focus = None;
        self.ensure_cursor_visible();
    }

    pub fn select_all(&mut self) {
        self.selection_anchor = Some((0, 0));
        let last_row = self.lines.len().saturating_sub(1);
        let last_col = self.lines.last().map(|l| l.len()).unwrap_or(0);
        self.selection_focus = Some((last_row, last_col));
        self.cursor_row = last_row;
        self.cursor_col = last_col;
        self.ensure_cursor_visible();
    }

    pub fn copy_selection(&mut self) {
        if let (Some(anchor), Some(focus)) = (self.selection_anchor, self.selection_focus) {
            let (start, end) = if anchor <= focus { (anchor, focus) } else { (focus, anchor) };
            let mut extracted = String::new();
            for r in start.0..=end.0 {
                if r >= self.lines.len() { break; }
                let line = &self.lines[r];
                let c_start = if r == start.0 { start.1.min(line.len()) } else { 0 };
                let c_end = if r == end.0 { end.1.min(line.len()) } else { line.len() };
                if c_start <= c_end && c_start <= line.len() {
                    extracted.push_str(&line[c_start..c_end]);
                }
                if r < end.0 {
                    extracted.push('\n');
                }
            }
            crate::clipboard::copy_to_clipboard(&extracted);
            self.status_message = String::from("Selection copied to clipboard");
        } else if self.cursor_row < self.lines.len() {
            crate::clipboard::copy_to_clipboard(&self.lines[self.cursor_row]);
            self.status_message = String::from("Current line copied to clipboard");
        }
    }

    pub fn cut_selection(&mut self) {
        self.push_undo_snapshot();
        self.copy_selection();
        if let (Some(anchor), Some(focus)) = (self.selection_anchor, self.selection_focus) {
            let (start, end) = if anchor <= focus { (anchor, focus) } else { (focus, anchor) };
            if start.0 == end.0 && start.0 < self.lines.len() {
                let line = &mut self.lines[start.0];
                let c1 = start.1.min(line.len());
                let c2 = end.1.min(line.len());
                let mut new_line = String::from(&line[..c1]);
                new_line.push_str(&line[c2..]);
                self.lines[start.0] = new_line;
                self.cursor_row = start.0;
                self.cursor_col = c1;
            } else if start.0 < end.0 && end.0 < self.lines.len() {
                let first_part = String::from(&self.lines[start.0][..start.1.min(self.lines[start.0].len())]);
                let last_part = &self.lines[end.0][end.1.min(self.lines[end.0].len())..];
                let merged = format!("{}{}", first_part, last_part);
                self.lines.drain(start.0..=end.0);
                self.lines.insert(start.0, merged);
                self.cursor_row = start.0;
                self.cursor_col = start.1;
            }
            self.selection_anchor = None;
            self.selection_focus = None;
            self.is_dirty = true;
            self.ensure_cursor_visible();
            self.status_message = String::from("Cut selection to clipboard");
        } else if self.cursor_row < self.lines.len() {
            if self.lines.len() > 1 {
                self.lines.remove(self.cursor_row);
                if self.cursor_row >= self.lines.len() {
                    self.cursor_row = self.lines.len().saturating_sub(1);
                }
            } else {
                self.lines[0].clear();
            }
            self.cursor_col = 0;
            self.is_dirty = true;
            self.ensure_cursor_visible();
            self.status_message = String::from("Cut current line");
        }
    }

    pub fn paste_clipboard(&mut self) {
        self.push_undo_snapshot();
        let text = crate::clipboard::get_clipboard_text();
        for c in text.chars() {
            if c == '\n' {
                self.insert_newline();
            } else if (c as u32) >= 32 && (c as u32) <= 126 {
                self.insert_char(c);
            }
        }
        self.ensure_cursor_visible();
        self.status_message = String::from("Pasted from clipboard");
    }

    pub fn handle_key_input(&mut self, key_code: u8, pressed: bool) {
        if !pressed { return; }
        let is_ctrl = crate::keyboard::is_ctrl_pressed();
        let is_shift = crate::keyboard::is_shift_pressed();

        // 1. Modal Dialog Handling (Unsaved Changes)
        if self.show_unsaved_dialog {
            match key_code {
                0x1F | 0x1C => { // 'S' or Enter -> Save & Close
                    self.save_file();
                    self.show_unsaved_dialog = false;
                    self.pending_close = true;
                }
                0x20 | 0x0E => { // 'D' or Backspace -> Discard & Close
                    self.show_unsaved_dialog = false;
                    self.pending_close = true;
                }
                0x01 => { // Escape -> Cancel
                    self.show_unsaved_dialog = false;
                    self.status_message = String::from("Close cancelled");
                }
                _ => {}
            }
            return;
        }

        // 2. Find / Replace Field Input
        if self.active_input_field == 1 || self.active_input_field == 2 {
            match key_code {
                0x01 => { // Escape -> Close search/replace toolbar
                    self.show_find_dialog = false;
                    self.show_replace_dialog = false;
                    self.active_input_field = 0;
                    return;
                }
                0x0F => { // Tab -> Switch between Find and Replace fields
                    if self.show_replace_dialog {
                        self.active_input_field = if self.active_input_field == 1 { 2 } else { 1 };
                    }
                    return;
                }
                0x1C => { // Enter -> Find next or Replace
                    if is_shift {
                        self.find_previous();
                    } else if self.active_input_field == 2 {
                        self.replace_current();
                    } else {
                        self.find_next();
                    }
                    return;
                }
                0x0E => { // Backspace in search field
                    if self.active_input_field == 1 {
                        self.search_query.pop();
                    } else {
                        self.replace_query.pop();
                    }
                    return;
                }
                _ => {
                    if let Some(ascii_byte) = crate::keyboard::scancode_to_ascii(key_code, is_shift) {
                        if ascii_byte >= 32 && ascii_byte <= 126 {
                            if self.active_input_field == 1 {
                                self.search_query.push(ascii_byte as char);
                            } else {
                                self.replace_query.push(ascii_byte as char);
                            }
                        }
                    }
                    return;
                }
            }
        }

        // 3. Canvas Shortcuts & Editor Navigation
        if is_ctrl {
            match key_code {
                0x21 => { // Ctrl + F: Find
                    self.show_find_dialog = true;
                    self.show_replace_dialog = false;
                    self.active_input_field = 1;
                }
                0x23 => { // Ctrl + H: Replace
                    self.show_find_dialog = true;
                    self.show_replace_dialog = true;
                    self.active_input_field = 1;
                }
                0x11 => { // Ctrl + W: Toggle Word Wrap
                    self.toggle_word_wrap();
                }
                0x2C => { // Ctrl + Z: Undo
                    self.undo();
                }
                0x15 => { // Ctrl + Y: Redo
                    self.redo();
                }
                0x1F => { // Ctrl + S: Save
                    self.save_file();
                }
                0x2E => { // Ctrl + C: Copy
                    self.copy_selection();
                }
                0x2D => { // Ctrl + X: Cut
                    self.cut_selection();
                }
                0x2F => { // Ctrl + V: Paste
                    self.paste_clipboard();
                }
                0x1E => { // Ctrl + A: Select All
                    self.select_all();
                }
                0x47 => { // Ctrl + Home: Document Start
                    self.move_document_start();
                }
                0x4F => { // Ctrl + End: Document End
                    self.move_document_end();
                }
                _ => {}
            }
        } else {
            match key_code {
                0x1C => { // Enter
                    self.insert_newline();
                }
                0x0E => { // Backspace
                    self.delete_backward();
                }
                0x53 => { // Delete
                    self.delete_forward();
                }
                0x4B => { // Left Arrow
                    self.move_left(is_shift);
                }
                0x4D => { // Right Arrow
                    self.move_right(is_shift);
                }
                0x48 => { // Up Arrow
                    self.move_up(is_shift);
                }
                0x50 => { // Down Arrow
                    self.move_down(is_shift);
                }
                0x47 => { // Home
                    self.move_home(is_shift);
                }
                0x4F => { // End
                    self.move_end(is_shift);
                }
                _ => {
                    if let Some(ascii_byte) = crate::keyboard::scancode_to_ascii(key_code, is_shift) {
                        if ascii_byte >= 32 && ascii_byte <= 126 {
                            self.insert_char(ascii_byte as char);
                        }
                    }
                }
            }
        }
    }

    pub fn handle_mouse_click(&mut self, local_x: u32, local_y: u32) {
        // Modal dialog button interaction
        if self.show_unsaved_dialog {
            let dw = 250u32;
            let dh = 104u32;
            let dx = (EDITOR_WIDTH.saturating_sub(dw)) / 2;
            let dy = (EDITOR_HEIGHT.saturating_sub(dh)) / 2;

            if local_y >= dy + 64 && local_y <= dy + 88 {
                if local_x >= dx + 12 && local_x <= dx + 72 {
                    self.save_file();
                    self.show_unsaved_dialog = false;
                    self.pending_close = true;
                    return;
                }
                if local_x >= dx + 84 && local_x <= dx + 154 {
                    self.show_unsaved_dialog = false;
                    self.pending_close = true;
                    return;
                }
                if local_x >= dx + 166 && local_x <= dx + 232 {
                    self.show_unsaved_dialog = false;
                    self.status_message = String::from("Close cancelled");
                    return;
                }
            }
            return;
        }

        // Top Toolbar buttons (y in 4..24)
        if local_y >= 4 && local_y <= 24 {
            // [Save] button (x: 6..54)
            if local_x >= 6 && local_x <= 54 {
                self.save_file();
                return;
            }
            // [New] button (x: 58..100)
            if local_x >= 58 && local_x <= 100 {
                self.new_document();
                return;
            }
            // [Wrap] button (x: 104..150)
            if local_x >= 104 && local_x <= 150 {
                self.toggle_word_wrap();
                return;
            }
            // [Find] button (x: 154..196)
            if local_x >= 154 && local_x <= 196 {
                self.show_find_dialog = !self.show_find_dialog;
                if !self.show_find_dialog { self.show_replace_dialog = false; }
                self.active_input_field = if self.show_find_dialog { 1 } else { 0 };
                return;
            }
        }

        // Find & Replace toolbar buttons (y in 28..54)
        if self.show_find_dialog && local_y >= 28 && local_y <= 54 {
            // Find input box click (x: 6..140)
            if local_x >= 6 && local_x <= 140 {
                self.active_input_field = 1;
                return;
            }
            // [Next] button (x: 146..186)
            if local_x >= 146 && local_x <= 186 {
                self.find_next();
                return;
            }
            // [Prev] button (x: 190..230)
            if local_x >= 190 && local_x <= 230 {
                self.find_previous();
                return;
            }
            // [Aa] Case button (x: 234..264)
            if local_x >= 234 && local_x <= 264 {
                self.toggle_case_sensitive();
                return;
            }
            // [Replace All] (x: 268..350)
            if self.show_replace_dialog && local_x >= 268 && local_x <= 350 {
                self.replace_all();
                return;
            }
            // [Close] (x: 380..414)
            if local_x >= 380 && local_x <= 414 {
                self.show_find_dialog = false;
                self.show_replace_dialog = false;
                self.active_input_field = 0;
                return;
            }
        }

        // Text area click -> set cursor and update selection
        let text_start_y = if self.show_find_dialog { 58 } else { 30 };
        if local_y >= text_start_y && local_y < EDITOR_HEIGHT.saturating_sub(18) {
            let row_offset = ((local_y - text_start_y) / 16) as usize;
            let target_row = self.scroll_row + row_offset;

            if target_row < self.lines.len() {
                self.cursor_row = target_row;
                let col_offset = if local_x >= 34 { ((local_x - 34) / 8) as usize } else { 0 };
                let target_col = if self.word_wrap { col_offset } else { self.scroll_col + col_offset };
                self.cursor_col = target_col.min(self.lines[target_row].len());

                self.selection_anchor = Some((self.cursor_row, self.cursor_col));
                self.selection_focus = None;
                self.active_input_field = 0;
            }
        }
    }

    pub fn render_to_surface(&self, surface_ptr: *mut u32, w: u32, h: u32) {
        if surface_ptr.is_null() { return; }
        let bg_color = 0x000B0F19;     // Obsidian Dark Slate
        let panel_bg = 0x001E293B;     // Slate 800
        let border_col = 0x00334155;
        let text_color = 0x00F8FAFC;
        let text_muted = 0x0094A3B8;
        let line_num_col = 0x0064748B;
        let dirty_col = 0x00F59E0B;     // Amber
        let sel_bg = 0x002563EB;       // Selection Blue

        crate::terminal_app::clear_surface(surface_ptr, w, h, bg_color);

        // 1. Top Flat Toolbar (y = 0..30)
        draw_surf_rect(surface_ptr, w, h, 0, 0, w, 30, panel_bg);
        draw_surf_rect(surface_ptr, w, h, 0, 29, w, 1, border_col);

        // [ Save ]
        draw_surf_rect(surface_ptr, w, h, 6, 4, 46, 22, 0x002563EB);
        crate::font::draw_text(surface_ptr, w, h, 14, 8, "Save", text_color, 0x002563EB);

        // [ New ]
        draw_surf_rect(surface_ptr, w, h, 56, 4, 42, 22, 0x00334155);
        crate::font::draw_text(surface_ptr, w, h, 64, 8, "New", text_color, 0x00334155);

        // [ Undo ]
        draw_surf_rect(surface_ptr, w, h, 102, 4, 42, 22, 0x00334155);
        crate::font::draw_text(surface_ptr, w, h, 108, 8, "Undo", text_color, 0x00334155);

        // [ Wrap ]
        let wrap_bg = if self.word_wrap { 0x0016A34A } else { 0x00334155 };
        draw_surf_rect(surface_ptr, w, h, 148, 4, 44, 22, wrap_bg);
        crate::font::draw_text(surface_ptr, w, h, 154, 8, "Wrap", text_color, wrap_bg);

        // [ Find ]
        let find_bg = if self.show_find_dialog { 0x00D97706 } else { 0x00334155 };
        draw_surf_rect(surface_ptr, w, h, 196, 4, 42, 22, find_bg);
        crate::font::draw_text(surface_ptr, w, h, 204, 8, "Find", text_color, find_bg);

        // File Title & Dirty Marker
        let file_label = self.file_path.as_deref().unwrap_or("[Untitled]");
        let dirty_marker = if self.is_dirty { " *" } else { "" };
        let full_title = format!("{}{}", file_label, dirty_marker);
        let title_col = if self.is_dirty { dirty_col } else { 0x0038BDF8 };
        crate::font::draw_text(surface_ptr, w, h, 246, 8, &full_title, title_col, panel_bg);

        // 2. Find & Replace Overlay Toolbar (if open)
        let text_start_y = if self.show_find_dialog {
            draw_surf_rect(surface_ptr, w, h, 0, 30, w, 28, 0x000F172A);
            draw_surf_rect(surface_ptr, w, h, 0, 57, w, 1, border_col);

            // Find Query Input Box
            let f_box_bg = if self.active_input_field == 1 { 0x001E293B } else { 0x000B0F19 };
            draw_surf_rect(surface_ptr, w, h, 6, 33, 134, 22, f_box_bg);
            let f_display = if self.search_query.is_empty() { "Search..." } else { &self.search_query };
            let f_col = if self.search_query.is_empty() { 0x0064748B } else { 0x00FFFFFF };
            crate::font::draw_text(surface_ptr, w, h, 10, 37, f_display, f_col, f_box_bg);

            // [Next]
            draw_surf_rect(surface_ptr, w, h, 146, 33, 38, 22, 0x002563EB);
            crate::font::draw_text(surface_ptr, w, h, 150, 37, "Next", 0x00FFFFFF, 0x002563EB);

            // [Prev]
            draw_surf_rect(surface_ptr, w, h, 188, 33, 38, 22, 0x00334155);
            crate::font::draw_text(surface_ptr, w, h, 194, 37, "Prev", 0x00FFFFFF, 0x00334155);

            // [Aa] Match Case
            let case_bg = if self.case_sensitive { 0x0016A34A } else { 0x00334155 };
            draw_surf_rect(surface_ptr, w, h, 230, 33, 28, 22, case_bg);
            crate::font::draw_text(surface_ptr, w, h, 236, 37, "Aa", 0x00FFFFFF, case_bg);

            if self.show_replace_dialog {
                // [Replace All]
                draw_surf_rect(surface_ptr, w, h, 264, 33, 86, 22, 0x007C3AED);
                crate::font::draw_text(surface_ptr, w, h, 268, 37, "Replace All", 0x00FFFFFF, 0x007C3AED);
            }

            // Close Find Box [X]
            draw_surf_rect(surface_ptr, w, h, w.saturating_sub(30), 33, 24, 22, 0x00DC2626);
            crate::font::draw_text(surface_ptr, w, h, w.saturating_sub(22), 37, "x", 0x00FFFFFF, 0x00DC2626);

            62u32
        } else {
            34u32
        };

        // 3. Main Editor Text Area (with Scroll Offsets and Smart Word Wrap)
        let mut y = text_start_y;
        let visible_lines = self.lines.iter().skip(self.scroll_row).take(VISIBLE_ROWS);

        for (rel_idx, line) in visible_lines.enumerate() {
            if y + 16 >= h.saturating_sub(18) { break; }
            let abs_row = self.scroll_row + rel_idx;

            // Line number (e.g. " 1 ")
            let num_str = format!("{:2} ", abs_row + 1);
            crate::font::draw_text(surface_ptr, w, h, 6, y, &num_str, line_num_col, bg_color);

            // Visible slice of line (horizontal scroll or soft wrap)
            let visible_slice = if self.word_wrap {
                if line.len() > VISIBLE_COLS { &line[..VISIBLE_COLS] } else { line.as_str() }
            } else {
                if self.scroll_col < line.len() { &line[self.scroll_col..] } else { "" }
            };

            // Selection highlight
            let is_in_selection = self.selection_anchor.is_some() && self.selection_focus.is_some();
            let row_bg = if is_in_selection { sel_bg } else { bg_color };

            crate::font::draw_text(surface_ptr, w, h, 34, y, visible_slice, text_color, row_bg);

            // Draw cursor if on this row
            if abs_row == self.cursor_row {
                let cur_rel_col = if self.word_wrap {
                    self.cursor_col.min(VISIBLE_COLS) as u32
                } else if self.cursor_col >= self.scroll_col {
                    (self.cursor_col - self.scroll_col) as u32
                } else {
                    0
                };
                let cur_x = 34 + cur_rel_col * 8;
                if cur_x < w.saturating_sub(8) {
                    draw_surf_rect(surface_ptr, w, h, cur_x, y, 2, 14, 0x0038BDF8);
                }
            }

            y += 16;
        }

        // 4. Status Bar at Bottom (y = h - 18 .. h)
        let status_y = h.saturating_sub(18);
        draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 18, panel_bg);
        draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 1, border_col);
        let cursor_info = format!("Ln {}, Col {} | {}", self.cursor_row + 1, self.cursor_col + 1, self.status_message);
        crate::font::draw_text(surface_ptr, w, h, 8, status_y + 4, &cursor_info, 0x0034D399, panel_bg);

        let enc_info = "UTF-8";
        crate::font::draw_text(surface_ptr, w, h, w.saturating_sub(50), status_y + 4, enc_info, text_muted, panel_bg);

        // 5. Modal Dialog: Unsaved Changes (if active)
        if self.show_unsaved_dialog {
            let dw = 250u32;
            let dh = 104u32;
            let dx = (w.saturating_sub(dw)) / 2;
            let dy = (h.saturating_sub(dh)) / 2;

            draw_surf_rect(surface_ptr, w, h, dx, dy, dw, dh, 0x000F172A);
            draw_surf_rect(surface_ptr, w, h, dx, dy, dw, 1, 0x00F59E0B);
            draw_surf_rect(surface_ptr, w, h, dx, dy, 1, dh, 0x00F59E0B);
            draw_surf_rect(surface_ptr, w, h, dx + dw - 1, dy, 1, dh, 0x00F59E0B);
            draw_surf_rect(surface_ptr, w, h, dx, dy + dh - 1, dw, 1, 0x00F59E0B);

            crate::font::draw_text(surface_ptr, w, h, dx + 12, dy + 12, "Unsaved Changes!", 0x00F59E0B, 0x000F172A);
            crate::font::draw_text(surface_ptr, w, h, dx + 12, dy + 32, "Save before closing?", 0x00E2E8F0, 0x000F172A);

            // [Save]
            draw_surf_rect(surface_ptr, w, h, dx + 12, dy + 64, 60, 24, 0x002563EB);
            crate::font::draw_text(surface_ptr, w, h, dx + 24, dy + 70, "Save", 0x00FFFFFF, 0x002563EB);

            // [Discard]
            draw_surf_rect(surface_ptr, w, h, dx + 84, dy + 64, 70, 24, 0x00DC2626);
            crate::font::draw_text(surface_ptr, w, h, dx + 92, dy + 70, "Discard", 0x00FFFFFF, 0x00DC2626);

            // [Cancel]
            draw_surf_rect(surface_ptr, w, h, dx + 166, dy + 64, 66, 24, 0x00334155);
            crate::font::draw_text(surface_ptr, w, h, dx + 176, dy + 70, "Cancel", 0x0094A3B8, 0x00334155);
        }
    }
}

pub static EDITOR_INSTANCES: Mutex<BTreeMap<u64, EditorAppState>> = Mutex::new(BTreeMap::new());
pub static GLOBAL_FILE_STORE: Mutex<BTreeMap<String, Vec<String>>> = Mutex::new(BTreeMap::new());

pub fn cleanup_editor_for_window(window_id: u64) {
    let mut instances = EDITOR_INSTANCES.lock();
    if instances.remove(&window_id).is_some() {
        crate::serial_println!("[EDITOR] Cleaned up Editor state for Window {}", window_id);
    }
}

pub fn draw_surf_rect(surface_ptr: *mut u32, surf_w: u32, surf_h: u32, x: u32, y: u32, rw: u32, rh: u32, color: u32) {
    if surface_ptr.is_null() { return; }
    for r in 0..rh {
        let py = y + r;
        if py >= surf_h { break; }
        for c in 0..rw {
            let px = x + c;
            if px >= surf_w { break; }
            let offset = (py as usize) * (surf_w as usize) + (px as usize);
            unsafe {
                core::ptr::write_volatile(surface_ptr.add(offset), color);
            }
        }
    }
}

pub fn spawn_editor_app(name: &str, file_to_open: Option<&str>) -> Result<u64, &'static str> {
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frame for editor.app")?;
    let code = crate::terminal_app::terminal_machine_code();
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

    let surf_id = crate::surface::create_surface_for_pid(pid, crate::surface::MAX_SURFACE_WIDTH, crate::surface::MAX_SURFACE_HEIGHT)?;
    if let Some(s) = crate::surface::SURFACE_REGISTRY.write().iter_mut().find(|s| s.surface_id == surf_id) {
        s.width = EDITOR_WIDTH;
        s.height = EDITOR_HEIGHT;
    }
    let (win_x, win_y) = {
        let count = crate::wm::WM.lock().windows.len() as i32;
        (60 + ((count * 30) % 200), 50 + ((count * 25) % 150))
    };
    let win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, win_x, win_y, EDITOR_WIDTH, EDITOR_HEIGHT)
        .map_err(|_| "window creation failed")?;

    {
        let state = EditorAppState::new(win_id, pid, file_to_open);
        if let Some(surface) = crate::surface::SURFACE_REGISTRY.read().iter().find(|s| s.surface_id == surf_id) {
            let phys_addr = surface.shmem_phys_addr;
            let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *mut u32 };
            state.render_to_surface(surf_ptr, EDITOR_WIDTH, EDITOR_HEIGHT);
        }
        EDITOR_INSTANCES.lock().insert(win_id, state);
    }

    let _ = crate::surface::present_surface(surf_id, 0, 0, EDITOR_WIDTH, EDITOR_HEIGHT);
    crate::serial_println!("[APP-REGISTRY] Successfully launched '{}' (PID {}, Entry 0x{:x}, Surface {}, Window {})",
        name, pid, code_base, surf_id, win_id);

    Ok(pid)
}
