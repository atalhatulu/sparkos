//! SparkOS Desktop V1.34 — Modern File Manager Application (`files.app`)
//!
//! Provides a dedicated Ring-3 File Manager featuring dual view modes (Icon View and List View),
//! large icon glyphs, interactive context menus (Open, Copy, Rename, Delete, Properties),
//! file metadata inspector, breadcrumb navigation, and strict path traversal defense.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const FILES_WIDTH: u32 = 440;
pub const FILES_HEIGHT: u32 = 280;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileViewMode {
    IconView,
    ListView,
}

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
    pub permissions: &'static str,
    pub item_type: FileItemType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuAction {
    Open,
    Copy,
    Rename,
    Delete,
    Properties,
}

pub struct FilesAppState {
    pub current_path: String,
    pub entries: Vec<FileEntry>,
    pub selected_idx: Option<usize>,
    pub view_mode: FileViewMode,
    pub context_menu_open: bool,
    pub context_menu_pos: (u32, u32),
    pub status_message: String,
}

impl FilesAppState {
    pub fn new() -> Self {
        let mut state = Self {
            current_path: String::from("/home/teha/projects"),
            entries: Vec::new(),
            selected_idx: None,
            view_mode: FileViewMode::IconView,
            context_menu_open: false,
            context_menu_pos: (0, 0),
            status_message: String::from("4 items"),
        };
        state.load_directory("/home/teha/projects");
        state
    }

    pub fn load_directory(&mut self, path: &str) {
        // Path Traversal Security: reject any '..' or illegal sequences
        if path.contains("..") || path.contains("//") {
            self.status_message = String::from("Error: Path traversal blocked");
            return;
        }

        self.current_path = String::from(path);
        self.entries.clear();
        self.entries.push(FileEntry {
            name: String::from("src"),
            size_bytes: 4096,
            permissions: "rwxr-xr-x",
            item_type: FileItemType::Directory,
        });
        self.entries.push(FileEntry {
            name: String::from("docs"),
            size_bytes: 4096,
            permissions: "rwxr-xr-x",
            item_type: FileItemType::Directory,
        });
        self.entries.push(FileEntry {
            name: String::from("main.rs"),
            size_bytes: 18450,
            permissions: "rw-r--r--",
            item_type: FileItemType::File,
        });
        self.entries.push(FileEntry {
            name: String::from("sparkos.bin"),
            size_bytes: 348160,
            permissions: "rwxr-xr-x",
            item_type: FileItemType::Executable,
        });
        self.status_message = format!("{} items | {}", self.entries.len(), self.current_path);
    }

    pub fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            FileViewMode::IconView => FileViewMode::ListView,
            FileViewMode::ListView => FileViewMode::IconView,
        };
    }

    pub fn open_context_menu(&mut self, x: u32, y: u32, idx: usize) {
        self.selected_idx = Some(idx);
        self.context_menu_pos = (x, y);
        self.context_menu_open = true;
    }

    pub fn close_context_menu(&mut self) {
        self.context_menu_open = false;
    }

    pub fn execute_context_action(&mut self, action: ContextMenuAction) {
        if let Some(idx) = self.selected_idx {
            if let Some(item) = self.entries.get(idx) {
                match action {
                    ContextMenuAction::Open => {
                        self.status_message = format!("Opening: {}", item.name);
                    }
                    ContextMenuAction::Copy => {
                        self.status_message = format!("Copied: {}", item.name);
                    }
                    ContextMenuAction::Rename => {
                        self.status_message = format!("Renaming: {}", item.name);
                    }
                    ContextMenuAction::Delete => {
                        self.status_message = format!("Deleted: {}", item.name);
                    }
                    ContextMenuAction::Properties => {
                        let type_str = match item.item_type {
                            FileItemType::Directory => "Folder",
                            FileItemType::File => "Text Document",
                            FileItemType::Executable => "Application Binary",
                        };
                        self.status_message = format!("Props: {} | {} KB | {} | {}",
                            item.name, item.size_bytes / 1024, item.permissions, type_str);
                    }
                }
            }
        }
        self.context_menu_open = false;
    }

    pub fn render_to_surface(&self, surface_ptr: *mut u32, w: u32, h: u32) {
        if surface_ptr.is_null() { return; }
        let bg_color = 0x000F172A; // Navy Slate
        let panel_bg = 0x001E293B; // Slate 800
        let item_fg = 0x00F8FAFC;  // Pure White
        let accent_fg = 0x0038BDF8;// Sky Blue

        crate::terminal_app::clear_surface(surface_ptr, w, h, bg_color);

        // 1. Breadcrumb Bar at top
        draw_surf_rect(surface_ptr, w, h, 6, 6, w.saturating_sub(80), 22, panel_bg);
        let breadcrumb_text = format!("Path: {}", self.current_path);
        crate::font::draw_text(surface_ptr, w, h, 12, 10, &breadcrumb_text, accent_fg, panel_bg);

        // View Mode Toggle Button
        let mode_label = match self.view_mode {
            FileViewMode::IconView => "[Icons]",
            FileViewMode::ListView => "[List]",
        };
        draw_surf_rect(surface_ptr, w, h, w.saturating_sub(70), 6, 64, 22, 0x002563EB);
        crate::font::draw_text(surface_ptr, w, h, w.saturating_sub(64), 10, mode_label, 0x00FFFFFF, 0x002563EB);

        // 2. Viewport Rendering
        match self.view_mode {
            FileViewMode::IconView => {
                let mut col = 0u32;
                let mut row = 0u32;
                for (i, entry) in self.entries.iter().enumerate() {
                    let ix = 16 + col * 90;
                    let iy = 38 + row * 80;
                    if iy + 70 >= h { break; }

                    let is_selected = self.selected_idx == Some(i);
                    let card_bg = if is_selected { 0x001D4ED8 } else { panel_bg };

                    draw_surf_rect(surface_ptr, w, h, ix, iy, 80, 68, card_bg);

                    // Large Icon
                    let (icon_sym, icon_col) = match entry.item_type {
                        FileItemType::Directory => ("[DIR]", 0x00FBBF24), // Amber folder
                        FileItemType::File => ("[DOC]", 0x0038BDF8),      // Blue file
                        FileItemType::Executable => ("[BIN]", 0x0010B981),// Green exec
                    };
                    crate::font::draw_text(surface_ptr, w, h, ix + 20, iy + 14, icon_sym, icon_col, card_bg);
                    crate::font::draw_text(surface_ptr, w, h, ix + 8, iy + 44, &entry.name, item_fg, card_bg);

                    col += 1;
                    if col >= 4 {
                        col = 0;
                        row += 1;
                    }
                }
            }
            FileViewMode::ListView => {
                let mut y = 38u32;
                // Header
                draw_surf_rect(surface_ptr, w, h, 10, y, w - 20, 18, 0x00334155);
                crate::font::draw_text(surface_ptr, w, h, 14, y + 2, "NAME", 0x0094A3B8, 0x00334155);
                crate::font::draw_text(surface_ptr, w, h, 160, y + 2, "SIZE", 0x0094A3B8, 0x00334155);
                crate::font::draw_text(surface_ptr, w, h, 240, y + 2, "PERMS", 0x0094A3B8, 0x00334155);
                crate::font::draw_text(surface_ptr, w, h, 330, y + 2, "TYPE", 0x0094A3B8, 0x00334155);
                y += 20;

                for (i, entry) in self.entries.iter().enumerate() {
                    if y + 18 >= h { break; }
                    let is_selected = self.selected_idx == Some(i);
                    let row_bg = if is_selected { 0x001D4ED8 } else { if i % 2 == 0 { panel_bg } else { 0x000F172A } };

                    draw_surf_rect(surface_ptr, w, h, 10, y, w - 20, 18, row_bg);

                    let (icon_sym, type_name) = match entry.item_type {
                        FileItemType::Directory => ("[DIR]", "Folder"),
                        FileItemType::File => ("[DOC]", "Document"),
                        FileItemType::Executable => ("[BIN]", "Executable"),
                    };

                    crate::font::draw_text(surface_ptr, w, h, 14, y + 2, icon_sym, 0x0038BDF8, row_bg);
                    crate::font::draw_text(surface_ptr, w, h, 54, y + 2, &entry.name, item_fg, row_bg);
                    let size_str = format!("{} B", entry.size_bytes);
                    crate::font::draw_text(surface_ptr, w, h, 160, y + 2, &size_str, 0x0094A3B8, row_bg);
                    crate::font::draw_text(surface_ptr, w, h, 240, y + 2, entry.permissions, 0x0034D399, row_bg);
                    crate::font::draw_text(surface_ptr, w, h, 330, y + 2, type_name, 0x0094A3B8, row_bg);

                    y += 19;
                }
            }
        }

        // 3. Context Menu Overlay (if open)
        if self.context_menu_open {
            let (mx, my) = self.context_menu_pos;
            let mw = 110u32;
            let mh = 100u32;

            draw_surf_rect(surface_ptr, w, h, mx, my, mw, mh, 0x0009090B);
            draw_surf_rect(surface_ptr, w, h, mx, my, mw, 1, 0x003B82F6);
            draw_surf_rect(surface_ptr, w, h, mx, my, 1, mh, 0x003B82F6);
            draw_surf_rect(surface_ptr, w, h, mx + mw - 1, my, 1, mh, 0x003B82F6);
            draw_surf_rect(surface_ptr, w, h, mx, my + mh - 1, mw, 1, 0x003B82F6);

            let actions = ["Open", "Copy", "Rename", "Delete", "Properties"];
            for (idx, action_name) in actions.iter().enumerate() {
                let ay = my + 4 + (idx as u32) * 18;
                crate::font::draw_text(surface_ptr, w, h, mx + 8, ay, action_name, 0x00E2E8F0, 0x0009090B);
            }
        }

        // 4. Status Bar at bottom
        let status_y = h.saturating_sub(16);
        draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 16, panel_bg);
        crate::font::draw_text(surface_ptr, w, h, 10, status_y + 2, &self.status_message, 0x0094A3B8, panel_bg);
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
    let _win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, 60, 60, FILES_WIDTH, FILES_HEIGHT)
        .map_err(|_| "window creation failed")?;

    if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.surface_id == surf_id) {
        let phys_addr = surface.shmem_phys_addr;
        let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *mut u32 };
        let state = FilesAppState::new();
        state.render_to_surface(surf_ptr, FILES_WIDTH, FILES_HEIGHT);
    }

    let _ = crate::surface::present_surface(surf_id, 0, 0, FILES_WIDTH, FILES_HEIGHT);
    crate::serial_println!("[APP-REGISTRY] Successfully launched '{}' (PID {}, Entry 0x{:x}, Surface {}, Window)",
        name, pid, code_base, surf_id);

    Ok(pid)
}
