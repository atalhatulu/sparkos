use core::mem::size_of;

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
