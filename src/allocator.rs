use linked_list_allocator::LockedHeap;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

pub static KERNEL_HEAP_RANGE: spin::Mutex<(u64, u64)> = spin::Mutex::new((0, 0));

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

    // 48 MB kernel heap, kalan tüm bellek kullanıcı süreçleri ve Frame Allocator için ayrılır
    let heap_size = if best_size > 48 * 1024 * 1024 { 48 * 1024 * 1024 } else { ((best_size / 2) as usize).max(4 * 1024 * 1024) };
    let phys_start = best_start;
    let virt_start = (phys_start + phys_mem_offset) as usize;

    *KERNEL_HEAP_RANGE.lock() = (phys_start, phys_start + heap_size as u64);

    unsafe {
        ALLOCATOR.lock().init(virt_start as *mut u8, heap_size);
    }

    crate::serial_println!("[OK] Modern Memory Allocator Init: {:#x} - {:#x} ({} MB)",
        virt_start, virt_start + heap_size, heap_size / 1024 / 1024);
}
