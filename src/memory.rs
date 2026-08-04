use x86_64::{
    structures::paging::{FrameAllocator, Mapper, OffsetPageTable, PageTable, PhysFrame, Size4KiB, PageTableFlags, Page},
    PhysAddr, VirtAddr,
};
use bootloader::bootinfo::{MemoryMap, MemoryRegionType};

pub static mut VGA_VIRT_ADDR: u64 = 0;

pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    let level_4_table = active_level_4_table(physical_memory_offset);
    OffsetPageTable::new(level_4_table, physical_memory_offset)
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
pub fn map_vga_uc(recursive_addr: u64, _phys_offset: u64) {
    unsafe {
        VGA_VIRT_ADDR = 0xB8000;
    }
}
