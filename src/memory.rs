use x86_64::{
    structures::paging::{FrameAllocator, Mapper, OffsetPageTable, PageTable, PhysFrame, Size4KiB, PageTableFlags, Page},
    PhysAddr, VirtAddr,
};
use bootloader::bootinfo::{MemoryMap, MemoryRegionType};
use spin::Mutex;
use alloc::vec::Vec;

pub static mut VGA_VIRT_ADDR: u64 = 0;

// ---------------------------------------------------------------------------
// User address-space segmentation (memory isolation).
// ---------------------------------------------------------------------------

/// Upper bound of the user address space (lower half of the 48-bit space).
/// Any virtual address at or above this value belongs to the kernel and must
/// never be passed as a user pointer.
pub const USER_ADDR_LIMIT: u64 = 0x0000_0000_8000_0000;

/// Base of the dedicated, kernel-heap-independent user mapping region.
/// User code/data pages are placed here so that user PTEs never alias kernel
/// heap or kernel data pages.
pub const USER_ADDR_BASE: u64 = 0x0000_0000_4000_0000;

/// Top of the reserved user stack region (stacks grow downwards in x86_64).
pub const USER_STACK_TOP: u64 = 0x0000_0000_7FFF_0000;

/// Returns true if `addr` is a canonical 48-bit virtual address.
pub fn is_canonical(addr: u64) -> bool {
    let sign_extended = (addr << 16) as i64 >> 16;
    (sign_extended as u64) == addr
}

/// True if a `len`-byte range beginning at `ptr` lies entirely within the
/// user half of the address space (canonical, zero-extended, bounded).
pub fn is_user_range(ptr: u64, len: usize) -> bool {
    if ptr == 0 {
        return false;
    }
    if !is_canonical(ptr) {
        return false;
    }
    // The user half is the low half; top bits must be clear.
    if ptr >= USER_ADDR_LIMIT {
        return false;
    }
    let Some(end) = ptr.checked_add(len as u64) else {
        return false;
    };
    end <= USER_ADDR_LIMIT
}

// ---------------------------------------------------------------------------
// User physical frame allocator.
// ---------------------------------------------------------------------------
//
// User frames come from a dedicated bump allocator over the usable physical
// regions snapshot at boot. These frames are mapped into the *user* virtual
// region (USER_ADDR_BASE..USER_ADDR_LIMIT) so that user PTEs never extend into
// kernel pages —— the allocator always produces a fresh physical frame which is
// then mapped with USER_ACCESSIBLE at a user virtual address.
//
// `init_user_memory` must be called once after the memory map is available so
// that usable regions are snapshotted. If it is never called the allocator
// stays empty and callers fall back to a degraded (still isolated) path.

// ---------------------------------------------------------------------------
// Faz 25: Virtual Memory Evolution — Reclaimable Physical Frame Allocator
// ---------------------------------------------------------------------------

static USER_FRAME_ALLOC: Mutex<Option<crate::frame_pool::FramePool>> = Mutex::new(None);

#[cfg(feature = "kernel-regression")]
static FAIL_ALLOCATION_AFTER: core::sync::atomic::AtomicIsize = core::sync::atomic::AtomicIsize::new(-1);

#[cfg(feature = "kernel-regression")]
pub fn fail_allocation_after(remaining: isize) {
    FAIL_ALLOCATION_AFTER.store(remaining, core::sync::atomic::Ordering::SeqCst);
}

/// Seeds the user frame allocator with the usable regions from the boot memory
/// map. Safe to call multiple times; the first successful call wins.
pub fn init_user_memory(memory_map: &'static MemoryMap) {
    let mut guard = USER_FRAME_ALLOC.lock();
    if guard.is_some() {
        return;
    }
    let (heap_phys_start, heap_phys_end) = *crate::allocator::KERNEL_HEAP_RANGE.lock();
    let mut regions = Vec::new();

    // The user frame pool resides safely in the primary usable memory block after the kernel heap
    for region in memory_map.iter() {
        if region.region_type == MemoryRegionType::Usable {
            let start = region.range.start_addr();
            let end = region.range.end_addr();

            if start <= heap_phys_start && end >= heap_phys_end {
                let user_start = (heap_phys_end + 4095) & !4095;
                if user_start < end {
                    regions.push((user_start, end));
                }
            } else if start >= 0x100_0000 && (start >= heap_phys_end || end <= heap_phys_start) {
                let start_aligned = (start + 4095) & !4095;
                if start_aligned < end {
                    regions.push((start_aligned, end));
                }
            }
        }
    }

    *guard = Some(crate::frame_pool::FramePool::new(regions));
}

/// Allocate and zero a single frame from the shared user/frame-table pool.
pub fn user_alloc_frame() -> Option<PhysFrame> {
    user_alloc_contiguous_frames(1)
}

/// Reserve an entire contiguous extent or leave the pool unchanged.
pub fn user_alloc_contiguous_frames(count: usize) -> Option<PhysFrame> {
    #[cfg(feature = "kernel-regression")]
    {
        use core::sync::atomic::Ordering;
        let remaining = FAIL_ALLOCATION_AFTER.load(Ordering::SeqCst);
        if remaining == 0 { return None; }
        if remaining > 0 { FAIL_ALLOCATION_AFTER.fetch_sub(1, Ordering::SeqCst); }
    }
    let start = USER_FRAME_ALLOC.lock().as_mut()?.allocate(count)?;
    let dest = (unsafe { crate::gui::PHYS_OFFSET } + start) as *mut u8;
    unsafe { core::ptr::write_bytes(dest, 0, count * 4096); }
    Some(PhysFrame::containing_address(PhysAddr::new(start)))
}

pub fn user_free_frame(frame: PhysFrame) {
    if let Some(pool) = USER_FRAME_ALLOC.lock().as_mut() {
        pool.release(frame.start_address().as_u64());
    }
}

/// Returns allocated frame bytes and the original usable frame-pool capacity.
pub fn get_memory_stats() -> (u64, u64) {
    USER_FRAME_ALLOC.lock().as_ref().map(|pool| pool.stats()).unwrap_or((0, 0))
}

/// Maps a freshly allocated page into the active address space.
pub fn map_user_page(virt: u64, writable: bool) -> Result<u64, &'static str> {
    if !is_canonical(virt) || virt < USER_ADDR_BASE || virt >= USER_ADDR_LIMIT {
        return Err("user mapping target outside user space");
    }
    let frame = user_alloc_frame().ok_or("no free user frames")?;
    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt));
    let phys_offset = VirtAddr::new(unsafe { crate::gui::PHYS_OFFSET });
    let mut mapper = unsafe { init(phys_offset) };

    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if writable {
        flags |= PageTableFlags::WRITABLE;
    }
    unsafe {
        // parent_table_flags ara katman (L3/L2/L1) girişlerine yazılır; boş bırakılırsa
        // girişler PRESENT'siz ama adresli kalır → ikinci sayfa eşlenirken
        // mapped_page_table "entry should be mapped" panikler. Ayrıca USER_ACCESSIBLE
        // olmadan Ring-3 erişimi (tüm seviyelerde U/S=1 gerekir) page fault verirdi.
        let parent = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE;
        match mapper.map_to_with_table_flags(page, frame, flags, parent, &mut UserFrameAllocatorAdapter) {
            Ok(tlb) => {
                tlb.flush();
                Ok(virt)
            }
            Err(x86_64::structures::paging::mapper::MapToError::PageAlreadyMapped(_)) => Ok(virt),
            Err(_) => Err("failed to map user page"),
        }
    }
}

/// Unmaps a single user page. Returns `Ok(true)` if it was present.
pub fn unmap_user_page(virt: u64) -> Result<bool, &'static str> {
    if !is_canonical(virt) {
        return Ok(false);
    }
    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt));
    let phys_offset = VirtAddr::new(unsafe { crate::gui::PHYS_OFFSET });
    let mut mapper = unsafe { init(phys_offset) };
    let (frame, flush) = mapper.unmap(page).map_err(|_| "unmap failed")?;
    flush.flush();
    crate::smp::tlb_shootdown(virt, 1);
    user_free_frame(frame);
    Ok(true)
}

/// Unmaps a range of `count` pages starting at `virt`, flushing TLB and freeing physical frames.
pub fn unmap_user_range(virt: u64, count: u64) -> Result<(), &'static str> {
    for i in 0..count {
        let v = virt.checked_add(i.checked_mul(4096).ok_or("Virtual address overflow")?).ok_or("Virtual address overflow")?;
        let _ = unmap_user_page(v);
    }
    Ok(())
}

/// Maps a range of `count` pages starting at `virt`, allocating a fresh frame
/// for each. Returns the number of pages successfully mapped.
pub fn map_user_range(virt: u64, count: u64, writable: bool) -> Result<u64, &'static str> {
    for i in 0..count {
        map_user_page(virt + i * 4096, writable)?;
    }
    Ok(count)
}

pub fn map_user_phys_range_in_cr3(
    cr3: u64,
    virt: u64,
    phys_start: PhysAddr,
    count: u64,
    writable: bool,
) -> Result<(), &'static str> {
    if !is_canonical(virt) || virt >= USER_ADDR_LIMIT {
        return Err("user mapping target outside user space");
    }
    let phys_offset = VirtAddr::new(unsafe { crate::gui::PHYS_OFFSET });
    let target = if cr3 == 0 {
        unsafe { active_level_4_table(phys_offset) }
    } else {
        let ptr = (phys_offset + PhysAddr::new(cr3).as_u64()).as_mut_ptr::<PageTable>();
        unsafe { &mut *ptr }
    };
    let curr_flags = target[0].flags();
    target[0].set_flags(curr_flags | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE | PageTableFlags::PRESENT);
    let mut mapper = unsafe { OffsetPageTable::new(target, phys_offset) };

    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if writable {
        flags |= PageTableFlags::WRITABLE;
    }
    let parent = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE;

    for i in 0..count {
        let v = virt + i * 4096;
        let p = phys_start + i * 4096;
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(v));
        let frame = PhysFrame::<Size4KiB>::containing_address(p);

        unsafe {
            match mapper.map_to_with_table_flags(page, frame, flags, parent, &mut UserFrameAllocatorAdapter) {
                Ok(tlb) => tlb.flush(),
                Err(x86_64::structures::paging::mapper::MapToError::PageAlreadyMapped(_)) => {
                    let _ = mapper.update_flags(page, flags);
                },
                Err(_) => return Err("failed to map phys page to user"),
            }
        }
    }
    Ok(())
}

/// Maps a specific physical frame range to a user virtual address range.
/// Used for capability-gated DMA region mapping (Aşama 6.2).
pub fn map_user_phys_range(
    virt: u64,
    phys_start: PhysAddr,
    count: u64,
    writable: bool,
) -> Result<(), &'static str> {
    map_user_phys_range_in_cr3(0, virt, phys_start, count, writable)
}

/// Frame allocator adapter multiplexing to the dedicated user pool.
/// Only internal page-table table frames are allocated through this path.
struct UserFrameAllocatorAdapter;

unsafe impl FrameAllocator<Size4KiB> for UserFrameAllocatorAdapter {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        user_alloc_frame()
    }
}

pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    let level_4_table = active_level_4_table(physical_memory_offset);
    OffsetPageTable::new(level_4_table, physical_memory_offset)
}

// ---------------------------------------------------------------------------
// Per-process address spaces (real CR3 isolation for user processes).
// ---------------------------------------------------------------------------
//
// To give each Ring-3 process its own page table we clone the *active* level-4
// table (which already maps the high-half kernel: physical-memory offset,
// recursive mapping, kernel image, VGA, etc.) into a freshly allocated frame
// and hand that frame's physical address to the process as its `user_cr3`.
// Because every kernel mapping is inherited, the kernel stays reachable no
// matter which process's CR3 is currently loaded; the only thing that differs
// between address spaces is the low-half user region. `enter_user_current`
// loads this CR3 right before iretq, so each Ring-3 execution truly runs in
// its own address space.

/// Own a new address space until it is handed to a process. Any early `?`
/// during ELF/service construction reclaims already mapped pages and tables.
pub struct AddressSpaceBuilder { cr3: u64 }

impl AddressSpaceBuilder {
    pub fn new() -> Result<Self, &'static str> {
        Ok(Self { cr3: clone_active_cr3().ok_or("No free frames for address space")? })
    }
    pub fn cr3(&self) -> u64 { self.cr3 }
    pub fn commit(mut self) -> u64 {
        let cr3 = self.cr3;
        self.cr3 = 0;
        cr3
    }
}

impl Drop for AddressSpaceBuilder {
    fn drop(&mut self) {
        if self.cr3 != 0 { let _ = destroy_user_address_space(self.cr3); }
    }
}

/// Create a fresh userspace (1..2 GiB) while retaining supervisor mappings.
/// The PML4 alone is not sufficient: sharing its low PDPT shares every user PTE.
pub fn clone_active_cr3() -> Option<u64> {
    crate::task::process::capture_shared_kernel_cr3();
    let root = user_alloc_frame()?;
    let low = match user_alloc_frame() {
        Some(frame) => frame,
        None => { user_free_frame(root); return None; }
    };
    let offset = VirtAddr::new(unsafe { crate::gui::PHYS_OFFSET });
    unsafe {
        let source = &*((offset + crate::task::process::shared_kernel_cr3()?).as_ptr::<PageTable>());
        let dest = &mut *((offset + root.start_address().as_u64()).as_mut_ptr::<PageTable>());
        for i in 0..512 { dest[i] = source[i].clone(); }
        let low_dest = &mut *((offset + low.start_address().as_u64()).as_mut_ptr::<PageTable>());
        if source[0].flags().contains(PageTableFlags::PRESENT) {
            let low_source = &*((offset + source[0].addr().as_u64()).as_ptr::<PageTable>());
            for i in 0..512 { low_dest[i] = low_source[i].clone(); }
        }
        // Keep the low kernel image supervisor-only. User binaries must be linked
        // at USER_ADDR_BASE; they may not replace the kernel at 2..~5 MiB.
        let kernel_flags = low_dest[0].flags() & !PageTableFlags::USER_ACCESSIBLE;
        low_dest[0].set_flags(kernel_flags);
        low_dest[1].set_unused();
        dest[0].set_frame(low, PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE);
    }
    Some(root.start_address().as_u64())
}

/// Map externally owned surface frames transactionally, into the owner only.
/// Existing mappings are never changed. On failure only this call's PTEs go away.
pub fn map_surface_frames(cr3: u64, virt: u64, phys: u64, pages: u64) -> Result<(), &'static str> {
    if cr3 == 0 || virt < USER_ADDR_BASE || virt % 4096 != 0 || phys % 4096 != 0
        || pages == 0 || !is_user_range(virt, pages.checked_mul(4096).ok_or("Surface range overflow")? as usize) {
        return Err("Invalid surface mapping");
    }
    let offset = VirtAddr::new(unsafe { crate::gui::PHYS_OFFSET });
    let target = unsafe { &mut *((offset + cr3).as_mut_ptr::<PageTable>()) };
    let mut mapper = unsafe { OffsetPageTable::new(target, offset) };
    let parent = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE;
    let flags = parent | PageTableFlags::NO_EXECUTE;
    for i in 0..pages {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt + i * 4096));
        let frame = PhysFrame::containing_address(PhysAddr::new(phys + i * 4096));
        let result = unsafe { mapper.map_to_with_table_flags(page, frame, flags, parent, &mut UserFrameAllocatorAdapter) };
        match result {
            Ok(flush) => flush.flush(),
            Err(_) => {
                for j in 0..i {
                    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt + j * 4096));
                    if let Ok((_, flush)) = mapper.unmap(page) { flush.flush(); }
                }
                drop(mapper);
                prune_empty_user_tables(cr3);
                return Err("Surface mapping failed or overlaps an existing mapping");
            }
        }
    }
    Ok(())
}

/// Remove the owner's mappings without freeing the shared backing a second time.
/// Validate all PTEs before mutation so a stale record cannot unmap somebody else's pages.
pub fn unmap_surface_frames(cr3: u64, virt: u64, phys: u64, pages: u64) -> Result<(), &'static str> {
    let offset = VirtAddr::new(unsafe { crate::gui::PHYS_OFFSET });
    let target = unsafe { &mut *((offset + cr3).as_mut_ptr::<PageTable>()) };
    let mut mapper = unsafe { OffsetPageTable::new(target, offset) };
    for i in 0..pages {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt + i * 4096));
        if mapper.translate_page(page).map(|f| f.start_address().as_u64()).ok() != Some(phys + i * 4096) {
            return Err("Surface backing does not match the owner page table");
        }
    }
    for i in 0..pages {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt + i * 4096));
        let (_, flush) = mapper.unmap(page).map_err(|_| "Surface unmap failed")?;
        flush.flush();
    }
    drop(mapper);
    crate::smp::tlb_shootdown(virt, pages as usize);
    prune_empty_user_tables(cr3);
    Ok(())
}

/// Reclaim empty paging structures in the private 1..2 GiB subtree.
fn prune_empty_user_tables(cr3: u64) {
    let offset = unsafe { crate::gui::PHYS_OFFSET };
    unsafe {
        let root = &mut *((offset + cr3) as *mut PageTable);
        let l3 = &mut *((offset + root[0].addr().as_u64()) as *mut PageTable);
        if !l3[1].flags().contains(PageTableFlags::PRESENT) { return; }
        let l2_phys = l3[1].addr();
        let l2 = &mut *((offset + l2_phys.as_u64()) as *mut PageTable);
        for entry in l2.iter_mut() {
            if entry.flags().contains(PageTableFlags::PRESENT) && !entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                let table_phys = entry.addr();
                let l1 = &*((offset + table_phys.as_u64()) as *const PageTable);
                if l1.iter().all(|e| e.is_unused()) {
                    entry.set_unused();
                    user_free_frame(PhysFrame::containing_address(table_phys));
                }
            }
        }
        if l2.iter().all(|e| e.is_unused()) {
            l3[1].set_unused();
            user_free_frame(PhysFrame::containing_address(l2_phys));
        }
    }
}

/// Reclaim a stopped process's private tables and owned code/stack pages.
/// BIT_9 marks pages allocated by map_user_page_in_cr3; external DMA/surface
/// backing is managed by its owner and must not be freed by this walk.
pub fn destroy_user_address_space(cr3: u64) -> Result<(), &'static str> {
    use x86_64::registers::control::Cr3;
    if cr3 == 0 || Some(cr3) == crate::task::process::shared_kernel_cr3()
        || Cr3::read().0.start_address().as_u64() == cr3 {
        return Err("Cannot destroy the active or shared address space");
    }
    let offset = unsafe { crate::gui::PHYS_OFFSET };
    unsafe {
        let root = &mut *((offset + cr3) as *mut PageTable);
        let l3_phys = root[0].addr();
        let l3 = &mut *((offset + l3_phys.as_u64()) as *mut PageTable);
        if l3[1].flags().contains(PageTableFlags::PRESENT) {
            let l2_phys = l3[1].addr();
            let l2 = &mut *((offset + l2_phys.as_u64()) as *mut PageTable);
            for e2 in l2.iter_mut() {
                if !e2.flags().contains(PageTableFlags::PRESENT) { continue; }
                if e2.flags().contains(PageTableFlags::HUGE_PAGE) {
                    return Err("Unexpected huge page in private userspace");
                }
                let l1_phys = e2.addr();
                let l1 = &mut *((offset + l1_phys.as_u64()) as *mut PageTable);
                for e1 in l1.iter_mut() {
                    if e1.flags().contains(PageTableFlags::PRESENT | PageTableFlags::BIT_9) {
                        user_free_frame(PhysFrame::containing_address(e1.addr()));
                    }
                    e1.set_unused();
                }
                e2.set_unused();
                user_free_frame(PhysFrame::containing_address(l1_phys));
            }
            l3[1].set_unused();
            user_free_frame(PhysFrame::containing_address(l2_phys));
        }
        root[0].set_unused();
        user_free_frame(PhysFrame::containing_address(l3_phys));
        user_free_frame(PhysFrame::containing_address(PhysAddr::new(cr3)));
    }
    Ok(())
}

/// Map a freshly allocated user frame at `virt` in the *given* address space
/// (`cr3` is a physical level-4 table address; `0` means the active/shared
/// table). Returns the mapped virtual address on success.
pub fn map_user_page_in_cr3(cr3: u64, virt: u64, writable: bool) -> Result<u64, &'static str> {
    if !is_canonical(virt) || virt < USER_ADDR_BASE || virt >= USER_ADDR_LIMIT {
        return Err("user mapping target outside user space");
    }
    let frame = user_alloc_frame().ok_or("no free user frames")?;
    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt));
    let phys_offset = VirtAddr::new(unsafe { crate::gui::PHYS_OFFSET });

    // Target L4: the given `cr3`, or the active table when `cr3 == 0`.
    let target = if cr3 == 0 {
        unsafe { active_level_4_table(phys_offset) }
    } else {
        let ptr = (phys_offset + PhysAddr::new(cr3).as_u64()).as_mut_ptr::<PageTable>();
        unsafe { &mut *ptr }
    };
    let curr_flags = target[0].flags();
    target[0].set_flags(curr_flags | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE | PageTableFlags::PRESENT);
    let mut mapper = unsafe { OffsetPageTable::new(target, phys_offset) };

    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::BIT_9;
    if writable {
        flags |= PageTableFlags::WRITABLE;
    }
    unsafe {
        // Aynı kök neden: parent_table_flags boş kalırsa ara katman girişleri
        // PRESENT'siz olur ve ikinci sayfada mapped_page_table panikler. Ring-3
        // erişimi için ara katmanlarda da USER_ACCESSIBLE + WRITABLE gerekir.
        let parent = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE;
        match mapper.map_to_with_table_flags(page, frame, flags, parent, &mut UserFrameAllocatorAdapter) {
            Ok(tlb) => {
                tlb.flush();
                Ok(virt)
            }
            Err(_) => {
                user_free_frame(frame);
                Err("user page already mapped or page-table allocation failed")
            }
        }
    }
}

/// Translates a virtual address in the given CR3 to its physical frame address.
pub fn translate_page_in_cr3(cr3: u64, virt: u64) -> Option<PhysAddr> {
    let phys_offset = VirtAddr::new(unsafe { crate::gui::PHYS_OFFSET });
    let target = if cr3 == 0 {
        unsafe { active_level_4_table(phys_offset) }
    } else {
        let ptr = (phys_offset + PhysAddr::new(cr3).as_u64()).as_mut_ptr::<PageTable>();
        unsafe { &mut *ptr }
    };
    let mapper = unsafe { OffsetPageTable::new(target, phys_offset) };
    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt));
    mapper.translate_page(page).ok().map(|f| f.start_address())
}

/// Write `bytes` into a user region already mapped in the given address space
/// (`cr3`), zero-padding the page(s). Directly writes via physical memory offset
/// without causing page faults in the current address space.
pub fn write_user_region_in_cr3(cr3: u64, virt: u64, bytes: &[u8], len: u64) {
    let phys_offset = unsafe { crate::gui::PHYS_OFFSET };
    let total_len = len.max(bytes.len() as u64);
    let mut written = 0usize;

    while (written as u64) < total_len {
        let page_virt = virt + written as u64;
        let page_offset = (page_virt % 4096) as usize;
        let chunk_len = ((4096 - page_offset) as usize).min((total_len as usize) - written);

        if let Some(phys_addr) = translate_page_in_cr3(cr3, page_virt) {
            let direct_ptr = (phys_offset + phys_addr.as_u64() + page_offset as u64) as *mut u8;
            unsafe {
                core::ptr::write_bytes(direct_ptr, 0, chunk_len);
                if written < bytes.len() {
                    let copy_len = chunk_len.min(bytes.len() - written);
                    core::ptr::copy_nonoverlapping(bytes.as_ptr().add(written), direct_ptr, copy_len);
                }
            }
        }
        written += chunk_len;
    }
}

/// Convenience: map a `len`-sized user region (rounded up to whole pages) with
/// fresh frames in the given address space. Returns the base virtual address.
pub fn map_user_region_in_cr3(cr3: u64, virt: u64, len: u64, writable: bool) -> Result<u64, &'static str> {
    let end = virt.checked_add(len.max(1)).ok_or("User region overflow")?;
    if virt < USER_ADDR_BASE || end > USER_ADDR_LIMIT { return Err("User region out of bounds"); }
    let start = virt & !4095;
    let pages = (end - start + 4095) / 4096;
    for i in 0..pages {
        if let Err(e) = map_user_page_in_cr3(cr3, start + i * 4096, writable) {
            let offset = VirtAddr::new(unsafe { crate::gui::PHYS_OFFSET });
            let target = if cr3 == 0 {
                unsafe { active_level_4_table(offset) }
            } else {
                unsafe { &mut *((offset + cr3).as_mut_ptr::<PageTable>()) }
            };
            let mut mapper = unsafe { OffsetPageTable::new(target, offset) };
            for j in 0..i {
                let page = Page::<Size4KiB>::containing_address(VirtAddr::new(start + j * 4096));
                if let Ok((frame, flush)) = mapper.unmap(page) {
                    flush.flush();
                    user_free_frame(frame);
                }
            }
            drop(mapper);
            if cr3 != 0 { prune_empty_user_tables(cr3); }
            return Err(e);
        }
    }
    Ok(virt)
}

// -----------------------------------------------------------------------------
// Faz 37: Copy-on-Write (CoW) & Demand Paging Subsystem
// -----------------------------------------------------------------------------

static COW_PAGES: spin::Mutex<alloc::collections::BTreeMap<(u64, u64), PhysAddr>> = spin::Mutex::new(alloc::collections::BTreeMap::new());

/// Registers a virtual page as Copy-on-Write for a given CR3.
pub fn register_cow_page(cr3: u64, virt: u64, original_phys: PhysAddr) {
    let page_aligned = virt & !0xFFFu64;
    COW_PAGES.lock().insert((cr3, page_aligned), original_phys);
}

/// Checks if a page fault at `virt` in `cr3` is an active CoW page.
pub fn is_cow_page(cr3: u64, virt: u64) -> bool {
    let page_aligned = virt & !0xFFFu64;
    COW_PAGES.lock().contains_key(&(cr3, page_aligned))
}

/// Resolves a Copy-on-Write page fault: allocates a private frame, copies the content,
/// remaps with WRITABLE, and removes from CoW tracking.
pub fn handle_cow_fault(cr3: u64, fault_virt: u64) -> Result<(), &'static str> {
    let page_aligned = fault_virt & !0xFFFu64;
    let old_phys = {
        let mut guard = COW_PAGES.lock();
        guard.remove(&(cr3, page_aligned)).ok_or("Not a tracked CoW page")?
    };

    let new_frame = user_alloc_frame().ok_or("No physical memory for CoW duplication")?;
    let phys_offset = unsafe { crate::gui::PHYS_OFFSET };

    // Kopyalama: eski fiziksel sayfadan yeni fiziksel sayfaya kopyala
    let src_ptr = (phys_offset + old_phys.as_u64()) as *const u8;
    let dst_ptr = (phys_offset + new_frame.start_address().as_u64()) as *mut u8;
    unsafe {
        core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, 4096);
    }

    // Yeni sayfayı WRITABLE olarak hedef CR3'e eşle
    let target = if cr3 == 0 {
        unsafe { active_level_4_table(VirtAddr::new(phys_offset)) }
    } else {
        let ptr = (VirtAddr::new(phys_offset) + PhysAddr::new(cr3).as_u64()).as_mut_ptr::<PageTable>();
        unsafe { &mut *ptr }
    };
    let mut mapper = unsafe { OffsetPageTable::new(target, VirtAddr::new(phys_offset)) };
    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_aligned));
    let flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE;

    unsafe {
        // Eski eşlemeyi kaldır (unmap) ve yeni çerçeveyi eşle
        if let Ok((_unmapped_frame, flush)) = mapper.unmap(page) {
            flush.flush();
        }
        let parent = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE;
        match mapper.map_to_with_table_flags(page, new_frame, flags, parent, &mut UserFrameAllocatorAdapter) {
            Ok(tlb) => {
                tlb.flush();
                crate::smp::tlb_shootdown(page_aligned, 1);
                Ok(())
            }
            Err(_) => Err("Failed to remap CoW page"),
        }
    }
}

// ---------------------------------------------------------------------------

/// Legacy helper: flips USER_ACCESSIBLE/WRITABLE flags on an existing mapping.
/// Retained for backward compatibility. New code should use `map_user_page`
/// which allocates dedicated user frames instead of sharing kernel pages.
pub unsafe fn set_user_accessible(start_virt: u64, size: usize) {
    let phys_offset = VirtAddr::new(crate::gui::PHYS_OFFSET);
    let mut mapper = init(phys_offset);
    
    let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(start_virt));
    let end_page = Page::<Size4KiB>::containing_address(VirtAddr::new(start_virt + size as u64 - 1));

    for page in Page::range_inclusive(start_page, end_page) {
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;
        if let Ok(flush) = mapper.update_flags(page, flags) {
            flush.flush();
        }
    }
}

unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    use x86_64::registers::control::Cr3;
    let (level_4_table_frame, _) = Cr3::read();
    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();
    &mut *page_table_ptr
}

pub struct BootInfoFrameAllocator {
    memory_map: &'static MemoryMap,
    next: usize,
}

impl BootInfoFrameAllocator {
    pub unsafe fn init(memory_map: &'static MemoryMap) -> Self {
        BootInfoFrameAllocator {
            memory_map,
            next: 0,
        }
    }

    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> {
        let regions = self.memory_map.iter();
        let usable_regions = regions.filter(|r| r.region_type == MemoryRegionType::Usable);
        let addr_ranges = usable_regions.map(|r| r.range.start_addr()..r.range.end_addr());
        let frame_addresses = addr_ranges.flat_map(|r| r.step_by(4096));
        frame_addresses.map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)))
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}

/// Creates an example mapping for a specific virtual page.
pub fn create_example_mapping(
    page: Page,
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), &'static str> {
    let frame = frame_allocator.allocate_frame().ok_or("Bellek doldu, cerceve bulunamadi")?;
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;

    unsafe {
        match mapper.map_to(page, frame, flags, frame_allocator) {
            Ok(tlb) => {
                tlb.flush();
                Ok(())
            }
            Err(x86_64::structures::paging::mapper::MapToError::PageAlreadyMapped(_)) => {
                Ok(())
            }
            Err(e) => {
                crate::serial_println!("Mapper Error: {:?}", e);
                Err("Sayfa tablosuna yazilamadi")
            },
        }
    }
}

// Eski VGA mapping (geriye donuk uyumluluk)
pub fn map_vga_uc(_recursive_addr: u64, _phys_offset: u64) {
    unsafe {
        VGA_VIRT_ADDR = 0xB8000;
    }
}

pub fn alloc_backbuffer(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<u64, &'static str> {
    let start_virt = VirtAddr::new(0xE0000000); // 0xE0000000 sanal adresi
    // 1920 * 1080 * 2 * 4 bytes = 16,588,800 bytes = 4050 sayfalar (4KB)
    let pages = 4050; 
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;

    for i in 0..pages {
        let page = Page::containing_address(start_virt + i * 4096);
        let frame = frame_allocator.allocate_frame().ok_or("Bellek doldu, cerceve bulunamadi")?;
        unsafe {
            match mapper.map_to(page, frame, flags, frame_allocator) {
                Ok(tlb) => tlb.flush(),
                Err(_) => return Err("Haritalama hatasi"),
            }
        }
    }
    
    // Belleği temizle (siyah ekran)
    unsafe {
        core::ptr::write_bytes(start_virt.as_mut_ptr::<u8>(), 0, (pages * 4096) as usize);
    }
    
    Ok(start_virt.as_u64())
}
