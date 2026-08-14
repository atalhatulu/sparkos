# SparkOS

Rust ile yazılmış, freestanding (bağımsız) bir **x86_64 işletim sistemi çekirdeği**.

> **Mevcut yol haritası:** Monolitik makro-kernel → **capability-based microkernel** (kademeli evrim, asla yeşil alandan yeniden yazım). Aşağıda hem 1.0 özellik seti hem de devam eden **capability mimari evrimi** detaylı anlatılmaktadır.

---

## 🚀 Öne Çıkanlar

- **`no_std` + `no_main`** freestanding kernel, kendi paging/heap/allocator altyapısı
- Tam teşekküllü **Ring 3 kullanıcı alanı** (userspace) + sistem çağrıları (`int 0x80`)
- Async/await **görev yürütücü** + round-robin timer scheduler + preemptive process model
- Bellek içi + ATA disk dosya sistemi, ELF64 yükleyici, RTL8139 ağ yığını (ARP, TCP handshake)
- Grafik masaüstü (GUI), nano-benzeri tam ekran editör, tamir kabuğu
- **Capability tabanlı yetki kontrolü** — syscall gating, bellek/pointer sertleştirmesi, capability IPC
- **QEMU + KVM** üzerinde test edilir; önyükleme `bootloader 0.9` + `bootimage`

---

## 🗺️ Yol Haritası Durumu (Skor Tablosu)

SparkOS önce **L0–L12** modeliyle stabilize edildi (1.0 FINAL), ardından **capability microkernel evrimi** başlatıldı. Güncel aşama ilerlemesi:

| Aşama | İçerik | Commit | Durum |
|---|---|---|---|
| **Aşama 1** | İzole capability core (`src/cap.rs`) — grant/transfer/lend/revoke/deref | `85c14a1` | ✅ Tamamlandı |
| **Aşama 2.0** | `cap::init()` boot'a bağlandı + SYS_EXEC exploit fix | `d3c0a2e` | ✅ Tamamlandı |
| **Aşama 2** | SYS_CAP — syscall yetki kontrolü köprüsü (`syscall_cap.rs`, dispatcher gating) | `5c92526` | ✅ Tamamlandı |
| **Aşama 3** | Bellek & pointer güvenliği sertleştirmesi (UB temizliği, `validate_user_ptr`) | `c8253f8` | ✅ Tamamlandı |
| **Aşama 4** | Capability-based IPC (`CapChannel`, Transfer/Lend) | `c8253f8` | ✅ Tamamlandı |
| **Aşama 5** | Driver/service ayrıştırması → microkernel | — | 🔴 Sırada |

**Doğrulama standardı (her aşama):** `cargo build` **0 hata** + host unit test + QEMU boot regression.

> ✅ Test durumu: **14/14 host test** (9 capability invariant + 3 syscall_cap PURE + 2 IPC);
> QEMU boot + IPC Producer/Consumer sıfır regresyon.

---

## 🏛️ Yeni Kernel Mimarisi: Capability-Based Microkernel

### Vizyon: Neye evriliyoruz?

Mevcut sistem "process her şeyi yapabilir" (monolitik) modelindeydi. Yeni mimaride **hiçbir işlem (process) bir kaynağa sahip olmadığı bir capability (yetki belgesi) olmadan erişemez.** Bu, sistem çağrılarından IPC'ye, bellek erişiminden cihaz sürücülerine kadar tüm yetki akışını denetlenebilir kılar.

### Capability modeli (FROZEN)

- **Capability ≠ Resource:** Capability, bir *kaynağa erişim yetkisini* taşır (kaynağın kendisi değil). Handle + generation + rights'tan oluşur.
- **Lineage & Epoch:** Her capability, bir üretim zincirinin (lineage) halkasıdır. Revocation, zincir boyunca `epoch` artırılarak tüm alt dalları da geçersiz kılar.
- **Rights modeli:** READ (1) · WRITE (2) · MAP (4) · IO (8) · DMA (16) · TRANSFER (32) · GRANT (64) · DESTROY (128) · EXECUTE (256) · MANAGE (512).
- **Hata kodu katmanları (asla karışmaz):** generation mismatch → `Invalid` · epoch → `Revoked` · rights → `NoRights`.

### İşlem semantiği (STEP 2 FROZEN)

| Operasyon | Tür | Anlamı |
|---|---|---|
| **GRANT** | copy | Aynı yetkiyi kopyalar; parent'a dokunmaz |
| **TRANSFER** | move | Sahipliği aktarır; eski lineage'dan tamamen koparır, alıcı yeni root olur |
| **LEND** | temporal | Geçici ödünç; iptal edilebilir (revoke edilirse alıcı `Revoked` alır) |
| **REVOKE** | recall | Zinciri ve alt dalları geçersiz kılar |
| **CLOSE** | dispose | Yalnızca kendi handle'ını serbest bırakır |

### Syscall yetki köprüsü (Aşama 2)

Dispatcher her sistem çağrısında, çağıran process'in capability tablosundan ilgili right'ı kontrol eder; yetki yoksa **EACCES** döner. Kapsanan çağrılar: read/write/close/lseek (fd), connect/send/recv (socket), fork/exec (process).

### Capability IPC (Aşama 4)

- `CapChannel<M>` — mesaj zarfı (`CapMessage<M>`) üzerinde **capability taşınabilir**.
- Kanal uç noktası (`ObjectKind::Endpoint`): gönderen `WRITE`, alan `READ` hakkına sahiptir.
- `TransferMode::Transfer` (kalıcı devir) / `TransferMode::Lend` (iptal edilebilir — iptal edilirse alıcı `Revoked` alır, payload yine teslim edilir).
- **Sessiz drop YASAK** (FROZEN). Geriye dönük uyumluluk: eski `BlockingChannel` ve `SYSTEM_CHAN` korunur.

---

## 📦 SparkOS 1.0 Özellik Seti (L0–L12)

L0–L12 yol haritası tamamlanarak **SparkOS 1.0 FINAL** kararlılığına ulaşılmıştır.

### Seviye özeti

- **L0–L2:** Bootloader, ekran (VGA/seri), CPU kesmeleri (IDT/GDT), timer, klavye
- **L3–L4:** Bellek yönetimi (paging, frame allocator, heap), kernel allocator
- **L5–L6:** Çoklu görev (async/await, executor, preemptive scheduler), bellek içi + ATA dosya sistemi
- **L7–L8:** Donanım (PCI, RTL8139, ağ yığını), güvenlik (sec_mem, UID/GID)
- **L9–L10:** Kullanıcı modu (Ring 3 `iretq`), ELF64 yükleyici, sistem çağrıları (`int 0x80`)
- **L11–L12:** SMP/ACPI hazırlığı, tip güvenli IPC, userspace API sözleşmesi (`app.rs` / `sysapi.rs`), GUI entegrasyonu

### Kullanıcı uygulama sözleşmesi (Userspace API)

- **Entry point:** `#[no_mangle] pub extern "C" fn _start()`
- **Sistem çağrıları:** `int 0x80` üzerinden (bkz. `src/sysapi.rs`)
  - `SYS_READ (0)` · `SYS_EXIT (1)` · `SYS_OPEN (2)` · `SYS_CLOSE (3)` · `SYS_WRITE (4)` · `SYS_LSEEK (8)`
- **Bellek:** Uygulamalara otomatik 4KB stack; kod/veri segmentleri ELF başlığında belirtilen şekilde Ring 3 haritalanır
- **İskelet:** `src/lib_userspace.rs` — `no_std` kullanıcı uygulaması şablonu; `app::run_app` ile yüklenip çalıştırılır

---

## 🔧 Gereksinimler & Derleme

- Rust **nightly** toolchain (`rust-toolchain.toml`)
- `bootimage` (`cargo install bootimage`)
- **QEMU** (`qemu-system-x86_64`)

```bash
cargo build      # 0 hata
cargo bootimage  # bootimage-sparkos.bin üretir
./run.sh         # QEMU başlat
```

### Host unit testleri (capability + IPC, harici harness)

```bash
scratch/run_cap_tests.sh   # 14/14 — cap invariant + syscall_cap PURE + IPC
```

---

## 🖥️ Kabuk Komutları

| Komut | Açıklama |
|---|---|
| `help` / `yardim` | Komut listesi |
| `clear` | Ekranı temizle |
| `info` | Sistem bilgisi |
| `tick` / `uptime` | Zamanlayıcı sayacı / çalışma süresi |
| `color <renk>` | Yazı rengini değiştir |
| `echo <mesaj>` | Mesaj bas |
| `pwd`, `cd`, `ls`, `mkdir`, `write`, `rm`, `cat` | Dosya/dizin yönetimi |
| `edit` | Tam ekran (nano benzeri) metin editörü |
| `ps` / `kill <pid>` | Süreçleri listele / sonlandır |
| `lspci` | PCI donanımlarını tara |
| `ifconfig` | Ağ kartı ve MAC adresi |
| `ping` | `8.8.8.8`'e ICMP paketi yolla |
| `host <domain>` | DNS ile domain'i IP'ye çevir (UDP) |
| `run_app` | Ring 3 kullanıcı modunda `scratch/hello.elf` çalıştır |
| `gui` | Grafik masaüstünü başlat |
| `disk_write` / `disk_read` | Disk sektörüne yaz / oku |
| `reboot` / `shutdown` | Sistemi yeniden başlat / kapat (QEMU) |
| `panic` | Kernel panic testi |

---

## 📁 Proje Yapısı

```
sparkos/
├── Cargo.toml
├── rust-toolchain.toml
├── .cargo/config.toml
├── run.sh                  # Derle + bootimage + QEMU
├── scratch/                # Ring 3 ELF denemeleri + host test harness
├── docs/
│   ├── EVOLUTION_PLAN_V2.md        # 5 aşamalı capability evrim planı
│   ├── architecture/               # CAPABILITY_MODEL, IPC_CONTRACT, RESOURCE_LIFETIME…
│   └── SPARKOS_STAGE_EVOLUTION_REPORT.md
└── src/
    ├── main.rs             # Giriş: kernel_main + mod wiring
    ├── cap.rs              # Capability core (Aşama 1, FROZEN)
    ├── syscall_cap.rs      # Syscall–capability köprüsü (Aşama 2)
    ├── ipc.rs              # Capability IPC: CapChannel / Transfer / Lend (Aşama 4)
    ├── syscall.rs          # Syscall dispatcher + gating
    ├── syscall_storage.rs  # sys_open/sys_close — capability provision
    ├── net_socket.rs       # sys_socket — capability provision
    ├── sec_mem.rs          # validate_user_ptr — güvenli dilim validasyonu
    ├── app.rs / sysapi.rs / lib_userspace.rs  # Userspace API
    ├── user.rs / elf.rs    # Ring 3 geçişi + ELF64 yükleyici
    └── …                   # bellek, gui, fs, ağ ve diğer çekirdek modülleri
```

---

## 🎯 Yol Haritası (Aşama 5 — Sıradaki)

Capability microkernel'e kalan büyük adım — **microkernel driver izolasyonu**:

1. **IRQ Notification Endpoint:** Sürücülerin user-space'e taşınabilmesi için kernel kesmelerinin IPC bildirimine dönüştürülmesi
2. **Port I/O & MMIO izinleri:** Serial sürücüsü için I/O port (`0x3F8`) veya bellek sayfalarının capability ile sürece bağlanması
3. **Donanımsız servis önce** (ör. `keyboard` / `fb_query`), ilk donanım adayı **serial** (rtl8139 QEMU'da yok)
4. Driver/servis crash → kernel etkilenmeden restart (fault recovery)

> **Not:** Aşama 4 tamamlanmadan Aşama 5'e girilmez (IPC capability tabanlı olmalıdır) — bu kural sağlanmıştır.

---

*Son güncelleme: 2026-08-14 · Aşama 1 → 4 tamamlandı, Aşama 5 bekliyor.*
