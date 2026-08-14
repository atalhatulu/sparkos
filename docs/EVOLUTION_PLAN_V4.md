# SparkOS — EVOLUTION PLAN V4 (Aşama 6.2 → Aşama 10)

> Tarih: 2026-08-14 · Öncül: V3 (Aşama 6; 6.1 kapandı, commit `a80a3df`)
> Kapsam: 6.2'den Aşama 10'a kapamayı detaylı yol haritası. Kod YAZILMAZ —
> her aşama /goal üst sözleşmesi görevi üretilecek (agy/fcc/Hermes dağılımı ile).
> Durum: PLAN — onay sonrası aşama aşama uygulanır; her aşama bağımsız doğrulanır.

---

## Mevcut Gerçek Varlıklar (Bu Planı Besleyen Kanıt)

| Bileşen | Dosya | Durum | Not |
|---|---|---|---|
| RTL8139 sürücü | `src/rtl8139.rs` (189L) | Kernel | Port-I/O (BAR0), RX **Vec tabanlı** (hatalı model), `poll_rx()` |
| DMA bölgesi | `src/dma_region.rs` (6.1, YENİ) | Çalışır | Kernel-ayrık sayfa-hizalı DMA, capability-gated, 33/33 test |
| TCP/UDP yığını | `src/net_socket.rs` (609L) + `src/net` | Kernel | TCP handshake, UDP, `handle_incoming_frame` — **monolitik, IPC'siz** |
| ATA disk | `src/ata.rs` (135L) | Kernel | PIO `read/write_sector` LBA, `AtaDrive::new(0x1F0)` |
| User-space servis | `src/task/process.rs` (1482L) | Çalışır | `enter_service`/`spawn_service`, CR3-izole, keysvc/serdrv/faultsvc |
| Syscall | `src/syscall.rs`, `src/sysapi.rs` | Altyapı | Mevcut numaralar 0–25; **boş slot 5,6,7,16,17,18,19** ve 26+ |
| IO izolasyonu | `src/gdt.rs`, `src/cap.rs` | Çalışır | TSS IOPB, `create_device_ports`, 5.5'te switch senkronu |
| SMP | `src/smp.rs`, `src/acpi.rs` | **ŞEMA** | APIC/CPU/IO-APIC ID tespiti VAR, **aktivasyon YOK** (tekil çekirdek çalışıyor) |
| Test | `scratch/run_cap_tests.sh` | Çalışır | Şu an 33 test, 0 fail |

**Kritik sayısal gerçek:** Syscall numara alanında 26+ serbest. 6.2–10 için önerilen
yeni syscall'lar (kesin atama uygulama anında) — plan boyunca `(SYS_<X> = 26+)` işaretli.

---

# AŞAMA 6 — Donanım Servisi: RTL8139 → `netdrv` (user-space L2 NIC)

> Amaç: 5.x altyapısını ilk GERÇEK donanım sürücüsünde doğrulamak.
> 6.1 kapandı (DmaRegion). Gerisi aşağıda.

### 6.2 — netdrv user-space servisi (DİP AMBİT: L2 frame RX/TX, üst yığın bağsız)

**Amaç:** RTL8139 RX'i `DmaRegion`'a taşımak + Ring-3 `netdrv` servisine almak;
IRQ → endpoint üzerinden L2 frame RX kanıtı. Üst yığın (net_socket) **bağlanmaz** (6.3).

| # | İş | Dosya | Defter |
|---|---|---|---|
| a | RX'i `Vec` → `DmaRegion` (frame allocator, 3 sayfa); RBSTART hizalanır | `rtl8139.rs` | fcc |
| b | `sys_ioperm`+`create_device_ports` ile BAR0 (0xC000..=0xC0FF, PCI keşfiyle teyit) netdrv'e | `syscall.rs`, `sysapi.rs` | fcc |
| c | `netdrv` Ring-3 servis: `spawn_service`, CR3-izole, DMA cap + port cap | `process.rs`, yeni demo | fcc |
| d | RX IRQ → 5.1 endpoint (`sys_ipc_bind_irq`), frame teslim | `ipc.rs` | fcc |
| e | Host test: DmaRegion entegrasyonu, slot/rights; QEMU: `[NETDRV] alive` + L2 frame | test + run.sh | Hermes |

**Doğrulama:** build 0 hata · test artar (33+) · QEMU `[NETDRV] alive`, RX ring frame,
PANIC yok, EXIT=124.
**Risk:** RX DMA fiziksel adresi sayfa-uyumlu değilse RBSTART bozuk → fault. Netdrv'i
fault-recovery'ye bağla (5.4 altyapısı hazır).
**Alt karar:** BAR0 değeri QEMU'da deterministik değil — PCI config keşfi ile,
sabit değil.

### 6.3 — Üst ağ yığını IPC bağı (`net_socket` ↔ `netdrv`)

**Amaç:** Kernel'deki tek parça TCP/UDP/ARP işlemini `netdrv` üzerinden tüketmek;
`net_socket.rs`'i **servis-istemciye** çevirmek (kendi NIC erişimi yok — IPC çağırır).

| # | İş | Dosya | Defter |
|---|---|---|---|
| a | `netdrv` → üst yığın istemci API: `SYS_NET_SEND_FRAME`/`SYS_NET_RECV_FRAME` (26+) | `syscall.rs`, `net_socket.rs` | fcc |
| b | `handle_incoming_frame` IPC üzerinden netdrv'e yönlendir; TCP/UDP state net_socket'te kalır | `net.rs`, `net_socket.rs` | agy (izole) |
| c | L3 IP ayrıştırıcıyı netdrv'den ayır, net_socket'e taşı | `net_socket.rs` | agy |
| d | QEMU: TCP handshake + UDP `ping 8.8.8.8` uçtan uca | run.sh | Hermes |

**Doğrulama:** QEMU'da `ping 8.8.8.8` (mevcut kernel yığını senkron kalır, netdrv üstüne biner).
**Risk:** Socket fd → endpoint fd map karmaşıklaşır. IPC thread/poll deadlock'ı — 7.1'e öncül.
**Bağımlılık:** 6.2 (netdrv çalışmalı).

---

# AŞAMA 7 — Servis Altyapısı Olgunlaşması

> Amaç: Çok istemcili, istek/yanıt servisler (netdrv, disksvc) güvenli çalışsın.

### 7.1 — Cooperative IPC iptali (`SYS_IPC_CANCEL = 26`)

**Amaç:** Bloke bekleyen `SYS_IPC_RECV`/`SEND` isteğe bağlı iptali; multiplex servislerde
istek TCP timeout'u düşünce hang-up yapabilsin. DEFERRED→PROVISIONAL.

- İptal edilebilirlik: IPC bloke eden syscall'ları kayıt altına al (syscall context);
  iptal zinciri `IPC_CANCELLED` hata koduyla döner. FROZEN: `IPC_CANCELLED` zaten listede.
- `dequeue revocation` (5.5 B) ile etkileşim: iptal bekleyen-in-flight kap'ı sessiz
  drop etmez, revoke eder.
- Test: bloke recv → CANCEL → `IPC_CANCELLED`; in-flight transfer node temizlenir.
**Worker:** fcc (derin, syscall + scheduler entegrasyonu).

### 7.2 — Lend expiry (timer-temelli sızıntı önlemi)

**Amaç:** `TransferMode::Lend` ile ödünç verilen capability timeout'a bağlanma;
serviste kaynak (RX buffer, socket) iade edilmezse scheduler/zamanlayıcı geri çevirir.
- Zamanlayıcı: timer tick'te lend rekorlarını gez, süresi dolanı revoke et.
- FROZEN: `lend→grant` yasağı korunur; expiry = revoked, free değil.
**Worker:** agy (izole lenişletme), ana zamanlayıcı fcc.
**Risk:** Zamanlayıcı saat kaynağı — PIT/APIC timer (smp.rs'ten I/O APIC hazır).

### 7.3 — Zero-copy IPC / buffer handle (büyük DMA verisinde)

**Amaç:** netdrv frame → üst yığın kopyasız geçiş; `DmaRegion::define_slot` → endpoint'e
buffer handle teslimi (`handle` + offset, kopyalama yok; yetki dar).
- Yeni syscall `SYS_IPC_SEND_BUF`/`SYS_IPC_RECV_BUF` (27–28).
- FROZEN: buffer paylaşımı capability-gated, `Rights::DMA` sarmalı.
**Worker:** fcc.

---

# AŞAMA 8 — Disk Servisi + VFS (bağımsız FS katmanı)

> Amaç: ATA'yı user-space `disksvc`'e almak + VFS'i birbirine gevşek bağlı bağımsız
> servise çevirmek. PIO (DMA'sız) — SYS_IOPERM ile hazır.

### 8.1 — `disksvc` user-space servisi (PIO, ATA 0x1F0)

**Amaç:** `ata.rs` (kernel) → Ring-3 `disksvc`. PIO PIO — DMA gerektirmez.

| İş | Dosya | Defter |
|---|---|---|
| `ata.rs`'i `DmaRegion`-bağımsız ama capability-gated port erişimiyle servise taşı | yeni `disksvc` demo | fcc |
| ATA IRQ → endpoint (disk read/write sonuç bildirimi) | `ipc.rs` | fcc |
| Block I/O capability: `Rights::READ|WRITE` + port range | `cap.rs` | agy |
| QEMU: `[DISKSVC] alive` + sector okuma kanıtı | run.sh | Hermes |

**Doğrulama:** QEMU disk.img'den sector 0 okuma; PANIC yok.
**Risk:** ATA IRQ + PIO busy-wait karışımında zamanlama — bounded-timeout (mevcut
`read_sector` buna sahip).

### 8.2 — VFS'i bağımsız servise çevir

**Amaç:** Mevcut dosya işlemleri (SYS_OPEN/READ/WRITE) kernel monolitik VFS'ten
servis-istemciye; `disksvc` üzerinde blok okuyan **FS servisi** (`fssvc`).
- `fs.rs` (VFS), `fs/` alt modülleri → `fssvc`; syscall köprüsü IPC'ye indirgenir.
- SYS_OPEN/READ/WRITE ABI korunur (istemci tarafı değişmez) — iç IP C'ye bağlanır.
**Worker:** fcc (uzun, monolitik VFS'i parçalar).

### 8.3 — `blockcache` / ayrık caching servis

**Amaç:** Disk sector cache'i kernel'den ayrı; `disksvc`'e isteğe bağlı katman.
**Worker:** agy/opsiyonel — 8.1–8.2 sonrası.

---

# AŞAMA 9 — SMP (Çok Çekirdekli) Aktivasyonu

> Amaç: `smp.rs` şemasını gerçek aktivasyona çevir. En riskli, EN SON donanım servisi
> katmanı oturunca. Aşağıdaki 4 hat sıralı.

### 9.1 — Per-CPU veri + lock dönüşümü
- Uyumlu (spinlock→per-cpu sorunlu kesitler: allocator, scheduler, TSS, cap table).
- **Ön koşul:** 6.1 DmaRegion + allocator izolasyonu. Tek global lock darboğazını
  per-cpu'ya dağıt.
**Worker:** fcc (en derin).

### 9.2 — IRQ routing (her CPU'ya IRQ)
- I/O APIC (smp.rs tespit ediyor) → hedef CPU; RTL8139/ATA IRQ'ları işleyici CPU'ya
  sabitlenebilir (affinity).
**Worker:** fcc.

### 9.3 — Multicore scheduling (work stealing)
- Ready queue per-cpu + çekirdekler arası çalma; `SMP` ölçütü: 2 CPU'da 2 servis paralel.
**Worker:** fcc + agy (sch struct'ı izole).

### 9.4 — QEMU `-smp 2` doğrulama
- 2 çekirdekte keysvc/serdrv/netdrv/disksvc paralel çalışır, PANIC yok.
**Worker:** Hermes.

**Risk (en yüksek):** cap table ve allocator paylaşımı. Kök zaman: atomic/per-cpu
geçişi. **Son çare önlemi:** 9.1'den önce allocator'ı per-cpu yap (6.1 habitat).

---

# AŞAMA 10 — Güvenlik Tamamlayıcı + Koruma

### 10.1 — DMA/IOMMU (Intel VT-d varsayımı, QEMU `-device intel-iommu`)
- RTL8139/ATA DMA'sını IOMMU altına al; capability-gated DMA bölgelerini donanıma
  bölgelendir. `DmaRegion` → IOMMU mapping.
**Ön koşul:** Aşama 9 (SMP) + 6 (donanım servisleri) — IOMMU çok CPU'da anlamlı.
**Worker:** fcc.

### 10.2 — Formal/Capability invariant doğrulama
- agy 5.5 review'undaki CAP_INV-1..18 invariantlerini `proptest`/`loom` ile otomatikleştir;
  `tests/cap_invariants.rs` (agy P2 önerisi). Sıralı herhangi capability geçişi invarianti korur.
**Worker:** agy (izole test modülü).

### 10.3 — Boot güvenliği (opsiyonel, zamanlama esneklik)
- Signed boot (bootloader zinciri), kernel/capability root'a ölçüm. TPM yoksa yazılım
  doğrulama. [DEFERRED kalabilir — düşük öncelik.]

---

## Genel Kurallar (Tüm Aşamalar)

1. **Worker dağılımı:** fcc = uzun/derin (6.2-6.3, 7.1, 7.3, 8.1-8.2, 9.1-9.3, 10.1);
   agy = izole/standalone (7.2 izole, 8.1 cap tarafı, 10.2); Hermes = plan/test/QEMU/commit.
2. **Her aşama bağımsız doğrulanır:** `cargo build` 0 hata → `run_cap_tests.sh` (test
   sayısı ARTAR, 0 fail) → QEMU demo kanıtı → commit+push.
3. **Yeni syscall ataması tekil:** `syscall.rs` + `sysapi.rs` aynı anda; boş slotlar
   (5,6,7,16,17,18,19) ve 26+ ortak envanterden. Çakışma yok (tek kaynak).
4. **FROZEN sözleşme değişmez:** capability attenasyonu, sessiz drop yasağı, hata kodu
   tekil kaynağı, lend→grant yasağı — değişmez. Ek yeni bit (Rights::DMA) tekil ve belgeli.
5. **DEFERRED kalıcı listesi (dokunulmaz, aşama sonrası değerlendirilir):** Priority
   Donation · Nested/chained donation · Lend granularity dalları · NUMA · Transactional
   capability. — YALNIZ 10.2 proptest bu invariantleri KAPSAR.

## Öncelikli Sıra (Uygulama)
6.2 → 6.3 → 7.1 → 7.2 → 7.3 → 8.1 → 8.2 → 8.3 → 9.1 → 9.2 → 9.3 → 9.4 → 10.1 → 10.2 → 10.3
6.2 ve 6.3 bağımlı; 7.x zincire paralel (6.3'ün deadlock ihtiyacı 7.1'i de tetikleyebilir)
kurgulanabilir — fcc tek akış tutarlı ilerler.

---

*Bu doküman, agy/fcc'ye verilecek her /goal görev dosyasının (docs/goal_*) ana sözleşmesidir.
Her görev, "Mevcut Gerçek Varlıklar" bölümündeki kaynak kodu kendi içinde taşır.*
