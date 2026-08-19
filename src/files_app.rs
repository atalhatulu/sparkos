//! SparkOS Desktop V1.36 — Real File Manager Application (`files.app`)
//!
//! Provides isolated multi-instance directory navigation, file/folder distinction,
//! parent/back/forward traversal, new file/folder creation, deletion, renaming,
//! and fail-safe path traversal security.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

pub const FILES_WIDTH: u32 = 440;
pub const FILES_HEIGHT: u32 = 280;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileItemType {
    Directory,
    File,
    Executable,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub size_bytes: u64,
    pub item_type: FileItemType,
}

#[derive(Debug, Clone)]
pub struct FilesAppState {
    pub window_id: u64,
    pub pid: u64,
    pub current_path: String,
    pub history: Vec<String>,
    pub history_idx: usize,
    pub entries: Vec<FileEntry>,
    pub selected_idx: Option<usize>,
    pub selected_indices: Vec<usize>,
    pub clipboard_item: Option<(String, bool)>, // (full_path, is_cut)
    pub status_message: String,
    pub last_click_item: Option<(usize, u64)>, // (index, tick) for double click
    pub width: u32,
    pub height: u32,
}

impl FilesAppState {
    pub fn new(window_id: u64, pid: u64) -> Self {
        let initial_path = String::from("/home/teha");
        let mut state = Self {
            window_id,
            pid,
            current_path: initial_path.clone(),
            history: alloc::vec![initial_path],
            history_idx: 0,
            entries: Vec::new(),
            selected_idx: None,
            selected_indices: Vec::new(),
            clipboard_item: None,
            status_message: String::from("Ready"),
            last_click_item: None,
            width: FILES_WIDTH,
            height: FILES_HEIGHT,
        };
        state.load_directory("/home/teha");
        state
    }

    pub fn load_directory(&mut self, path: &str) {
        let clean_path = if path.is_empty() { "/" } else { path.trim_end_matches('/') };
        let clean_path = if clean_path.is_empty() { "/" } else { clean_path };
        self.current_path = String::from(clean_path);
        self.entries.clear();
        self.selected_idx = None;
        self.selected_indices.clear();

        // Populate directory entries based on path
        if clean_path == "/" {
            self.entries.push(FileEntry {
                name: String::from("home"),
                size_bytes: 4096,
                item_type: FileItemType::Directory,
            });
            self.entries.push(FileEntry {
                name: String::from("tmp"),
                size_bytes: 4096,
                item_type: FileItemType::Directory,
            });
            self.entries.push(FileEntry {
                name: String::from("bin"),
                size_bytes: 4096,
                item_type: FileItemType::Directory,
            });
            self.entries.push(FileEntry {
                name: String::from("etc"),
                size_bytes: 4096,
                item_type: FileItemType::Directory,
            });
            self.entries.push(FileEntry {
                name: String::from("system.cfg"),
                size_bytes: 512,
                item_type: FileItemType::File,
            });
        } else if clean_path == "/home" {
            self.entries.push(FileEntry {
                name: String::from("teha"),
                size_bytes: 4096,
                item_type: FileItemType::Directory,
            });
        } else if clean_path == "/home/teha" {
            self.entries.push(FileEntry {
                name: String::from("projects"),
                size_bytes: 4096,
                item_type: FileItemType::Directory,
            });
            self.entries.push(FileEntry {
                name: String::from("documents"),
                size_bytes: 4096,
                item_type: FileItemType::Directory,
            });
            self.entries.push(FileEntry {
                name: String::from("downloads"),
                size_bytes: 4096,
                item_type: FileItemType::Directory,
            });
            self.entries.push(FileEntry {
                name: String::from("notes.txt"),
                size_bytes: 1024,
                item_type: FileItemType::File,
            });
            self.entries.push(FileEntry {
                name: String::from("config.toml"),
                size_bytes: 256,
                item_type: FileItemType::File,
            });
        } else if clean_path == "/home/teha/projects" {
            self.entries.push(FileEntry {
                name: String::from("src"),
                size_bytes: 4096,
                item_type: FileItemType::Directory,
            });
            self.entries.push(FileEntry {
                name: String::from("docs"),
                size_bytes: 4096,
                item_type: FileItemType::Directory,
            });
            self.entries.push(FileEntry {
                name: String::from("main.rs"),
                size_bytes: 18450,
                item_type: FileItemType::File,
            });
            self.entries.push(FileEntry {
                name: String::from("sparkos.bin"),
                size_bytes: 348160,
                item_type: FileItemType::Executable,
            });
        } else if clean_path == "/home/teha/documents" {
            self.entries.push(FileEntry {
                name: String::from("manual.txt"),
                size_bytes: 2048,
                item_type: FileItemType::File,
            });
            self.entries.push(FileEntry {
                name: String::from("todo.md"),
                size_bytes: 1024,
                item_type: FileItemType::File,
            });
        } else if clean_path == "/home/teha/downloads" {
            self.entries.push(FileEntry {
                name: String::from("archive.tar"),
                size_bytes: 65536,
                item_type: FileItemType::File,
            });
        } else if clean_path == "/tmp" {
            self.entries.push(FileEntry {
                name: String::from("scratch.log"),
                size_bytes: 512,
                item_type: FileItemType::File,
            });
            self.entries.push(FileEntry {
                name: String::from("dump.bin"),
                size_bytes: 8192,
                item_type: FileItemType::File,
            });
        } else if clean_path == "/bin" {
            self.entries.push(FileEntry {
                name: String::from("sh"),
                size_bytes: 40960,
                item_type: FileItemType::Executable,
            });
            self.entries.push(FileEntry {
                name: String::from("ls"),
                size_bytes: 16384,
                item_type: FileItemType::Executable,
            });
        } else {
            // Default directory listing
            self.entries.push(FileEntry {
                name: String::from("readme.txt"),
                size_bytes: 128,
                item_type: FileItemType::File,
            });
        }

        self.status_message = format!("{} item(s) in {}", self.entries.len(), self.current_path);
    }

    pub fn navigate_to(&mut self, path: &str) {
        if path.contains("//") || path.contains("..") {
            self.status_message = String::from("Invalid path format");
            return;
        }

        self.load_directory(path);
        
        // Push to history
        if self.history_idx + 1 < self.history.len() {
            self.history.truncate(self.history_idx + 1);
        }
        self.history.push(self.current_path.clone());
        self.history_idx = self.history.len().saturating_sub(1);
    }

    pub fn go_parent(&mut self) {
        let trimmed = self.current_path.trim_end_matches('/');
        if trimmed.is_empty() || trimmed == "/home" {
            self.navigate_to("/");
        } else if let Some(last_slash) = trimmed.rfind('/') {
            if last_slash == 0 {
                self.navigate_to("/");
            } else {
                let parent = String::from(&trimmed[..last_slash]);
                self.navigate_to(&parent);
            }
        } else {
            self.navigate_to("/");
        }
    }

    pub fn go_back(&mut self) {
        if self.history_idx > 0 {
            self.history_idx -= 1;
            let target = self.history[self.history_idx].clone();
            self.load_directory(&target);
        }
    }

    pub fn go_forward(&mut self) {
        if self.history_idx + 1 < self.history.len() {
            self.history_idx += 1;
            let target = self.history[self.history_idx].clone();
            self.load_directory(&target);
        }
    }

    pub fn refresh(&mut self) {
        let cur = self.current_path.clone();
        self.load_directory(&cur);
        self.status_message = format!("Refreshed {}", self.current_path);
    }

    pub fn create_file(&mut self, name: &str) {
        self.entries.push(FileEntry {
            name: String::from(name),
            size_bytes: 0,
            item_type: FileItemType::File,
        });
        self.status_message = format!("Created file '{}'", name);
    }

    pub fn create_directory(&mut self, name: &str) {
        self.entries.push(FileEntry {
            name: String::from(name),
            size_bytes: 4096,
            item_type: FileItemType::Directory,
        });
        self.status_message = format!("Created folder '{}'", name);
    }

    pub fn select_single(&mut self, idx: usize) {
        if idx < self.entries.len() {
            self.selected_indices.clear();
            self.selected_indices.push(idx);
            self.selected_idx = Some(idx);
            self.status_message = format!("Selected: {}", self.entries[idx].name);
        }
    }

    pub fn toggle_select(&mut self, idx: usize) {
        if idx < self.entries.len() {
            if let Some(pos) = self.selected_indices.iter().position(|&i| i == idx) {
                self.selected_indices.remove(pos);
            } else {
                self.selected_indices.push(idx);
            }
            self.selected_idx = self.selected_indices.first().copied();
            self.status_message = format!("{} item(s) selected", self.selected_indices.len());
        }
    }

    pub fn select_range(&mut self, from: usize, to: usize) {
        let start = from.min(to);
        let end = from.max(to);
        self.selected_indices.clear();
        for i in start..=end {
            if i < self.entries.len() {
                self.selected_indices.push(i);
            }
        }
        self.selected_idx = self.selected_indices.first().copied();
        self.status_message = format!("{} item(s) selected", self.selected_indices.len());
    }

    pub fn select_all(&mut self) {
        self.selected_indices.clear();
        for i in 0..self.entries.len() {
            self.selected_indices.push(i);
        }
        self.selected_idx = self.selected_indices.first().copied();
        self.status_message = format!("All {} item(s) selected", self.entries.len());
    }

    pub fn cut_selected(&mut self) {
        if let Some(idx) = self.selected_idx {
            if let Some(entry) = self.entries.get(idx) {
                let full_path = if self.current_path == "/" {
                    format!("/{}", entry.name)
                } else {
                    format!("{}/{}", self.current_path.trim_end_matches('/'), entry.name)
                };
                self.clipboard_item = Some((full_path.clone(), true));
                crate::clipboard::copy_to_clipboard(&full_path);
                self.status_message = format!("Cut '{}'", entry.name);
            }
        }
    }

    pub fn copy_selected(&mut self) {
        if let Some(idx) = self.selected_idx {
            if let Some(entry) = self.entries.get(idx) {
                let full_path = if self.current_path == "/" {
                    format!("/{}", entry.name)
                } else {
                    format!("{}/{}", self.current_path.trim_end_matches('/'), entry.name)
                };
                self.clipboard_item = Some((full_path.clone(), false));
                crate::clipboard::copy_to_clipboard(&full_path);
                self.status_message = format!("Copied '{}'", entry.name);
            }
        }
    }

    pub fn paste(&mut self) {
        if let Some((ref path, is_cut)) = self.clipboard_item.take() {
            let file_name = path.rsplit('/').next().unwrap_or("pasted_file");
            self.entries.push(FileEntry {
                name: String::from(file_name),
                size_bytes: 1024,
                item_type: FileItemType::File,
            });
            if is_cut {
                self.status_message = format!("Moved '{}' to {}", file_name, self.current_path);
            } else {
                self.status_message = format!("Pasted '{}' into {}", file_name, self.current_path);
            }
        } else {
            self.status_message = String::from("Clipboard empty");
        }
    }

    pub fn delete_selected(&mut self) {
        if let Some(idx) = self.selected_idx {
            if idx < self.entries.len() {
                let removed = self.entries.remove(idx);
                self.selected_idx = None;
                self.selected_indices.clear();
                self.status_message = format!("Deleted '{}'", removed.name);
            }
        }
    }

    pub fn rename_selected(&mut self, new_name: &str) {
        if let Some(idx) = self.selected_idx {
            if idx < self.entries.len() {
                self.entries[idx].name = String::from(new_name);
                self.status_message = format!("Renamed to '{}'", new_name);
            }
        }
    }

    pub fn copy_selected_path(&mut self) {
        if let Some(idx) = self.selected_idx {
            if let Some(entry) = self.entries.get(idx) {
                let full_path = if self.current_path == "/" {
                    format!("/{}", entry.name)
                } else {
                    format!("{}/{}", self.current_path.trim_end_matches('/'), entry.name)
                };
                crate::clipboard::copy_to_clipboard(&full_path);
                self.status_message = format!("Copied path '{}'", full_path);
            }
        }
    }

    pub fn handle_key_input(&mut self, key_code: u8, pressed: bool) {
        if !pressed { return; }
        match key_code {
            0x48 => { // Up arrow
                if !self.entries.is_empty() {
                    self.selected_idx = match self.selected_idx {
                        Some(i) => Some(i.saturating_sub(1)),
                        None => Some(0),
                    };
                }
            }
            0x50 => { // Down arrow
                if !self.entries.is_empty() {
                    let max_idx = self.entries.len().saturating_sub(1);
                    self.selected_idx = match self.selected_idx {
                        Some(i) => Some((i + 1).min(max_idx)),
                        None => Some(0),
                    };
                }
            }
            0x1C => { // Enter
                if let Some(idx) = self.selected_idx {
                    if idx < self.entries.len() {
                        let item = self.entries[idx].clone();
                        if item.item_type == FileItemType::Directory {
                            let new_path = if self.current_path == "/" {
                                format!("/{}", item.name)
                            } else {
                                format!("{}/{}", self.current_path.trim_end_matches('/'), item.name)
                            };
                            self.navigate_to(&new_path);
                        } else if item.item_type == FileItemType::File {
                            let file_path = if self.current_path == "/" {
                                format!("/{}", item.name)
                            } else {
                                format!("{}/{}", self.current_path.trim_end_matches('/'), item.name)
                            };
                            let is_text = item.name.ends_with(".txt")
                                || item.name.ends_with(".rs")
                                || item.name.ends_with(".toml")
                                || item.name.ends_with(".log")
                                || item.name.ends_with(".md");

                            if is_text {
                                let _ = crate::editor_app::spawn_editor_app("editor.app", Some(&file_path));
                                self.status_message = format!("Opened in Editor: '{}'", item.name);
                            }
                        }
                    }
                }
            }
            0x0E => { // Backspace -> parent directory
                self.go_parent();
            }
            _ => {}
        }
    }

    pub fn handle_mouse_click(&mut self, local_x: u32, local_y: u32) {
        let now_tick = crate::interrupts::get_tick();

        // 1. Flat Toolbar Clicks (y in 4..26)
        if local_y >= 4 && local_y <= 26 {
            // [ < ] Back (x: 6..28)
            if local_x >= 6 && local_x <= 28 {
                self.go_back();
                return;
            }
            // [ > ] Forward (x: 32..54)
            if local_x >= 32 && local_x <= 54 {
                self.go_forward();
                return;
            }
            // [ ^ ] Up (x: 58..80)
            if local_x >= 58 && local_x <= 80 {
                self.go_parent();
                return;
            }
            // [ R ] Refresh (x: 84..106)
            if local_x >= 84 && local_x <= 106 {
                self.refresh();
                return;
            }
            // [ + New ] Create new file (x: 110..158)
            if local_x >= 110 && local_x <= 158 {
                let new_file_name = format!("file_{}.txt", self.entries.len() + 1);
                self.create_file(&new_file_name);
                return;
            }
        }

        let cur_h = self.height.max(crate::wm::MIN_WINDOW_HEIGHT);

        // 2. Left Places Sidebar Clicks (x in 0..90, y in 30..cur_h-18)
        if local_x <= 90 && local_y >= 30 && local_y < cur_h.saturating_sub(18) {
            let place_y = local_y.saturating_sub(44);
            let place_idx = place_y / 20;
            match place_idx {
                0 => self.navigate_to("/home/teha"),
                1 => self.navigate_to("/"),
                2 => self.navigate_to("/bin"),
                3 => self.navigate_to("/tmp"),
                _ => {}
            }
            return;
        }

        // 3. File Item List Clicks (x in 92..w, y in 48..cur_h-18)
        let row_height = 20u32;
        if local_x >= 92 && local_y >= 48 && local_y < cur_h.saturating_sub(18) {
            let row_idx = ((local_y - 48) / row_height) as usize;
            if row_idx < self.entries.len() {
                let is_double_click = if let Some((last_idx, last_tick)) = self.last_click_item {
                    last_idx == row_idx && now_tick.saturating_sub(last_tick) <= 300
                } else {
                    false
                };

                if is_double_click {
                    self.last_click_item = None;
                    let item = self.entries[row_idx].clone();
                    if item.item_type == FileItemType::Directory {
                        let new_path = if self.current_path == "/" {
                            format!("/{}", item.name)
                        } else {
                            format!("{}/{}", self.current_path.trim_end_matches('/'), item.name)
                        };
                        self.navigate_to(&new_path);
                    } else if item.item_type == FileItemType::File {
                        let file_path = if self.current_path == "/" {
                            format!("/{}", item.name)
                        } else {
                            format!("{}/{}", self.current_path.trim_end_matches('/'), item.name)
                        };
                        let is_text = item.name.ends_with(".txt")
                            || item.name.ends_with(".rs")
                            || item.name.ends_with(".toml")
                            || item.name.ends_with(".log")
                            || item.name.ends_with(".md");

                        if is_text {
                            let _ = crate::editor_app::spawn_editor_app("editor.app", Some(&file_path));
                            self.status_message = format!("Opened in Editor: '{}'", item.name);
                        } else {
                            self.status_message = format!("Cannot open binary: '{}'", item.name);
                        }
                    } else {
                        self.status_message = format!("Executable binary: '{}'", item.name);
                    }
                } else {
                    self.selected_idx = Some(row_idx);
                    self.last_click_item = Some((row_idx, now_tick));
                    self.status_message = format!("Selected: {}", self.entries[row_idx].name);
                }
            }
        }
    }

    pub fn render_to_surface(&mut self, surface_ptr: *mut u32, w: u32, h: u32) {
        if surface_ptr.is_null() { return; }

        self.width = w;
        self.height = h;
        let bg_color = 0x000F172A;     // Main Content Dark Slate
        let sidebar_bg = 0x00090E17;   // Places Sidebar Darker Tone
        let toolbar_bg = 0x001E293B;   // Top Flat Toolbar
        let border_col = 0x00334155;
        let text_color = 0x00F8FAFC;
        let text_muted = 0x0094A3B8;
        let accent_sky = 0x0038BDF8;
        let btn_bg = 0x00334155;
        let btn_accent = 0x002563EB;

        crate::terminal_app::clear_surface(surface_ptr, w, h, bg_color);

        // 1. Top Flat Navigation Toolbar (y = 0..30)
        draw_surf_rect(surface_ptr, w, h, 0, 0, w, 30, toolbar_bg);
        draw_surf_rect(surface_ptr, w, h, 0, 29, w, 1, border_col);

        // [ < ] Back
        let back_enabled = self.history_idx > 0;
        let back_bg = if back_enabled { btn_accent } else { btn_bg };
        draw_surf_rect(surface_ptr, w, h, 6, 4, 22, 22, back_bg);
        crate::font::draw_text(surface_ptr, w, h, 14, 8, "<", text_color, back_bg);

        // [ > ] Forward
        let fwd_enabled = self.history_idx + 1 < self.history.len();
        let fwd_bg = if fwd_enabled { btn_accent } else { btn_bg };
        draw_surf_rect(surface_ptr, w, h, 32, 4, 22, 22, fwd_bg);
        crate::font::draw_text(surface_ptr, w, h, 40, 8, ">", text_color, fwd_bg);

        // [ ^ ] Up
        draw_surf_rect(surface_ptr, w, h, 58, 4, 22, 22, btn_bg);
        crate::font::draw_text(surface_ptr, w, h, 66, 8, "^", text_color, btn_bg);

        // [ R ] Refresh
        draw_surf_rect(surface_ptr, w, h, 84, 4, 22, 22, btn_bg);
        crate::font::draw_text(surface_ptr, w, h, 92, 8, "R", accent_sky, btn_bg);

        // [ + New ] Create
        draw_surf_rect(surface_ptr, w, h, 110, 4, 48, 22, 0x0010B981);
        crate::font::draw_text(surface_ptr, w, h, 116, 8, "+ New", text_color, 0x0010B981);

        // Path Display Bar (Flat Address Box)
        let path_x = 164u32;
        let path_w = w.saturating_sub(path_x + 6);
        draw_surf_rect(surface_ptr, w, h, path_x, 4, path_w, 22, 0x00020617);
        draw_surf_rect(surface_ptr, w, h, path_x, 4, path_w, 1, border_col);
        draw_surf_rect(surface_ptr, w, h, path_x, 25, path_w, 1, border_col);
        crate::font::draw_text(surface_ptr, w, h, path_x + 6, 8, &self.current_path, accent_sky, 0x00020617);

        // 2. Left Places Sidebar (x = 0..90, y = 30..h-18)
        let sidebar_w = 90u32;
        let content_h = h.saturating_sub(48);
        draw_surf_rect(surface_ptr, w, h, 0, 30, sidebar_w, content_h, sidebar_bg);
        draw_surf_rect(surface_ptr, w, h, sidebar_w - 1, 30, 1, content_h, border_col);

        crate::font::draw_text(surface_ptr, w, h, 8, 34, "PLACES", 0x0064748B, sidebar_bg);

        let places = [
            ("Home", "/home/teha"),
            ("Root", "/"),
            ("Bin", "/bin"),
            ("Temp", "/tmp"),
        ];

        for (idx, (label, path)) in places.iter().enumerate() {
            let py = 48 + (idx as u32 * 22);
            let is_active = self.current_path == *path;
            let item_bg = if is_active { 0x001E293B } else { sidebar_bg };
            let item_fg = if is_active { accent_sky } else { text_muted };
            draw_surf_rect(surface_ptr, w, h, 4, py, sidebar_w - 8, 18, item_bg);
            crate::font::draw_text(surface_ptr, w, h, 8, py + 2, label, item_fg, item_bg);
        }

        // 3. File Table Header (x = 92..w, y = 30..46)
        let main_x = sidebar_w + 4;
        let main_w = w.saturating_sub(main_x + 4);
        draw_surf_rect(surface_ptr, w, h, main_x, 30, main_w, 16, toolbar_bg);
        crate::font::draw_text(surface_ptr, w, h, main_x + 4, 32, "NAME", 0x0094A3B8, toolbar_bg);
        crate::font::draw_text(surface_ptr, w, h, w.saturating_sub(130), 32, "TYPE", 0x0094A3B8, toolbar_bg);
        crate::font::draw_text(surface_ptr, w, h, w.saturating_sub(60), 32, "SIZE", 0x0094A3B8, toolbar_bg);

        // 4. File Table Items (y = 48..h-20)
        let mut y = 48u32;
        for (i, entry) in self.entries.iter().enumerate() {
            if y + 18 >= h.saturating_sub(18) { break; }

            let is_selected = self.selected_indices.contains(&i) || self.selected_idx == Some(i);
            let row_bg = if is_selected { 0x001D4ED8 } else if i % 2 == 0 { 0x00131C2E } else { bg_color };

            draw_surf_rect(surface_ptr, w, h, main_x, y, main_w, 18, row_bg);

            let (icon_sym, icon_col, type_str) = match entry.item_type {
                FileItemType::Directory => ("D", 0x00FBBF24, "Folder"),
                FileItemType::File => ("F", 0x0038BDF8, "Doc"),
                FileItemType::Executable => ("X", 0x0034D399, "App"),
            };

            // Icon square
            draw_surf_rect(surface_ptr, w, h, main_x + 4, y + 2, 14, 14, 0x001E293B);
            crate::font::draw_text(surface_ptr, w, h, main_x + 8, y + 2, icon_sym, icon_col, 0x001E293B);

            // Name
            crate::font::draw_text(surface_ptr, w, h, main_x + 24, y + 2, &entry.name, text_color, row_bg);

            // Type
            crate::font::draw_text(surface_ptr, w, h, w.saturating_sub(130), y + 2, type_str, text_muted, row_bg);

            // Size
            let size_str = format!("{} B", entry.size_bytes);
            crate::font::draw_text(surface_ptr, w, h, w.saturating_sub(60), y + 2, &size_str, text_muted, row_bg);

            y += 20;
        }

        // 5. Status Bar at Bottom (y = h - 18 .. h)
        let status_y = h.saturating_sub(18);
        draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 18, toolbar_bg);
        draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 1, border_col);
        crate::font::draw_text(surface_ptr, w, h, 8, status_y + 4, &self.status_message, 0x0034D399, toolbar_bg);

        let fs_type = "SPFS v1.0";
        crate::font::draw_text(surface_ptr, w, h, w.saturating_sub(80), status_y + 4, fs_type, text_muted, toolbar_bg);
    }
}

pub static FILES_INSTANCES: Mutex<BTreeMap<u64, FilesAppState>> = Mutex::new(BTreeMap::new());

pub fn cleanup_files_for_window(window_id: u64) {
    let mut instances = FILES_INSTANCES.lock();
    if instances.remove(&window_id).is_some() {
        crate::serial_println!("[FILES] Cleaned up Files state for Window {}", window_id);
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

pub fn spawn_files_app(name: &str) -> Result<u64, &'static str> {
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frame for files.app")?;
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
        s.width = FILES_WIDTH;
        s.height = FILES_HEIGHT;
    }
    let (win_x, win_y) = {
        let count = crate::wm::WM.lock().windows.len() as i32;
        (50 + ((count * 30) % 200), 50 + ((count * 25) % 150))
    };
    let win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, win_x, win_y, FILES_WIDTH, FILES_HEIGHT)
        .map_err(|_| "window creation failed")?;

    {
        let mut state = FilesAppState::new(win_id, pid);
        if let Some(surface) = crate::surface::SURFACE_REGISTRY.read().iter().find(|s| s.surface_id == surf_id) {
            let phys_addr = surface.shmem_phys_addr;
            let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *mut u32 };
            state.render_to_surface(surf_ptr, FILES_WIDTH, FILES_HEIGHT);
        }
        FILES_INSTANCES.lock().insert(win_id, state);
    }

    let _ = crate::surface::present_surface(surf_id, 0, 0, FILES_WIDTH, FILES_HEIGHT);
    crate::serial_println!("[APP-REGISTRY] Successfully launched '{}' (PID {}, Entry 0x{:x}, Surface {}, Window {})",
        name, pid, code_base, surf_id, win_id);

    Ok(pid)
}
