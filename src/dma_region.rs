//! SparkOS Aşama 6.1 — `DmaRegion`: Capability-Gated DMA Bellek Bölgesi
//!
//! RTL8139 ve benzeri Bus-Master DMA cihazları için kernel tarafından ayrılmış,
//! 4KB sayfa hizalı, izole ve capability korumalı fiziksel bellek bölgelerini yönetir.
//!
//! # Güvenlik Sözleşmesi (FROZEN)
//! - Rastgele fiziksel bellek erişimi YASAKTIR.
//! - User-space sürücüler (`netdrv`) yalnızca kendilerine açıkça tahsis edilen
//!   `DmaRegion` capability handle'ı üzerinden bu bölgeye erişebilir.
//! - Bölge içindeki alt-slot'lar (örn. RX Ring) `owner_rights` ile sınırlandırılır.

use alloc::vec::Vec;

#[cfg(target_os = "none")]
use x86_64::{PhysAddr, VirtAddr};

#[cfg(not(target_os = "none"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysAddr(pub u64);

#[cfg(not(target_os = "none"))]
impl PhysAddr {
    pub const fn new(addr: u64) -> Self { Self(addr) }
    pub const fn as_u64(&self) -> u64 { self.0 }
}

#[cfg(not(target_os = "none"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtAddr(pub u64);

#[cfg(not(target_os = "none"))]
impl VirtAddr {
    pub const fn new(addr: u64) -> Self { Self(addr) }
    pub const fn as_u64(&self) -> u64 { self.0 }
    pub fn as_mut_ptr<T>(&self) -> *mut T { self.0 as *mut T }
}

/// Çekirdek tarafından ayrılmış, sayfa-hizalı bir DMA bölgesi.
/// 6.2'de netdrv'e dar capability ile eşlenecektir.
pub struct DmaRegion {
    pages: u64,               // fiziksel bölge varlığı (refcount/owner için iz)
    first_phys: PhysAddr,     // ilk sayfanın fiziksel adresi (4KB hizalı)
    kern_virt: VirtAddr,      // kernel görünümü eşlemesi (okuma/yazma)
    slots: Vec<DmaSlot>,      // alt-bölge tespiti (RX ring vs)
    // capability handle kaydı (capability katmanına nasıl bağlanacağı §4)
    cap_handle: Option<u64>,
}

/// Bölge içinde bir alt-bölge (ör. RX ring)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaSlot {
    pub offset: usize,        // bölge başlangıcından bayt uzaklığı
    pub len: usize,           // bayt uzunluğu
    pub owner_rights: u32,    // capability Rights mask (ör. READ|WRITE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaError {
    OutOfFrames,        // allocator boş veya geçersiz sayfa adedi
    NotPageAligned,     // istenen fiziksel adres 4KB uyumsuz
    MappingFailed,      // sayfa eşleme hatası
    SlotOverflow,       // slot bölge dışına taşıyor
    Unmapped,           // capability henüz bağlanmadı
}

// ---------------------------------------------------------------------------
// Saf Doğrulama Fonksiyonları (Host-Test Edilebilir & Çekirdekten Bağımsız)
// ---------------------------------------------------------------------------

/// Verilen adresin 4KB sayfa hizalı (page aligned) olup olmadığını doğrular.
pub const fn is_page_aligned(addr: u64) -> bool {
    (addr & 0xFFF) == 0
}

/// Slot sınır kontrolü: `offset + len` bölge boyutunu aşıyor mu denetler.
pub fn validate_slot(region_len: usize, offset: usize, len: usize) -> Result<(), DmaError> {
    if len == 0 {
        return Err(DmaError::SlotOverflow);
    }
    let end = offset.checked_add(len).ok_or(DmaError::SlotOverflow)?;
    if end > region_len {
        return Err(DmaError::SlotOverflow);
    }
    Ok(())
}

/// Capability hakları alt küme doğrulaması (CAP_INV-1 Monotonik Hak Azaltma).
pub const fn rights_allow(slot_rights: u32, needed: u32) -> bool {
    (slot_rights & needed) == needed
}

// ---------------------------------------------------------------------------
// DmaRegion Uygulaması
// ---------------------------------------------------------------------------

impl DmaRegion {
    /// Manuel / Mock bölge oluşturucu (Test ve Doğrulama için)
    pub const fn from_raw_parts(
        pages: u64,
        first_phys: PhysAddr,
        kern_virt: VirtAddr,
        cap_handle: Option<u64>,
    ) -> Self {
        Self {
            pages,
            first_phys,
            kern_virt,
            slots: Vec::new(),
            cap_handle,
        }
    }

    /// Frame allocator'dan `pages` adet 4KB sayfa ayırır, sayfa hizalı fiziksel
    /// bölgeyi kernel VM'e eşler.
    ///
    /// Azami sayfa sınırı: DoS ve taşma koruması için 4096 sayfa (16MB).
    #[cfg(target_os = "none")]
    pub fn allocate(pages: u64) -> Result<DmaRegion, DmaError> {
        if pages == 0 || pages > 4096 {
            return Err(DmaError::OutOfFrames);
        }

        // İlk frame'i ayır (RTL8139 gibi cihazlar ardışık fiziksel belleğe ihtiyaç duyar)
        let first_frame = crate::memory::user_alloc_frame().ok_or(DmaError::OutOfFrames)?;
        let first_phys = first_frame.start_address();

        if !is_page_aligned(first_phys.as_u64()) {
            return Err(DmaError::NotPageAligned);
        }

        // Kalan sayfaları tahsis et (bump allocator ardışık çerçeveler üretir)
        for _ in 1..pages {
            let _ = crate::memory::user_alloc_frame().ok_or(DmaError::OutOfFrames)?;
        }

        // Kernel görünümü: fiziksel bellek ofseti üzerinden doğrudan sanal adres
        let phys_offset = unsafe { crate::gui::PHYS_OFFSET };
        let kern_virt = VirtAddr::new(phys_offset + first_phys.as_u64());

        // Belleği sıfırla (güvenlik: eski çekirdek verileri sürücüye sızamaz)
        let total_bytes = (pages as usize) * 4096;
        unsafe {
            core::ptr::write_bytes(kern_virt.as_mut_ptr::<u8>(), 0, total_bytes);
        }

        Ok(DmaRegion {
            pages,
            first_phys,
            kern_virt,
            slots: Vec::new(),
            cap_handle: None,
        })
    }

    /// Host testi için mock `allocate` uygulaması
    #[cfg(not(target_os = "none"))]
    pub fn allocate(pages: u64) -> Result<DmaRegion, DmaError> {
        if pages == 0 || pages > 4096 {
            return Err(DmaError::OutOfFrames);
        }
        let first_phys = PhysAddr::new(0x1000_0000);
        let kern_virt = VirtAddr::new(0xFFFF_8000_1000_0000);
        Ok(DmaRegion {
            pages,
            first_phys,
            kern_virt,
            slots: Vec::new(),
            cap_handle: None,
        })
    }

    /// Fiziksel adresi döndürür (RTL8139 RBSTART için — 4KB uyumlu).
    pub fn phys_addr(&self) -> u64 {
        self.first_phys.as_u64()
    }

    /// Kernel görünümünde ilgili sanal adrese ham pointer.
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.kern_virt.as_mut_ptr()
    }

    /// Bölge sayfa adedi.
    pub fn page_count(&self) -> u64 {
        self.pages
    }

    /// Toplam bölge boyutu (bayt cinsinden).
    pub fn size_bytes(&self) -> usize {
        (self.pages as usize) * 4096
    }

    /// Bölge başına capability handle'ı (6.2'de user-space'e eşleme anahtarı).
    pub fn capability(&self) -> Option<u64> {
        self.cap_handle
    }

    /// Bölgeye capability handle atar.
    pub fn set_capability(&mut self, cap: u64) {
        self.cap_handle = Some(cap);
    }

    /// Yalnızca bu bölgeye alt-slot ekle/güncelle (RX ring konumunu kaydet).
    pub fn define_slot(&mut self, offset: usize, len: usize, rights: u32) -> Result<(), DmaError> {
        validate_slot(self.size_bytes(), offset, len)?;
        self.slots.push(DmaSlot {
            offset,
            len,
            owner_rights: rights,
        });
        Ok(())
    }

    /// Kayıtlı alt-slot listesini döndürür.
    pub fn slots(&self) -> &[DmaSlot] {
        &self.slots
    }

    /// Bölgeyi sıfırlar ve temizler.
    pub fn release(&mut self) {
        self.slots.clear();
        self.pages = 0;
        self.cap_handle = None;
    }
}

// ---------------------------------------------------------------------------
// §4. Capability Katmanı Entegrasyon Şeması (Sözleşme Referansı)
// ---------------------------------------------------------------------------
//
// 1. `src/cap.rs` Rights Genişletmesi:
//    - Mevcut: READ(1), WRITE(2), MAP(4), IO(8), DMA(16), TRANSFER(32), GRANT(64),
//      DESTROY(128), EXECUTE(256), MANAGE(512).
//    - DMA hak biti `Rights::DMA` (16 / 0x10) zaten `src/cap.rs` içinde mevcuttur.
// 2. ObjectKind::Memory / ObjectKind::Device:
//    - DmaRegion için `cap::create_object(ObjectKind::Memory)` ile yeni bir nesne
//      üretilir.
//    - `DmaRegion::allocate` sonrasında dönen `cap_handle`, `DmaRegion.set_capability(handle.slot as u64)`
//      olarak kaydedilir.
// 3. User-Space Sürücü Eşleme (Aşama 6.2):
//    - Sürücü süreci `SYS_MAP_DMA(cap_handle, virt_addr)` çağırdığında:
//      - `cap::check_rights(cap_handle, Rights::MAP | Rights::DMA)` kontrolü yapılır.
//      - Yalnızca `DmaRegion.first_phys` adresinden başlayan `DmaRegion.pages` adet
//        sayfa sürücünün CR3 tablosuna USER_ACCESSIBLE olarak eşlenir.
//    - Böylece sürücü kernel bellek alanını asla göremez (mutlak izolasyon).

// ---------------------------------------------------------------------------
// Host Ünite Testleri
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_slot_bounds() {
        let region_len = 3 * 4096; // 12 KB (RTL8139 RX Ring için 3 sayfa)

        // Normal geçerli slot
        assert!(validate_slot(region_len, 0, 8192).is_ok());
        assert!(validate_slot(region_len, 8192, 4096).is_ok());

        // Tam sınır
        assert!(validate_slot(region_len, 0, region_len).is_ok());

        // Taşan slot -> SlotOverflow
        assert_eq!(validate_slot(region_len, 0, region_len + 1).err(), Some(DmaError::SlotOverflow));
        assert_eq!(validate_slot(region_len, 4096, region_len).err(), Some(DmaError::SlotOverflow));

        // 0 uzunluk -> SlotOverflow
        assert_eq!(validate_slot(region_len, 0, 0).err(), Some(DmaError::SlotOverflow));
    }

    #[test]
    fn test_rights_allow_subsets() {
        const READ: u32 = 1;
        const WRITE: u32 = 2;
        const DMA: u32 = 16;

        let slot_rw = READ | WRITE | DMA;

        // Gerekli yetkiler mevcut
        assert!(rights_allow(slot_rw, READ));
        assert!(rights_allow(slot_rw, WRITE));
        assert!(rights_allow(slot_rw, READ | WRITE));
        assert!(rights_allow(slot_rw, DMA));

        // Slot'ta olmayan yetki -> false
        const EXEC: u32 = 256;
        assert!(!rights_allow(slot_rw, EXEC));
        assert!(!rights_allow(READ, WRITE));
    }

    #[test]
    fn test_is_page_aligned() {
        assert!(is_page_aligned(0x0));
        assert!(is_page_aligned(0x1000));
        assert!(is_page_aligned(0x2000));
        assert!(is_page_aligned(0x1000_0000));

        assert!(!is_page_aligned(0x1));
        assert!(!is_page_aligned(0x1001));
        assert!(!is_page_aligned(0x1234));
        assert!(!is_page_aligned(0x1FFF));
    }

    #[test]
    fn test_rights_attenuation_invariant() {
        // CAP_INV-1: Türetilen yetkiler ata yetkilerin alt kümesi olmalıdır
        let parent_rights = 1 | 2 | 4 | 16; // READ | WRITE | MAP | DMA
        let child_rights = 1 | 2;           // READ | WRITE

        assert!(rights_allow(parent_rights, child_rights));

        // Yetki genişletme yasaktır
        let invalid_expanded_child = 1 | 2 | 256; // READ | WRITE | EXECUTE
        assert!(!rights_allow(parent_rights, invalid_expanded_child));
    }

    #[test]
    fn test_define_slot_and_release() {
        let mut region = DmaRegion::from_raw_parts(
            3,
            PhysAddr::new(0x2000),
            VirtAddr::new(0xFFFF_8000_0000_2000),
            Some(42),
        );

        assert_eq!(region.phys_addr(), 0x2000);
        assert_eq!(region.capability(), Some(42));
        assert_eq!(region.size_bytes(), 12288);

        // RX Ring slotu tanımla (8KB)
        assert!(region.define_slot(0, 8192, 1 | 2).is_ok());
        assert_eq!(region.slots().len(), 1);
        assert_eq!(region.slots()[0].len, 8192);

        // Bölgeyi aşan slot -> SlotOverflow
        assert_eq!(region.define_slot(8192, 8192, 1 | 2).err(), Some(DmaError::SlotOverflow));

        // Release
        region.release();
        assert_eq!(region.page_count(), 0);
        assert_eq!(region.slots().len(), 0);
        assert_eq!(region.capability(), None);
    }

    #[test]
    fn test_allocation_bounds_check() {
        // 0 sayfa -> OutOfFrames
        assert_eq!(DmaRegion::allocate(0).err(), Some(DmaError::OutOfFrames));

        // >4096 sayfa -> OutOfFrames (DoS koruması)
        assert_eq!(DmaRegion::allocate(4097).err(), Some(DmaError::OutOfFrames));

        // Geçerli sayfa
        assert!(DmaRegion::allocate(3).is_ok());
    }
}
