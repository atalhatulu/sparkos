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
// Global DMA Region Registry (Capability Slot -> (PhysAddr, Pages))
// ---------------------------------------------------------------------------

use alloc::collections::BTreeMap;
use spin::Mutex;

pub static DMA_REGIONS: Mutex<BTreeMap<u32, (u64, u64)>> = Mutex::new(BTreeMap::new());

/// Registers a DMA region physical address and page count associated with a capability slot.
pub fn register_dma_region(cap_slot: u32, phys_addr: u64, pages: u64) {
    DMA_REGIONS.lock().insert(cap_slot, (phys_addr, pages));
}

/// Looks up registered DMA region (phys_addr, pages).
pub fn lookup_dma_region(cap_slot: u32) -> Option<(u64, u64)> {
    DMA_REGIONS.lock().get(&cap_slot).copied()
}

// ---------------------------------------------------------------------------
// Aşama 6.3 — DmaSlot Buffer-Cap Registry (Zero-Copy Üst Yığın Köprüsü)
// ---------------------------------------------------------------------------
//
// netdrv (Ring 3) bir DMA bölgesi içindeki alt-aralığı (örn. RX ring'de gelen
// bir frame) üst yığına capability ile ulaştırır: SYS_IPC_CREATE_SLOT yeni bir
// `ObjectKind::Memory` nesnesi üretir ve `object_idx`'i burada kaydeder. Donanım
// (DMA) ve kernel consumer aynı fiziksel sayfaları gördüğü için veri hiç
// kopyalanmaz — `resolve_slot_cap` yalnızca sanal adresi ve uzunluğu döndürür.
//
// SLOT_MAP `object_idx` ile anahtarlanır (cap slota değil): sys_ipc_create_slot
// her çağrıda benzersiz bir object ürettiği için, slot cap'in dma_cap'in
// lineage'ından tamamen bağımsız yaşaması ve netdrv çıkışında/revoke'unda
// geçerliliğini koruması garantilenir.

/// DMA bölgesi içinde capability'ye bağlanmış bir alt-aralık.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotMapEntry {
    /// DMA bölgesinin ilk sayfasının fiziksel adresi.
    pub region_phys: u64,
    /// Bölge başlangıcından bayt uzaklığı (ör. RX ring için 4 = frame başı).
    pub offset: usize,
    /// Alt-aralık uzunluğu (bayt).
    pub len: usize,
}

static SLOT_MAP: Mutex<BTreeMap<u32, SlotMapEntry>> = Mutex::new(BTreeMap::new());

/// Bir Memory-object `object_idx`'ini DMA bölgesi alt-aralığına bağlar.
pub fn register_slot(object_idx: u32, region_phys: u64, offset: usize, len: usize) {
    SLOT_MAP.lock().insert(
        object_idx,
        SlotMapEntry {
            region_phys,
            offset,
            len,
        },
    );
}

/// Kayıtlı slot girişini döndürür (test/denetim için).
pub fn lookup_slot(object_idx: u32) -> Option<SlotMapEntry> {
    SLOT_MAP.lock().get(&object_idx).copied()
}

/// Capability'yi doğrular ve işaret ettiği DMA alt-aralığının kernel-görünür
/// sıfır-kopya sanal adresini + uzunluğunu döndürür.
///
/// Gate'ler:
/// - `cap` `needed` haklarını içermeli (pasif `check_rights`) → NoRights/Invalid
/// - `cap` bir `ObjectKind::Memory` nesnesine işaret etmeli → NoRights
/// - o object_idx için SLOT_MAP kaydı mevcut olmalı → NotFound
pub fn resolve_slot_cap(
    cap: crate::cap::CapHandle,
    needed: crate::cap::Rights,
) -> Result<(*mut u8, usize), crate::cap::CapError> {
    crate::cap::check_rights(cap, needed)?;
    let (kind, object_idx) = crate::cap::object_identity(cap)?;
    if kind != crate::cap::ObjectKind::Memory {
        return Err(crate::cap::CapError::NoRights);
    }
    let entry = lookup_slot(object_idx).ok_or(crate::cap::CapError::NotFound)?;
    #[cfg(target_os = "none")]
    let base = unsafe { crate::gui::PHYS_OFFSET };
    #[cfg(not(target_os = "none"))]
    let base = 0u64; // host test: işaretçi yalnızca ofset mantığını doğrular
    let ptr = (base + entry.region_phys + entry.offset as u64) as *mut u8;
    Ok((ptr, entry.len))
}

/// Aşama 6.3: netsvc tarafından işlenen bir slot capability'sini netdrv'ye iade eder (Recycle).
/// SLOT_MAP kaydını kaldırır ve capability'yi kapatır.
pub fn recycle_slot_cap(cap: crate::cap::CapHandle) -> Result<(), crate::cap::CapError> {
    let (kind, object_idx) = crate::cap::object_identity(cap)?;
    if kind != crate::cap::ObjectKind::Memory {
        return Err(crate::cap::CapError::NoRights);
    }
    SLOT_MAP.lock().remove(&object_idx);
    crate::cap::close(cap)?;
    Ok(())
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

    #[test]
    fn test_slot_map_resolve_cap() {
        // Capability core'u tazele (STATE global — test-threads=1 ile koşar).
        crate::cap::init();

        // Memory object üret, SLOT_MAP'e kaydet, resolve aynı aralığı dönmeli.
        let mem = crate::cap::create_object(crate::cap::ObjectKind::Memory).unwrap();
        let (kind, object_idx) = crate::cap::object_identity(mem).unwrap();
        assert_eq!(kind, crate::cap::ObjectKind::Memory);

        let region_phys: u64 = 0x1234_0000;
        let offset = 4usize; // RX ring frame başı
        let len = 60usize;
        register_slot(object_idx, region_phys, offset, len);

        // Host stub base=0 → ptr = region_phys + offset; len korunur.
        let (ptr, got_len) = resolve_slot_cap(mem, crate::cap::Rights::READ).unwrap();
        assert_eq!(ptr as u64, region_phys + offset as u64);
        assert_eq!(got_len, len);

        // Kayıtlı olmayan object_idx → NotFound.
        let mem2 = crate::cap::create_object(crate::cap::ObjectKind::Memory).unwrap();
        assert_eq!(
            resolve_slot_cap(mem2, crate::cap::Rights::READ).err(),
            Some(crate::cap::CapError::NotFound)
        );

        // Memory olmayan object (Device) → NoRights.
        let dev = crate::cap::create_object(crate::cap::ObjectKind::Device).unwrap();
        assert_eq!(
            resolve_slot_cap(dev, crate::cap::Rights::READ).err(),
            Some(crate::cap::CapError::NoRights)
        );

        // Kayıtlı slot cap'i WRITE hakkıyla okumaya çalış → NoRights
        // (create_object Rights::all() verir, bu yüzden zayıflatılmış child üret).
        let child = crate::cap::grant(mem, crate::cap::Rights::READ).unwrap();
        let (_, child_idx) = crate::cap::object_identity(child).unwrap();
        register_slot(child_idx, region_phys, offset, len);
        // READ hakkı var → Ok; DMA hakkı yok → NoRights.
        assert!(resolve_slot_cap(child, crate::cap::Rights::READ).is_ok());
        assert_eq!(
            resolve_slot_cap(child, crate::cap::Rights::DMA).err(),
            Some(crate::cap::CapError::NoRights)
        );
    }
}
