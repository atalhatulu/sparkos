use linked_list_allocator::LockedHeap;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Heap'i en büyük usable bölgeye kur
pub fn init_heap(phys_mem_offset: u64, memory_map: &bootloader::bootinfo::MemoryMap) {
    use bootloader::bootinfo::MemoryRegionType;

    let mut best_start = 0u64;
    let mut best_size = 0u64;

    for region in memory_map.iter() {
        if region.region_type == MemoryRegionType::Usable {
            let size = region.range.end_addr() - region.range.start_addr();
            if size > best_size {
                best_start = region.range.start_addr();
                best_size = size;
            }
        }
    }

    // Bellek sızıntılarını önlemek ve gerçek bir allocator kullanmak için 128 MB'a kadar limit
    let heap_size = if best_size > 128 * 1024 * 1024 { 128 * 1024 * 1024 } else { best_size as usize };
    let phys_start = best_start;
    let virt_start = (phys_start + phys_mem_offset) as usize;

    unsafe {
        ALLOCATOR.lock().init(virt_start as *mut u8, heap_size);
    }

    crate::serial_println!("[OK] Modern Memory Allocator Init: {:#x} - {:#x} ({} MB)",
        virt_start, virt_start + heap_size, heap_size / 1024 / 1024);
}
