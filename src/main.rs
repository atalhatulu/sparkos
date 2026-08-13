#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

pub mod ui;
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
pub mod editor;
pub mod pci;
pub mod rtl8139;
pub mod net;
pub mod user;
pub mod elf;
pub mod syscall;
pub mod sync;
pub mod ipc;
pub mod fd;
pub mod syscall_storage;
pub mod display;
pub mod usb;
pub mod net_socket;
pub mod security;
pub mod sec_mem;
pub mod acpi;
pub mod smp;
pub mod klog;
pub mod panic;
pub mod ktrace;
pub mod app;
pub mod sysapi;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial::SerialWriter::force_write("KERNEL PANIC: ");
    
    crate::panic::crash_dump(info);
    crate::panic::halt_loop()
}

entry_point!(kernel_main);

async fn clock_task() {
    loop {
        let current_tick = crate::interrupts::get_tick();
        let seconds = current_tick / 1000;
        
        let mut time_str = alloc::string::String::new();
        core::fmt::write(&mut time_str, format_args!(" UP: {:04}s ", seconds)).unwrap();
        
        if crate::vga_buffer::GUI_MODE.load(core::sync::atomic::Ordering::Relaxed) {
            let mut px = 1920 - 95;
            for c in time_str.chars() {
                crate::gui::draw_char(px, 1080 - 20, c, 0x00000000, 0x00C0C0C0);
                px += 8;
            }
            crate::gui::flush_rect(1920 - 100, 1080 - 30, 96, 26);
        } else {
            let mut w = crate::vga_buffer::WRITE_LOCK.lock();
            w.write_at(0, 69, &time_str, crate::vga_buffer::Color::Yellow, crate::vga_buffer::Color::Blue);
        }
        
        // 24 FPS hissi ve saat guncellemesi (1000ms / 24 ~ 41ms ama saniye basi guncellesek de yeter. User 24fps dedigi icin
        // ~41 ms bekleyebiliriz ya da saat oldugu icin 1 saniye bekleriz. Ama arayuz animasyonu istediyse 41 ms yapalim)
        let target = current_tick + 41; // ~24 FPS refresh rate
        while crate::interrupts::get_tick() < target {
            crate::task::yield_now().await;
        }
    }
}

static TEST_CHAN: crate::sync::BlockingChannel<u32> = crate::sync::BlockingChannel::new(10);

async fn ipc_consumer() {
    let mut count = 0;
    while count < 4 {
        if let Some(val) = TEST_CHAN.try_recv() {
            crate::serial_println!("[IPC Consumer] Received: {}", val);
            count += 1;
        } else {
            crate::task::yield_now().await;
        }
    }
    crate::serial_println!("[IPC Consumer] Test complete.");
}

async fn ipc_producer_1() {
    for i in 1..=2 {
        while TEST_CHAN.try_send(i).is_err() {
            crate::task::yield_now().await;
        }
        crate::serial_println!("[IPC Producer 1] Sent: {}", i);
        crate::task::yield_now().await;
    }
}

async fn ipc_producer_2() {
    for i in 3..=4 {
        while TEST_CHAN.try_send(i).is_err() {
            crate::task::yield_now().await;
        }
        crate::serial_println!("[IPC Producer 2] Sent: {}", i);
        crate::task::yield_now().await;
    }
}

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    serial::SerialWriter::init();
    serial_println!("[OK] Serial port ready");
    serial_println!("[OK] Phys mem offset: {:#x}", boot_info.physical_memory_offset);
    unsafe {
        gui::PHYS_OFFSET = boot_info.physical_memory_offset;
    }
    
    // Heap (GUI buffer için önce heap başlatılmalı)
    allocator::init_heap(boot_info.physical_memory_offset, &boot_info.memory_map);
    // Dedicated user-space frame pool (syscall/user izolasyonu için)
    memory::init_user_memory(&boot_info.memory_map);

    // VGA çıktı
    vga_buffer::WRITE_LOCK.lock().clear();
    {
        let mut w = vga_buffer::WRITE_LOCK.lock();
        w.set_color(vga_buffer::Color::Cyan, vga_buffer::Color::Black);
        writeln!(w, " SparkOS v0.1 - Rust x86_64                        ").unwrap();
        w.set_color(vga_buffer::Color::White, vga_buffer::Color::Black);
        writeln!(w, "=====================================================").unwrap();
    }
    
    // Paging ve Memory Protection Baslatiliyor
    let phys_mem_offset = x86_64::VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe { memory::BootInfoFrameAllocator::init(&boot_info.memory_map) };
    serial_println!("[OK] Virtual Memory (Paging) Initialized");
    
    // Paging test: 0x1000 sanal adresini boş bir fiziksel çerçeveye haritala (Map)
    let page = x86_64::structures::paging::Page::containing_address(x86_64::VirtAddr::new(0x1000));
    match memory::create_example_mapping(page, &mut mapper, &mut frame_allocator) {
        Ok(_) => serial_println!("[OK] Paging Test: Virtual Address 0x1000 successfully mapped to a physical frame!"),
        Err(e) => serial_println!("[FAIL] Paging Test Error: {}", e),
    }

    // Eski sistem geriye dönük uyumluluk
    memory::map_vga_uc(boot_info.recursive_page_table_addr, boot_info.physical_memory_offset);
    serial_println!("[OK] VGA mapped");
    
    // Ag Kartini (RTL8139) Baslat
    match rtl8139::init_network() {
        Ok(_) => serial_println!("[OK] RTL8139 Network Card Initialized"),
        Err(e) => serial_println!("[FAIL] Network Init Error: {}", e),
    }

    // Heap artık daha erken başlatılıyor.
    
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
    
    // Syscall dispatcher (Linux/SysV ABI, int 0x80 üzerinden)
    syscall::init();
    
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
    // Initialize async keyboard scancode queue BEFORE enabling interrupts
    // so the first timer/keyboard IRQ never sees an uninitialized queue.
    task::keyboard::init();

    x86_64::instructions::interrupts::enable();
    serial_println!("[OK] Interrupts enabled");
    
    core::fmt::Write::write_str(&mut *vga_buffer::WRITE_LOCK.lock(), "\n[OK] Loading filesystem from ATA disk...\n").unwrap();
    fs::load_from_disk();
    
    core::fmt::Write::write_str(&mut *vga_buffer::WRITE_LOCK.lock(), "[OK] Starting shell task (Async)...\n").unwrap();
    
    let mut executor = task::simple_executor::SimpleExecutor::new();
    
    let mut shell = shell::Shell::new();
    executor.spawn(task::Task::new("shell", async move {
        shell.run().await;
    }));
    
    // Arka plan saati görevini başlat (Multitasking Gösterimi)
    executor.spawn(task::Task::new("clock", clock_task()));
    
    // Senkron test: 2 producer, 1 consumer üzerinden sayı akışı (Scheduler kitlenmeden çalışır)
    executor.spawn(task::Task::new("ipc_consumer", ipc_consumer()));
    executor.spawn(task::Task::new("ipc_prod_1", ipc_producer_1()));
    executor.spawn(task::Task::new("ipc_prod_2", ipc_producer_2()));
    
    executor.spawn(task::Task::new("mouse", mouse::mouse_task())); // Mouse task (GUI sonraki faz)

    // NOTE: GUI devre dışı — kullanıcı planı: GUI EN SONA, önce terminali
    // Linux terminali kadar güçlü yap. GUI backbuffer alloc + init şu an
    // heap'i taşırıp panic üretiyordu (VBE/backbuffer henüz stabil değil).
    // Sistem text-mode VGA + shell üzerinden boot eder. GUI bu faz zincirinin
    // en sonuna bağlanacak.
    // crate::vga_buffer::GUI_MODE varsayılan false kalır (text mode).

    executor.run();
    
    // Fallback loop in case executor exits
    loop { x86_64::instructions::hlt(); }
}
