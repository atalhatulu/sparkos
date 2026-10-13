use x86_64::VirtAddr;
use x86_64::structures::tss::TaskStateSegment;
use x86_64::structures::gdt::{GlobalDescriptorTable, Descriptor, SegmentSelector};
use spin::Lazy;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;
pub const MAX_CPUS: usize = 8;
pub const KERNEL_STACK_SIZE: usize = 4096 * 5;

#[repr(C, align(16))]
pub struct TssWithIopb {
    pub tss: TaskStateSegment,
    pub io_bitmap: [u8; 8192],
    pub trailing_byte: u8,
}

impl TssWithIopb {
    pub const fn new() -> Self {
        Self {
            tss: TaskStateSegment::new(),
            io_bitmap: [0xFF; 8192], // Varsayılan: Tüm portlar Ring 3 için KAPALI (#GP)
            trailing_byte: 0xFF,      // x86_64 donanım gereksinimi: Son bayt her zaman 0xFF
        }
    }
}

pub static mut PER_CPU_TSS: [TssWithIopb; MAX_CPUS] = [
    TssWithIopb::new(),
    TssWithIopb::new(),
    TssWithIopb::new(),
    TssWithIopb::new(),
    TssWithIopb::new(),
    TssWithIopb::new(),
    TssWithIopb::new(),
    TssWithIopb::new(),
];

pub static mut PER_CPU_KERNEL_STACKS: [[u8; KERNEL_STACK_SIZE]; MAX_CPUS] = [[0; KERNEL_STACK_SIZE]; MAX_CPUS];
static mut PER_CPU_DF_STACKS: [[u8; 4096 * 5]; MAX_CPUS] = [[0; 4096 * 5]; MAX_CPUS];

pub fn set_tss_rsp0_for_cpu(cpu_id: usize, kstack_top: u64) {
    if cpu_id < MAX_CPUS {
        unsafe {
            PER_CPU_TSS[cpu_id].tss.privilege_stack_table[0] = VirtAddr::new(kstack_top);
        }
    }
}

pub fn set_tss_rsp0(kstack_top: u64) {
    set_tss_rsp0_for_cpu(0, kstack_top);
}

pub fn allow_port_range_for_cpu(cpu_id: usize, start: u16, end_inclusive: u16) {
    if cpu_id < MAX_CPUS {
        unsafe {
            for port in start..=end_inclusive {
                let byte_idx = (port / 8) as usize;
                let bit_idx = (port % 8) as u8;
                if byte_idx < 8192 {
                    PER_CPU_TSS[cpu_id].io_bitmap[byte_idx] &= !(1 << bit_idx);
                }
            }
        }
    }
}

pub fn allow_port_range(start: u16, end_inclusive: u16) {
    allow_port_range_for_cpu(0, start, end_inclusive);
}

pub fn deny_port_range_for_cpu(cpu_id: usize, start: u16, end_inclusive: u16) {
    if cpu_id < MAX_CPUS {
        unsafe {
            for port in start..=end_inclusive {
                let byte_idx = (port / 8) as usize;
                let bit_idx = (port % 8) as u8;
                if byte_idx < 8192 {
                    PER_CPU_TSS[cpu_id].io_bitmap[byte_idx] |= 1 << bit_idx;
                }
            }
        }
    }
}

pub fn deny_port_range(start: u16, end_inclusive: u16) {
    deny_port_range_for_cpu(0, start, end_inclusive);
}

pub fn reset_io_bitmap_for_cpu(cpu_id: usize) {
    if cpu_id < MAX_CPUS {
        unsafe {
            PER_CPU_TSS[cpu_id].io_bitmap = [0xFF; 8192];
        }
    }
}

pub fn reset_io_bitmap() {
    reset_io_bitmap_for_cpu(0);
}

#[derive(Clone, Copy)]
pub struct Selectors {
    pub code_selector: SegmentSelector,
    pub data_selector: SegmentSelector,
    pub user_code_selector: SegmentSelector,
    pub user_data_selector: SegmentSelector,
    pub tss_selector: SegmentSelector,
}

fn create_tss_descriptor(ptr: *const TssWithIopb) -> (u64, u64) {
    let limit = (core::mem::size_of::<TssWithIopb>() - 1) as u64;
    let base = ptr as u64;

    let limit_low = limit & 0xFFFF;
    let limit_high = (limit >> 16) & 0x0F;

    let base_low = base & 0xFFFF;
    let base_mid = (base >> 16) & 0xFF;
    let base_high_mid = (base >> 24) & 0xFF;
    let base_high = base >> 32;

    // Type = 0x9 (64-bit Available TSS), Present = 1, DPL = 0
    let flags: u64 = 0b1000_1001; // P=1, DPL=00, Type=1001

    let low = limit_low
        | (base_low << 16)
        | (base_mid << 32)
        | (flags << 40)
        | (limit_high << 48)
        | (base_high_mid << 56);

    let high = base_high;
    (low, high)
}

pub struct PerCpuGdtData {
    pub gdt: GlobalDescriptorTable,
    pub selectors: Selectors,
}

pub static PER_CPU_GDT: Lazy<[PerCpuGdtData; MAX_CPUS]> = Lazy::new(|| {
    let mut gdts = alloc::vec::Vec::with_capacity(MAX_CPUS);

    for i in 0..MAX_CPUS {
        let mut gdt = GlobalDescriptorTable::new();
        
        let code_selector = gdt.append(Descriptor::kernel_code_segment());
        let data_selector = gdt.append(Descriptor::kernel_data_segment());
        
        let user_data_selector = gdt.append(Descriptor::user_data_segment());
        let user_code_selector = gdt.append(Descriptor::user_code_segment());

        unsafe {
            let stack_start = VirtAddr::from_ptr(PER_CPU_DF_STACKS[i].as_ptr());
            PER_CPU_TSS[i].tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = stack_start + (4096 * 5) as u64;

            let kstack_start = VirtAddr::from_ptr(PER_CPU_KERNEL_STACKS[i].as_ptr());
            PER_CPU_TSS[i].tss.privilege_stack_table[0] = kstack_start + KERNEL_STACK_SIZE as u64;

            // iomap_base: TSS başlangıcından io_bitmap alanına olan bayt uzaklığı (104)
            PER_CPU_TSS[i].tss.iomap_base = 104;

            let (tss_low, tss_high) = create_tss_descriptor(&raw const PER_CPU_TSS[i]);
            let tss_selector = gdt.append(Descriptor::SystemSegment(tss_low, tss_high));

            gdts.push(PerCpuGdtData {
                gdt,
                selectors: Selectors {
                    code_selector,
                    data_selector,
                    user_code_selector,
                    user_data_selector,
                    tss_selector,
                },
            });
        }
    }

    match gdts.try_into() {
        Ok(arr) => arr,
        Err(_) => panic!("failed to convert GDT vec to array"),
    }
});

pub static GDT: Lazy<(&'static GlobalDescriptorTable, Selectors)> = Lazy::new(|| {
    (&PER_CPU_GDT[0].gdt, PER_CPU_GDT[0].selectors)
});

pub fn init_cpu_gdt_tss(cpu_id: usize) {
    use x86_64::instructions::tables::load_tss;
    use x86_64::instructions::segmentation::{Segment, DS, ES, FS, GS, SS};
    
    if cpu_id >= MAX_CPUS {
        return;
    }

    let gdt_data = &PER_CPU_GDT[cpu_id];
    gdt_data.gdt.load();

    unsafe {
        DS::set_reg(gdt_data.selectors.data_selector);
        ES::set_reg(gdt_data.selectors.data_selector);
        FS::set_reg(gdt_data.selectors.data_selector);
        GS::set_reg(gdt_data.selectors.data_selector);
        SS::set_reg(gdt_data.selectors.data_selector);

        load_tss(gdt_data.selectors.tss_selector);

        if cpu_id == 0 {
            let tss_addr = &raw const PER_CPU_TSS[cpu_id] as u64;
            let rsp0_addr = PER_CPU_TSS[cpu_id].tss.privilege_stack_table[0].as_u64();
            crate::serial_println!("[SMP] CPU {}: TSS loaded (selector={:#x}, tss_addr={:#x}, rsp0={:#x})", cpu_id, gdt_data.selectors.tss_selector.0, tss_addr, rsp0_addr);
        }
    }
}

pub fn init() {
    init_cpu_gdt_tss(0);
}
