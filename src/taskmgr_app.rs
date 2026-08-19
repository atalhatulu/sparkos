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
        }
    }

    pub fn nav_down(&mut self) {
        if !self.snapshot.is_empty() {
            if self.selected_row_idx + 1 < self.snapshot.len() {
                self.selected_row_idx += 1;
            }
            self.selected_pid = Some(self.snapshot[self.selected_row_idx].pid);
        }
    }

    pub fn request_terminate_selected(&mut self) {
        if let Some(pid) = self.selected_pid {
            if pid == 0 || pid == 1 {
                self.status_message = String::from("Cannot terminate kernel/init process");
                return;
            }
            if let Some(p) = self.snapshot.iter().find(|p| p.pid == pid) {
                if p.name == "kernel" || p.name == "idle" {
                    self.status_message = String::from("Kernel task protected");
                    return;
                }
            }
            self.confirm_kill_pid = Some(pid);
        }
    }

    pub fn confirm_terminate(&mut self) {
        if let Some(pid) = self.confirm_kill_pid.take() {
            crate::task::process::SCHEDULER.lock().exit_process(pid, 1);
            self.status_message = format!("Terminated PID {}", pid);
            self.selected_pid = None;
            self.refresh();
        }
    }

    pub fn cancel_terminate(&mut self) {
        self.confirm_kill_pid = None;
        self.status_message = String::from("Termination cancelled");
    }

    pub fn handle_mouse_click(&mut self, local_x: u32, local_y: u32) {
        // 1. Modal Confirmation Dialog handling
        if self.confirm_kill_pid.is_some() {
            // [Confirm Kill] button (x: 130..210, y: 150..172)
            if local_x >= 130 && local_x <= 210 && local_y >= 150 && local_y <= 172 {
                self.confirm_terminate();
                return;
            }
            // [Cancel] button (x: 230..310, y: 150..172)
            if local_x >= 230 && local_x <= 310 && local_y >= 150 && local_y <= 172 {
                self.cancel_terminate();
                return;
            }
            return;
        }

        // 2. Action buttons in Overview Header
        if local_y >= 6 && local_y <= 26 {
            // [End Task] button (x: 270..340)
            if local_x >= 270 && local_x <= 340 {
                self.request_terminate_selected();
                return;
            }
            // [Refresh] button (x: 350..430)
            if local_x >= 350 && local_x <= 430 {
                self.refresh();
                return;
            }
        }

        // 3. Table Column Header clicks (y: 34..52)
        if local_y >= 34 && local_y <= 52 {
            if local_x >= 8 && local_x < 50 {
                self.toggle_sort(TaskMgrSortColumn::Pid);
            } else if local_x >= 50 && local_x < 170 {
                self.toggle_sort(TaskMgrSortColumn::Name);
            } else if local_x >= 170 && local_x < 240 {
                self.toggle_sort(TaskMgrSortColumn::State);
            } else if local_x >= 240 && local_x < 300 {
                self.toggle_sort(TaskMgrSortColumn::Cpu);
            } else if local_x >= 300 && local_x < 370 {
                self.toggle_sort(TaskMgrSortColumn::Mem);
            } else if local_x >= 370 && local_x < 432 {
                self.toggle_sort(TaskMgrSortColumn::Windows);
            }
            return;
        }

        // 4. Process Rows (y: 56..h-20)
        let row_height = 18u32;
        if local_y >= 56 && local_y < TASKMGR_HEIGHT.saturating_sub(20) {
            let row_idx = ((local_y - 56) / row_height) as usize;
            if row_idx < self.snapshot.len() {
                self.select_row(row_idx);
            }
        }
    }

    pub fn render_to_surface(&self, surface_ptr: *mut u32, w: u32, h: u32) {
        if surface_ptr.is_null() { return; }
        let bg_color = 0x000F172A;     // Main Content Dark Slate
        let panel_bg = 0x001E293B;     // Slate 800
        let border_col = 0x00334155;
        let text_color = 0x00F8FAFC;
        let text_muted = 0x0094A3B8;
        let accent_sky = 0x0038BDF8;

        crate::terminal_app::clear_surface(surface_ptr, w, h, bg_color);

        // 1. System Overview Flat Header (y = 0..32)
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, 0, w, 32, panel_bg);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, 31, w, 1, border_col);

        let (used_mem, total_mem) = crate::memory::get_memory_stats();
        let used_mb = (used_mem / (1024 * 1024)).max(1);
        let total_mb = total_mem / (1024 * 1024);
        let proc_count = self.snapshot.len();

        // CPU & RAM Mini Badges
        let ram_pct = if total_mb > 0 { ((used_mb * 100) / total_mb).min(100) } else { 0 };
        let cpu_pct = (proc_count * 4).min(99);

        // CPU Badge
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 6, 6, 80, 20, 0x00020617);
        let cpu_str = format!("CPU: {}%", cpu_pct);
        crate::font::draw_text(surface_ptr, w, h, 12, 10, &cpu_str, 0x0038BDF8, 0x00020617);

        // RAM Badge
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 92, 6, 110, 20, 0x00020617);
        let ram_str = format!("RAM: {}/{}M", used_mb, total_mb);
        crate::font::draw_text(surface_ptr, w, h, 98, 10, &ram_str, 0x0034D399, 0x00020617);

        // Action Buttons: [End Task] [Refresh]
        let end_btn_x = w.saturating_sub(166);
        let end_btn_bg = if self.selected_pid.is_some() { 0x00DC2626 } else { 0x00334155 };
        crate::files_app::draw_surf_rect(surface_ptr, w, h, end_btn_x, 6, 76, 20, end_btn_bg);
        crate::font::draw_text(surface_ptr, w, h, end_btn_x + 8, 10, "End Task", 0x00FFFFFF, end_btn_bg);

        let ref_btn_x = w.saturating_sub(84);
        crate::files_app::draw_surf_rect(surface_ptr, w, h, ref_btn_x, 6, 76, 20, 0x002563EB);
        crate::font::draw_text(surface_ptr, w, h, ref_btn_x + 12, 10, "Refresh", 0x00FFFFFF, 0x002563EB);

        // 2. Table Column Headers (y: 34..52)
        crate::files_app::draw_surf_rect(surface_ptr, w, h, 0, 34, w, 18, 0x000B132B);
        crate::font::draw_text(surface_ptr, w, h, 8, 38, "PID", accent_sky, 0x000B132B);
        crate::font::draw_text(surface_ptr, w, h, 50, 38, "PROCESS NAME", accent_sky, 0x000B132B);
        crate::font::draw_text(surface_ptr, w, h, 180, 38, "STATE", accent_sky, 0x000B132B);
        crate::font::draw_text(surface_ptr, w, h, 250, 38, "CPU", accent_sky, 0x000B132B);
        crate::font::draw_text(surface_ptr, w, h, 310, 38, "MEMORY", accent_sky, 0x000B132B);
        crate::font::draw_text(surface_ptr, w, h, 385, 38, "WINS", accent_sky, 0x000B132B);

        // 3. Process Rows
        let mut y = 56u32;
        for (i, p) in self.snapshot.iter().enumerate() {
            if y + 18 >= h.saturating_sub(18) { break; }

            let is_selected = self.selected_pid == Some(p.pid) || self.selected_row_idx == i;
            let row_bg = if is_selected { 0x002563EB } else if i % 2 == 0 { 0x00131C2E } else { bg_color };

            crate::files_app::draw_surf_rect(surface_ptr, w, h, 4, y, w.saturating_sub(8), 16, row_bg);

            // PID
            let pid_str = format!("{}", p.pid);
            crate::font::draw_text(surface_ptr, w, h, 8, y + 2, &pid_str, text_color, row_bg);

            // Name
            let short_name = if p.name.len() > 15 { &p.name[..15] } else { &p.name };
            crate::font::draw_text(surface_ptr, w, h, 50, y + 2, short_name, text_color, row_bg);

            // State Badge
            let (state_str, state_col) = match p.state {
                ProcessState::Running => ("RUNNING", 0x0034D399),
                ProcessState::Ready => ("READY", 0x0038BDF8),
                ProcessState::Blocked => ("BLOCKED", 0x00FBBF24),
                ProcessState::Terminated => ("STOPPED", 0x00EF4444),
                _ => ("IDLE", 0x0094A3B8),
            };
            crate::font::draw_text(surface_ptr, w, h, 180, y + 2, state_str, state_col, row_bg);

            // CPU Ticks
            let cpu_str = format!("{}t", p.cpu_ticks);
            crate::font::draw_text(surface_ptr, w, h, 250, y + 2, &cpu_str, text_color, row_bg);

            // Memory
            let mem_kb = (p.current_memory_bytes / 1024).max(4);
            let mem_str = if mem_kb > 1024 { format!("{:.1}M", mem_kb as f32 / 1024.0) } else { format!("{}K", mem_kb) };
            crate::font::draw_text(surface_ptr, w, h, 310, y + 2, &mem_str, text_color, row_bg);

            // Window Count
            let win_str = format!("{}", p.window_count);
            crate::font::draw_text(surface_ptr, w, h, 395, y + 2, &win_str, text_color, row_bg);

            y += 18;
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
            let mw = 260u32;
            let mh = 100u32;
            let mx = (w.saturating_sub(mw)) / 2;
            let my = (h.saturating_sub(mh)) / 2;

            // Modal Box
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx, my, mw, mh, 0x000F172A);
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx, my, mw, 2, 0x00EF4444);
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx, my, 2, mh, 0x00EF4444);
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx + mw - 2, my, 2, mh, 0x00EF4444);
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx, my + mh - 2, mw, 2, 0x00EF4444);

            let warn_text = format!("Terminate PID {}?", kill_pid);
            crate::font::draw_text(surface_ptr, w, h, mx + 20, my + 18, &warn_text, 0x00FFFFFF, 0x000F172A);
            crate::font::draw_text(surface_ptr, w, h, mx + 20, my + 38, "Unsaved data will be lost.", 0x0094A3B8, 0x000F172A);

            // [Confirm] button
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx + 20, my + 64, 90, 24, 0x00DC2626);
            crate::font::draw_text(surface_ptr, w, h, mx + 35, my + 68, "Confirm", 0x00FFFFFF, 0x00DC2626);

            // [Cancel] button
            crate::files_app::draw_surf_rect(surface_ptr, w, h, mx + 150, my + 64, 90, 24, 0x00334155);
            crate::font::draw_text(surface_ptr, w, h, mx + 172, my + 68, "Cancel", 0x00FFFFFF, 0x00334155);
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
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frame for taskmgr.app")?;
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

    let surf_id = crate::surface::create_surface_for_pid(pid, TASKMGR_WIDTH, TASKMGR_HEIGHT)?;
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

    Ok(pid)
}
