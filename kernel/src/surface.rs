use alloc::vec::Vec;
use spin::RwLock;
use x86_64::PhysAddr;

pub const USER_SURFACE_BASE: u64 = 0x70000000;
pub const SURFACE_SLOT_SIZE: u64 = 0x01000000; // 16 MB per surface slot
pub const MAX_SURFACES_PER_PROCESS: usize = 16; // Max 16 slots before hitting 0x80000000 limit
pub const MAX_SURFACE_WIDTH: u32 = 1280;
pub const MAX_SURFACE_HEIGHT: u32 = 720;

#[derive(Debug, Clone)]
pub struct SurfaceInfo {
    pub surface_id: u64,
    pub owner_pid: u64,
    pub owner_cr3: u64,
    pub slot: u8,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub vma_addr: u64,
    pub pages: u64,
    pub shmem_phys_addr: u64,
    pub shmem_size: usize,
    pub dirty: bool,
    pub dirty_rect: (u32, u32, u32, u32), // (x, y, w, h)
}

static NEXT_SURFACE_ID: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);
pub static SURFACE_REGISTRY: RwLock<Vec<SurfaceInfo>> = RwLock::new(Vec::new());

/// Creates a new shared memory surface for the current running process.
pub fn create_surface(width: u32, height: u32) -> Result<u64, &'static str> {
    create_surface_for_pid(crate::task::process::current_pid(), width, height)
}

/// Creates a new shared memory surface for a specific owner process PID.
/// Maps the allocated physical frames into the process's address space at `0x70000000 + slot * 16MB`.
pub fn create_surface_for_pid(owner_pid: u64, width: u32, height: u32) -> Result<u64, &'static str> {
    if width == 0 || width > MAX_SURFACE_WIDTH || height == 0 || height > MAX_SURFACE_HEIGHT {
        return Err("Invalid surface dimensions");
    }

    let stride = width.checked_mul(4).ok_or("Stride calculation overflow")?;
    let total_bytes = (stride as usize).checked_mul(height as usize).ok_or("Buffer size overflow")?;

    let pages = ((total_bytes + 4095) / 4096) as u64;
    let charged_bytes = pages * 4096;
    // Registry -> scheduler is the common lifecycle lock order. Never call this
    // module while holding SCHEDULER (including process-exit cleanup).
    let mut reg = SURFACE_REGISTRY.write();
    let used_mask = reg.iter().filter(|s| s.owner_pid == owner_pid)
        .fold(0u16, |mask, s| mask | (1u16 << s.slot));
    let slot = (0..MAX_SURFACES_PER_PROCESS).find(|&i| used_mask & (1u16 << i) == 0)
        .ok_or("Process reached maximum surface limit (16 surfaces max)")? as u8;
    let vma_addr = USER_SURFACE_BASE + slot as u64 * SURFACE_SLOT_SIZE;
    reg.try_reserve(1).map_err(|_| "Out of kernel memory for surface record")?;
    let owner_cr3 = {
        let mut sched = crate::task::process::SCHEDULER.lock();
        let proc = sched.get_process_mut(owner_pid).ok_or("Surface owner not found")?;
        if proc.exited || proc.reaped || proc.user_cr3 == 0 {
            return Err("Surface owner has no live user address space");
        }
        proc.try_charge_memory(charged_bytes)?;
        if let Err(e) = proc.increment_surface_count() {
            proc.uncharge_memory(charged_bytes);
            return Err(e);
        }
        proc.user_cr3
    };

    let result = (|| {
        // A surface requires ALL its backing frames. A single-frame fallback
        // would expose unrelated physical memory after the first 4096 bytes.
        let first = crate::memory::user_alloc_contiguous_frames(pages as usize)
            .ok_or("Out of contiguous memory for surface backing")?;
        let phys = first.start_address().as_u64();
        if let Err(e) = crate::memory::map_surface_frames(owner_cr3, vma_addr, phys, pages) {
            free_backing(phys, pages);
            return Err(e);
        }
        Ok(phys)
    })();
    let phys_frame = match result {
        Ok(phys) => phys,
        Err(e) => {
            uncharge_surface(owner_pid, charged_bytes);
            return Err(e);
        }
    };

    let surface_id = NEXT_SURFACE_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let surface = SurfaceInfo {
        surface_id,
        owner_pid,
        owner_cr3,
        slot,
        width,
        height,
        stride,
        vma_addr,
        pages,
        shmem_phys_addr: phys_frame,
        shmem_size: charged_bytes as usize,
        dirty: false,
        dirty_rect: (0, 0, width, height),
    };

    reg.push(surface);

    crate::serial_println!("[SURFACE] Process {} created surface {} (slot {}, {}x{}, vma 0x{:x}, phys 0x{:x})",
        owner_pid, surface_id, slot, width, height, vma_addr, phys_frame);

    Ok(surface_id)
}

/// Presents a dirty rectangle of the specified surface to the compositor for a specific PID.
pub fn present_surface_for_pid(pid: u64, surface_id: u64, x: u32, y: u32, w: u32, h: u32) -> Result<(), &'static str> {
    let (clip_x, clip_y, clip_w, clip_h) = {
        let mut reg = SURFACE_REGISTRY.write();

        let surface = reg.iter_mut().find(|s| s.surface_id == surface_id)
            .ok_or("Surface not found")?;

        // Ownership check (Confinement)
        if surface.owner_pid != pid {
            return Err("Permission denied: caller is not surface owner");
        }

        // Boundary check & clipping
        let clip_x = x.min(surface.width);
        let clip_y = y.min(surface.height);
        let clip_w = w.min(surface.width.saturating_sub(clip_x));
        let clip_h = h.min(surface.height.saturating_sub(clip_y));

        surface.dirty = true;
        surface.dirty_rect = (clip_x, clip_y, clip_w, clip_h);

        (clip_x, clip_y, clip_w, clip_h)
    };

    // Notify Window Manager of the affected window area
    let mut wm = crate::wm::WM.lock();
    if let Some(win) = wm.windows.iter().find(|w| w.surface_id == surface_id && w.owner_pid == pid) {
        let win_x = win.x + clip_x as i32;
        let win_y = win.y + 20 + clip_y as i32; // title bar is 20px
        wm.mark_damage(win_x, win_y, clip_w, clip_h);
    }

    crate::serial_println!("[SURFACE] Process {} presented surface {} dirty_rect [{}, {}, {}, {}]",
        pid, surface_id, clip_x, clip_y, clip_w, clip_h);

    Ok(())
}

/// Presents a dirty rectangle of the specified surface for the current process.
pub fn present_surface(surface_id: u64, x: u32, y: u32, w: u32, h: u32) -> Result<(), &'static str> {
    present_surface_for_pid(crate::task::process::current_pid(), surface_id, x, y, w, h)
}


fn free_backing(phys: u64, pages: u64) {
    for i in 0..pages {
        crate::memory::user_free_frame(x86_64::structures::paging::PhysFrame::containing_address(
            PhysAddr::new(phys + i * 4096)));
    }
}

fn uncharge_surface(pid: u64, bytes: u64) {
    if let Some(proc) = crate::task::process::SCHEDULER.lock().get_process_mut(pid) {
        proc.uncharge_memory(bytes);
        proc.decrement_surface_count();
    }
}

/// Syscall entry: ownership comes from the caller, never a user-supplied PID.
pub fn destroy_surface(surface_id: u64) -> Result<(), &'static str> {
    destroy_surface_for_pid(crate::task::process::current_pid(), surface_id)
}

/// Kernel lifecycle entry shared by window close, syscall and process exit.
/// Only frees backing after removing its mappings from the recorded owner CR3.
pub(crate) fn destroy_surface_for_pid(pid: u64, surface_id: u64) -> Result<(), &'static str> {
    let mut reg = SURFACE_REGISTRY.write();
    let idx = reg.iter().position(|s| s.surface_id == surface_id).ok_or("Surface not found")?;
    let surface = &reg[idx];
    if surface.owner_pid != pid {
        return Err("Permission denied: caller is not surface owner");
    }
    crate::memory::unmap_surface_frames(surface.owner_cr3, surface.vma_addr,
        surface.shmem_phys_addr, surface.pages)?;
    let surface = reg.remove(idx);
    free_backing(surface.shmem_phys_addr, surface.pages);
    uncharge_surface(pid, surface.shmem_size as u64);
    crate::serial_println!("[SURFACE] Process {} destroyed surface {} (slot {} reclaimed)", pid, surface_id, surface.slot);
    Ok(())
}

/// Safely retrieves the physical backing pointer and dimensions for rendering.
/// Enforces exact surface_id AND owner_pid match, and clamps width/height to backing bounds.
pub fn get_surface_render_target(surface_id: u64, owner_pid: u64, req_w: u32, req_h: u32) -> Option<(*mut u32, u32, u32)> {
    let mut reg = SURFACE_REGISTRY.write();
    let surf = reg.iter_mut().find(|s| s.surface_id == surface_id && s.owner_pid == owner_pid)?;

    let max_safe_w = (surf.stride / 4).min(MAX_SURFACE_WIDTH);
    let max_safe_h = if surf.stride > 0 {
        ((surf.shmem_size / surf.stride as usize) as u32).min(MAX_SURFACE_HEIGHT)
    } else {
        0
    };

    let safe_w = req_w.clamp(1, max_safe_w);
    let safe_h = req_h.clamp(1, max_safe_h);

    surf.width = safe_w;
    surf.height = safe_h;

    let ptr = unsafe { (crate::gui::PHYS_OFFSET + surf.shmem_phys_addr) as *mut u32 };
    Some((ptr, safe_w, safe_h))
}

/// Called after releasing SCHEDULER, while the process's CR3 is still alive.
pub fn cleanup_surfaces_for_pid(pid: u64) {
    loop {
        let id = SURFACE_REGISTRY.read().iter().find(|s| s.owner_pid == pid).map(|s| s.surface_id);
        let Some(id) = id else { break; };
        if let Err(e) = destroy_surface_for_pid(pid, id) {
            crate::serial_println!("[SURFACE] Cleanup failed for PID {}, surface {}: {}", pid, id, e);
            break; // Keep the record/backing alive rather than free mapped memory.
        }
    }
}

