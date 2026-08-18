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

    // Resource Accounting & Quota Check
    {
        let mut sched = crate::task::process::SCHEDULER.lock();
        if let Some(proc) = sched.get_process_mut(owner_pid) {
            proc.try_charge_memory(total_bytes as u64)?;
            proc.increment_surface_count()?;
        }
    }

    let pages = ((total_bytes + 4095) / 4096) as u64;
    let first_frame = crate::memory::user_alloc_frame().ok_or("Out of memory for surface backing")?;
    let phys_frame = first_frame.start_address().as_u64();
    for _ in 1..pages {
        let _ = crate::memory::user_alloc_frame().ok_or("Out of memory for surface backing")?;
    }

    let mut reg = SURFACE_REGISTRY.write();
    
    // SEC-05 Fix: Bitmask-based slot allocator for reuse & hard limit
    let used_mask: u16 = reg.iter()
        .filter(|s| s.owner_pid == owner_pid)
        .map(|s| 1u16 << s.slot)
        .fold(0, |acc, m| acc | m);

    let mut free_slot = None;
    for i in 0..MAX_SURFACES_PER_PROCESS {
        if (used_mask & (1 << i)) == 0 {
            free_slot = Some(i as u8);
            break;
        }
    }

    let slot = free_slot.ok_or("Process reached maximum surface limit (16 surfaces max)")?;
    let slot_offset = (slot as u64).checked_mul(SURFACE_SLOT_SIZE).ok_or("VMA calculation overflow")?;
    let vma_addr = USER_SURFACE_BASE.checked_add(slot_offset).ok_or("VMA calculation overflow")?;

    if !crate::memory::is_user_range(vma_addr, total_bytes) {
        return Err("Surface VMA target outside user address space");
    }

    // Map into client's user address space at slot-specific VMA with read/write access
    let owner_cr3 = crate::task::process::get_process_user_cr3(owner_pid).unwrap_or(0);
    if owner_cr3 != 0 {
        crate::memory::map_user_phys_range_in_cr3(owner_cr3, vma_addr, PhysAddr::new(phys_frame), pages, true)?;
    }
    crate::memory::map_user_phys_range_in_cr3(0, vma_addr, PhysAddr::new(phys_frame), pages, true)?;

    let surface_id = NEXT_SURFACE_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let surface = SurfaceInfo {
        surface_id,
        owner_pid,
        slot,
        width,
        height,
        stride,
        vma_addr,
        pages,
        shmem_phys_addr: phys_frame,
        shmem_size: total_bytes,
        dirty: false,
        dirty_rect: (0, 0, width, height),
    };

    reg.push(surface);

    crate::serial_println!("[SURFACE] Process {} created surface {} (slot {}, {}x{}, vma 0x{:x}, phys 0x{:x})",
        owner_pid, surface_id, slot, width, height, vma_addr, phys_frame);

    Ok(surface_id)
}

/// Presents a dirty rectangle of the specified surface to the compositor.
pub fn present_surface(surface_id: u64, x: u32, y: u32, w: u32, h: u32) -> Result<(), &'static str> {
    let pid = crate::task::process::current_pid();
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
    if let Some(win) = wm.windows.iter().find(|w| w.surface_id == surface_id || w.owner_pid == pid) {
        let win_x = win.x + clip_x as i32;
        let win_y = win.y + 20 + clip_y as i32; // title bar is 20px
        wm.mark_damage(win_x, win_y, clip_w, clip_h);
    }

    crate::serial_println!("[SURFACE] Process {} presented surface {} dirty_rect [{}, {}, {}, {}]",
        pid, surface_id, clip_x, clip_y, clip_w, clip_h);

    Ok(())
}

/// Destroys a surface and cleans up its registry record & VMA page mapping.
pub fn destroy_surface(surface_id: u64) -> Result<(), &'static str> {
    let pid = crate::task::process::current_pid();
    let mut reg = SURFACE_REGISTRY.write();

    let idx = reg.iter().position(|s| s.surface_id == surface_id)
        .ok_or("Surface not found")?;

    if reg[idx].owner_pid != pid {
        return Err("Permission denied: caller is not surface owner");
    }

    let surface = reg.remove(idx);
    
    // SEC-06 Fix: Unmap virtual pages & free frames
    let _ = crate::memory::unmap_user_range(surface.vma_addr, surface.pages);
    for i in 0..surface.pages {
        let paddr = surface.shmem_phys_addr + i * 4096;
        crate::memory::user_free_frame(x86_64::structures::paging::PhysFrame::containing_address(PhysAddr::new(paddr)));
    }

    crate::serial_println!("[SURFACE] Process {} destroyed surface {} (slot {} unmapped)", pid, surface.surface_id, surface.slot);

    Ok(())
}

/// Destroys and cleans up all surfaces owned by a terminating process (Zero-Leak Teardown).
pub fn cleanup_surfaces_for_pid(pid: u64) {
    let mut reg = SURFACE_REGISTRY.write();
    let mut removed = alloc::vec::Vec::new();

    reg.retain(|s| {
        if s.owner_pid == pid {
            removed.push((s.vma_addr, s.pages, s.shmem_phys_addr));
            false
        } else {
            true
        }
    });

    let count = removed.len();
    for (vma, pages, phys_addr) in removed {
        let _ = crate::memory::unmap_user_range(vma, pages);
        for i in 0..pages {
            let paddr = phys_addr + i * 4096;
            crate::memory::user_free_frame(x86_64::structures::paging::PhysFrame::containing_address(PhysAddr::new(paddr)));
        }
    }

    if count > 0 {
        crate::serial_println!("[SURFACE] Cleaned up, freed & unmapped {} orphaned surface(s) for terminating PID {}", count, pid);
    }
}
