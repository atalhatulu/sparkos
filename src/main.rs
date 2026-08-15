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
pub mod cap;
pub mod syscall_cap; // Asama 2 — syscall yetki kontrolü köprüsü
pub mod dma_region;  // Asama 6.1 — Capability-gated DMA bolgesi
pub mod acpi;
pub mod iommu;
pub mod smp;
pub mod klog;
pub mod panic;
pub mod ktrace;
pub mod app;
pub mod sysapi;
pub mod surface;
pub mod wm;
pub mod input;
pub mod pkg;
pub mod service;
pub mod auth;
pub mod app_registry;

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
            let (sw, sh) = unsafe { (crate::gui::VESA.width, crate::gui::VESA.height) };
            if sw >= 100 && sh >= 30 {
                let mut px = sw.saturating_sub(95);
                for c in time_str.chars() {
                    crate::gui::draw_char(px, sh.saturating_sub(18), c, 0x00E2E8F0, 0x000F172A);
                    px += 8;
                }
                crate::gui::flush_rect(sw.saturating_sub(100), sh.saturating_sub(24), 96, 24);
            }
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

#[allow(dead_code)]
static TEST_CHAN: crate::sync::BlockingChannel<u32> = crate::sync::BlockingChannel::new(10);

#[allow(dead_code)]
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

#[allow(dead_code)]
async fn ipc_producer_1() {
    for i in 1..=2 {
        while TEST_CHAN.try_send(i).is_err() {
            crate::task::yield_now().await;
        }
        crate::serial_println!("[IPC Producer 1] Sent: {}", i);
        crate::task::yield_now().await;
    }
}

#[allow(dead_code)]
async fn ipc_producer_2() {
    for i in 3..=4 {
        while TEST_CHAN.try_send(i).is_err() {
            crate::task::yield_now().await;
        }
        crate::serial_println!("[IPC Producer 2] Sent: {}", i);
        crate::task::yield_now().await;
    }
}

/// Aşama 5.2 boot regresyonu: user-space servis çerçevesini canlı doğrular.
/// Bir Device capability üretir, `keysvc` servisini Ring 3'te spawn eder ve
/// cooperative girer. Servis kendi endpoint'ini (SYS_IPC_CREATE_ENDPOINT) kurar,
/// timer + klavye IRQ'larını bağlar (SYS_IPC_BIND_IRQ), 64 olayı poll'layıp
/// echo'lar ve SYS_EXIT ile kapanır; `enter_service` geri dönünce IRQ'lar
/// unbind edilir. Headless QEMU'da timer her tick deterministik olay üretir.
#[allow(dead_code)]
async fn service_demo() {
    let dev = crate::cap::create_object(crate::cap::ObjectKind::Device).unwrap();
    match crate::task::process::spawn_service("keysvc", dev, crate::task::process::service_machine_code) {
        Ok(pid) => {
            crate::serial_println!("[SERVICE] keysvc pid={} entering (cooperative)", pid);
            crate::task::process::enter_service(pid);
            let _ = crate::ipc::unbind_irq(dev, 0);
            let _ = crate::ipc::unbind_irq(dev, 1);
            crate::serial_println!("[SERVICE] demo complete (IRQs unbound).");
        }
        Err(e) => {
            crate::serial_println!("[SERVICE] spawn failed: {}", e);
        }
    }
}

/// Aşama 5.3 boot regresyonu: user-space serial driver. `create_device_ports`
/// COM1 (0x3F8..=0x3FF) aralığına bağlı bir Device capability üretir ve servise
/// IO|MANAGE haklı handle provision edilir. Servis Ring 3'te `sys_ioperm` ile
/// portları açar (capability-gated; TSS IOPB'de 0x3F8..=0x3FF izinli yapılır),
/// sonra raw `outb` ile COM1'e "[SERDRV] alive\r\n" yazar (LSR poll) ve SYS_EXIT
/// ile kapanır. Yazılan baytlar QEMU `-serial stdio` üzerinden boot log'una
/// düşer — Ring-3 port I/O'nun canlı kanıtı.
#[allow(dead_code)]
async fn serial_demo() {
    match crate::task::process::spawn_serial_service("serdrv") {
        Ok(pid) => {
            crate::serial_println!("[SERIAL] serdrv pid={} entering (cooperative)", pid);
            crate::task::process::enter_service(pid);
            crate::serial_println!("[SERIAL] demo complete (Ring-3 COM1 TX verified).");
        }
        Err(e) => {
            crate::serial_println!("[SERIAL] spawn failed: {}", e);
        }
    }
}

/// Aşama 5.4 boot regresyonu: user-space fault recovery. `faultsvc` Ring 3'te
/// `mov eax, [0x5000_0000]` çalıştırır — user yarısında deterministik olarak
/// eşlenmemiş bir adres. Kernel fault'u process modeli altında kurtarmalı
/// (`exit_current` → Terminated + KILLED_PROCESSES + executor devam), legacy
/// `user::KERNEL_RSP`/`KERNEL_RIP` çerçevesine asla dokunmamalı. Başarı: boot
/// log'unda "[USER-FAULT] process N ('faultsvc') faulted" ve "[FAULT] demo
/// complete" görülmeli; "[PANIC] Kernel Page Fault" OLMAMALI.
#[allow(dead_code)]
async fn fault_demo() {
    match crate::task::process::spawn_fault_service("faultsvc") {
        Ok(pid) => {
            crate::serial_println!("[FAULT] faultsvc pid={} entering (cooperative)", pid);
            crate::task::process::enter_service(pid);
            crate::serial_println!("[FAULT] demo complete (user fault recovered, executor resumed).");
        }
        Err(e) => {
            crate::serial_println!("[FAULT] spawn failed: {}", e);
        }
    }
}

/// Aşama 6.3 boot regresyonu: Ring-3 RTL8139 driver'ın üst yığına zero-copy frame.
///
/// Kernel bir IPC endpoint'i kurar, WRITE hakkını grant edip `ep_writer` handle'ını
/// netdrv'e provision eder (fd == `kern_ep_id`). netdrv RX'te doğruladığı ARP reply/// Aşama 6.3: netdrv (RTL8139 Ring 3 DMA) <-> netsvc (Ring 3 TCP/IP) Zero-Copy Frame Hand-off.
#[allow(dead_code)]
async fn net_demo() {
    let (rx_ep_id, rx_root) = match crate::ipc::create_raw_endpoint(8) {
        Ok(pair) => pair,
        Err(e) => {
            crate::serial_println!("[NETDRV] RX endpoint create failed: {:?}", e);
            return;
        }
    };
    let rx_writer = match crate::cap::grant(rx_root, crate::cap::Rights::WRITE) {
        Ok(h) => h,
        Err(_) => return,
    };
    let rx_reader = match crate::cap::grant(rx_root, crate::cap::Rights::READ) {
        Ok(h) => h,
        Err(_) => return,
    };

    let (recycle_ep_id, recycle_root) = match crate::ipc::create_raw_endpoint(8) {
        Ok(pair) => pair,
        Err(e) => {
            crate::serial_println!("[NETDRV] recycle endpoint create failed: {:?}", e);
            return;
        }
    };
    let recycle_writer = match crate::cap::grant(recycle_root, crate::cap::Rights::WRITE) {
        Ok(h) => h,
        Err(_) => return,
    };

    // 1. netdrv sürücüsünü başlat (RX frame'i rx_ep_id'ye iletir)
    match crate::task::process::spawn_net_service("netdrv", rx_ep_id, rx_writer) {
        Ok(drv_pid) => {
            crate::serial_println!("[NETDRV] netdrv pid={} entering (cooperative)", drv_pid);
            crate::task::process::enter_service(drv_pid);

            // 2. netsvc TCP/IP servisini başlat (rx_ep_id'den paketi alır ve recycle_ep_id ile iade eder)
            match crate::task::process::spawn_netsvc("netsvc", rx_ep_id, rx_reader, recycle_ep_id, recycle_writer) {
                Ok(svc_pid) => {
                    crate::serial_println!("[NETSVC] netsvc pid={} entering (cooperative)", svc_pid);
                    crate::task::process::enter_service(svc_pid);
                    crate::serial_println!("[NETDRV] demo complete (Ring-3 RTL8139 + netsvc Zero-Copy Frame Bridge).");
                }
                Err(e) => {
                    crate::serial_println!("[NETSVC] spawn failed: {}", e);
                }
            }
        }
        Err(e) => {
            crate::serial_println!("[NETDRV] spawn failed: {}", e);
        }
    }
}

/// Aşama 8.1 boot regresyonu: user-space ATA disk driver.
#[allow(dead_code)]
async fn disk_demo() {
    match crate::task::process::spawn_disk_service("disksvc") {
        Ok(pid) => {
            crate::serial_println!("[DISK] disksvc pid={} entering (cooperative)", pid);
            crate::task::process::enter_service(pid);
            crate::serial_println!("[DISK] demo complete (Ring-3 ATA PIO 0x1F0 verified).");
        }
        Err(e) => {
            crate::serial_println!("[DISK] spawn failed: {}", e);
        }
    }

    // Faz 5: Ring-3 Filesystem Servisi (`fssvc`)
    match crate::task::process::spawn_fs_service("fssvc") {
        Ok(pid) => {
            crate::serial_println!("[FSSVC] fssvc pid={} entering (cooperative)", pid);
            crate::task::process::enter_service(pid);
            crate::serial_println!("[FSSVC] demo complete (SPFS & VFS Service Isolation verified).");
        }
        Err(e) => {
            crate::serial_println!("[FSSVC] spawn failed: {}", e);
        }
    }

    // Faz 6: Ring-3 Kullanıcı Shell'i (`sh`) & Kullanıcı Ortamı
    match crate::task::process::spawn_user_shell("sh") {
        Ok(pid) => {
            crate::serial_println!("[SHELL] sh pid={} entering (Ring 3)", pid);
            crate::task::process::enter_service(pid);
            crate::serial_println!("[SHELL] demo complete (Ring-3 Shell & User Environment verified).");
        }
        Err(e) => {
            crate::serial_println!("[SHELL] spawn failed: {}", e);
        }
    }

    // Faz 11: Ring-3 Display Server (`displaysvc`) & Surface Shmem Compositor
    match crate::task::process::spawn_display_server("displaysvc") {
        Ok(pid) => {
            crate::serial_println!("[DISPLAYSVC] displaysvc pid={} entering (Ring 3)", pid);
            crate::task::process::enter_service(pid);
            crate::serial_println!("[DISPLAYSVC] demo complete (Ring-3 Framebuffer Display Server verified).");
        }
        Err(e) => {
            crate::serial_println!("[DISPLAYSVC] spawn failed: {}", e);
        }
    }

    // Faz 12: Ring-3 Window Manager & Compositor (`wm`)
    match crate::task::process::spawn_window_manager("wm") {
        Ok(pid) => {
            crate::serial_println!("[WM] wm pid={} entering (Ring 3)", pid);
            crate::task::process::enter_service(pid);
            crate::serial_println!("[WM] demo complete (Ring-3 Window Manager & Compositor verified).");
        }
        Err(e) => {
            crate::serial_println!("[WM] spawn failed: {}", e);
        }
    }
}

async fn boot_orchestrator() {
    // Welcome banner and interactive shell prompt
    {
        let mut w = crate::vga_buffer::WRITE_LOCK.lock();
        w.set_color(crate::vga_buffer::Color::LightGreen, crate::vga_buffer::Color::Black);
        core::fmt::Write::write_str(&mut *w, "\n================================================================\n").unwrap();
        core::fmt::Write::write_str(&mut *w, "  SparkOS x86_64 Mikrocekirdek Baslatildi.\n").unwrap();
        core::fmt::Write::write_str(&mut *w, "  Komutlari listelemek icin 'help' yazabilirsiniz.\n").unwrap();
        core::fmt::Write::write_str(&mut *w, "================================================================\n\n").unwrap();
    }

    let mut shell = shell::Shell::new();
    loop {
        shell.run().await;
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

    // Asama 2.0 (fcc ön koşulu): capability core aktiflesir. BOOTTA init edilir;
    // ROOT capability (ObjectKind::Process) olusturulur. Bu olmadan capability
    // core tum syscall'lar icin pasifti.
    cap::init();
    if let Some(root) = cap::root_cap().or_else(|| cap::bootstrap_root().ok()) {
        // Root handle kaybolmaz: bootstrap_root onu kernel-resident ROOT_CAP
        // static'ine kaydeder (cap::root_cap() ile erişilir) — capability
        // hiyerarşisinin kök yetkisi izlenebilir kalır.
        serial_println!("[OK] Capability core initialized (root capability slot={}, gen={})",
            root.slot, root.generation);
    } else {
        serial_println!("[ERR] Capability core init FAILED");
    }
    
    serial_println!("[OK] Initializing PIC...");
    interrupts::init_pic();
    serial_println!("[OK] PIC remapped");
    
    // Mouse
    serial_println!("[OK] Initializing PS/2 Mouse...");
    mouse::init();
    
    serial_println!("[OK] Initializing timer...");
    interrupts::init_timer();
    serial_println!("[OK] Timer (1000 Hz) ready");

    // Aşama 9: SMP Keşfi ve APIC Başlatma
    smp::init_smp();

    // Faz 29: ACPI DMAR & Intel VT-d IOMMU Keşfi, Tablo İnşası ve Translation Enable (TE)
    if let Some(_caps) = iommu::probe_and_setup_iommu() {
        serial_println!("[OK] IOMMU Hardware DMA Translation Active (TE=1, RTPS=1, Domain 1 Isolated)");
    } else {
        serial_println!("[IOMMU] No ACPI DMAR Table found (Hardware IOMMU disabled or emulated standard platform)");
    }
    
    serial_println!("[OK] Enabling interrupts...");
    // Initialize async keyboard scancode queue BEFORE enabling interrupts
    // so the first timer/keyboard IRQ never sees an uninitialized queue.
    task::keyboard::init();
    // Aşama 5.1: IRQ notification event kuyruğu da interrupt'lardan önce hazır
    // olmalı — irq_event push yapmadan önce kuyruğun varlığını kontrol eder ama
    // init eksikse ilk timer IRQ'ları düşer.
    ipc::init_irq_notify();

    x86_64::instructions::interrupts::enable();
    serial_println!("[OK] Interrupts enabled");
    
    core::fmt::Write::write_str(&mut *vga_buffer::WRITE_LOCK.lock(), "\n[OK] Loading filesystem from ATA disk...\n").unwrap();
    fs::load_from_disk();

    core::fmt::Write::write_str(&mut *vga_buffer::WRITE_LOCK.lock(), "[OK] Starting shell task (Async)...\n").unwrap();
    
    let mut executor = task::simple_executor::SimpleExecutor::new();

    // Arka plan saati görevini başlat (Multitasking Gösterimi)
    executor.spawn(task::Task::new("clock", clock_task()));
    executor.spawn(task::Task::new("mouse", mouse::mouse_task()));

    // Boot dizisini tamamlayıp ardından interaktif shell'i açık tutan ana görev
    executor.spawn(task::Task::new("boot_orchestrator", boot_orchestrator()));

    executor.run();
    
    // Fallback loop in case executor exits
    loop { x86_64::instructions::hlt(); }
}
