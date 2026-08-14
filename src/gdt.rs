use x86_64::VirtAddr;
use x86_64::structures::tss::TaskStateSegment;
use x86_64::structures::gdt::{GlobalDescriptorTable, Descriptor, SegmentSelector};
use spin::Lazy;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

pub const KERNEL_STACK_SIZE: usize = 4096 * 5;
pub static mut KERNEL_STACK: [u8; KERNEL_STACK_SIZE] = [0; KERNEL_STACK_SIZE];

static mut DF_STACK: [u8; 4096 * 5] = [0; 4096 * 5];

#[repr(C, packed)]
pub struct TssWithIopb {
    pub tss: TaskStateSegment,
    pub io_bitmap: [u8; 8192],
    pub trailing_byte: u8,
}

pub static mut TSS_DATA: TssWithIopb = TssWithIopb {
    tss: TaskStateSegment::new(),
    io_bitmap: [0xFF; 8192], // Varsayılan: Tüm portlar Ring 3 için KAPALI (#GP)
    trailing_byte: 0xFF,      // x86_64 donanım gereksinimi: Son bayt her zaman 0xFF
};

pub fn set_tss_rsp0(kstack_top: u64) {
    unsafe {
        TSS_DATA.tss.privilege_stack_table[0] = VirtAddr::new(kstack_top);
    }
}

pub fn allow_port_range(start: u16, end_inclusive: u16) {
    unsafe {
        for port in start..=end_inclusive {
            let byte_idx = (port / 8) as usize;
            let bit_idx = (port % 8) as u8;
            if byte_idx < 8192 {
                TSS_DATA.io_bitmap[byte_idx] &= !(1 << bit_idx);
            }
        }
    }
}

pub fn deny_port_range(start: u16, end_inclusive: u16) {
    unsafe {
        for port in start..=end_inclusive {
            let byte_idx = (port / 8) as usize;
            let bit_idx = (port % 8) as u8;
            if byte_idx < 8192 {
                TSS_DATA.io_bitmap[byte_idx] |= 1 << bit_idx;
            }
        }
    }
}

pub fn reset_io_bitmap() {
    unsafe {
        TSS_DATA.io_bitmap = [0xFF; 8192];
    }
}

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

pub static GDT: Lazy<(GlobalDescriptorTable, Selectors)> = Lazy::new(|| {
    let mut gdt = GlobalDescriptorTable::new();
    
    let code_selector = gdt.append(Descriptor::kernel_code_segment());
    let data_selector = gdt.append(Descriptor::kernel_data_segment());
    
    let user_data_selector = gdt.append(Descriptor::user_data_segment());
    let user_code_selector = gdt.append(Descriptor::user_code_segment());
    
    // TSS IST ve Priv Stack yapılandırması
    unsafe {
        let stack_start = VirtAddr::from_ptr(&raw const DF_STACK);
        TSS_DATA.tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = stack_start + (4096 * 5) as u64;

        let kstack_start = VirtAddr::from_ptr(&raw const KERNEL_STACK);
        TSS_DATA.tss.privilege_stack_table[0] = kstack_start + KERNEL_STACK_SIZE as u64;

        // iomap_base: TSS başlangıcından io_bitmap alanına olan bayt uzaklığı (104)
        TSS_DATA.tss.iomap_base = 104;
    }

    let (tss_low, tss_high) = create_tss_descriptor(unsafe { &raw const TSS_DATA });
    let tss_selector = gdt.append(Descriptor::SystemSegment(tss_low, tss_high));
    
    (gdt, Selectors {
        code_selector,
        data_selector,
        user_code_selector,
        user_data_selector,
        tss_selector,
    })
});

pub fn init() {
    use x86_64::instructions::tables::load_tss;
    use x86_64::instructions::segmentation::{Segment, CS, DS, ES, FS, GS, SS};
    
    GDT.0.load();
    unsafe {
        CS::set_reg(GDT.1.code_selector);
        DS::set_reg(GDT.1.data_selector);
        ES::set_reg(GDT.1.data_selector);
        FS::set_reg(GDT.1.data_selector);
        GS::set_reg(GDT.1.data_selector);
        SS::set_reg(GDT.1.data_selector);
        load_tss(GDT.1.tss_selector);
    }
}
