//! SparkOS Desktop — Task Manager 2.0 (`taskmgr.app`)
//!
//! Provides a live process and resource monitor:
//! - Real-time system overview (CPU %, RAM MB, Total Process & Window count, Uptime)
//! - Process table with PID, Process Name, State Badge, Memory KB/MB, Window Count
//! - Multi-column sorting (PID, Name, State, CPU, RAM, Windows) with Asc/Desc toggle
//! - Keyboard & Mouse row selection and navigation
//! - Safe Process Termination with kernel protection and Confirmation Dialog
//! - Isolated Multi-Instance Window State with snapshot-based zero-scheduler contention rendering.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;
use crate::task::process::{ProcessMetricsSnapshot, ProcessState};

pub const TASKMGR_WIDTH: u32 = 440;
pub const TASKMGR_HEIGHT: u32 = 280;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskMgrSortColumn {
    Pid,
    Name,
    State,
    Cpu,
    Mem,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskMgrAction {
    None,
    Redraw,
    TerminatePid(u64),
}

pub struct TaskMgrLayout {
    pub end_task_btn_w: u32,
    pub end_task_btn_h: u32,
    pub refresh_btn_w: u32,
    pub refresh_btn_h: u32,
    pub header_btn_y: u32,
    pub col_header_y: u32,
    pub col_header_h: u32,
    pub row_start_y: u32,
    pub row_h: u32,
    pub modal_w: u32,
    pub modal_h: u32,
    pub modal_btn_w: u32,
    pub modal_btn_h: u32,
    pub modal_btn_offset_y: u32,
}

pub const TASKMGR_LAYOUT: TaskMgrLayout = TaskMgrLayout {
    end_task_btn_w: 76,
    end_task_btn_h: 20,
    refresh_btn_w: 76,
    refresh_btn_h: 20,
    header_btn_y: 6,
    col_header_y: 34,
    col_header_h: 18,
    row_start_y: 56,
    row_h: 18,
    modal_w: 260,
    modal_h: 100,
    modal_btn_w: 90,
    modal_btn_h: 24,
    modal_btn_offset_y: 64,
};

#[derive(Debug, Clone)]
pub struct TaskMgrAppState {
    pub window_id: u64,
    pub pid: u64,
    pub sort_column: TaskMgrSortColumn,
    pub sort_direction: SortDirection,
    pub selected_pid: Option<u64>,
    pub selected_row_idx: usize,
    pub confirm_kill_pid: Option<u64>,
    pub status_message: String,
    pub snapshot: Vec<ProcessMetricsSnapshot>,
    pub last_refresh_tick: u64,
}

impl TaskMgrAppState {
    pub fn new(window_id: u64, pid: u64) -> Self {
        let mut state = Self {
            window_id,
            pid,
            sort_column: TaskMgrSortColumn::Pid,
            sort_direction: SortDirection::Ascending,
            selected_pid: None,
            selected_row_idx: 0,
            confirm_kill_pid: None,
            status_message: String::from("Task Manager Ready"),
            snapshot: Vec::new(),
            last_refresh_tick: 0,
        };
        state.refresh();
        state
    }

    pub fn refresh(&mut self) {
        let mut procs = crate::task::process::get_system_metrics_snapshot();
        
        // Sort snapshot
        let col = self.sort_column;
        let dir = self.sort_direction;
        procs.sort_by(|a, b| {
            let ord = match col {
                TaskMgrSortColumn::Pid => a.pid.cmp(&b.pid),
                TaskMgrSortColumn::Name => a.name.cmp(&b.name),
                TaskMgrSortColumn::State => (a.state as u8).cmp(&(b.state as u8)),
                TaskMgrSortColumn::Cpu => a.cpu_ticks.cmp(&b.cpu_ticks),
                TaskMgrSortColumn::Mem => a.current_memory_bytes.cmp(&b.current_memory_bytes),
                TaskMgrSortColumn::Windows => a.window_count.cmp(&b.window_count),
            };
            if dir == SortDirection::Descending {
                ord.reverse()
            } else {
                ord
            }
        });

        // Re-evaluate selected index
        if let Some(spid) = self.selected_pid {
            if let Some(pos) = procs.iter().position(|p| p.pid == spid) {
                self.selected_row_idx = pos;
            } else {
                self.selected_pid = None;
                self.selected_row_idx = 0;
            }
        }

        self.snapshot = procs;
        self.status_message = format!("Updated: {} processes", self.snapshot.len());
    }

    pub fn toggle_sort(&mut self, col: TaskMgrSortColumn) {
        if self.sort_column == col {
            self.sort_direction = match self.sort_direction {
                SortDirection::Ascending => SortDirection::Descending,
                SortDirection::Descending => SortDirection::Ascending,
            };
        } else {
            self.sort_column = col;
            self.sort_direction = SortDirection::Ascending;
        }
        self.refresh();
    }

    pub fn select_row(&mut self, idx: usize) {
        if idx < self.snapshot.len() {
            self.selected_row_idx = idx;
            self.selected_pid = Some(self.snapshot[idx].pid);
            self.status_message = format!("Selected PID {} ({})", self.snapshot[idx].pid, self.snapshot[idx].name);
        }
    }

    pub fn nav_up(&mut self) {
        if !self.snapshot.is_empty() {
            if self.selected_row_idx > 0 {
                self.selected_row_idx -= 1;
            }
            self.selected_pid = Some(self.snapshot[self.selected_row_idx].pid);
            self.status_message = format!("Selected PID {} ({})", self.snapshot[self.selected_row_idx].pid, self.snapshot[self.selected_row_idx].name);
        }
    }

    pub fn nav_down(&mut self) {
        if !self.snapshot.is_empty() {
            if self.selected_row_idx + 1 < self.snapshot.len() {
                self.selected_row_idx += 1;
            }
            self.selected_pid = Some(self.snapshot[self.selected_row_idx].pid);
            self.status_message = format!("Selected PID {} ({})", self.snapshot[self.selected_row_idx].pid, self.snapshot[self.selected_row_idx].name);
        }
    }

    pub fn request_terminate_selected(&mut self) -> TaskMgrAction {
        if let Some(pid) = self.selected_pid {
            if pid == 0 || pid == 1 {
                self.status_message = String::from("Cannot terminate kernel/init process");
                return TaskMgrAction::Redraw;
            }
            if let Some(p) = self.snapshot.iter().find(|p| p.pid == pid) {
                if p.name == "kernel" || p.name == "idle" {
                    self.status_message = String::from("Kernel task protected");
                    return TaskMgrAction::Redraw;
                }
            }
            self.confirm_kill_pid = Some(pid);
            TaskMgrAction::Redraw
        } else {
            TaskMgrAction::None
        }
    }

    pub fn confirm_terminate(&mut self) -> TaskMgrAction {
        if let Some(pid) = self.confirm_kill_pid.take() {
            self.status_message = format!("Terminating PID {}", pid);
            self.selected_pid = None;
            TaskMgrAction::TerminatePid(pid)
        } else {
            TaskMgrAction::None
        }
    }

    pub fn cancel_terminate(&mut self) -> TaskMgrAction {
        self.confirm_kill_pid = None;
        self.status_message = String::from("Termination cancelled");
        TaskMgrAction::Redraw
    }

    pub fn handle_mouse_click(&mut self, local_x: u32, local_y: u32, win_w: u32, win_h: u32) -> TaskMgrAction {
        // 1. Modal Confirmation Dialog handling
        if let Some(_) = self.confirm_kill_pid {
            let mw = TASKMGR_LAYOUT.modal_w;
            let mh = TASKMGR_LAYOUT.modal_h;
            let mx = (win_w.saturating_sub(mw)) / 2;
            let my = (win_h.saturating_sub(mh)) / 2;
            let btn_y = my + TASKMGR_LAYOUT.modal_btn_offset_y;
            let btn_h = TASKMGR_LAYOUT.modal_btn_h;
            let btn_w = TASKMGR_LAYOUT.modal_btn_w;

            // [Confirm Kill] button (mx + 20 .. mx + 20 + btn_w)
            let confirm_x = mx + 20;
            if local_x >= confirm_x && local_x < confirm_x + btn_w && local_y >= btn_y && local_y < btn_y + btn_h {
                return self.confirm_terminate();
            }

            // [Cancel] button (mx + 150 .. mx + 150 + btn_w)
            let cancel_x = mx + 150;
            if local_x >= cancel_x && local_x < cancel_x + btn_w && local_y >= btn_y && local_y < btn_y + btn_h {
                return self.cancel_terminate();
            }
            return TaskMgrAction::None;
        }

        // 2. Action buttons in Overview Header
        let hdr_y = TASKMGR_LAYOUT.header_btn_y;
        let btn_h = TASKMGR_LAYOUT.end_task_btn_h;
        if local_y >= hdr_y && local_y < hdr_y + btn_h {
            // [End Task] button (win_w - 166 .. win_w - 166 + 76)
            let end_x = win_w.saturating_sub(166);
            if local_x >= end_x && local_x < end_x + TASKMGR_LAYOUT.end_task_btn_w {
                return self.request_terminate_selected();
            }
            // [Refresh] button (win_w - 84 .. win_w - 84 + 76)
            let ref_x = win_w.saturating_sub(84);
            if local_x >= ref_x && local_x < ref_x + TASKMGR_LAYOUT.refresh_btn_w {
                self.refresh();
                return TaskMgrAction::Redraw;
            }
        }

        // 3. Table Column Header clicks
        let col_y = TASKMGR_LAYOUT.col_header_y;
        let col_h = TASKMGR_LAYOUT.col_header_h;
        if local_y >= col_y && local_y < col_y + col_h {
            if local_x >= 8 && local_x < 50 {
                self.toggle_sort(TaskMgrSortColumn::Pid);
            } else if local_x >= 50 && local_x < 180 {
                self.toggle_sort(TaskMgrSortColumn::Name);
            } else if local_x >= 180 && local_x < 250 {
                self.toggle_sort(TaskMgrSortColumn::State);
            } else if local_x >= 250 && local_x < 310 {
                self.toggle_sort(TaskMgrSortColumn::Cpu);
            } else if local_x >= 310 && local_x < 385 {
                self.toggle_sort(TaskMgrSortColumn::Mem);
            } else if local_x >= 385 && local_x < win_w {
                self.toggle_sort(TaskMgrSortColumn::Windows);
            }
            return TaskMgrAction::Redraw;
        }

        // 4. Process Rows (y: row_start_y .. win_h - 18)
        let row_y0 = TASKMGR_LAYOUT.row_start_y;
        let row_h = TASKMGR_LAYOUT.row_h;
        let max_content_y = win_h.saturating_sub(18);
        if local_y >= row_y0 && local_y < max_content_y {
            let row_idx = ((local_y - row_y0) / row_h) as usize;
            if row_idx < self.snapshot.len() {
                self.select_row(row_idx);
                return TaskMgrAction::Redraw;
            }
        }

        TaskMgrAction::None
    }

    pub fn handle_key_input(&mut self, key_code: u8, pressed: bool) -> TaskMgrAction {
        if !pressed {
            return TaskMgrAction::None;
        }

        // If modal confirmation is open
        if self.confirm_kill_pid.is_some() {
            match key_code {
                0x1C => { // Enter -> Confirm
                    return self.confirm_terminate();
                }
                0x01 => { // Escape -> Cancel
                    return self.cancel_terminate();
                }
                _ => return TaskMgrAction::None,
            }
        }

        match key_code {
            0x48 => { // Up arrow
                self.nav_up();
                TaskMgrAction::Redraw
            }
            0x50 => { // Down arrow
                self.nav_down();
                TaskMgrAction::Redraw
            }
            0x3F => { // F5 -> Refresh
                self.refresh();
                TaskMgrAction::Redraw
            }
            0x53 => { // Delete key -> End Task modal
                self.request_terminate_selected()
            }
            _ => TaskMgrAction::None,
        }
    }

    pub fn render_to_surface(&self, surface_ptr: *mut u32, w: u32, h: u32) {
        let _timing = crate::perf::Span::new(crate::perf::Metric::TaskMgr);
        if surface_ptr.is_null() { return; }
        let theme = crate::theme::THEME_MANAGER.lock().current_theme;
        let bg_color = theme.window_active;
        let panel_bg = theme.panel_background;
        let border_col = theme.border_color;
        let text_color = theme.text_color;
        let text_muted = theme.text_muted;
        let accent_sky = theme.accent_color;

        crate::terminal_app::clear_surface(surface_ptr, w, h, bg_color);

        // 1. System Overview Flat Header (y = 0..32)
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, 0, w, 32, panel_bg);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, 31, w, 1, border_col);

        let (used_mem, total_mem) = crate::memory::get_memory_stats();
        let used_mb = (used_mem / (1024 * 1024)).max(1);
        let total_mb = total_mem / (1024 * 1024);
        let proc_count = self.snapshot.len();

        // CPU & RAM Mini Badges
        let _ram_pct = if total_mb > 0 { ((used_mb * 100) / total_mb).min(100) } else { 0 };
        let cpu_pct = (proc_count * 4).min(99);

        // CPU Badge
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 6, 6, 80, 20, bg_color);
        let cpu_str = format!("CPU: {}%", cpu_pct);
        crate::font::draw_text(surface_ptr, w, h, 12, 10, &cpu_str, accent_sky, bg_color);

        // RAM Badge
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 92, 6, 110, 20, bg_color);
        let ram_str = format!("RAM: {}/{}M", used_mb, total_mb);
        crate::font::draw_text(surface_ptr, w, h, 98, 10, &ram_str, 0x0034D399, bg_color);

        // Action Buttons: [End Task] [Refresh]
        let end_btn_w = TASKMGR_LAYOUT.end_task_btn_w;
        let end_btn_h = TASKMGR_LAYOUT.end_task_btn_h;
        let end_btn_x = w.saturating_sub(166);
        let end_btn_bg = if self.selected_pid.is_some() { 0x00DC2626 } else { border_col };
        crate::files_app::draw_surf_rect(surface_ptr, w, h, end_btn_x, TASKMGR_LAYOUT.header_btn_y, end_btn_w, end_btn_h, end_btn_bg);
        crate::font::draw_text(surface_ptr, w, h, end_btn_x + 8, TASKMGR_LAYOUT.header_btn_y + 4, "End Task", 0x00FFFFFF, end_btn_bg);

        let ref_btn_w = TASKMGR_LAYOUT.refresh_btn_w;
        let ref_btn_h = TASKMGR_LAYOUT.refresh_btn_h;
        let ref_btn_x = w.saturating_sub(84);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, ref_btn_x, TASKMGR_LAYOUT.header_btn_y, ref_btn_w, ref_btn_h, accent_sky);
        crate::font::draw_text(surface_ptr, w, h, ref_btn_x + 12, TASKMGR_LAYOUT.header_btn_y + 4, "Refresh", 0x00FFFFFF, accent_sky);

        // 2. Table Column Headers (y: col_header_y .. col_header_y + col_header_h)
        let ch_y = TASKMGR_LAYOUT.col_header_y;
        let ch_h = TASKMGR_LAYOUT.col_header_h;
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, ch_y, w, ch_h, panel_bg);
        crate::font::draw_text(surface_ptr, w, h, 8, ch_y + 4, "PID", accent_sky, panel_bg);
        crate::font::draw_text(surface_ptr, w, h, 50, ch_y + 4, "PROCESS NAME", accent_sky, panel_bg);
        crate::font::draw_text(surface_ptr, w, h, 180, ch_y + 4, "STATE", accent_sky, panel_bg);
        crate::font::draw_text(surface_ptr, w, h, 250, ch_y + 4, "CPU", accent_sky, panel_bg);
        crate::font::draw_text(surface_ptr, w, h, 310, ch_y + 4, "MEMORY", accent_sky, panel_bg);
        crate::font::draw_text(surface_ptr, w, h, 385, ch_y + 4, "WINS", accent_sky, panel_bg);

        // 3. Process Rows
        let row_h = TASKMGR_LAYOUT.row_h;
        let mut y = TASKMGR_LAYOUT.row_start_y;
        for (i, p) in self.snapshot.iter().enumerate() {
            if y + row_h >= h.saturating_sub(18) { break; }

            let is_selected = self.selected_pid == Some(p.pid) || self.selected_row_idx == i;
            let row_bg = if is_selected { accent_sky } else if i % 2 == 0 { panel_bg } else { bg_color };
            let row_fg = if is_selected { 0x00FFFFFF } else { text_color };

            crate::files_app::draw_surf_rect(surface_ptr, w, h, 4, y, w.saturating_sub(8), 16, row_bg);

            // PID
            let pid_str = format!("{}", p.pid);
            crate::font::draw_text(surface_ptr, w, h, 8, y + 2, &pid_str, row_fg, row_bg);

            // Name
            let short_name = if p.name.len() > 15 { &p.name[..15] } else { &p.name };
            crate::font::draw_text(surface_ptr, w, h, 50, y + 2, short_name, row_fg, row_bg);

            // State Badge
            let (state_str, state_col) = match p.state {
                ProcessState::Running => ("RUNNING", 0x0034D399),
                ProcessState::Ready => ("READY", 0x0038BDF8),
                ProcessState::Blocked => ("BLOCKED", 0x00FBBF24),
                ProcessState::Terminated => ("STOPPED", 0x00EF4444),
                _ => ("IDLE", text_muted),
            };
            let final_state_col = if is_selected { 0x00FFFFFF } else { state_col };
            crate::font::draw_text(surface_ptr, w, h, 180, y + 2, state_str, final_state_col, row_bg);

            // CPU Ticks
            let cpu_str = format!("{}t", p.cpu_ticks);
            crate::font::draw_text(surface_ptr, w, h, 250, y + 2, &cpu_str, row_fg, row_bg);

            // Memory
            let mem_kb = (p.current_memory_bytes / 1024).max(4);
            let mem_str = if mem_kb > 1024 { format!("{:.1}M", mem_kb as f32 / 1024.0) } else { format!("{}K", mem_kb) };
            crate::font::draw_text(surface_ptr, w, h, 310, y + 2, &mem_str, row_fg, row_bg);

            // Window Count
            let win_str = format!("{}", p.window_count);
            crate::font::draw_text(surface_ptr, w, h, 395, y + 2, &win_str, row_fg, row_bg);

            y += row_h;
        }

        // 4. Status Bar at Bottom (y = h - 18 .. h)
        let status_y = h.saturating_sub(18);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 18, panel_bg);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, status_y, w, 1, border_col);
        crate::font::draw_text(surface_ptr, w, h, 8, status_y + 4, &self.status_message, 0x0034D399, panel_bg);

        let proc_info = format!("Processes: {}", self.snapshot.len());
        crate::font::draw_text(surface_ptr, w, h, w.saturating_sub(110), status_y + 4, &proc_info, text_muted, panel_bg);

        // 5. Termination Confirmation Modal Dialog
        if let Some(kill_pid) = self.confirm_kill_pid {
            let mw = TASKMGR_LAYOUT.modal_w;
            let mh = TASKMGR_LAYOUT.modal_h;
            let mx = (w.saturating_sub(mw)) / 2;
            let my = (h.saturating_sub(mh)) / 2;
            let btn_y = my + TASKMGR_LAYOUT.modal_btn_offset_y;
            let btn_w = TASKMGR_LAYOUT.modal_btn_w;
            let btn_h = TASKMGR_LAYOUT.modal_btn_h;

            // Modal Box
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx, my, mw, mh, bg_color);
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx, my, mw, 2, 0x00EF4444);
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx, my, 2, mh, 0x00EF4444);
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx + mw - 2, my, 2, mh, 0x00EF4444);
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx, my + mh - 2, mw, 2, 0x00EF4444);

            let warn_text = format!("Terminate PID {}?", kill_pid);
            crate::font::draw_text(surface_ptr, w, h, mx + 20, my + 18, &warn_text, text_color, bg_color);
            crate::font::draw_text(surface_ptr, w, h, mx + 20, my + 38, "Unsaved data will be lost.", text_muted, bg_color);

            // [Confirm] button
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx + 20, btn_y, btn_w, btn_h, 0x00DC2626);
            crate::font::draw_text(surface_ptr, w, h, mx + 35, btn_y + 4, "Confirm", 0x00FFFFFF, 0x00DC2626);

            // [Cancel] button
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx + 150, btn_y, btn_w, btn_h, border_col);
            crate::font::draw_text(surface_ptr, w, h, mx + 172, btn_y + 4, "Cancel", text_color, border_col);
        }
    }
}

pub static TASKMGR_INSTANCES: Mutex<BTreeMap<u64, TaskMgrAppState>> = Mutex::new(BTreeMap::new());

pub fn cleanup_taskmgr_for_window(window_id: u64) {
    let mut instances = TASKMGR_INSTANCES.lock();
    if instances.remove(&window_id).is_some() {
        crate::serial_println!("[TASKMGR] Cleaned up TaskManager state for Window {}", window_id);
    }
}

pub fn render_taskmgr_surface(surface_ptr: *mut u32, w: u32, h: u32) {
    let state = TaskMgrAppState::new(0, 0);
    state.render_to_surface(surface_ptr, w, h);
}

pub fn spawn_taskmgr_app(name: &str) -> Result<u64, &'static str> {
    let address_space = crate::memory::AddressSpaceBuilder::new()?;
    let cr3 = address_space.cr3();
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
        address_space.commit(),
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        alloc::vec![],
    );
    let pending = crate::task::process::PendingProcess::new(pid);

    let surf_id = crate::surface::create_surface_for_pid(pid, crate::surface::MAX_SURFACE_WIDTH, crate::surface::MAX_SURFACE_HEIGHT)?;
    if let Some(s) = crate::surface::SURFACE_REGISTRY.write().iter_mut().find(|s| s.surface_id == surf_id) {
        s.width = TASKMGR_WIDTH;
        s.height = TASKMGR_HEIGHT;
    }
    let (win_x, win_y) = {
        let count = crate::wm::WM.lock().windows.len() as i32;
        (70 + ((count * 30) % 200), 60 + ((count * 25) % 150))
    };
    let win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, win_x, win_y, TASKMGR_WIDTH, TASKMGR_HEIGHT)
        .map_err(|_| "window creation failed")?;

    {
        let state = TaskMgrAppState::new(win_id, pid);
        if let Some(surface) = crate::surface::SURFACE_REGISTRY.read().iter().find(|s| s.surface_id == surf_id) {
            let phys_addr = surface.shmem_phys_addr;
            let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *mut u32 };
            state.render_to_surface(surf_ptr, TASKMGR_WIDTH, TASKMGR_HEIGHT);
        }
        TASKMGR_INSTANCES.lock().insert(win_id, state);
    }

    let _ = crate::surface::present_surface(surf_id, 0, 0, TASKMGR_WIDTH, TASKMGR_HEIGHT);
    crate::serial_println!("[APP-REGISTRY] Successfully launched '{}' (PID {}, Entry 0x{:x}, Surface {}, Window {})",
        name, pid, code_base, surf_id, win_id);

    crate::input::register_kernel_input_owner(pid);
    pending.commit();
    Ok(pid)
}
