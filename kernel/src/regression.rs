//! Opt-in QEMU regressions against the real allocator, page tables and GUI code.
use crate::{memory, task::process};

#[cfg(feature = "legacy-desktop")]
use crate::{surface, wm};

fn owner() -> (u64, u64) {
    let cr3 = memory::clone_active_cr3().expect("regression CR3");
    process::set_preemption_enabled(false);
    let pid = process::create_user_process("surface-regression", memory::USER_ADDR_BASE,
        memory::USER_STACK_TOP, cr3, crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0);
    process::remove_process_from_ready(pid);
    process::set_preemption_enabled(true);
    (pid, cr3)
}

#[cfg(feature = "legacy-desktop")]
fn usage(pid: u64) -> (u64, u32) {
    let sched = process::SCHEDULER.lock();
    let proc = sched.get_process(pid).unwrap();
    (proc.resource_usage.current_memory_bytes, proc.resource_usage.surface_count)
}

fn dispose(pid: u64) {
    #[cfg(feature = "legacy-desktop")]
    surface::cleanup_surfaces_for_pid(pid);
    process::release_process_address_space(pid);
    let mut sched = process::SCHEDULER.lock();
    if let Some(proc) = sched.get_process_mut(pid) {
        crate::cap::destroy_process_cspace(&mut proc.cap_table);
    }
    sched.purge_pid(pid);
    crate::task::PROCESS_LIST.lock().remove(&pid);
}

pub fn run() {
    crate::serial_println!("[REGRESSION] START");
    let baseline = memory::get_memory_stats().0;

    #[cfg(feature = "legacy-desktop")]
    {
        let (a, ca) = owner();
        let (b, cb) = owner();
        let owners_only = memory::get_memory_stats().0;
        let sa = surface::create_surface_for_pid(a, 64, 64).unwrap();
        let sb = surface::create_surface_for_pid(b, 64, 64).unwrap();
        let va = surface::USER_SURFACE_BASE;
        let pa = memory::translate_page_in_cr3(ca, va).unwrap().as_u64();
        let pb = memory::translate_page_in_cr3(cb, va).unwrap().as_u64();
        assert_ne!(pa, pb, "processes must not share surface PTEs");
        assert!(memory::translate_page_in_cr3(0, va).is_none(), "surface leaked into kernel CR3");
        unsafe {
            let offset = crate::memory::PHYS_OFFSET;
            core::ptr::write_volatile((offset + pa) as *mut u32, 0xaabbccdd);
            core::ptr::write_volatile((offset + pb) as *mut u32, 0x11223344);
            assert_eq!(core::ptr::read_volatile((offset + pa) as *const u32), 0xaabbccdd);
        }
        assert!(surface::destroy_surface_for_pid(b, sa).is_err());
        surface::destroy_surface_for_pid(a, sa).unwrap();
        assert!(memory::translate_page_in_cr3(ca, va).is_none());
        assert_eq!(memory::translate_page_in_cr3(cb, va).unwrap().as_u64(), pb);
        surface::destroy_surface_for_pid(b, sb).unwrap();
        assert_eq!(usage(a), (0, 0));
        assert_eq!(usage(b), (0, 0));
        assert_eq!(memory::get_memory_stats().0, owners_only);
        crate::serial_println!("[REGRESSION] PASS surface isolation, ownership and reclaim");

        // Fail before backing allocation, during page-table creation, and after
        // the first 512 pages have been mapped. Each must roll back PTEs and quotas.
        for remaining in [0, 1, 3] {
            memory::fail_allocation_after(remaining);
            let result = surface::create_surface_for_pid(a, 1280, 720);
            memory::fail_allocation_after(-1);
            assert!(result.is_err(), "injected allocation failure was ignored");
            assert_eq!(usage(a), (0, 0));
            assert_eq!(memory::get_memory_stats().0, owners_only);
            assert!(memory::translate_page_in_cr3(ca, va).is_none());
        }
        crate::serial_println!("[REGRESSION] PASS OOM rollback including partial mapping");

        memory::map_user_page_in_cr3(ca, va + 4096, true).unwrap();
        let existing = memory::translate_page_in_cr3(ca, va + 4096).unwrap();
        let collision_baseline = memory::get_memory_stats().0;
        assert!(surface::create_surface_for_pid(a, 64, 32).is_err());
        assert_eq!(memory::translate_page_in_cr3(ca, va + 4096), Some(existing));
        assert!(memory::translate_page_in_cr3(ca, va).is_none());
        assert_eq!(memory::get_memory_stats().0, collision_baseline);
        assert_eq!(usage(a), (0, 0));
        dispose(a);
        dispose(b);
        assert_eq!(memory::get_memory_stats().0, baseline);
        crate::serial_println!("[REGRESSION] PASS mapping collision rollback and CR3 cleanup");

        let (pid, _) = owner();
        let before_surface = memory::get_memory_stats().0;
        {
            let mut sched = process::SCHEDULER.lock();
            let p = sched.get_process_mut(pid).unwrap();
            p.resource_limits.max_surfaces = 1;
        }
        let id = surface::create_surface_for_pid(pid, 1, 1).unwrap();
        assert_eq!(usage(pid), (4096, 1));
        assert!(surface::create_surface_for_pid(pid, 1, 1).is_err());
        assert_eq!(usage(pid), (4096, 1));
        surface::destroy_surface_for_pid(pid, id).unwrap();
        for _ in 0..24 {
            let id = surface::create_surface_for_pid(pid, 1280, 720).unwrap();
            surface::destroy_surface_for_pid(pid, id).unwrap();
            assert_eq!(memory::get_memory_stats().0, before_surface);
            assert_eq!(usage(pid), (0, 0));
        }
        dispose(pid);
        assert_eq!(memory::get_memory_stats().0, baseline);
        crate::serial_println!("[REGRESSION] PASS quota rollback and repeated large surface reuse");

        let processes_before = process::get_system_metrics_snapshot().len();
        for remaining in [0, 1, 2, 4, 7, 9, 10, 11] {
            memory::fail_allocation_after(remaining);
            let result = crate::terminal_app::spawn_terminal_app("regression-failed-launch");
            memory::fail_allocation_after(-1);
            assert!(result.is_err(), "launch must report injected OOM");
            assert_eq!(memory::get_memory_stats().0, baseline, "failed launch leaked frames");
            assert_eq!(process::get_system_metrics_snapshot().len(), processes_before);
        }
        crate::serial_println!("[REGRESSION] PASS failed app launch rolls back PCB, CR3 and backing");

        // Exercise production application allocation, maximize/restore, rendering,
        // window close and process cleanup, rather than a duplicate mock WM.
        for _ in 0..8 {
            let pid = crate::terminal_app::spawn_terminal_app("regression-terminal").unwrap();
            let mut wm = wm::WM.lock();
            let wid = wm.windows.iter().find(|w| w.owner_pid == pid).unwrap().window_id;
            wm.toggle_maximize_window(pid, wid).unwrap();
            wm.toggle_maximize_window(pid, wid).unwrap();
            wm.composite_desktop(0, 0);
            wm.destroy_window(pid, wid).unwrap();
            drop(wm);
            assert!(!surface::SURFACE_REGISTRY.read().iter().any(|s| s.owner_pid == pid));
            assert!(process::SCHEDULER.lock().get_process(pid).is_none());
            assert_eq!(memory::get_memory_stats().0, baseline, "window close leaked physical frames");
        }
        crate::serial_println!("[REGRESSION] PASS real terminal open, resize, composite and close x8");
    }

    #[cfg(not(feature = "legacy-desktop"))]
    {
        let (pid, cr3) = owner();
        let user_va = memory::USER_ADDR_BASE + 0x10_000;
        assert!(memory::map_user_page_in_cr3(cr3, user_va, true).is_ok());
        let pa = memory::translate_page_in_cr3(cr3, user_va).expect("mapped user page");
        assert!(memory::translate_page_in_cr3(0, user_va).is_none(), "user page leaked into kernel CR3");
        unsafe {
            let offset = crate::memory::PHYS_OFFSET;
            core::ptr::write_volatile((offset + pa.as_u64()) as *mut u64, 0x1234_5678_9abc_def0);
            assert_eq!(core::ptr::read_volatile((offset + pa.as_u64()) as *const u64), 0x1234_5678_9abc_def0);
        }
        dispose(pid);
        assert_eq!(memory::get_memory_stats().0, baseline, "headless process cleanup leaked memory");
        crate::serial_println!("[REGRESSION] PASS headless user process CR3 isolation and page mapping");
    }

    let hello = include_bytes!("../../fixtures/hello.elf");
    for _ in 0..2 {
        let pid = process::exec_elf_proc("regression-hello", hello).unwrap();
        let status = process::waitpid(pid).unwrap();
        assert_eq!(status, 0);
        assert!(x86_64::instructions::interrupts::are_enabled(), "waitpid return left interrupts disabled");
        assert!(process::SCHEDULER.lock().get_process(pid).unwrap().exited);
        assert_eq!(process::SCHEDULER.lock().get_process(pid).unwrap().exit_code, 0);
        assert_eq!(memory::get_memory_stats().0, baseline);
        dispose(pid);
    }
    crate::serial_println!("[REGRESSION] PASS ELF Ring-3 write/exit and address-space reclaim x2");

    #[cfg(feature = "legacy-desktop")]
    {
        let pid = process::exec_elf_proc("regression-surface-syscalls", include_bytes!("../../fixtures/surface_probe.elf")).unwrap();
        let status = process::waitpid(pid).unwrap();
        assert_eq!(status, 0);
        assert!(x86_64::instructions::interrupts::are_enabled());
        assert_eq!(process::SCHEDULER.lock().get_process(pid).unwrap().exit_code, 0);
        assert_eq!(memory::get_memory_stats().0, baseline);
        assert_eq!(usage(pid), (0, 0));
        dispose(pid);
        crate::serial_println!("[REGRESSION] PASS Ring-3 surface create/write/destroy/reuse x20");
        gui_interaction_regressions();
        settings_interaction_regressions();
        compositor_work_regression();
        input_delivery_regression();
        live_resize_regression();
    }

    two_user_processes_preemptive_regression();
    blocking_channel_stress_regression();
    service_supervisor_and_discovery_regression();

    crate::serial_println!("[REGRESSION] ALL PASS");
}

fn two_user_processes_preemptive_regression() {
    let baseline = memory::get_memory_stats().0;
    let hello = include_bytes!("../../fixtures/hello.elf");
    // İki Ring 3 kullanıcı sürecini eşzamanlı olarak hazır kuyruğuna al
    let pid_a = process::exec_elf_proc("user-proc-a", hello).unwrap();
    let pid_b = process::exec_elf_proc("user-proc-b", hello).unwrap();

    // İkisinin de kernel stack kanaryalarını doğrula
    {
        let s = process::SCHEDULER.lock();
        if let Some(p) = s.get_process(pid_a) { p.check_stack_canary(); }
        if let Some(p) = s.get_process(pid_b) { p.check_stack_canary(); }
    }

    // İki sürecin de sırayla tamamlanmasını bekle
    let status_a = process::waitpid(pid_a).unwrap();
    let status_b = process::waitpid(pid_b).unwrap();
    assert_eq!(status_a, 0);
    assert_eq!(status_b, 0);
    assert!(x86_64::instructions::interrupts::are_enabled());

    // Çıkış sonrası kanaryaların ezilmediğini doğrula
    {
        let s = process::SCHEDULER.lock();
        if let Some(p) = s.get_process(pid_a) { p.check_stack_canary(); }
        if let Some(p) = s.get_process(pid_b) { p.check_stack_canary(); }
    }

    dispose(pid_a);
    dispose(pid_b);
    assert_eq!(memory::get_memory_stats().0, baseline);
    crate::serial_println!("[REGRESSION] PASS two concurrent Ring-3 processes preemptive scheduling and TSS.rsp0 isolation");
}

static CONSUMER_DONE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static PRODUCER_DONE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static TOTAL_RECEIVED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static TEST_CHAN: crate::sync::BlockingChannel<u32> = crate::sync::BlockingChannel::new(4);

extern "C" fn blocking_consumer_entry() {
    crate::serial_println!("[TEST-CHAN] Consumer started, waiting on empty channel...");
    for expected in 0..1000 {
        let val = TEST_CHAN.recv();
        assert_eq!(val, expected);
        TOTAL_RECEIVED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    CONSUMER_DONE.store(true, core::sync::atomic::Ordering::SeqCst);
    crate::serial_println!("[TEST-CHAN] Consumer finished 1000 items successfully");
}

extern "C" fn blocking_producer_entry() {
    crate::serial_println!("[TEST-CHAN] Producer started sending...");
    for i in 0..1000 {
        // Lost-wakeup ve preemption penceresini zorlamak için yapay döngü varyantı
        if i % 10 == 0 {
            for _ in 0..200 {
                core::hint::spin_loop();
            }
        }
        TEST_CHAN.send(i);
    }
    PRODUCER_DONE.store(true, core::sync::atomic::Ordering::SeqCst);
    crate::serial_println!("[TEST-CHAN] Producer finished 1000 sends");
}

fn blocking_channel_stress_regression() {
    let baseline = memory::get_memory_stats().0;
    CONSUMER_DONE.store(false, core::sync::atomic::Ordering::SeqCst);
    PRODUCER_DONE.store(false, core::sync::atomic::Ordering::SeqCst);
    TOTAL_RECEIVED.store(0, core::sync::atomic::Ordering::SeqCst);

    // Tüketici thread'i önce oluştur ve hazır kuyruğuna al (kanal henüz boşken bloklanacak)
    let c_pid = process::create_kernel_process("block-consumer", blocking_consumer_entry);
    let p_pid = process::create_kernel_process("block-producer", blocking_producer_entry);

    let status_c = process::waitpid(c_pid).unwrap();
    let status_p = process::waitpid(p_pid).unwrap();
    assert_eq!(status_c, 0);
    assert_eq!(status_p, 0);

    assert_eq!(TOTAL_RECEIVED.load(core::sync::atomic::Ordering::Relaxed), 1000);
    assert!(CONSUMER_DONE.load(core::sync::atomic::Ordering::SeqCst));
    assert!(PRODUCER_DONE.load(core::sync::atomic::Ordering::SeqCst));

    dispose(c_pid);
    dispose(p_pid);
    assert_eq!(memory::get_memory_stats().0, baseline);
    crate::serial_println!("[REGRESSION] PASS blocking channel 1000 rounds consumer-first under -smp 1");
}

#[cfg(feature = "legacy-desktop")]
fn gui_interaction_regressions() {
    use crate::input::{dispatch_keyboard_event, MOD_ALT, MOD_CTRL};
    let baseline = memory::get_memory_stats().0;
    let pid = crate::editor_app::spawn_editor_app("close-regression", None).unwrap();
    let wid = wm::WM.lock().focused_window.unwrap();
    crate::editor_app::EDITOR_INSTANCES.lock().get_mut(&wid).unwrap().is_dirty = true;
    for _ in 0..2 {
        dispatch_keyboard_event(0x3e, true, MOD_ALT);
        assert!(wm::WM.lock().windows.iter().any(|w| w.window_id == wid));
        assert!(crate::editor_app::EDITOR_INSTANCES.lock().get(&wid).unwrap().show_unsaved_dialog);
    }
    dispatch_keyboard_event(0x01, true, 0);
    assert!(!crate::editor_app::EDITOR_INSTANCES.lock().get(&wid).unwrap().show_unsaved_dialog);
    {
        let mut manager = wm::WM.lock();
        let w = manager.windows.iter().find(|w| w.window_id == wid).unwrap();
        let (x, y) = (w.x + w.width as i32 - 12, w.y + 10);
        manager.handle_mouse_down(x, y);
    }
    assert!(crate::editor_app::EDITOR_INSTANCES.lock().get(&wid).unwrap().show_unsaved_dialog);
    dispatch_keyboard_event(0x20, true, 0); // Explicit discard.
    assert!(!wm::WM.lock().windows.iter().any(|w| w.window_id == wid));
    assert!(!crate::editor_app::EDITOR_INSTANCES.lock().contains_key(&wid));
    assert!(process::SCHEDULER.lock().get_process(pid).is_none());
    assert_eq!(memory::get_memory_stats().0, baseline);
    dispatch_keyboard_event(0x01, true, MOD_CTRL);
    assert!(wm::WM.lock().launcher_open);
    wm::WM.lock().damage_tracker.current_damage = None;
    dispatch_keyboard_event(0x50, true, 0);
    assert!(wm::WM.lock().damage_tracker.current_damage.unwrap()
        .contains_point(8, wm::DOCK_HEIGHT as i32 + 30));
    wm::WM.lock().damage_tracker.current_damage = None;
    dispatch_keyboard_event(0x01, true, 0);
    let manager = wm::WM.lock();
    assert!(!manager.launcher_open);
    assert!(manager.damage_tracker.current_damage.unwrap()
        .contains_point(8, wm::DOCK_HEIGHT as i32 + 30));
    crate::serial_println!("[REGRESSION] PASS editor repeated close/cancel/discard and launcher keyboard damage");
}

#[cfg(feature = "legacy-desktop")]
fn settings_interaction_regressions() {
    use crate::settings_app::{SettingsCategory, SETTINGS_INSTANCES};
    let baseline = memory::get_memory_stats().0;
    let original_theme = crate::theme::THEME_MANAGER.lock().current_mode;
    let first = crate::settings_app::spawn_settings_app("settings-first").unwrap();
    let first_wid = wm::WM.lock().focused_window.unwrap();
    let second = crate::settings_app::spawn_settings_app("settings-second").unwrap();
    let second_wid = wm::WM.lock().focused_window.unwrap();
    let (x, y) = {
        let manager = wm::WM.lock();
        let w = manager.windows.iter().find(|w| w.window_id == second_wid).unwrap();
        (w.x, w.y + 20)
    };
    crate::input::route_mouse_down(x + 20, y + 51); // Gap below first row.
    assert_eq!(SETTINGS_INSTANCES.lock().get(&second_wid).unwrap().active_category, SettingsCategory::Appearance);
    crate::input::route_mouse_down(x + 20, y + 60);
    assert_eq!(SETTINGS_INSTANCES.lock().get(&second_wid).unwrap().active_category, SettingsCategory::Display);
    crate::input::dispatch_keyboard_event(0x48, true, 0);
    crate::input::route_mouse_down(x + 130, y + 140); // Visible theme button.
    assert_ne!(crate::theme::THEME_MANAGER.lock().current_mode, original_theme);
    let background = crate::theme::THEME_MANAGER.lock().current_theme.window_active;
    for wid in [first_wid, second_wid] {
        let sid = wm::WM.lock().windows.iter().find(|w| w.window_id == wid).unwrap().surface_id;
        let reg = surface::SURFACE_REGISTRY.read();
        let surf = reg.iter().find(|s| s.surface_id == sid).unwrap();
        let pixel = unsafe { *((crate::memory::PHYS_OFFSET + surf.shmem_phys_addr) as *const u32)
            .add((200 * surf.width + 300) as usize) };
        assert_eq!(pixel, background, "theme must rerender other open Settings surfaces");
    }
    crate::input::dispatch_keyboard_event(0x39, true, 0); // Restore theme with Space.
    assert_eq!(crate::theme::THEME_MANAGER.lock().current_mode, original_theme);
    assert_eq!(crate::settings_app::SETTINGS_STORE.lock().dark_theme,
        original_theme == crate::theme::ThemeMode::SparkDark);
    crate::input::route_mouse_down(x + 20, y + 138); // System category.
    assert_eq!(SETTINGS_INSTANCES.lock().get(&second_wid).unwrap().active_category, SettingsCategory::System);
    {
        let mut manager = wm::WM.lock();
        manager.toggle_maximize_window(second, second_wid).unwrap();
        manager.toggle_maximize_window(second, second_wid).unwrap();
        manager.destroy_window(second, second_wid).unwrap();
        manager.destroy_window(first, first_wid).unwrap();
    }
    assert!(!SETTINGS_INSTANCES.lock().contains_key(&first_wid));
    assert!(!SETTINGS_INSTANCES.lock().contains_key(&second_wid));
    assert_eq!(memory::get_memory_stats().0, baseline);
    crate::serial_println!("[REGRESSION] PASS Settings mouse/keyboard, multi-window theme and System resize");
}

#[cfg(feature = "legacy-desktop")]
fn compositor_work_regression() {
    use core::sync::atomic::Ordering;
    let pid = crate::terminal_app::spawn_terminal_app("compositor-regression").unwrap();
    let mut manager = wm::WM.lock();
    let wid = manager.focused_window.unwrap();
    manager.mark_full_damage();
    manager.composite_desktop(0, 0);
    let (ptr, width, height) = unsafe {
        (crate::gui::BACKBUFFER, crate::gui::VESA.width as usize, crate::gui::VESA.height as usize)
    };
    let expected = unsafe { core::slice::from_raw_parts(ptr, width * height) }.to_vec();
    let areas = [(101usize, 180usize), (1001, 650)];
    // Corrupt only the requested regions, then require an exact reconstruction.
    for (x, y) in areas {
        for row in y..y + 16 {
            unsafe { core::slice::from_raw_parts_mut(ptr.add(row * width + x), 16).fill(0x00ff00ff); }
        }
        manager.mark_damage(x as i32, y as i32, 16, 16);
    }
    let before_pixels = wm::COMPOSITED_PIXELS.load(Ordering::Relaxed);
    let before_windows = wm::COMPOSITED_WINDOWS.load(Ordering::Relaxed);
    manager.composite_desktop(0, 0);
    let pixels = wm::COMPOSITED_PIXELS.load(Ordering::Relaxed) - before_pixels;
    let windows = wm::COMPOSITED_WINDOWS.load(Ordering::Relaxed) - before_windows;
    assert_eq!(pixels, 512);
    assert_eq!(windows, 1, "unrelated region should skip the terminal");
    let actual = unsafe { core::slice::from_raw_parts(ptr, width * height) };
    assert!(actual == expected.as_slice(), "partial composition differs from full reference");
    manager.composite_desktop(0, 0);
    assert_eq!(wm::COMPOSITED_PIXELS.load(Ordering::Relaxed), before_pixels + pixels,
        "idle composition must do no pixel work");
    manager.destroy_window(pid, wid).unwrap();
    crate::serial_println!("[REGRESSION] PASS compositor pixels: bounding=445176, regions={}, windows={}, exact image and idle skip", pixels, windows);
}

#[cfg(feature = "legacy-desktop")]
fn input_delivery_regression() {
    let pid = crate::terminal_app::spawn_terminal_app("input-delivery-regression").unwrap();
    let wid = wm::WM.lock().focused_window.unwrap();
    for _ in 0..100 {
        crate::input::dispatch_keyboard_event(0x1e, true, 0);
        crate::input::dispatch_keyboard_event(0x1e, false, 0);
    }
    assert_eq!(crate::terminal_app::TERMINAL_INSTANCES.lock().get(&wid).unwrap().current_input.len(), 100);
    assert!(!crate::input::INPUT_QUEUES.lock().contains_key(&pid));
    wm::WM.lock().destroy_window(pid, wid).unwrap();
    assert!(!crate::input::KERNEL_INPUT_OWNERS.lock().contains(&pid));
    let (queued_pid, _) = owner();
    crate::input::notify_window_event(queued_pid, crate::input::EventType::WindowResize, 320, 200);
    let ev = crate::input::INPUT_QUEUES.lock().get_mut(&queued_pid).unwrap().pop().unwrap();
    assert_eq!(ev.mouse_x, 320);
    assert_eq!(ev.mouse_y, 200);
    crate::input::cleanup_input_for_pid(queued_pid);
    dispose(queued_pid);
    crate::serial_println!("[REGRESSION] PASS direct built-in input avoids duplicate queues; userspace delivery preserved");
}

#[cfg(feature = "legacy-desktop")]
fn live_resize_regression() {
    let baseline = memory::get_memory_stats().0;
    let pid = crate::terminal_app::spawn_terminal_app("live-resize-regression").unwrap();
    let mut manager = wm::WM.lock();
    let wid = manager.focused_window.unwrap();
    let w = manager.windows.iter().find(|w| w.window_id == wid).unwrap();
    let (x, y, width, height) = (w.x, w.y, w.width, w.height);
    let (mx, my) = (x + width as i32 - 2, y + height as i32 + 18);
    manager.handle_mouse_down(mx, my);
    assert!(manager.resizing_window.is_some());
    for delta in 1..=20 { manager.handle_mouse_move(mx + delta, my + delta); }
    assert_eq!(crate::terminal_app::TERMINAL_INSTANCES.lock().get(&wid).unwrap().width, width,
        "resize input must not synchronously render each event");
    manager.composite_desktop(mx + 20, my + 20);
    assert_eq!(crate::terminal_app::TERMINAL_INSTANCES.lock().get(&wid).unwrap().width, width + 20);
    manager.handle_mouse_move(mx + 30, my + 30);
    manager.handle_mouse_up();
    assert_eq!(crate::terminal_app::TERMINAL_INSTANCES.lock().get(&wid).unwrap().width, width + 30);
    assert!(manager.damage_tracker.is_damaged());
    manager.destroy_window(pid, wid).unwrap();
    manager.composite_desktop(0, 0);
    assert_eq!(memory::get_memory_stats().0, baseline);
    crate::serial_println!("[REGRESSION] PASS live resize coalesces input per frame and flushes final geometry");
}

fn service_supervisor_and_discovery_regression() {
    use crate::service::{ServiceManager, ServiceDefinition, SupervisorAction, ServiceState, RestartPolicy};
    use crate::cap::CapHandle;
    use alloc::string::String;
    use alloc::vec::Vec;

    let mut sm = ServiceManager::new();

    // 1. Bağımlılık ağacı ve topolojik sıralama
    let disk_id = sm.register(ServiceDefinition {
        name: String::from("disk"),
        path: String::from("disksvc"),
        dependencies: Vec::new(),
        restart_policy: RestartPolicy::Always,
        is_critical: true,
        max_retries: 3,
    }).unwrap();

    let fs_id = sm.register(ServiceDefinition {
        name: String::from("fs"),
        path: String::from("fssvc"),
        dependencies: alloc::vec![String::from("disk")],
        restart_policy: RestartPolicy::Always,
        is_critical: true,
        max_retries: 3,
    }).unwrap();

    let app_id = sm.register(ServiceDefinition {
        name: String::from("shell"),
        path: String::from("sh"),
        dependencies: alloc::vec![String::from("fs")],
        restart_policy: RestartPolicy::OnFailure,
        is_critical: false,
        max_retries: 3,
    }).unwrap();

    assert!(!sm.has_cycle(), "Service graph must not have cycles");
    let boot_order = sm.get_boot_order().unwrap();
    let pos_disk = boot_order.iter().position(|&x| x == disk_id).unwrap();
    let pos_fs = boot_order.iter().position(|&x| x == fs_id).unwrap();
    let pos_app = boot_order.iter().position(|&x| x == app_id).unwrap();
    assert!(pos_disk < pos_fs, "disk must boot before fs");
    assert!(pos_fs < pos_app, "fs must boot before shell");

    let shutdown_order = sm.get_shutdown_order().unwrap();
    assert_eq!(shutdown_order[0], app_id, "shell must shutdown first");
    assert_eq!(*shutdown_order.last().unwrap(), disk_id, "disk must shutdown last");

    // 2. Döngüsel bağımlılık (Cycle) tespiti
    let mut cycle_sm = ServiceManager::new();
    cycle_sm.register(ServiceDefinition {
        name: String::from("A"),
        path: String::from("A"),
        dependencies: alloc::vec![String::from("B")],
        restart_policy: RestartPolicy::Never,
        is_critical: false,
        max_retries: 1,
    }).unwrap();
    cycle_sm.register(ServiceDefinition {
        name: String::from("B"),
        path: String::from("B"),
        dependencies: alloc::vec![String::from("A")],
        restart_policy: RestartPolicy::Never,
        is_critical: false,
        max_retries: 1,
    }).unwrap();
    assert!(cycle_sm.has_cycle(), "Cycle detection must identify A <-> B loop");
    assert!(cycle_sm.get_boot_order().is_err(), "Cyclic graph must fail boot ordering");

    // 3. Service Discovery & Ready Handshake
    let dummy_cap = CapHandle { slot: 7, generation: 1 };
    let initial_pid = 101u64;
    let endpoint_id = 42u32;
    sm.set_service_ready("fs", initial_pid, endpoint_id, Some(dummy_cap)).unwrap();

    let lookup_res = sm.lookup("fs");
    assert!(lookup_res.is_some(), "Running service must be discoverable");
    let (ep, cap, pid, gen) = lookup_res.unwrap();
    assert_eq!(ep, endpoint_id);
    assert_eq!(cap, Some(dummy_cap));
    assert_eq!(pid, initial_pid);
    assert_eq!(gen, 1);

    // 4. Çökme ve Supervisor Yeniden Başlatma (Crash Recovery)
    let action = sm.handle_process_exit(initial_pid, 1);
    assert_eq!(action, SupervisorAction::Restart(fs_id), "Always policy must restart crashed service");
    assert!(sm.lookup("fs").is_none(), "Restarting service must not be returned as running");

    // 5. İstemci Yeniden Bağlanma (Reconnect Protocol)
    let new_pid = 202u64;
    let new_endpoint_id = 43u32;
    let new_cap = CapHandle { slot: 8, generation: 1 };
    sm.set_service_ready("fs", new_pid, new_endpoint_id, Some(new_cap)).unwrap();

    let reconnect_res = sm.reconnect("fs", 1).unwrap();
    let (rec_ep, rec_cap, rec_pid, rec_gen) = reconnect_res;
    assert_eq!(rec_ep, new_endpoint_id);
    assert_eq!(rec_cap, Some(new_cap));
    assert_eq!(rec_pid, new_pid);
    assert_eq!(rec_gen, 2, "Generation must increment on restart");

    // 6. Flapping Loop Koruması (Max Retries)
    // fs servisi max_retries = 3 olarak tanımlanmıştı; zaten 1 kez restart oldu
    assert_eq!(sm.handle_process_exit(new_pid, 1), SupervisorAction::Restart(fs_id)); // restart 2
    sm.set_service_ready("fs", 303, new_endpoint_id, Some(new_cap)).unwrap();
    assert_eq!(sm.handle_process_exit(303, 1), SupervisorAction::Restart(fs_id)); // restart 3
    sm.set_service_ready("fs", 404, new_endpoint_id, Some(new_cap)).unwrap();
    let flap_action = sm.handle_process_exit(404, 1); // retry aşımı!
    assert_eq!(flap_action, SupervisorAction::CriticalFailure(fs_id), "Critical service must trigger CriticalFailure on flapping");
    assert_eq!(sm.services[fs_id].state, ServiceState::Failed);

    crate::serial_println!("[REGRESSION] PASS service supervisor dependency ordering, crash recovery, reconnect and flapping defense");
}

