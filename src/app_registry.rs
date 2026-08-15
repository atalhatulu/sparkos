//! SparkOS — Application Registry & Icon System (Desktop V1.1 / Steps 4-6)
//!
//! Provides static application discovery, icon metadata, and process isolation wrappers.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIcon {
    Logo,
    Terminal,
    Demo,
    Files,
    Generic,
}

#[derive(Debug, Clone, Copy)]
pub struct AppDescriptor {
    pub id: u8,
    pub name: &'static str,
    pub exec_name: &'static str,
    pub icon: AppIcon,
    pub default_w: u32,
    pub default_h: u32,
}

pub const REGISTERED_APPS: &[AppDescriptor] = &[
    AppDescriptor {
        id: 1,
        name: "Terminal",
        exec_name: "gui_terminal",
        icon: AppIcon::Terminal,
        default_w: 380,
        default_h: 140,
    },
    AppDescriptor {
        id: 2,
        name: "Demo App",
        exec_name: "demo_app",
        icon: AppIcon::Demo,
        default_w: 260,
        default_h: 140,
    },
    AppDescriptor {
        id: 3,
        name: "Files",
        exec_name: "file_mgr",
        icon: AppIcon::Files,
        default_w: 220,
        default_h: 110,
    },
];

pub fn find_app_by_id(id: u8) -> Option<&'static AppDescriptor> {
    REGISTERED_APPS.iter().find(|app| app.id == id)
}

pub fn find_app_by_name(name: &str) -> Option<&'static AppDescriptor> {
    REGISTERED_APPS.iter().find(|app| app.name == name || app.exec_name == name)
}

/// Spawns a registered application as an isolated Ring-3 process with its own CR3,
/// CSpace, Shmem Surface, and WM Window.
pub fn spawn_registered_app(app_id: u8) -> Result<u64, &'static str> {
    let app = find_app_by_id(app_id).ok_or("Unknown application ID")?;

    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frame for process CR3")?;
    let pid = crate::task::process::create_user_process_with_caps(
        app.exec_name,
        crate::memory::USER_ADDR_BASE,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        alloc::vec![],
    );

    let surf_id = crate::surface::create_surface(app.default_w, app.default_h)?;
    
    // Spawn with cascaded layout based on PID
    let win_x = 30 + ((pid as i32 * 30) % 200);
    let win_y = 35 + ((pid as i32 * 25) % 120);

    let _win_id = crate::wm::WM.lock()
        .create_window(pid, surf_id, win_x, win_y, app.default_w, app.default_h)
        .map_err(|_| "window creation failed")?;

    // Fill initial surface buffer with application default theme
    if let Some(s) = crate::surface::SURFACE_REGISTRY.lock().iter().find(|s| s.surface_id == surf_id) {
        let ptr = unsafe { (crate::gui::PHYS_OFFSET + s.shmem_phys_addr) as *mut u32 };
        let total_px = (app.default_w * app.default_h) as usize;
        let bg_color = match app.icon {
            AppIcon::Terminal => 0x0009090B, // Zinc Black
            AppIcon::Demo => 0x00064E3B,     // Dark Emerald
            AppIcon::Files => 0x001E293B,    // Dark Slate
            _ => 0x001E3A8A,                // Blue
        };
        unsafe {
            for i in 0..total_px {
                *ptr.add(i) = bg_color;
            }
        }
    }
    let _ = crate::surface::present_surface(surf_id, 0, 0, app.default_w, app.default_h);

    crate::serial_println!("[APP-REGISTRY] Successfully launched '{}' (PID {}, Surface {}, Window)",
        app.name, pid, surf_id);

    Ok(pid)
}
