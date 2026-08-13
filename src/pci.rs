//! PCI Configuration Space driver.
//!
//! L7 genişletmesi: mevcut `lspci` taramasını korur, üzerine şunları ekler:
//!   - BAR (Base Address Register) okuma/yazma, mem/io ayrımı ve boyut hesabı
//!   - Bus mastering / DMA enable bitleri (Command register)
//!   - `PciDevice` yapısı (header type, prog_if, revision, BAR'lar, IRQ)
//!   - Genel `Driver` trait iskeleti + `probe_all` yardımcısı
//!   - MSI (Message Signaled Interrupt) isteğe bağlı etkinleştirme

use x86_64::instructions::port::Port;
use alloc::vec::Vec;
use alloc::string::{String, ToString};
use alloc::format;

// ---------------------------------------------------------------------------
// PCI config space register offset sabitleri
// ---------------------------------------------------------------------------
pub const PCI_VENDOR_ID: u8 = 0x00;
pub const PCI_COMMAND: u8 = 0x04;
pub const PCI_STATUS: u8 = 0x06;
pub const PCI_REVISION_ID: u8 = 0x08;
pub const PCI_PROG_IF: u8 = 0x09;
pub const PCI_SUBCLASS: u8 = 0x0A;
pub const PCI_CLASS: u8 = 0x0B;
pub const PCI_CACHE_LINE_SIZE: u8 = 0x0C;
pub const PCI_LATENCY_TIMER: u8 = 0x0D;
pub const PCI_HEADER_TYPE: u8 = 0x0E;
pub const PCI_BIST: u8 = 0x0F;
pub const PCI_BAR0: u8 = 0x10;
pub const PCI_BAR1: u8 = 0x14;
pub const PCI_BAR2: u8 = 0x18;
pub const PCI_BAR3: u8 = 0x1C;
pub const PCI_BAR4: u8 = 0x20;
pub const PCI_BAR5: u8 = 0x24;
pub const PCI_CAPABILITIES_POINTER: u8 = 0x34;
pub const PCI_INTERRUPT_LINE: u8 = 0x3C;
pub const PCI_INTERRUPT_PIN: u8 = 0x3D;

/// PCI Command register (offset 0x04) bitleri.
pub mod command {
    pub const IO_SPACE: u16 = 1 << 0;
    pub const MEMORY_SPACE: u16 = 1 << 1;
    pub const BUS_MASTER: u16 = 1 << 2;
    pub const SPECIAL_CYCLES: u16 = 1 << 3;
    pub const MEMORY_WRITE_INVALIDATE: u16 = 1 << 4;
    pub const VGA_PALETTE_SNOOP: u16 = 1 << 5;
    pub const PARITY_ERROR_RESPONSE: u16 = 1 << 6;
    pub const SERR_ENABLE: u16 = 1 << 8;
    pub const FAST_BACK_TO_BACK: u16 = 1 << 9;
    pub const INT_DISABLE: u16 = 1 << 10;
}

/// PCI Status register (offset 0x06) bitleri.
pub mod status {
    /// Capabilities List biti: aygıtın capability zinciri var mı?
    pub const CAPABILITIES_LIST: u16 = 1 << 4;
}

// ---------------------------------------------------------------------------
// BAR (Base Address Register)
// ---------------------------------------------------------------------------

/// Bir BAR'ın tipini ve adres genişliğini özetler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarType {
    /// Memory-mapped BAR (32 veya 64 bit).
    Memory { is_64bit: bool },
    /// I/O port BAR (16 bit adresleme pratikte).
    Io,
}

/// Çözümlenmiş bir BAR: taban adres, boyut ve öznitelikler.
#[derive(Clone, Copy, Debug)]
pub struct PciBar {
    /// BAR'ın indeksi (0..5). 64-bit BAR'larda `index+1` upper dword'dur.
    pub index: u8,
    /// Taban adres (memory BAR'lar için fiziksel; I/O BAR'lar için port numarası).
    pub base: u64,
    /// BAR boyutu (byte cinsinden, 2'nin kuvveti).
    pub size: u64,
    /// I/O space BAR'ı mı?
    pub is_io: bool,
    /// Prefetchable (cache'lenebilir) mı?
    pub prefetchable: bool,
    /// 64-bit BAR mı?
    pub is_64bit: bool,
}

impl PciBar {
    /// BAR dolu ve kullanılabilir mi?
    pub fn is_valid(&self) -> bool {
        self.size != 0 && self.base != 0
    }
}

impl BarType {
    pub fn is_io(&self) -> bool {
        matches!(self, BarType::Io)
    }
}

// ---------------------------------------------------------------------------
// PciDevice
// ---------------------------------------------------------------------------

/// PCI function'ı tanımlayan yapı.
/// `scan_pci` tarafından doldurulur; sürücüler BAR/komut okumak için kullanır.
#[derive(Clone, Debug)]
pub struct PciDevice {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision: u8,
    pub header_type: u8,
    /// 6 BAR slotu; 64-bit BAR'lar üst dword'u yutar (indeks atlanır).
    pub bars: [Option<PciBar>; 6],
}

impl PciDevice {
    /// Cihazın bilinen bir adı varsa onu, yoksa hex id'leri döndürür.
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

    /// PCI class/subclass için kısa bir isim (lspci stili).
    pub fn class_name(&self) -> String {
        match (self.class, self.subclass) {
            (0x00, _) => "Unclassified".to_string(),
            (0x01, 0x01) => "IDE Controller".to_string(),
            (0x01, 0x06) => "SATA Controller".to_string(),
            (0x02, _) => "Network Controller".to_string(),
            (0x03, _) => "Display Controller".to_string(),
            (0x06, 0x00) => "Host Bridge".to_string(),
            (0x06, 0x01) => "ISA Bridge".to_string(),
            (0x0C, 0x03) => "USB Controller".to_string(),
            (0x0C, 0x05) => "SMBus Controller".to_string(),
            _ => format!("Class {:#04X}/{:#04X}", self.class, self.subclass),
        }
    }

    /// Önceden çözümlenmiş BAR'ı döndürür (yoksa canlı okuyup önbelleğe alır).
    pub fn bar(&mut self, index: u8) -> Option<PciBar> {
        if (index as usize) >= self.bars.len() {
            return None;
        }
        if let Some(bar) = self.bars[index as usize] {
            return Some(bar);
        }
        let bar = read_bar(self, index)?;
        self.bars[index as usize] = Some(bar);
        Some(bar)
    }

    /// PCI interrupt line (IRQ vektörü). 255 = bağlı değil.
    pub fn interrupt_line(&self) -> u8 {
        unsafe { pci_read_u8(self.bus, self.slot, self.func, PCI_INTERRUPT_LINE) }
    }
}

// ---------------------------------------------------------------------------
// Config space okuma/yazma
// ---------------------------------------------------------------------------

/// PCI config space'ten 32-bit dword okur. `offset` 4 hizalı olmak zorunda değil
/// (içeride `& 0xFC` ile hizalanır).
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

/// PCI config space'e 32-bit dword yazar.
pub unsafe fn pci_write_u32(bus: u8, slot: u8, func: u8, offset: u8, value: u32) {
    let address = 0x80000000u32
                | ((bus as u32) << 16)
                | ((slot as u32) << 11)
                | ((func as u32) << 8)
                | ((offset as u32) & 0xFC);

    let mut config_address: Port<u32> = Port::new(0xCF8);
    let mut config_data: Port<u32> = Port::new(0xCFC);

    config_address.write(address);
    config_data.write(value);
}

/// PCI config space'ten 16-bit word okur (dword içinden kaydırmalı).
pub unsafe fn pci_read_u16(bus: u8, slot: u8, func: u8, offset: u8) -> u16 {
    (pci_read_u32(bus, slot, func, offset) >> ((offset & 0x03) * 8)) as u16
}

/// PCI config space'ten 8-bit byte okur (dword içinden kaydırmalı).
pub unsafe fn pci_read_u8(bus: u8, slot: u8, func: u8, offset: u8) -> u8 {
    (pci_read_u32(bus, slot, func, offset) >> ((offset & 0x03) * 8)) as u8
}

/// PCI config space'e 16-bit word yazar (komşu baytlara dokunmadan).
pub unsafe fn pci_write_u16(bus: u8, slot: u8, func: u8, offset: u8, value: u16) {
    let aligned = offset & 0xFC;
    let shift = ((offset & 0x03) * 8) as u32;
    let mut dword = pci_read_u32(bus, slot, func, aligned);
    dword = (dword & !(0xFFFFu32 << shift)) | ((value as u32) << shift);
    pci_write_u32(bus, slot, func, aligned, dword);
}

/// PCI config space'e 8-bit byte yazar (komşu baytlara dokunmadan).
pub unsafe fn pci_write_u8(bus: u8, slot: u8, func: u8, offset: u8, value: u8) {
    let aligned = offset & 0xFC;
    let shift = ((offset & 0x03) * 8) as u32;
    let mut dword = pci_read_u32(bus, slot, func, aligned);
    dword = (dword & !(0xFFu32 << shift)) | ((value as u32) << shift);
    pci_write_u32(bus, slot, func, aligned, dword);
}

// ---------------------------------------------------------------------------
// BAR çözümleme
// ---------------------------------------------------------------------------

/// Tek bir BAR'ı çözümler: taban adres, tip (mem/io), boyut.
///
/// Boyut bulmak için register'a tüm 1'ler yazılıp okunur (PCI standardı),
/// ardından orijinal değer geri yüklenir.
pub fn read_bar(dev: &PciDevice, index: u8) -> Option<PciBar> {
    if index >= 6 {
        return None;
    }
    let offset = PCI_BAR0 + index * 4;
    let orig = unsafe { pci_read_u32(dev.bus, dev.slot, dev.func, offset) };
    if orig == 0 {
        return None; // BAR kullanılmıyor
    }

    let is_io = (orig & 1) == 1;
    let is_64bit = !is_io && (orig & 0b110) == 0b100;
    let prefetchable = !is_io && (orig & 0b100) != 0;

    // 64-bit BAR üst dword gerektirir; indeks 5'te yer yok.
    if is_64bit && index >= 5 {
        return None;
    }

    let low_mask = if is_io { 0xFFFF_FFFCu32 } else { 0xFFFF_FFF0u32 };
    let orig_upper = if is_64bit {
        unsafe { pci_read_u32(dev.bus, dev.slot, dev.func, offset + 4) }
    } else {
        0
    };

    // Taban adres (fiziksel).
    let base = if is_64bit {
        ((orig & low_mask) as u64) | ((orig_upper as u64) << 32)
    } else {
        (orig & low_mask) as u64
    };

    // Boyut probe'u: tüm 1'ler yaz, oku, geri yükle.
    unsafe {
        pci_write_u32(dev.bus, dev.slot, dev.func, offset, 0xFFFF_FFFF);
        if is_64bit {
            pci_write_u32(dev.bus, dev.slot, dev.func, offset + 4, 0xFFFF_FFFF);
        }
        let lo = pci_read_u32(dev.bus, dev.slot, dev.func, offset);
        let size = if is_64bit {
            let hi = pci_read_u32(dev.bus, dev.slot, dev.func, offset + 4);
            let mask = ((hi as u64) << 32) | ((lo & low_mask) as u64);
            !mask + 1
        } else {
            (!(lo & low_mask) + 1) as u64
        };

        // Orijinal değerleri geri yükle.
        pci_write_u32(dev.bus, dev.slot, dev.func, offset, orig);
        if is_64bit {
            pci_write_u32(dev.bus, dev.slot, dev.func, offset + 4, orig_upper);
        }

        Some(PciBar {
            index,
            base,
            size,
            is_io,
            prefetchable,
            is_64bit,
        })
    }
}

/// Cihazın tüm BAR'larını çözer. 64-bit BAR'lar üst dword'u yuttuğu için
/// sonraki indeks atlanır.
pub fn bars_of(dev: &PciDevice) -> [Option<PciBar>; 6] {
    let mut bars: [Option<PciBar>; 6] = [None; 6];
    let mut i = 0usize;
    while i < 6 {
        if let Some(bar) = read_bar(dev, i as u8) {
            let skip = if bar.is_64bit { 2 } else { 1 };
            bars[i] = Some(bar);
            i += skip;
        } else {
            i += 1;
        }
    }
    bars
}

// ---------------------------------------------------------------------------
// Command register / DMA yardımcıları
// ---------------------------------------------------------------------------

/// Command register'ı (offset 0x04, düşük 16 bit) okur.
pub fn read_command(dev: &PciDevice) -> u16 {
    unsafe { pci_read_u32(dev.bus, dev.slot, dev.func, PCI_COMMAND) as u16 }
}

/// Command register'a yazar; status (üst 16 bit) korunur.
pub fn write_command(dev: &PciDevice, cmd: u16) {
    unsafe {
        let val = pci_read_u32(dev.bus, dev.slot, dev.func, PCI_COMMAND);
        pci_write_u32(dev.bus, dev.slot, dev.func, PCI_COMMAND, (val & 0xFFFF_0000) | cmd as u32);
    }
}

/// Command register'da belirtilen bitleri set/clear eder (atomik read-modify-write).
pub fn set_command_bits(dev: &PciDevice, set: u16, clear: u16) {
    let cmd = read_command(dev);
    write_command(dev, (cmd | set) & !clear);
}

/// Bus mastering (DMA) etkinleştir — cihazın memory'ye doğrudan erişmesine izin verir.
pub fn enable_bus_mastering(dev: &PciDevice) {
    set_command_bits(dev, command::BUS_MASTER, 0);
}

/// Bus mastering devre dışı.
pub fn disable_bus_mastering(dev: &PciDevice) {
    set_command_bits(dev, 0, command::BUS_MASTER);
}

/// I/O space erişimini etkinleştir (I/O BAR'ları kullanan cihazlar).
pub fn enable_io_space(dev: &PciDevice) {
    set_command_bits(dev, command::IO_SPACE, 0);
}

/// Memory space erişimini etkinleştir (MMIO BAR'ları kullanan cihazlar).
pub fn enable_memory_space(dev: &PciDevice) {
    set_command_bits(dev, command::MEMORY_SPACE, 0);
}

/// INTA# kesmesini etkinleştir (INT_DISABLE bitini temizle).
pub fn enable_intx(dev: &PciDevice) {
    set_command_bits(dev, 0, command::INT_DISABLE);
}

/// Bir cihazı DMA kullanımına hazırlar: memory/io space + bus mastering.
pub fn enable_device(dev: &PciDevice) {
    enable_io_space(dev);
    enable_memory_space(dev);
    enable_bus_mastering(dev);
    enable_intx(dev);
}

// ---------------------------------------------------------------------------
// MSI (isteğe bağlı)
// ---------------------------------------------------------------------------

/// Cihazın capability zincirinde `cap_id` ile başlayan capability'nin
/// config space offset'ini döndürür.
pub fn find_capability(dev: &PciDevice, cap_id: u8) -> Option<u8> {
    // Capability listesi yalnızca standard header (tip 0/1) cihazlarda.
    if dev.header_type & 0x7F > 0x01 {
        return None;
    }
    unsafe {
        let sts = (pci_read_u32(dev.bus, dev.slot, dev.func, PCI_STATUS) >> 16) as u16;
        if sts & status::CAPABILITIES_LIST == 0 {
            return None;
        }
        let mut ptr = pci_read_u8(dev.bus, dev.slot, dev.func, PCI_CAPABILITIES_POINTER);
        while ptr != 0 {
            let cap = pci_read_u32(dev.bus, dev.slot, dev.func, ptr);
            if (cap & 0xFF) as u8 == cap_id {
                return Some(ptr);
            }
            ptr = ((cap >> 8) & 0xFC) as u8;
        }
    }
    None
}

/// MSI (Message Signaled Interrupt) etkinleştirir.
///
/// `msg_addr`/`msg_data` sürücü tarafından (ör. APIC tabanına) ayarlanır.
/// 64-bit MSI destekleyen cihazlarda `msg_addr` 64-bit kullanılır.
pub fn enable_msi(dev: &PciDevice, msg_addr: u64, msg_data: u16) -> Result<(), &'static str> {
    let cap = find_capability(dev, 0x05).ok_or("MSI capability not found")?;
    unsafe {
        let mctl = pci_read_u16(dev.bus, dev.slot, dev.func, cap + 2);
        let is_64 = (mctl & 0x0080) != 0;

        // Message Address (ve 64-bit ise üst dword) + Message Data
        pci_write_u32(dev.bus, dev.slot, dev.func, cap + 4, msg_addr as u32);
        if is_64 {
            pci_write_u32(dev.bus, dev.slot, dev.func, cap + 8, (msg_addr >> 32) as u32);
            pci_write_u16(dev.bus, dev.slot, dev.func, cap + 12, msg_data);
        } else {
            pci_write_u16(dev.bus, dev.slot, dev.func, cap + 8, msg_data);
        }

        // MSI Enable (Message Control bit 0).
        pci_write_u16(dev.bus, dev.slot, dev.func, cap + 2, mctl | 0x0001);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Genel sürücü trait iskeleti
// ---------------------------------------------------------------------------

/// Genel PCI sürücü arayüzü. Her donanım sürücüsü bu trait'i uygular ve
/// `probe_all::<T>()` ile cihazlarla eşleştirilir.
pub trait Driver {
    /// Sürücü adı (log'larda kullanılır).
    fn name(&self) -> &'static str;

    /// Bir PCI cihazını destekleyip desteklemediğini kontrol eder.
    /// Destekliyorsa `Some(driver)` döndürür (henüz init edilmemiş).
    fn probe(dev: &PciDevice) -> Option<Self>
    where
        Self: Sized;

    /// Cihazı ayağa kaldırır (BAR'ları programla, bus mastering aç, vs).
    fn init(&mut self) -> Result<(), &'static str>;

    /// İsteğe bağlı teardown kancası.
    fn shutdown(&mut self) {}
}

/// `devices` listesini tarayıp desteklenen her cihaz için sürücü örneği üretir.
pub fn probe_all<D: Driver>(devices: &[PciDevice]) -> Vec<D> {
    devices.iter().filter_map(|d| D::probe(d)).collect()
}

// ---------------------------------------------------------------------------
// Aygıt numaralandırma (mevcut lspci taraması)
// ---------------------------------------------------------------------------

pub fn check_device(bus: u8, slot: u8, func: u8) -> Option<PciDevice> {
    let reg0 = unsafe { pci_read_u32(bus, slot, func, 0) };
    let vendor_id = (reg0 & 0xFFFF) as u16;

    if vendor_id == 0xFFFF {
        return None; // Aygit yok
    }

    let device_id = (reg0 >> 16) as u16;

    let reg2 = unsafe { pci_read_u32(bus, slot, func, 0x08) };
    let revision = (reg2 & 0xFF) as u8;
    let prog_if = (reg2 >> 8) as u8;
    let subclass = (reg2 >> 16) as u8;
    let class = (reg2 >> 24) as u8;

    let reg3 = unsafe { pci_read_u32(bus, slot, func, 0x0C) };
    let header_type = (reg3 >> 16) as u8;

    let mut dev = PciDevice {
        bus,
        slot,
        func,
        vendor_id,
        device_id,
        class,
        subclass,
        prog_if,
        revision,
        header_type,
        bars: [None; 6],
    };
    dev.bars = bars_of(&dev);

    Some(dev)
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
