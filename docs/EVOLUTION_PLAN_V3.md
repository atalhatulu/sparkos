# SparkOS — EVOLUTION PLAN V3 (Aşama 6: Gerçek Donanım Servisi — RTL8139 netdrv)

> Tarih: 2026-08-14 · Öncül: V2 · Durum: YÖNETİCİ ONAYLI
> Strateji: monolitik kernel'in capability microkernel'e evriminde, SMP/DMA-IOMMU
> yerine **ÖNCE** mevcut 5.x altyapısını (IRQ endpoint, sys_ioperm, fault recovery,
> user-space servis framework) ilk GERÇEK donanım servisiyle sınayıp derinleştirmek.
> 5.5'te kurulan köprü, donanımlı bir sürücüde kanıtlanmadan ilerleme risklidir.

---

## 1. Durum / Nereden Devam

Aşama 5.5 tamamlandı (commit `89d6144`): capability-gated user-space servisleri
çalışıyor — keysvc (IRQ endpoint), serdrv (Ring-3 COM1 TX, `sys_ioperm`), faultsvc
(fault recovery). Hepsi **demo/echo** seviyesinde. Framework güvenlik açıkları
(P0: exit temizliği, dequeue revocation; P1: TSS IOPB switch) kapatıldı.

**Boşluk:** Hiçbir GERÇEK donanım sürücüsü user-space'e alınmadı. `rtl8139.rs`
ve `ata.rs` hâlâ kernel'de. 5.5'in tüm altyapısı tek bir gerçek donanım servisinde
doğrulanmadı.

## 2. Hedef (Aşama 6)

RTL8139 ağ sürücüsünü **user-space `netdrv` servisi** olarak izole etmek;
L2 (Ethernet frame RX/TX) servis sınırında, L3/L4 (TCP/UDP, `net_socket.rs`)
IPC üzerinden ayrı soyutlama olarak kalmak üzere.

**Kapanış tanımı:** Kernel boot, `netdrv`'i Ring-3'te başlatır; netdrv BAR0 port
erişimi + dar DMA capability'si alır; L2 frame alıp verir; kernel'in üst ağ yığını
buna IPC ile bağlanır. QEMU'da `ping 8.8.8.8` veya L2 frame gösterimi çalışır.

## 3. Mimari Kararlar (Aşama 6 için — kullanıcı değerlendirmesiyle netleşti)

### D1. RX DMA tamponu = kernel-ayrılmış, sayfa hizalı, dar-capability eşlemeli (ZORUNLU)
Mevcut `rtl8139.rs` RX buffer'ı **kernel heap'teki bir `Vec`** (bkz. §5) — DMA
erişimi `as_ptr() - PHYS_OFFSET` ile hesaplanıyor. Bu, user-space'e verilemez
(rastgele VM eşleme = kernel belleği sızıntısı riski).

**Kural:** RX tamponu, `memory::BootInfoFrameAllocator::allocate_frame()` ile
ayrılmış **fiziksel sayfa hizalı, çekirdek ayrıcalıklı** bir tampon olmalı.
Bu tampon yalnızca `netdrv`'in CR3'üne ve YALNIZCA bu tamponu gösteren **dar bir
capability (Rights::MAP + yeni DMA hakkı)** ile eşlenmeli. Sürücü, kernel'in
başka hiçbir belleğine erişememeli.

### D2. Port I/O öncelikli (BAR0), MMIO ikincil (ZORUNLU)
RTL8139 hem Port I/O (BAR0) hem MMIO destekler. **BAR0 port aralığı** (tipik
0xC000..=0xC0FF — PCI keşfiyle teyit edilir) 5.0'daki `create_device_ports` +
`sys_ioperm` + TSS IOPB altyapısı ile ZATEN hazır. İlk aşamada MMIO'ya girilmez.
MMIO + IOMMU ayrı (DEFERRED) kalemdir.

### D3. netdrv yalnızca L2 (ZORUNLU)
`netdrv` sadece Ethernet frame RX/TX yapar. TCP/IP/ARP (`net_socket.rs`) sürücüye
gömülmez; IPC üzerinden ayrı soyutlama olarak kalır. X sürücü → Y yığın ayrımı
mikrokernel felsefesinin değişmezi.

## 4. Alt-Görevler (Sıralı, worker dağılımlı)

| # | İş | Teslim | Worker |
|---|----|--------|--------|
| **6.0** | Bu plan + 6.1 /goal dosyası | docs/ | Hermes |
| **6.1** | RX DMA tamponunu sayfa-hizalı kernel tamponuna çevir + `Rights::MAP|DMA` capability tanımı + BAR0 port capability'si | `src/cap.rs`, `src/memory.rs`(yardımcı), `src/rtl8139.rs` | **agy** (izole, sınırlı) |
| **6.2** | `netdrv` user-space servisi — `spawn_service`/`enter_service` ile, CR3 izole, port+DMA cap'ler Ring-3'te | `src/task/process.rs`, yeni `netdrv` demo | **fcc** (derin, çok dosyalı) |
| **6.3** | Ağ servisi L2 endpoint'i + üst yığın IPC bağı (`net_socket.rs` ↔ netdrv) | `src/ipc.rs`, `src/net_socket.rs` | **fcc** devam |
| **6.4** | Kabul testi: QEMU'da netdrv RX/TX kanıtı + `ping 8.8.8.8` (üst yığın senkron) | QEMU serial kanıtı | Hermes (doğrulama) |

## 5. Kritik Kaynak (6.1'de dokunulacak gerçek kod)

### `src/rtl8139.rs` — mevcut RX (kernel'de, Vec tabanlı = hatalı model):
```rust
pub struct Rtl8139 {
    io_base: u16,
    rx_buffer: Vec<u8>,                                   // heap Vec — 8KB+16+1500
    rx_idx: usize,
}
pub fn new(io_base: u16) -> Self {
    let mut rx_buf = Vec::with_capacity(8192 + 16 + 1500);
    rx_buf.resize(8192 + 16 + 1500, 0);
    ...
}
fn init(&mut self) {
    ...
    // RBSTART (reg 0x30) — RX tampon başlangıcı: Vec ptr → phys (ENDEYSİZ!)
    let phys_addr = (self.rx_buffer.as_ptr() as u64) - crate::gui::PHYS_OFFSET;
    ...
}
```
`PHYS_OFFSET` = `boot_info.physical_memory_offset` (main.rs:194; gui.rs:15 static).

### `src/memory.rs` — frame allocator (mevcut, doğru model):
```rust
pub struct BootInfoFrameAllocator { ... }          // 320
impl BootInfoFrameAllocator { pub fn init(&boot_info) -> Self {...} } // 327
unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> { ... }   // 343
}
```

### `src/cap.rs` — capability katmanı (5.5, FROZEN):
```rust
pub struct Rights(pub u32);                    // READ=1|WRITE=2|MAP=4|... bootstrap_root 1|2|4|256
pub fn grant(parent: CapHandle, req: Rights) -> Result<CapHandle>
pub fn revoke(cap: CapHandle) -> Result<()>
pub fn create_device_ports(start: u16, end: u16) -> Result<CapHandle>   // 527
pub fn port_range_allowed(cap: CapHandle, s: u16, e: u16) -> Result<()> // 555
```
`Rights::all()` = 0x3FF. 256=EXECUTE (bootstrap_root'te). Yeni DMA hakkı için
mevcut bit boşluğundan (örn. 1024=0x400) tek tanımlı bit ayrılır — **frozen hal
yok, eklenebilir ama tekil ve belgeli olmalı.**

### `src/syscall.rs` / `sysapi.rs` — köprü:
```rust
// SYS_IOPERM: 5.0'dan; create_device_ports(darı cap) + gdt::allow_port_range(start,end)
// sys_ipc_create_endpoint / sys_ipc_bind_irq: 5.1'den; IRQ endpoint
```

## 6. Doğrulama (Her alt-görev, birebir 5.5 standardı)
- `cargo build` 0 hata
- `scratch/run_cap_tests.sh` → test sayısı ARTAR (şu an 27), 0 fail
- QEMU: `[OK] Capability core` + netdrv `[NETDRV] alive` + NET RX kanıtı; **PANIC yok**,
  EXIT=124 (temiz timeout)
- Aşama 6 kapanışta: 6.4'te `net_socket.rs` (üst yığın) netdrv ile IPC üzerinden çalışır

## 7. DEFERRED (Bu aşamada DOKUNMA)
SMP aktivasyonu · DMA/IOMMU · Priority Donation · Lend expiry (timer-temelli) ·
Nested/chained donation · Detaylı device-service framework (MMIO servisleri) ·
NUMA. — hepsi Aşama 6 ve 7'den SONRA.

## 8. Aşama 6'dan Sonra (Öneri)
- **7.** IPC iptali (cooperative `SYS_IPC_CANCEL`) — netdrv gibi multiplex servisler buna ihtiyaç duyar; DEFERRED→PROVISIONAL
- **8.** Lend expiry — servis kaynaklarının timeout ile geri çevrilmesi

---

*Bu plan, agy'ye /goal olarak verilecek görevlerin (docs/goal_s6_*.md) üst düzey sözleşmesidir.
Agy yerel kodu göremez; her /goal dosyası §5 kaynağını içinde taşır.*
