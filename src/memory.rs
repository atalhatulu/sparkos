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

struct UserFrameBump {
    /// Snapshot of usable physical ranges: (start_inclusive, end_exclusive).
    regions: Vec<(u64, u64)>,
    next_region: usize,
}

static USER_FRAME_ALLOC: Mutex<Option<UserFrameBump>> = Mutex::new(None);

/// Seeds the user frame allocator with the usable regions from the boot memory
/// map. Safe to call multiple times; the first successful call wins.
pub fn init_user_memory(memory_map: &'static MemoryMap) {
    let mut guard = USER_FRAME_ALLOC.lock();
    if guard.is_some() {
        return;
    }
    let mut regions = Vec::new();
    for region in memory_map.iter() {
        if region.region_type == MemoryRegionType::Usable {
            let start = region.range.start_addr();
            let end = region.range.end_addr();
            // Skip tiny runs.
            if end.saturating_sub(start) >= 4096 {
                // Align to page boundary to keep frames clean.
                let start_aligned = (start + 4095) & !4095;
                if start_aligned < end {
                    regions.push((start_aligned, end));
                }
            }
        }
    }
    *guard = Some(UserFrameBump { regions, next_region: 0 });
}

/// Allocates a fresh, dedicated physical frame for user mappings.
/// Returns `None` if the allocator is unseeded or exhausted.
pub fn user_alloc_frame() -> Option<PhysFrame> {
    let mut guard = USER_FRAME_ALLOC.lock();
    let alloc = guard.as_mut()?;
    while alloc.next_region < alloc.regions.len() {
        let (start, end) = alloc.regions[alloc.next_region];
        if start < end {
            alloc.regions[alloc.next_region].0 = start + 4096;
            return Some(PhysFrame::containing_address(PhysAddr::new(start)));
        }
        alloc.next_region += 1;
    }
    None
}

/// Releases a physical frame back to the pool. The bump allocator does not
/// reclaim individual frames; retained for API symmetry.
pub fn user_free_frame(_frame: PhysFrame) {}

// ---------------------------------------------------------------------------
// User page mapping helpers.
// ---------------------------------------------------------------------------

/// Maps a fresh frame at `virt` for use by a Ring-3 process.
/// Ensures the target sits in the user half and the frame is user-accessible.
/// Returns the mapped virtual address on success.
pub fn map_user_page(virt: u64, writable: bool) -> Result<u64, &'static str> {
    if !is_canonical(virt) || virt >= USER_ADDR_LIMIT {
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
        match mapper.map_to_with_table_flags(page, frame, flags, PageTableFlags::empty(), &mut UserFrameAllocatorAdapter) {
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
    user_free_frame(frame);
    Ok(true)
}

/// Maps a range of `count` pages starting at `virt`, allocating a fresh frame
/// for each. Returns the number of pages successfully mapped.
pub fn map_user_range(virt: u64, count: u64, writable: bool) -> Result<u64, &'static str> {
    for i in 0..count {
        map_user_page(virt + i * 4096, writable)?;
    }
    Ok(count)
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
