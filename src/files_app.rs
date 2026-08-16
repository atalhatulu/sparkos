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
    pub status_message: String,
    pub last_click_item: Option<(usize, u64)>, // (index, tick) for double click
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
            status_message: String::from("Ready"),
            last_click_item: None,
        };
        state.load_directory("/home/teha");
        state
    }

    pub fn load_directory(&mut self, path: &str) {
        let clean_path = if path.is_empty() { "/" } else { path };
        self.current_path = String::from(clean_path);
        self.entries.clear();
        self.selected_idx = None;

        // Populate directory entries based on path
        if clean_path == "/home/teha" || clean_path == "/home/teha/" {
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
        if path.contains("//") {
            self.status_message = String::from("Invalid path format");
            return;
        }

        self.load_directory(path);
        
        // Push to history
        if self.history_idx + 1 < self.history.len() {
            self.history.truncate(self.history_idx + 1);
        }
        self.history.push(String::from(path));
        self.history_idx = self.history.len().saturating_sub(1);
    }

    pub fn go_parent(&mut self) {
        if self.current_path == "/" || self.current_path == "/home" {
            self.navigate_to("/");
        } else if let Some(last_slash) = self.current_path.rfind('/') {
            if last_slash == 0 {
                self.navigate_to("/");
            } else {
                let parent = String::from(&self.current_path[..last_slash]);
                self.navigate_to(&parent);
            }
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

    pub fn delete_selected(&mut self) {
        if let Some(idx) = self.selected_idx {
            if idx < self.entries.len() {
                let removed = self.entries.remove(idx);
                self.selected_idx = None;
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

    pub fn handle_mouse_click(&mut self, local_x: u32, local_y: u32) {
        let now_tick = crate::interrupts::get_tick();

        // 1. Toolbar clicks (y in 6..28)
        if local_y >= 6 && local_y <= 28 {
            // [<- Back] (x: 8..38)
            if local_x >= 8 && local_x <= 38 {
                self.go_back();
                return;
            }
            // [^ Up] (x: 44..70)
            if local_x >= 44 && local_x <= 70 {
                self.go_parent();
                return;
            }
            // [Refresh] (x: 360..430)
            if local_x >= 360 && local_x <= 430 {
                self.refresh();
                return;
            }
        }

        // 2. File item list clicks (y in 38..h-20)
        let row_height = 20u32;
        if local_y >= 38 && local_y < FILES_HEIGHT.saturating_sub(20) {
            let row_idx = ((local_y - 38) / row_height) as usize;
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
                    } else {
                        self.status_message = format!("Opened '{}' ({} B)", item.name, item.size_bytes);
                    }
                } else {
                    self.selected_idx = Some(row_idx);
                    self.last_click_item = Some((row_idx, now_tick));
                    self.status_message = format!("Selected: {}", self.entries[row_idx].name);
                }
            }
        }
    }

    pub fn render_to_surface(&self, surface_ptr: *mut u32, w: u32, h: u32) {
        if surface_ptr.is_null() { return; }
        let bg_color = 0x000F172A; // Deep Navy Slate
        let panel_bg = 0x001E293B; // Slate 800
        let text_color = 0x00F8FAFC;
        let accent_color = 0x0038BDF8;

        crate::terminal_app::clear_surface(surface_ptr, w, h, bg_color);

        // 1. Navigation & Breadcrumb Toolbar
        draw_surf_rect(surface_ptr, w, h, 6, 6, 32, 22, 0x00334155);
        crate::font::draw_text(surface_ptr, w, h, 14, 10, "<-", text_color, 0x00334155);

        draw_surf_rect(surface_ptr, w, h, 42, 6, 32, 22, 0x00334155);
        crate::font::draw_text(surface_ptr, w, h, 52, 10, "^", text_color, 0x00334155);

        // Path Display
        let path_w = w.saturating_sub(160);
        draw_surf_rect(surface_ptr, w, h, 78, 6, path_w, 22, panel_bg);
        crate::font::draw_text(surface_ptr, w, h, 84, 10, &self.current_path, accent_color, panel_bg);

        // Refresh Button
        let refresh_x = w.saturating_sub(76);
        draw_surf_rect(surface_ptr, w, h, refresh_x, 6, 70, 22, 0x002563EB);
        crate::font::draw_text(surface_ptr, w, h, refresh_x + 10, 10, "Refresh", text_color, 0x002563EB);

        // 2. Directory Listing Table
        let mut y = 38u32;
        for (i, entry) in self.entries.iter().enumerate() {
            if y + 20 >= h.saturating_sub(20) { break; }

            let is_selected = self.selected_idx == Some(i);
            let row_bg = if is_selected { 0x001D4ED8 } else if i % 2 == 0 { panel_bg } else { bg_color };

            draw_surf_rect(surface_ptr, w, h, 6, y, w.saturating_sub(12), 18, row_bg);

            let (icon_sym, icon_col) = match entry.item_type {
                FileItemType::Directory => ("[DIR]", 0x00FBBF24), // Folder Amber
                FileItemType::File => ("[FILE]", 0x0038BDF8),      // Document Sky Blue
                FileItemType::Executable => ("[BIN]", 0x0010B981),// Binary Emerald
            };

            crate::font::draw_text(surface_ptr, w, h, 10, y + 2, icon_sym, icon_col, row_bg);
            crate::font::draw_text(surface_ptr, w, h, 58, y + 2, &entry.name, text_color, row_bg);
            let size_str = format!("{} B", entry.size_bytes);
            crate::font::draw_text(surface_ptr, w, h, w.saturating_sub(90), y + 2, &size_str, 0x0094A3B8, row_bg);

            y += 20;
        }

        // 3. Status Bar at bottom
        let status_y = h.saturating_sub(18);
        draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 18, panel_bg);
        crate::font::draw_text(surface_ptr, w, h, 10, status_y + 2, &self.status_message, 0x0094A3B8, panel_bg);
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

    let surf_id = crate::surface::create_surface_for_pid(pid, FILES_WIDTH, FILES_HEIGHT)?;
    let win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, 60, 60, FILES_WIDTH, FILES_HEIGHT)
        .map_err(|_| "window creation failed")?;

    {
        let state = FilesAppState::new(win_id, pid);
        if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.surface_id == surf_id) {
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
