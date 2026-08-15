# SparkOS

Rust ile sıfırdan (`no_std`, `no_main`) yazılmış, **Capability-Based (CSpace)** güvenlik modeline, **Per-Process CR3 İzolasyonuna**, **Intel IOMMU (VT-d) Donanım DMA Korumasına**, **SMP Çok Çekirdekli Work-Stealing Çizelgeleyicisine** ve **Mikroçekirdek Servis Mimarisine** sahip x86_64 işletim sistemi.

> **Mevcut Sürüm:** v0.30.0 (Architecture Freeze #7)  
> **Doğrulama Durumu:** 265 / 265 Host Invariant Testi Başarılı (%100 Geçiş)  
> **Donanım ve Emülasyon:** QEMU `-M q35,kernel-irqchip=split -device intel-iommu -smp 2` / Bare-Metal x86_64

---

## 1. Mimari Genel Bakış

SparkOS, monolitik çekirdeklerin getirdiği güvenlik ve hata yayılımı risklerini ortadan kaldırmak amacıyla katı bir mikroçekirdek (microkernel) felsefesiyle tasarlanmıştır. Çekirdek alanı yalnızca temel bellek yönetimi, capability yönlendirmesi, kesme dağıtımı ve per-CPU iş çizelgelemeden sorumludur.

```text
+-------------------------------------------------------------------------------+
|                             KULLANICI ALANI (RING 3)                          |
|  +-------------+  +-------------+  +-------------+  +-----------------------+ |
|  |   keysvc    |  |   serdrv    |  |   netdrv    |  |        netsvc         | |
|  |  (Klavye)   |  |   (Seri)    |  |  (RTL8139)  |  |     (UDP Soket)       | |
|  +------+------+  +------+------+  +------+------+  +-----------+-----------+ |
|         |                |                |                     |             |
|  +------+----------------+----------------+---------------------+-----------+ |
|  |              Masaüstü & Grafik Katmanı: wm, displaysvc, sh               | |
|  +------------------------------------+-------------------------------------+ |
+---------------------------------------|---------------------------------------+
                                        | (Syscall: IPC, Capability, Shmem)
+---------------------------------------v---------------------------------------+
|                            ÇEKİRDEK ALANI (RING 0)                            |
|  +-------------------------------------------------------------------------+  |
|  |                       Capability & CSpace Sistemi                       |  |
|  |          - Nesne Yaşam Döngüsü, Hiyerarşik İptal (Cascading Revocation) |  |
|  +-------------------------------------------------------------------------+  |
|  +------------------------------------+ +----------------------------------+  |
|  |       SMP Per-CPU Çizelgeleyici     | |      Bellek & Sayfalama          |  |
|  | - Bağımsız Per-CPU Kuyrukları       | | - Per-Process CR3 İzolasyonu     |  |
|  | - Deadlock-Free Work-Stealing      | | - IPI TLB Shootdown (0xFD)       |  |
|  | - Düşük Güç HLT Uyku Yönetimi       | | - Virtual Memory Reclaim         |  |
|  +------------------------------------+ +----------------------------------+  |
|  +------------------------------------+ +----------------------------------+  |
|  |     Intel IOMMU (VT-d) Sürücüsü    | |    Servis & Bağımlılık Yöneticisi|  |
|  | - İkinci Seviye Sayfa Çevirimi (TE)| | - Topolojik Başlatma Sırası      |  |
|  | - DMA Bellek İzolasyonu (Domain 1) | | - Çökme İzolasyonu & Devre Kesici|  |
|  +------------------------------------+ +----------------------------------+  |
+-------------------------------------------------------------------------------+
```

---

## 2. Temel Sistem Yetenekleri

### A. Güvenlik ve Capability Modeli (CSpace)
- **Doğrudan İşaretçi Yasağı:** Kullanıcı alanı süreçleri ham fiziksel veya mantıksal adreslere doğrudan erişemez. Tüm kaynaklar (bellek, DMA alanı, G/Ç portları, IPC uç noktaları) birer `CapHandle` üzerinden korunur.
- **Hiyerarşik İptal (Cascading Revocation):** Bir ana capability iptal edildiğinde (revoke), ondan türetilmiş tüm alt izinler (lending/delegation) anında ve atomik olarak geçersiz kılınır.
- **Port ve Bellek İzolasyonu:** `sys_ioperm` ve `SYS_MAP_DMA` çağrıları capability doğrulamasından geçmeden donanıma erişim sağlayamaz.

### B. Donanımsal DMA İzolasyonu: Intel IOMMU (VT-d)
- **İkinci Seviye Sayfa Tablosu (SLPT):** RTL8139 ağ kartı gibi G/Ç aygıtlarının doğrudan ana belleğe rastgele yazmasını engellemek için 48-bit 4-seviyeli donanımsal sayfa çevirimi aktif edilmiştir (`Translation Enable = 1`).
- **Domain İzolasyonu:** Aygıtlar kendilerine atanmış izole etki alanlarına (Domain 1) kısıtlanır. Yetkisiz DMA erişim denemeleri donanım seviyesinde yakalanır ve çekirdek çökmesi olmadan savuşturulur (`Fault Reason 0x07`).

### C. SMP Çok Çekirdekli Çizelgeleme ve Work-Stealing
- **Bağımsız Per-CPU Kuyrukları:** Çekirdekler arası küresel kilit çekişmesini önlemek için her işlemcinin kendi `PerCpuRunQueue` yapısı bulunur.
- **IPI Tabanlı TLB Shootdown:** Sayfa eşleme iptallerinde `0xFD` IPI vektörüyle tüm AP çekirdeklerinin TLB önbellekleri senkronize olarak temizlenir. ACK spin-wait döngülerinde zaman aşımı koruması bulunur.
- **Deadlock-Free Work-Stealing:** Yerel kuyruğu boşalan işlemci, akran çekirdeklerin kuyruklarından blokajsız `try_lock()` ile kuyruğun arkasından (`pop_back()`) iş çalar. Kuyruk sahibi ise önden (`pop_front()`) çalışarak önbellek yerelliğini korur.
- **Boşta Kalma (Idle HLT):** Sistem genelinde iş kalmadığında çekirdekler meşgul bekleme (busy-spin) yapmaz; doğrudan düşük güç durumuna (`hlt`) geçer.

### D. Servis Yöneticisi ve SPFS v2 Dosya Sistemi
- **Topolojik Başlatma Sıralaması:** Bağımlılık grafı doğrulanarak servisler doğru sırayla ayağa kaldırılır. Döngüsel bağımlılıklar reddedilir.
- **Hata Kurtarma ve Devre Kesici (Flapping Circuit Breaker):** Çöken servisler `always_restart` politikasıyla yeniden başlatılır; kısa sürede art arda çöken servisler için otomatik devre kesici devreye girer.
- **SPFS v2 Dosya Sistemi:** 64-bayt hizalı Inode yapısı, doğrudan ve tek/çift dolaylı (single/double indirect) blok adresleme, POSIX izin bitleri ve atomik blok geri kazanımı.

### E. Geliştirici SDK'sı (`libspark`)
- Kullanıcı alanı uygulamaları için `no_std` uyumlu standart kütüphane (`libspark`).
- ABI başlık dosyaları (`sparkos_abi.h`), bağımsız ELF bağlayıcı scriptleri (`app.ld`) ve `spark` CLI geliştirici aracı.

---

## 3. Mimari Dondurma (Freeze) Geçmişi

| Freeze | Kapsam ve Başlıca Bileşenler | Invariant Sayısı | Durum |
| :--- | :--- | :---: | :---: |
| **Freeze #1** | Çekirdek Güçlendirme, Preemption, CR3/CSpace Teardown, ELF Loader | 50 | Tamamlandı |
| **Freeze #2** | Per-Client IPC, RTL8139 Ring 3 DMA, Terminal STDIO, `waitpid` | 100 | Tamamlandı |
| **Freeze #3** | Display Server, Shmem Surface, Pencere Yöneticisi (Z-Order, Focus) | 145 | Tamamlandı |
| **Freeze #4** | SPFS v2 Storage Engine, Inode v2, Indirect Block Allocation | 175 | Tamamlandı |
| **Freeze #5** | Servis Yöneticisi, Bağımlılık Grafı, Paket Yöneticisi (`pkg`) | 230 | Tamamlandı |
| **Freeze #6** | Intel IOMMU (VT-d) DMA İzolasyonu, Donanımsal Fault Recovery | 258 | Tamamlandı |
| **Freeze #7** | SMP TLB Shootdown, Per-CPU Queues, Work-Stealing, Idle HLT | **265** | **Aktif (Donduruldu)** |

---

## 4. Derleme ve Çalıştırma

### Gereksinimler
- Rust `nightly` derleyicisi (`rust-src`, `llvm-tools-preview` bileşenleri ile)
- `cargo-bootimage` (`cargo install bootimage`)
- `qemu-system-x86_64`

### Derleme
```bash
# Çekirdek imajını derle
cargo bootimage
```

### QEMU ile Çalıştırma (SMP + IOMMU Desteğiyle)
```bash
qemu-system-x86_64 \
  -M q35,kernel-irqchip=split \
  -device intel-iommu \
  -drive format=raw,file=target/x86_64-unknown-none/debug/bootimage-sparkos.bin,index=0,media=disk \
  -drive format=raw,file=disk.img,index=1,media=disk \
  -serial stdio \
  -display none \
  -m 256M \
  -netdev user,id=net0 \
  -device rtl8139,netdev=net0 \
  -smp 2
```

### Invariant Test Paketini Koşma
```bash
cd scratch/cap_test
cargo test -Zbuild-std= --target x86_64-unknown-linux-gnu -- --test-threads=1
```

---

## 5. Dizin Yapısı

```text
sparkos/
├── src/                    # Mikroçekirdek Kaynak Kodları
│   ├── main.rs             # Çekirdek Giriş Noktası ve Boot Dizisi
│   ├── smp.rs              # SMP Başlatma, TLB Shootdown, Work-Stealing
│   ├── iommu.rs            # Intel VT-d IOMMU Sürücüsü ve SLPT Yönetimi
│   ├── cap.rs              # CSpace ve Capability Güvenlik Çekirdeği
│   ├── memory.rs           # Bellek Yönetimi, Sayfa Tabloları, Frame Allocator
│   ├── interrupts.rs       # IDT, LAPIC/IOAPIC ve IPI Kesme Dağıtımı
│   ├── ipc.rs              # Mikroçekirdek Eşzamansız Mesajlaşma Kanalları
│   ├── service.rs          # Servis Bağımlılık Grafı ve Süreç Denetimi
│   ├── fs.rs               # SPFS v2 Dosya Sistemi ve VFS Katmanı
│   ├── wm.rs               # Pencere Yöneticisi ve Olay Yönlendirme
│   └── surface.rs          # Shmem Tabanlı Grafik Yüzey Çizimi
├── sdk/                    # Kullanıcı Alanı Uygulama Geliştirme Kiti
│   ├── libspark/           # Kullanıcı Alanı Standart Kütüphanesi
│   ├── headers/            # C/Rust ABI Başlık Dosyaları (sparkos_abi.h)
│   ├── linker/             # Bağımsız ELF Linker Scripti (app.ld)
│   └── examples/           # Örnek Ring 3 Uygulamaları (sh, cat, echo, ls, wm)
└── TECHNICAL_DEBT.md       # Teknik Borç Takibi ve Çözüm Planları
```

---

## 6. Lisans
Bu proje eğitim, araştırma ve güvenli işletim sistemi tasarımı amacıyla geliştirilmiştir.
