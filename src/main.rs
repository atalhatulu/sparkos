#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

extern crate alloc;

use bootloader::{entry_point, BootInfo};
use bootloader::bootinfo::MemoryRegionType;
use core::panic::PanicInfo;
use core::fmt::Write;

pub mod serial;
pub mod vga_buffer;
pub mod interrupts;
pub mod memory;
pub mod allocator;
pub mod keyboard;
pub mod shell;
pub mod fs;
pub mod task;
pub mod ata;
pub mod gdt;
pub mod font;
pub mod gui;
pub mod mouse;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial::SerialWriter::force_write("KERNEL PANIC: ");
    if let Some(msg) = info.message().as_str() {
        serial::SerialWriter::force_write(msg);
    }
    serial::SerialWriter::force_write("\n");
    loop { x86_64::instructions::hlt(); }
}

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    serial::SerialWriter::init();
    serial_println!("[OK] Serial port ready");
    serial_println!("[OK] Phys mem offset: {:#x}", boot_info.physical_memory_offset);
    unsafe {
        gui::PHYS_OFFSET = boot_info.physical_memory_offset;
    }
    
    // Boot directly into GUI
    serial_println!("[OK] Switching to GUI Pixel Mode...");
    gui::init();
    gui::draw_desktop_and_window(100, 100, 800, 500, false);
    
    use core::fmt::Write;
    {
        let mut w = gui::WRITER.lock();
        writeln!(w, "SparkOS GUI Mode Initialized!").unwrap();
        writeln!(w, "Physical memory offset: {:#x}", boot_info.physical_memory_offset).unwrap();
    }
    
    // VGA uncacheable map (No longer strictly needed for text, but keeping it for structure)
    memory::map_vga_uc(boot_info.recursive_page_table_addr, boot_info.physical_memory_offset);
    serial_println!("[OK] VGA mapped as UC");
    
    // Heap
    allocator::init_heap(boot_info.physical_memory_offset, &boot_info.memory_map);
    
    // VGA çıktı
    vga_buffer::WRITE_LOCK.lock().clear();
    {
        let mut w = vga_buffer::WRITE_LOCK.lock();
        w.set_color(vga_buffer::Color::Cyan, vga_buffer::Color::Black);
        writeln!(w, " SparkOS v0.1 - Rust x86_64                        ").unwrap();
        w.set_color(vga_buffer::Color::White, vga_buffer::Color::Black);
        writeln!(w, "=====================================================").unwrap();
    }
    
    // Bellek bilgisi
    let memory_map = &boot_info.memory_map;
    let total_memory: u64 = memory_map
        .iter()
        .filter(|r| r.region_type == MemoryRegionType::Usable)
        .map(|r| r.range.end_addr() - r.range.start_addr())
        .sum();
    serial_println!("[OK] Memory: {} MB usable", total_memory / (1024 * 1024));
    {
        let mut w = vga_buffer::WRITE_LOCK.lock();
        writeln!(w, " Bellek: {} MB usable                              ", total_memory / (1024 * 1024)).unwrap();
    }
    
    // GDT
    serial_println!("[OK] Initializing GDT/TSS...");
    gdt::init();
    
    // Interrupts
    serial_println!("[OK] Initializing IDT...");
    interrupts::init_idt();
    serial_println!("[OK] IDT loaded");
    
    serial_println!("[OK] Initializing PIC...");
    interrupts::init_pic();
    serial_println!("[OK] PIC remapped");
    
    // Mouse
    serial_println!("[OK] Initializing PS/2 Mouse...");
    mouse::init();
    
    serial_println!("[OK] Initializing timer...");
    interrupts::init_timer();
    serial_println!("[OK] Timer (1000 Hz) ready");
    
    // Klavye handler'ını keyboard IRQ'ya bağla
    // keyboard_handler zaten interrupts.rs'de, onu güncellemek lazım
    // Şimdilik shell başlat
    
    serial_println!("[OK] Enabling interrupts...");
    x86_64::instructions::interrupts::enable();
    serial_println!("[OK] Interrupts enabled");
    
    // Initialize async keyboard scancode queue
    task::keyboard::init();
    
    core::fmt::Write::write_str(&mut *gui::WRITER.lock(), "\n[OK] Loading filesystem from ATA disk...\n").unwrap();
    fs::load_from_disk();
    
    core::fmt::Write::write_str(&mut *gui::WRITER.lock(), "[OK] Starting shell task (Async)...\n").unwrap();
    
    let mut executor = task::simple_executor::SimpleExecutor::new();
    
    let mut shell = shell::Shell::new();
    executor.spawn(task::Task::new(async move {
        shell.run().await;
    }));
    
    executor.spawn(task::Task::new(mouse::mouse_task()));
    
    executor.run();
    
    // Fallback loop in case executor exits
    loop { x86_64::instructions::hlt(); }
}
