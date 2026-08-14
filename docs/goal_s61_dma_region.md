# /goal — SparkOS 6.1: `DmaRegion` — Capability-Gated DMA Bellek Bölgesi

> Ver KURALLARI önce oku. Bu görev, SparkOS capability microkernel'ine (Aşama 6,
> RTL8139 netdrv'nin ilk adımı) kernel-ayrılmış, sayfa-hizalı, capability-gated bir
> DMA belleği bölgesi (`DmaRegion`) ekler.
>
> **CRİTİK:** Yerel dosyaları OKUYAMAZSIN (Google Cloud). Aşağıdaki kaynak parçaları
> güncel SparkOS kodundan birebir alındı. Bunlara dayanarak **tam, derlenebilir,
> `#![no_std]` uyumlu** yeni bir modül üreteceksin. Yerel dosyaya YAZMA — kodu cevabında
> `rust` kod bloğu olarak döndür. `<!-- GOAL_COMPLETE -->` ile bitir.
>
> Mevcut HİÇBİR dosyayı değiştirme istenmiyor — yeni izole modül yazıyorsun
> (standalone-module paterni). Modül, yalnızca `alloc` + `spin` global allocator'ına
> ve `x86_64` crate'inin sayfa/frame API'lerine bağımlı olmalı.

---

## 1. Amaç ve Bağlam

Aşama 6'da RTL8139 ağ sürücüsü user-space `netdrv` servisine taşınacak.
RTL8139, gelen paketleri CPU'ya sormadan **Bus-Master DMA** ile doğrudan fiziksel
belleğe yazar (RX Ring, 8KB + wrap). Bu fiziksel bölge user-space netdrv'in CR3
sayfa tablosuna eşlenmek zorunda. **Yanlış model, sürücüye rastgele kernel belleği
okuma yetkisi verir.** Doğru model: bölgeyi **çekirdek ayrır** (frame allocator +
sayfa eşleme), netdrv'e yalnızca O BÖLGEYİ gösteren **dar capability** ver.

Bu görev 6.1: yeni `DmaRegion` modülü. (netdrv servisinin kendisi — 6.2 — sonraki
görevdir; burada YAPMA.)

## 2. Zorunlu Gereksinimler (FROZEN sözleşme bağları)

1. **Kernel-ayrılmış:** Bölge fiziksel belleği kaynak ile işaretsiz olarak ayrılır.
   Azami bölge: bir dizi 4KB sayfa (RTL8139 için 3 sayfa yeter = 12KB > 8KB+16+1500).
2. **Sayfa hizalı:** Başlangıç fiziksel adresi 4KB-uyumlu olmalı (RTL8139 RBSTART kuralı).
3. **Capability-gated:** Bölgeyi elde etmek, alıcıya yalnızca **o bölgenin** sanal
   eşlemesini gösteren dar bir handle döner. Capability mantığı `src/cap.rs`'in
   mevcut API'sine OTURUR — yeni `Rights` bitini ve capability kaydını oraya nasıl
   bağlayacağını BELİRLE ama `src/cap.rs`'i YENİDEN YAZMA (sadece nasıl entegre
   edileceğini yorum olarak işaretle).
4. **Bellek yerine:** Bölge üstüne yazılan DMA verisi, kernel tarafından verilen bir
   `UserAccessable` yetkisi olmadan KERNEL için CALIŞMAz; ancak bölge yönetimi
   gereği eşleme her iki tarafta da (kernel fiske + kullanıcı uygulama VM'de) ayrı
   yapılır. Modül, **kernel görünümü eşlemesini** kurar; user-space eşlemesi 6.2'de.
5. `no_std` + `alloc`: Bölge yaşam döngüsü `Box<[u8]>` gibi ikincil alan kullanmaz —
   fiziksel frame'ler ve onların sanal eşlemesi KERNEL-rezervli olmalı. İzin verilen
   bağımlılıklar: `alloc`, `core`, `spin`, `x86_64` (FrameAllocator, PhysFrame,
   Mapper/PageTable, VirtAddr/PhysAddr, Size4KiB, flags).

## 3. API Sözleşmesi (kesin — yeniden adlandırma)

```rust
// Yeni modül: src/dma_region.rs

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
pub struct DmaSlot {
    pub offset: usize,        // bölge başlangıcından bayt uzaklığı
    pub len: usize,           // bayt uzunluğu
    pub owner_rights: u32,    // capability Rights mask (ör. READ|WRITE)
}

impl DmaRegion {
    /// Frame allocator'dan `pages` adet sayfa ayırır, sayfa hizalı fiziksel
    /// bölgeyi kernel VM'e eşler, capability handle'ı türetir.
    /// `alloc` = mevcut `memory::BootInfoFrameAllocator`'ın allocate_frame'i.
    pub fn allocate(pages: u64) -> Result<DmaRegion, DmaError>;

    /// Fiziksel adresi döndürür (RTL8139 RBSTART için — 4KB uyumlu olmalı).
    pub fn phys_addr(&self) -> u64;

    /// Kernel görünümünde ilgili sanal adrese ham pointer.
    pub fn as_mut_ptr(&self) -> *mut u8;

    /// Bölge başına capability handle'ı (6.2'de user-space'e eşleme anahtarı).
    pub fn capability(&self) -> Option<u64>;

    /// Yalnızca bu bölgeye alt-slot ekle/güncelle (RX ring konumunu kaydet).
    pub fn define_slot(&mut self, offset: usize, len: usize, rights: u32) -> Result<(), DmaError>;

    /// Bölgeyi eşlenmiş halinden çıkarıp frame'leri geri verir (refcount izini).
    pub fn release(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaError {
    OutOfFrames,        // allocator boş
    NotPageAligned,     // istenen fiziksel adres 4KB uyumsuz (olabilir değil)
    MappingFailed,      // sayfa eşleme hatası
    SlotOverflow,       // slot bölge dışına taşıyor
    Unmapped,           // capability henüz bağlanmadı
}
```

## 4. Capability Katmanı Entegrasyonu (yorum — `src/cap.rs`'i değiştirme)

Mevcut `src/cap.rs` (FROZEN, 5.5):

```rust
pub struct Rights(pub u32);             // READ=1, WRITE=2, MAP=4, ... EXECUTE=256 (0x100)
pub fn create_object(kind) -> Result<CapHandle>
pub fn grant(parent: CapHandle, req: Rights) -> Result<CapHandle>
pub fn revoke(cap: CapHandle) -> Result<()>
pub fn create_device_ports(start: u16, end: u16) -> Result<CapHandle>
```

`Rights::all()` = 0x3FF (bit 0-9). Aşama 6, `Rights`'a yeni bir **DMA hak biti**
ekler (`0x400` = 1024 öner): `pub const DMA: u32 = 0x400;`. Bunu NEREDE VE NASIL
yapacağını, `DmaRegion::allocate`'in türettiği capability handle'ın `cap.rs`'in
kayıt defterine (object → handle) nasıl yazılacağını **yorum olarak** belirt.
Kendin `src/cap.rs`'i değiştir DEMEZ; sözleşmeyi oturtur.

## 5. Host-Test Edilebilirlik (ZORUNLU)

Modül `#![no_std]` kalır ama HOST'ta test edilebilmelidir. Strateji:
- Gerçek `allocate()` QEMU'ya (x86_64-unknown-none) özgüdür; host'ta fiziksel
  frame allocator yoktur.
- **Bu yüzden çekirdek mantığı (slot yönetimi, offset kontrolü, capability
  right'ları, hizalama doğrulaması) `alloc`'a dayanan saf fonksiyonlara ayrılmalı**
  ve `#[cfg(test)]` ile host'ta test edilebilir:
  - `validate_slot(region_len, offset, len) -> Result<(), DmaError>` (boundcheck)
  - `rights_allow(slot_rights: u32, needed: u32) -> bool` (subset kontrol)
  - hizalama kontrolü `is_page_aligned(addr: u64) -> bool`
- Bu saf fonksiyonlar fiziksel bellek işleminden BAĞIMSIZ olmalı, böylece
  `scratch/run_cap_tests.sh` host harness'ı (`env -u PYTHONPATH cargo test -- --test-threads=1`)
  ile çalışır.
- `allocate`/`as_mut_ptr`/`release` gibi x86_64'e bağımlı kısımlar `#[cfg(not(test))]`
  altında veya izole; host test derlemesini KIRMAMALI.

## 6. Ünite Testleri (yaz, host'ta geçmeli)

1. `validate_slot` normal → Ok; taşan offset → SlotOverflow; tam sınır → Ok.
2. `rights_allow`: WRITE gerektiren, slot'ta READ yok → false; slot'ta WRITE varsa → true.
3. `is_page_aligned`: 0x1000 → true, 0x1234 → false.
4. Capability hakları azaltım alt kümesi: derive edilen hakler ana hakların alt kümesi
   (FROZEN CAP_INV-1) — `rights_allow(apparent, subset_of)` ile.
5. Bölge kapasite üst sınırı: `pages` çok büyükse (ör. >4096) reddeden `DmaError::OutOfFrames`
   gibi deterministik hata (DoS koruması).

## 7. DOKUNMA / YAZMA (Hard Quarantine)
- `src/rtl8139.rs`, `src/net_socket.rs` 'i DEĞİŞTİRME — 6.2 görevinde bağlanacak.
- `src/gui.rs` (PHYS_OFFSET), `src/memory.rs` (frame allocator imzası) değiştirme —
  yalnızca `allocate`'e allocator nasıl enjekte edileceğini yorumla.
- Defereed: SMP, IOMMU/DMA atomik, MMIO servisleri — YAZMA.
- DRY: `src/cap.rs` 'in Rights bitlerini çoğaltma — yeni DMA biti tek yerden türetilir.

## 8. Teslim Biçimi
Cevabında:
1. `## Module: src/dma_region.rs` başlıklı TAM `rust` kodu (derlenebilir, `no_std`+`alloc`).
2. Capability entegrasyonu için **yorum/şema** (nasıl bağlanacağı).
3. Testler (modül içi `#[cfg(test)]` dahil — 5-6 test).
4. Host test koşumu talimatı: `scratch/run_cap_tests.sh` (mevcut harness'a bu modülün
   `#[path]` ile nasıl ekleneceğini yorum olarak).
<!-- GOAL_COMPLETE -->
