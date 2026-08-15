use core::mem::size_of;
use alloc::vec::Vec;

fn phys_to_virt(phys: u64) -> u64 {
    unsafe { phys + crate::gui::PHYS_OFFSET }
}

#[repr(C, packed)]
pub struct Rsdp {
    pub signature: [u8; 8],
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub revision: u8,
    pub rsdt_address: u32,
    pub length: u32,
    pub xsdt_address: u64,
    pub extended_checksum: u8,
    pub reserved: [u8; 3],
}

impl Rsdp {
    pub fn find() -> Option<&'static Rsdp> {
        let start = phys_to_virt(0x000E0000) as *const u8;
        let end = phys_to_virt(0x000FFFFF) as *const u8;
        
        let mut ptr = start;
        while (ptr as u64) < (end as u64) {
            unsafe {
                if core::slice::from_raw_parts(ptr, 8) == b"RSD PTR " {
                    let rsdp = &*(ptr as *const Rsdp);
                    if rsdp.verify_checksum() {
                        return Some(rsdp);
                    }
                }
                ptr = ptr.add(16);
            }
        }
        None
    }
    
    fn verify_checksum(&self) -> bool {
        let ptr = self as *const _ as *const u8;
        let mut sum: u8 = 0;
        for i in 0..20 {
            sum = sum.wrapping_add(unsafe { *ptr.add(i) });
        }
        if sum != 0 {
            return false;
        }
        
        if self.revision >= 2 {
            let mut extended_sum: u8 = 0;
            for i in 0..size_of::<Rsdp>() {
                extended_sum = extended_sum.wrapping_add(unsafe { *ptr.add(i) });
            }
            if extended_sum != 0 {
                return false;
            }
        }
        true
    }
}

#[repr(C, packed)]
pub struct AcpiHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: u32,
    pub creator_revision: u32,
}

impl AcpiHeader {
    pub fn verify_checksum(&self) -> bool {
        let ptr = self as *const _ as *const u8;
        let mut sum: u8 = 0;
        for i in 0..self.length as usize {
            sum = sum.wrapping_add(unsafe { *ptr.add(i) });
        }
        sum == 0
    }
}

pub struct Madt {
    pub header: &'static AcpiHeader,
    pub local_apic_addr: u32,
    pub flags: u32,
}

impl Madt {
    pub fn parse(header: &'static AcpiHeader) -> Self {
        let ptr = header as *const _ as *const u8;
        let local_apic_addr = unsafe { core::ptr::read_unaligned(ptr.add(size_of::<AcpiHeader>()) as *const u32) };
        let flags = unsafe { core::ptr::read_unaligned(ptr.add(size_of::<AcpiHeader>() + 4) as *const u32) };
        
        Self {
            header,
            local_apic_addr,
            flags,
        }
    }
    
    pub fn iterate_entries<F>(&self, mut f: F)
    where
        F: FnMut(u8, u8, &[u8]),
    {
        let ptr = self.header as *const _ as *const u8;
        let mut offset = size_of::<AcpiHeader>() + 8;
        let end = self.header.length as usize;
        
        while offset < end {
            let entry_type = unsafe { *ptr.add(offset) };
            let entry_len = unsafe { *ptr.add(offset + 1) };
            if entry_len < 2 {
                break;
            }
            let data = unsafe { core::slice::from_raw_parts(ptr.add(offset + 2), (entry_len - 2) as usize) };
            f(entry_type, entry_len, data);
            offset += entry_len as usize;
        }
    }
}

pub struct Fadt {
    pub header: &'static AcpiHeader,
}

impl Fadt {
    pub fn parse(header: &'static AcpiHeader) -> Self {
        Self { header }
    }
}

pub fn get_madt() -> Option<Madt> {
    let rsdp = Rsdp::find()?;
    
    let rsdt_phys = rsdp.rsdt_address as u64;
    if rsdt_phys == 0 {
        return None;
    }
    
    let rsdt = unsafe { &*(phys_to_virt(rsdt_phys) as *const AcpiHeader) };
    if !rsdt.verify_checksum() {
        return None;
    }
    
    let header_size = size_of::<AcpiHeader>();
    let entries_count = (rsdt.length as usize).saturating_sub(header_size) / 4;
    let ptr = rsdt as *const _ as *const u8;
    let entries_start = unsafe { ptr.add(header_size) };
    
    for i in 0..entries_count {
        let entry_phys = unsafe { core::ptr::read_unaligned(entries_start.add(i * 4) as *const u32) };
        let table = unsafe { &*(phys_to_virt(entry_phys as u64) as *const AcpiHeader) };
        if table.verify_checksum() && &table.signature == b"APIC" {
            return Some(Madt::parse(table));
        }
    }
    
    None
}

pub fn get_fadt() -> Option<Fadt> {
    let rsdp = Rsdp::find()?;
    let rsdt_phys = rsdp.rsdt_address as u64;
    if rsdt_phys == 0 {
        return None;
    }
    let rsdt = unsafe { &*(phys_to_virt(rsdt_phys) as *const AcpiHeader) };
    if !rsdt.verify_checksum() {
        return None;
    }
    
    let header_size = size_of::<AcpiHeader>();
    let entries_count = (rsdt.length as usize).saturating_sub(header_size) / 4;
    let ptr = rsdt as *const _ as *const u8;
    let entries_start = unsafe { ptr.add(header_size) };
    
    for i in 0..entries_count {
        let entry_phys = unsafe { core::ptr::read_unaligned(entries_start.add(i * 4) as *const u32) };
        let table = unsafe { &*(phys_to_virt(entry_phys as u64) as *const AcpiHeader) };
        if table.verify_checksum() && &table.signature == b"FACP" {
            return Some(Fadt::parse(table));
        }
    }
    None
}

// -----------------------------------------------------------------------------
// Faz 29: ACPI DMAR (DMA Remapping Reporting) & Intel VT-d IOMMU Structures
// -----------------------------------------------------------------------------

#[repr(C, packed)]
pub struct DmarHeader {
    pub header: AcpiHeader,      // imza b"DMAR"
    pub host_addr_width: u8,     // fiziksel adres genişliği - 1
    pub flags: u8,               // bit 0: INTR_REMAP, bit 1: X2APIC_OPT_OUT
    pub reserved: [u8; 10],
}

const _: () = assert!(core::mem::size_of::<DmarHeader>() == 48);

#[repr(C, packed)]
pub struct DrhdHeader {
    pub struct_type: u16,        // 0x0 = DRHD
    pub length: u16,
    pub flags: u8,               // bit 0: INCLUDE_PCI_ALL
    pub reserved: u8,
    pub segment_number: u16,
    pub register_base_addr: u64, // IOMMU MMIO taban adresi
}

const _: () = assert!(core::mem::size_of::<DrhdHeader>() == 16);

#[derive(Debug, Clone)]
pub struct DmarTable {
    pub host_addr_width: u8,
    pub flags: u8,
    pub drhd_units: Vec<DrhdInfo>,
}

#[derive(Debug, Clone)]
pub struct DrhdInfo {
    pub register_base_addr: u64,
    pub segment_number: u16,
    pub include_all_devices: bool,  // flags bit 0 (INCLUDE_PCI_ALL)
    pub scoped_devices: Vec<(u8, u8, u8)>, // (bus, device, function)
}

/// ACPI DMAR tablosunu arar, doğrular ve DRHD birimlerini parse eder.
pub fn get_dmar() -> Option<DmarTable> {
    let rsdp = Rsdp::find()?;

    // 1. XSDT (64-bit işaretçiler, ACPI 2.0+) desteği
    if rsdp.revision >= 2 && rsdp.xsdt_address != 0 {
        let xsdt_phys = rsdp.xsdt_address;
        let xsdt = unsafe { &*(phys_to_virt(xsdt_phys) as *const AcpiHeader) };
        if xsdt.verify_checksum() {
            let header_size = size_of::<AcpiHeader>();
            let entries_count = (xsdt.length as usize).saturating_sub(header_size) / 8;
            let ptr = xsdt as *const _ as *const u8;
            let entries_start = unsafe { ptr.add(header_size) };
            for i in 0..entries_count {
                let entry_phys = unsafe { core::ptr::read_unaligned(entries_start.add(i * 8) as *const u64) };
                if entry_phys != 0 {
                    let table = unsafe { &*(phys_to_virt(entry_phys) as *const AcpiHeader) };
                    if table.verify_checksum() && &table.signature == b"DMAR" {
                        return parse_dmar_table(table);
                    }
                }
            }
        }
    }

    // 2. RSDT (32-bit işaretçiler, ACPI 1.0+) desteği
    let rsdt_phys = rsdp.rsdt_address as u64;
    if rsdt_phys != 0 {
        let rsdt = unsafe { &*(phys_to_virt(rsdt_phys) as *const AcpiHeader) };
        if rsdt.verify_checksum() {
            let header_size = size_of::<AcpiHeader>();
            let entries_count = (rsdt.length as usize).saturating_sub(header_size) / 4;
            let ptr = rsdt as *const _ as *const u8;
            let entries_start = unsafe { ptr.add(header_size) };
            for i in 0..entries_count {
                let entry_phys = unsafe { core::ptr::read_unaligned(entries_start.add(i * 4) as *const u32) };
                if entry_phys != 0 {
                    let table = unsafe { &*(phys_to_virt(entry_phys as u64) as *const AcpiHeader) };
                    if table.verify_checksum() && &table.signature == b"DMAR" {
                        return parse_dmar_table(table);
                    }
                }
            }
        }
    }

    None
}

fn parse_dmar_table(header: &'static AcpiHeader) -> Option<DmarTable> {
    if !header.verify_checksum() {
        crate::serial_println!("[DMAR] Checksum verification failed");
        return None;
    }

    let ptr = header as *const _ as *const u8;
    let dmar_header = unsafe { &*(ptr as *const DmarHeader) };
    let host_addr_width = dmar_header.host_addr_width;
    let flags = dmar_header.flags;

    let mut drhd_units = Vec::new();
    let total_len = header.length as usize;
    let mut offset = size_of::<DmarHeader>();

    while offset + 4 <= total_len {
        let struct_type = unsafe { core::ptr::read_unaligned(ptr.add(offset) as *const u16) };
        let struct_len = unsafe { core::ptr::read_unaligned(ptr.add(offset + 2) as *const u16) } as usize;

        if struct_len < 4 || offset + struct_len > total_len {
            crate::serial_println!("[DMAR] Malformed remapping structure at offset {}", offset);
            break;
        }

        if struct_type == 0x0 {
            // Type 0x0: DRHD (DMA Remapping Hardware Unit Definition)
            if struct_len >= size_of::<DrhdHeader>() {
                let raw_flags = unsafe { *ptr.add(offset + 4) };
                let raw_seg = unsafe { core::ptr::read_unaligned(ptr.add(offset + 6) as *const u16) };
                let raw_base = unsafe { core::ptr::read_unaligned(ptr.add(offset + 8) as *const u64) };
                let include_all_devices = (raw_flags & 1) != 0;

                let mut scoped_devices = Vec::new();
                let mut scope_offset = size_of::<DrhdHeader>();

                while scope_offset + 6 <= struct_len {
                    let scope_type = unsafe { *ptr.add(offset + scope_offset) };
                    let scope_len = unsafe { *ptr.add(offset + scope_offset + 1) } as usize;
                    if scope_len < 6 || scope_offset + scope_len > struct_len {
                        break;
                    }
                    let start_bus = unsafe { *ptr.add(offset + scope_offset + 5) };
                    let path_bytes = scope_len - 6;
                    let path_count = path_bytes / 2;

                    for p in 0..path_count {
                        let dev_byte = unsafe { *ptr.add(offset + scope_offset + 6 + p * 2) };
                        let func_byte = unsafe { *ptr.add(offset + scope_offset + 6 + p * 2 + 1) };
                        let dev = (dev_byte >> 3) & 0x1F;
                        let func = func_byte & 0x07;
                        scoped_devices.push((start_bus, dev, func));
                        crate::serial_println!("        [DMAR Scope] Type: 0x{:X}, Bus: {}, Dev: {}, Func: {}", scope_type, start_bus, dev, func);
                    }

                    scope_offset += scope_len;
                }

                crate::serial_println!(
                    "[DMAR] Found DRHD Unit at MMIO 0x{:08X}, Segment {}, RawFlags: 0x{:02X}, IncludeAll: {}, Scoped Devices: {}",
                    raw_base, raw_seg, raw_flags, include_all_devices, scoped_devices.len()
                );

                drhd_units.push(DrhdInfo {
                    register_base_addr: raw_base,
                    segment_number: raw_seg,
                    include_all_devices,
                    scoped_devices,
                });
            }
        } else {
            // Diğer tipler: 0x1 (RMRR), 0x2 (ATSR), 0x3 (RHSA), 0x4 (ANDD)
            crate::serial_println!(
                "[DMAR] Skipping non-DRHD remapping structure type 0x{:X} (length {} bytes)",
                struct_type, struct_len
            );
        }

        offset += struct_len;
    }

    Some(DmarTable {
        host_addr_width,
        flags,
        drhd_units,
    })
}
