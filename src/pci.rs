use x86_64::instructions::port::Port;
use alloc::vec::Vec;
use alloc::string::{String, ToString};
use alloc::format;

pub struct PciDevice {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
}

impl PciDevice {
    pub fn get_name(&self) -> String {
        // En bilinen donanimlarin ID'leri
        match (self.vendor_id, self.device_id) {
            (0x8086, 0x100E) => "Intel E1000 Gigabit Ethernet".to_string(),
            (0x10EC, 0x8139) => "Realtek RTL8139 Fast Ethernet".to_string(),
            (0x1234, 0x1111) => "QEMU Standard VGA".to_string(),
            (0x8086, 0x29C0) => "Intel Q35 Express Host Bridge".to_string(),
            (0x8086, 0x2918) => "Intel ICH9 LPC Interface".to_string(),
            (0x8086, 0x2922) => "Intel ICH9 SATA Controller".to_string(),
            _ => format!("Vendor: 0x{:04X}, Device: 0x{:04X}", self.vendor_id, self.device_id),
        }
    }
}

pub unsafe fn pci_read_u32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let address = 0x80000000u32 
                | ((bus as u32) << 16) 
                | ((slot as u32) << 11) 
                | ((func as u32) << 8) 
                | ((offset as u32) & 0xFC);
                
    let mut config_address: Port<u32> = Port::new(0xCF8);
    let mut config_data: Port<u32> = Port::new(0xCFC);
    
    config_address.write(address);
    config_data.read()
}

pub fn check_device(bus: u8, slot: u8, func: u8) -> Option<PciDevice> {
    let reg0 = unsafe { pci_read_u32(bus, slot, func, 0) };
    let vendor_id = (reg0 & 0xFFFF) as u16;
    
    if vendor_id == 0xFFFF {
        return None; // Aygit yok
    }
    
    let device_id = (reg0 >> 16) as u16;
    
    let reg2 = unsafe { pci_read_u32(bus, slot, func, 0x08) };
    let class = (reg2 >> 24) as u8;
    let subclass = (reg2 >> 16) as u8;

    Some(PciDevice {
        bus,
        slot,
        func,
        vendor_id,
        device_id,
        class,
        subclass,
    })
}

pub fn scan_pci() -> Vec<PciDevice> {
    let mut devices = Vec::new();
    // QEMU'da genellikle Bus 0 kullanilir, hizli tarama icin 0-5 arasina bakalim
    for bus in 0..=5 {
        for slot in 0..32 {
            // Fonksiyon 0 var mi kontrol et
            if let Some(dev) = check_device(bus, slot, 0) {
                devices.push(dev);
                // Multi-function cihaz mi? (Header Type'in 7. biti)
                let header_type = unsafe { pci_read_u32(bus, slot, 0, 0x0C) } >> 16;
                if (header_type & 0x80) != 0 {
                    for func in 1..8 {
                        if let Some(muti_dev) = check_device(bus, slot, func) {
                            devices.push(muti_dev);
                        }
                    }
                }
            }
        }
    }
    devices
}
