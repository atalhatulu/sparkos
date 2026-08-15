//! SparkOS Desktop V1.11 / V1.20 — Settings Application (`settings.app`)
//!
//! Provides a user-space system configuration utility utilizing SparkUI Framework widgets.

use crate::libspark_ui::{Button, Label, Widget};

pub const SETTINGS_WIDTH: u32 = 300;
pub const SETTINGS_HEIGHT: u32 = 180;

pub fn render_settings_surface(surface_ptr: *mut u32, w: u32, h: u32) {
    if surface_ptr.is_null() { return; }
    let bg_color = 0x000F172A;
    let text_color = 0x00F8FAFC;
    let accent_color = 0x0038BDF8;

    crate::terminal_app::clear_surface(surface_ptr, w, h, bg_color);

    // SparkUI Labels
    let title = Label::new(8, 8, "SparkOS Control Center", 0x0034D399, bg_color);
    title.draw(surface_ptr, w, h);

    let res_label = Label::new(8, 24, "Resolution: 1280x720 (HD)", text_color, bg_color);
    res_label.draw(surface_ptr, w, h);

    let depth_label = Label::new(8, 40, "Color Depth: 32-bit TrueColor", text_color, bg_color);
    depth_label.draw(surface_ptr, w, h);

    let theme_label = Label::new(8, 56, "Active Theme: Spark Dark", accent_color, bg_color);
    theme_label.draw(surface_ptr, w, h);

    let arch_label = Label::new(8, 72, "Architecture: x86-64 Microkernel", text_color, bg_color);
    arch_label.draw(surface_ptr, w, h);

    let sec_label = Label::new(8, 88, "Security: CSpace / CR3 Isolated", 0x0034D399, bg_color);
    sec_label.draw(surface_ptr, w, h);

    // SparkUI Button Widget
    let toggle_btn = Button::new(8, 110, 200, 22, "Toggle Theme");
    toggle_btn.draw(surface_ptr, w, h);
}

pub fn spawn_settings_app(name: &str) -> Result<u64, &'static str> {
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frame for settings.app")?;
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

    let surf_id = crate::surface::create_surface_for_pid(pid, SETTINGS_WIDTH, SETTINGS_HEIGHT)?;
    let _win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, 120, 90, SETTINGS_WIDTH, SETTINGS_HEIGHT)
        .map_err(|_| "window creation failed")?;

    if let Some(surface) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.surface_id == surf_id) {
        let phys_addr = surface.shmem_phys_addr;
        let surf_ptr = unsafe { (crate::gui::PHYS_OFFSET + phys_addr) as *mut u32 };
        render_settings_surface(surf_ptr, SETTINGS_WIDTH, SETTINGS_HEIGHT);
    }

    let _ = crate::surface::present_surface(surf_id, 0, 0, SETTINGS_WIDTH, SETTINGS_HEIGHT);
    crate::serial_println!("[APP-REGISTRY] Successfully launched '{}' (PID {}, Entry 0x{:x}, Surface {}, Window)",
        name, pid, code_base, surf_id);

    Ok(pid)
}
