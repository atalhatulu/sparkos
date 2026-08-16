//! SparkOS Desktop V1.12 — Process & Task Manager (`taskmgr.app`)
//!
//! Displays live process table, PID, memory consumption, CPU state, and
//! allows capability-authorized task termination.

pub const TASKMGR_WIDTH: u32 = 360;
pub const TASKMGR_HEIGHT: u32 = 200;

pub fn render_taskmgr_surface(surface_ptr: *mut u32, w: u32, h: u32) {
    if surface_ptr.is_null() { return; }
    let bg_color = 0x000F172A;
    let text_color = 0x00F8FAFC;
    let header_color = 0x0038BDF8;

    crate::terminal_app::clear_surface(surface_ptr, w, h, bg_color);
    crate::font::draw_text(surface_ptr, w, h, 8, 8, "SparkOS Task Manager", 0x0034D399, bg_color);
    crate::font::draw_text(surface_ptr, w, h, 8, 22, "PID   NAME            STATE    MEM", header_color, bg_color);
    crate::font::draw_text(surface_ptr, w, h, 8, 32, "-----------------------------------", 0x00475569, bg_color);

    let (used_mem, total_mem) = crate::memory::get_memory_stats();
    let used_mb = (used_mem / (1024 * 1024)).max(1);
    let total_mb = total_mem / (1024 * 1024);

    let procs = crate::task::process::get_system_metrics_snapshot();
    let mut y = 44u32;
    for p in procs.iter().take(8) {
        if y + 12 > h.saturating_sub(20) { break; }
        let state_str = match p.state {
            crate::task::process::ProcessState::Running => "Running",
            crate::task::process::ProcessState::Ready => "Ready",
            _ => "Sleeping",
        };
        let mem_kb = (p.current_memory_bytes / 1024).max(4);
        let row = alloc::format!("{:<5} {:<15} {:<8} {} KB", p.pid, p.name, state_str, mem_kb);
        crate::font::draw_text(surface_ptr, w, h, 8, y, &row, text_color, bg_color);
        y += 13;
    }

    let footer = alloc::format!("RAM: {} / {} MB | Active PIDs: {}", used_mb, total_mb, procs.len());
    crate::font::draw_text(surface_ptr, w, h, 8, h.saturating_sub(16), &footer, 0x00A855F7, bg_color);
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
    let _win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, win_x, win_y, TASKMGR_WIDTH, TASKMGR_HEIGHT)
        .map_err(|_| "window creation failed")?;

    if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.surface_id == surf_id) {
        let phys_addr = surface.shmem_phys_addr;
        let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *mut u32 };
        render_taskmgr_surface(surf_ptr, TASKMGR_WIDTH, TASKMGR_HEIGHT);
    }

    let _ = crate::surface::present_surface(surf_id, 0, 0, TASKMGR_WIDTH, TASKMGR_HEIGHT);
    crate::serial_println!("[APP-REGISTRY] Successfully launched '{}' (PID {}, Entry 0x{:x}, Surface {}, Window)",
        name, pid, code_base, surf_id);

    Ok(pid)
}
