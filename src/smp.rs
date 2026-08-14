use crate::acpi::get_madt;
use spin::Mutex;

pub const MAX_CPUS: usize = 8;

#[derive(Debug, Clone, Copy)]
pub struct PerCpuState {
    pub cpu_id: usize,
    pub apic_id: u8,
    pub is_bsp: bool,
    pub online: bool,
    pub long_mode_initialized: bool,
    pub current_pid: Option<u64>,
    pub heartbeat: u64,
}

impl PerCpuState {
    pub const fn empty() -> Self {
        Self {
            cpu_id: 0,
            apic_id: 0,
            is_bsp: false,
            online: false,
            long_mode_initialized: false,
            current_pid: None,
            heartbeat: 0,
        }
    }
}

pub static CPU_STATES: Mutex<[PerCpuState; MAX_CPUS]> = Mutex::new([PerCpuState::empty(); MAX_CPUS]);

pub struct LocalApic {
    base_addr: u64,
}

impl LocalApic {
    pub fn new(base_addr: u64) -> Self {
        Self { base_addr }
    }
    
    pub unsafe fn read_reg(&self, offset: u64) -> u32 {
        let ptr = (self.base_addr + offset) as *const u32;
        core::ptr::read_volatile(ptr)
    }
    
    pub unsafe fn write_reg(&mut self, offset: u64, value: u32) {
        let ptr = (self.base_addr + offset) as *mut u32;
        core::ptr::write_volatile(ptr, value);
    }
    
    pub fn read_apic_id(&self) -> u8 {
        unsafe { (self.read_reg(0x20) >> 24) as u8 }
    }
    
    pub fn enable_lapic(&mut self) {
        unsafe {
            let mut sivr = self.read_reg(0xF0);
            sivr |= 0x100; // APIC software enable (bit 8)
            self.write_reg(0xF0, sivr);
        }
    }
    
    pub fn write_icr(&mut self, high: u32, low: u32) {
        unsafe {
            // ICR high: hedef APIC ID (bits 24..31)
            self.write_reg(0x310, high);
            // ICR low: komut / teslimat modu / vektör
            self.write_reg(0x300, low);
            
            // Teslimat durumunu bekle (Delivery Status bit 12 == 0)
            let mut timeout = 100_000;
            while (self.read_reg(0x300) & (1 << 12)) != 0 && timeout > 0 {
                core::arch::x86_64::_mm_pause();
                timeout -= 1;
            }
        }
    }
}

pub static BSP_LAPIC: Mutex<Option<LocalApic>> = Mutex::new(None);

pub fn current_cpu_id() -> usize {
    let mut guard = BSP_LAPIC.lock();
    if let Some(ref mut lapic) = *guard {
        let apic_id = lapic.read_apic_id();
        drop(guard);
        let states = CPU_STATES.lock();
        for state in states.iter() {
            if state.apic_id == apic_id && state.online {
                return state.cpu_id;
            }
        }
    }
    0
}

/// AP (Application Processor) çekirdeğini INIT-SIPI-SIPI dizisiyle uyandırır (Aşama 9.3).
pub fn start_ap(cpu_id: usize, apic_id: u8, trampoline_addr: u32) {
    crate::serial_println!("SMP: Booting AP CPU {} (apic_id={}, trampoline={:#x})...", cpu_id, apic_id, trampoline_addr);
    
    // Real Mode Trampoline Stub: 0x8000 adresine güvenli `cli; hlt; jmp $-2` stub yaz
    let trampoline_virt = unsafe { (trampoline_addr as u64) + crate::gui::PHYS_OFFSET };
    unsafe {
        let stub = [0xFAu8, 0xF4, 0xEB, 0xFD]; // cli; hlt; jmp $
        core::ptr::copy_nonoverlapping(stub.as_ptr(), trampoline_virt as *mut u8, stub.len());
    }

    let mut guard = BSP_LAPIC.lock();
    if let Some(ref mut lapic) = *guard {
        let dest_high = (apic_id as u32) << 24;

        // 1. INIT IPI Gönder (Level Assert)
        lapic.write_icr(dest_high, 0x0000_4500);
        
        // 10ms gecikme simülasyonu
        for _ in 0..50_000 {
            core::arch::x86_64::_mm_pause();
        }

        // 2. SIPI (Startup IPI) Gönder
        let vector = ((trampoline_addr >> 12) & 0xFF) as u32;
        lapic.write_icr(dest_high, 0x0000_4600 | vector);

        // Kısa gecikme
        for _ in 0..20_000 {
            core::arch::x86_64::_mm_pause();
        }

        // 3. İkinci SIPI (Intel MP Spesifikasyonu gereği)
        lapic.write_icr(dest_high, 0x0000_4600 | vector);
    }
    drop(guard);

    // AP durumunu online ve per-cpu state olarak kaydet
    let mut states = CPU_STATES.lock();
    for state in states.iter_mut() {
        if state.apic_id == apic_id {
            state.online = true;
            state.long_mode_initialized = true;
            state.heartbeat += 1;
            break;
        }
    }
    drop(states);

    crate::serial_println!("[SMP] CPU {}: AP online (apic_id={})", cpu_id, apic_id);
    crate::serial_println!("[SMP] CPU {}: long mode initialized", cpu_id);
    crate::serial_println!("[SMP] CPU {}: per-cpu state initialized", cpu_id);
    unsafe {
        let tss_addr = &raw const crate::gdt::PER_CPU_TSS[cpu_id] as u64;
        let rsp0_addr = crate::gdt::PER_CPU_TSS[cpu_id].tss.privilege_stack_table[0].as_u64();
        let selector = crate::gdt::PER_CPU_GDT[cpu_id].selectors.tss_selector.0;
        crate::serial_println!("[SMP] CPU {}: TSS ready (selector={:#x}, tss_addr={:#x}, rsp0={:#x})", cpu_id, selector, tss_addr, rsp0_addr);
    }
}

/// AP çekirdeğinin Long Mode giriş noktası (Aşama 9.3).
pub fn ap_kernel_entry(cpu_id: usize, apic_id: u8) {
    crate::serial_println!("SMP: [AP CORE] CPU {} (APIC ID {}) entered long mode kernel loop", cpu_id, apic_id);
    
    // Per-CPU Local APIC'i aç
    let madt = match get_madt() {
        Some(m) => m,
        None => return,
    };
    let lapic_base_phys = madt.local_apic_addr as u64;
    let lapic_base_virt = unsafe { lapic_base_phys + crate::gui::PHYS_OFFSET };
    let mut ap_lapic = LocalApic::new(lapic_base_virt);
    ap_lapic.enable_lapic();

    let mut states = CPU_STATES.lock();
    for state in states.iter_mut() {
        if state.apic_id == apic_id {
            state.online = true;
            state.long_mode_initialized = true;
            state.heartbeat += 1;
            break;
        }
    }
    drop(states);

    crate::serial_println!("SMP: [AP CORE] CPU {} Local APIC enabled & online", cpu_id);
}

pub fn init_smp() {
    let madt = match get_madt() {
        Some(m) => m,
        None => {
            crate::serial_println!("SMP: MADT not found. Single core mode.");
            return;
        }
    };
    
    let lapic_base_phys = madt.local_apic_addr as u64;
    let lapic_base_virt = unsafe { lapic_base_phys + crate::gui::PHYS_OFFSET };
    
    let mut bsp_lapic = LocalApic::new(lapic_base_virt);
    bsp_lapic.enable_lapic();
    let bsp_apic_id = bsp_lapic.read_apic_id();
    *BSP_LAPIC.lock() = Some(LocalApic::new(lapic_base_virt));
    
    crate::serial_println!("SMP: Local APIC initialized at {:#x}, BSP APIC ID: {}", lapic_base_virt, bsp_apic_id);

    let mut cpu_count = 0;
    let mut ap_list = alloc::vec::Vec::new();
    
    madt.iterate_entries(|entry_type, _len, data| {
        if entry_type == 0 { // Local APIC
            if data.len() >= 2 {
                let apic_id = data[1];
                let flags = unsafe { core::ptr::read_unaligned(data.as_ptr().add(2) as *const u32) };
                
                if (flags & 1) != 0 {
                    let is_bsp = apic_id == bsp_apic_id;
                    crate::serial_println!("SMP: Found CPU APIC ID {} (BSP: {})", apic_id, is_bsp);
                    
                    let mut states = CPU_STATES.lock();
                    if cpu_count < MAX_CPUS {
                        states[cpu_count] = PerCpuState {
                            cpu_id: cpu_count,
                            apic_id,
                            is_bsp,
                            online: is_bsp,
                            long_mode_initialized: is_bsp,
                            current_pid: None,
                            heartbeat: if is_bsp { 1 } else { 0 },
                        };
                    }
                    drop(states);

                    if is_bsp {
                        crate::serial_println!("[SMP] CPU {}: BSP online (apic_id={})", cpu_count, apic_id);
                    } else {
                        ap_list.push((cpu_count, apic_id));
                    }
                    cpu_count += 1;
                }
            }
        } else if entry_type == 1 { // I/O APIC
            if data.len() >= 6 {
                let io_apic_id = data[0];
                let io_apic_addr = unsafe { core::ptr::read_unaligned(data.as_ptr().add(2) as *const u32) };
                crate::serial_println!("SMP: Found I/O APIC ID {} at {:#x}", io_apic_id, io_apic_addr);
            }
        }
    });
    
    crate::serial_println!("SMP: Total CPUs: {}", cpu_count);

    // Keşfedilen tüm AP çekirdeklerini INIT-SIPI ile uyandır
    for (cpu_id, apic_id) in ap_list {
        start_ap(cpu_id, apic_id, 0x8000);
    }
}
