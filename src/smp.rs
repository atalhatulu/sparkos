use crate::acpi::get_madt;
use alloc::vec::Vec;
use spin::Mutex;

#[derive(Debug, Clone, Copy)]
pub struct CpuInfo {
    pub apic_id: u8,
    pub is_bsp: bool,
    pub local_apic_ptr: u64,
}

pub struct PerCpu<T> {
    data: Mutex<Vec<Option<T>>>,
}

impl<T> PerCpu<T> {
    pub const fn new() -> Self {
        Self {
            data: Mutex::new(Vec::new()),
        }
    }
    
    pub fn init_for_cpu(&self, apic_id: u8, value: T) {
        let mut d = self.data.lock();
        let idx = apic_id as usize;
        if idx >= d.len() {
            d.resize_with(idx + 1, || None);
        }
        d[idx] = Some(value);
    }
}

pub struct LocalApic {
    base_addr: u64,
}

impl LocalApic {
    pub fn new(base_addr: u64) -> Self {
        Self { base_addr }
    }
    
    unsafe fn read_reg(&self, offset: u64) -> u32 {
        let ptr = (self.base_addr + offset) as *const u32;
        core::ptr::read_volatile(ptr)
    }
    
    unsafe fn write_reg(&mut self, offset: u64, value: u32) {
        let ptr = (self.base_addr + offset) as *mut u32;
        core::ptr::write_volatile(ptr, value);
    }
    
    pub fn read_apic_id(&self) -> u8 {
        unsafe { (self.read_reg(0x20) >> 24) as u8 }
    }
    
    pub fn enable_lapic(&mut self) {
        unsafe {
            let mut sivr = self.read_reg(0xF0);
            sivr |= 0x100; // APIC software enable
            self.write_reg(0xF0, sivr);
        }
    }
    
    pub fn write_icr(&mut self, high: u32, low: u32) {
        unsafe {
            self.write_reg(0x310, high);
            self.write_reg(0x300, low);
        }
    }
}

pub fn start_ap(apic_id: u8, trampoline_addr: u32) {
    crate::serial_println!("SMP: Starting AP {} at {:#x}", apic_id, trampoline_addr);
    // TODO: Send INIT IPI
    // TODO: Send STARTUP IPI
    // TODO: Wait for AP to boot
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
    crate::serial_println!("SMP: Local APIC initialized at {:#x}, BSP APIC ID: {}", lapic_base_virt, bsp_apic_id);

    let mut cpu_count = 0;
    
    madt.iterate_entries(|entry_type, _len, data| {
        if entry_type == 0 { // Local APIC
            if data.len() >= 2 {
                let apic_id = data[1];
                let flags = unsafe { core::ptr::read_unaligned(data.as_ptr().add(2) as *const u32) };
                
                if (flags & 1) != 0 {
                    let is_bsp = apic_id == bsp_apic_id;
                    crate::serial_println!("SMP: Found CPU APIC ID {} (BSP: {})", apic_id, is_bsp);
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
}
