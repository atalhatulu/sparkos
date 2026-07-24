use x86_64::structures::paging::{PageTable, PageTableFlags, PhysFrame, Size4KiB, Mapper};
use x86_64::PhysAddr;
use x86_64::VirtAddr;

pub static mut VGA_VIRT_ADDR: u64 = 0;

/// VGA buffer'ını Uncacheable (UC) olarak 0xB8000 adresine identity-map et
///
/// Bootloader fiziksel belleği Write-Back cache ile map ediyor.
/// VGA buffer'ı cache'te kalıp ekrana gitmediği için görüntü gelmiyor.
/// Bu fonksiyon 0xB8000'i UC (Uncacheable) olarak yeniden map eder.
pub fn map_vga_uc(recursive_addr: u64, _phys_offset: u64) {
    use x86_64::structures::paging::Page;
    
    let rec_virt = VirtAddr::new(recursive_addr);
    let page_table = unsafe { &mut *rec_virt.as_mut_ptr::<PageTable>() };
    let mut rec_page_table = 
        x86_64::structures::paging::RecursivePageTable::new(page_table)
            .expect("recursive page table creation failed");
    
    // VGA'yı 0xB8000'e identity-map et (fiziksel = sanal)
    let vga_phys = PhysAddr::new(0xB8000);
    let vga_virt = VirtAddr::new(0xB8000);
    
    let vga_page = Page::<Size4KiB>::containing_address(vga_virt);
    let vga_frame = PhysFrame::<Size4KiB>::containing_address(vga_phys);
    
    // UC flags: PRESENT | WRITABLE | NO_CACHE | WRITE_THROUGH
    let flags = PageTableFlags::PRESENT 
        | PageTableFlags::WRITABLE 
        | PageTableFlags::NO_CACHE 
        | PageTableFlags::WRITE_THROUGH;
    
    unsafe {
        match rec_page_table.map_to(vga_page, vga_frame, flags, &mut IgnoreAllocator) {
            Ok(flush) => flush.flush(),
            Err(x86_64::structures::paging::mapper::MapToError::PageAlreadyMapped(_)) => {
                if let Ok(flush) = rec_page_table.update_flags(vga_page, flags) {
                    flush.flush();
                }
            }
            Err(_) => {}
        }
    }
    
    unsafe {
        VGA_VIRT_ADDR = 0xB8000;
    }
}

struct IgnoreAllocator;

unsafe impl x86_64::structures::paging::FrameAllocator<Size4KiB> for IgnoreAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        None
    }
}
