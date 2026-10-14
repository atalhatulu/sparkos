# Faz 4 Rev 2: SMP Çok Çekirdek, Süreç Havuzu ve Ölçeklenebilirlik Tasarım Notu

Bu doküman, SparkOS mikroçekirdeğinde tek çekirdekli global `SCHEDULER` kilidinden çok çekirdekli (SMP) ölçeklenebilir mimariye geçiş için B1–B8 maddelerini ve teknik kabul kriterlerini tanımlar.

---

## 1. Tasarım Maddeleri (B1 — B8)

### B1. Süreç Durumu ve WaitQueue Kilitleri (Per-Process / Fine-Grained Locks)
- **Problem:** Mevcut modelde tüm thread durum geçişleri (`Ready`, `Running`, `Blocked`) ve `WaitQueue` işlemleri tek bir global `SCHEDULER: IrqMutex<Scheduler>` altında kilitlenmektedir. Bu durum çok çekirdekte yoğun lock contention yaratır.
- **Tasarım:** 
  - `ProcessState` ve `BlockReason` doğrudan PCB seviyesinde bağımsız atomik veya `IrqSpinlock` ile korunur.
  - Her `WaitQueue`, global zamanlayıcıdan bağımsız kendi iç kilit mekanizmasına kavuşturulur; böylece bir kuyruğa giriş/çıkış diğer CPU'ların zamanlama kararlarını kilitlemez.

### B2. `on_cpu` Handshake & `finish_switch`
- **Problem:** Bir iş parçacığı bir CPU üzerinde çalışırken (`Running`), zamanlayıcı tarafından diğer CPU'ya aktarılmak istendiğinde bağlamın eski CPU tarafından belleğe tam yazıldığından emin olunmalıdır (cross-CPU state race).
- **Tasarım & Durum:**
  - `on_cpu: Option<usize>` alanı korunur.
  - Bir CPU bağlam değişimini (`switch_context`) tamamladıktan sonra `finish_switch` aşamasında eski sürecin `on_cpu` kaydını temizler (`Release` semantiği).
  - Başka bir CPU, ilgili sürecin `on_cpu == None` olduğunu görmeden (`Acquire` semantiği) süreci `Running` yapamaz ve yürütemez.

### B3. Sabit Slotlu Süreç Havuzu (`[ProcessSlot; MAX_PROCESSES]`)
- **Problem:** `Scheduler.table: BTreeMap<u64, Process>` dinamik heap tahsisi (`alloc::collections::BTreeMap`) gerektirir. Süreç ekleme (`insert`) veya çıkarma (`remove`) sırasında heap kilidi alınır ve bellek parçalanması (fragmentation) riski doğar.
- **Tasarım:**
  - `Scheduler`, heap tahsisini sıfırlamak üzere sabit boyutlu bir diziye geçirilir:
    ```rust
    pub struct ProcessSlot {
        pub in_use: bool,
        pub generation: u32,
        pub process: Process,
    }
    pub static PROCESS_TABLE: [IrqMutex<ProcessSlot>; MAX_PROCESSES] = ...;
    ```
  - PID üretimi `(slot_index as u64) | ((generation as u64) << 16)` formatına çevrilerek $O(1)$ doğrudan erişim sağlanır.
  - Referanslar ham işaretçiler yerine kararlı slot indeksleri üzerinden çözümlenir; PCB taşınması engellenir.

### B4. Tahsissiz Hazır Kuyruğu (Zero-Allocation Intrusive RunQueue)
- **Problem:** `VecDeque<u64>` kuyruğu büyüdükçe heap tahsisi yapar.
- **Tasarım:**
  - Her PCB içerisine `run_next: Option<u64>` ve `run_prev: Option<u64>` alanları eklenir.
  - Per-CPU hazır kuyruğu (`IntrusiveRunQueue`), hazır durumdaki süreçleri bu göstericiler üzerinden $O(1)$ ekler ve çıkarır. Çekirdek zamanlayıcısı çalışma anında sıfır heap tahsisi ile çalışır.

### B5. Cross-Core TLB Shootdown (Faz 4.5)
- **Problem:** Bir CPU bir sayfa tablosu girdisini (PTE) değiştirdiğinde (`munmap`, `mprotect`, `COW`), aynı adres alanını paylaşan veya eşlemeyi kullanan diğer çekirdeklerin TLB önbelleği güncel kalmaz.
- **Tasarım:**
  - `TLB_SHOOTDOWN_IPI` donanımsal kesme vektörü tanımlanır.
  - Sayfa değişikliğini yapan CPU, ilgili CR3'ü çalıştıran hedef çekirdeklere APIC üzerinden IPI gönderir.
  - Atomik bekleme sayacı (`AtomicUsize` ACK) ile diğer çekirdeklerin `invlpg` tamamladığı doğrulanır; 100.000 döngü zaman aşımı koruması ile donanım kilitlenmeleri önlenir.

### B6. AP Başlatma Protokolü
- **Tasarım & Durum (Uygulandı):**
  - SIPI/APIC üzerinden Application Processor (AP) çekirdekleri güvenli biçimde başlatılır.
  - Trampoline alanı çekirdek-özel eşleme ile izole edilir (`map_kernel_phys_range`), `EFER.NXE` ve `CR4.SMEP` aktif edilir.
  - AP çekirdeği uzun moda geçip hazır olduğunu bildirdikten sonra düşük bellekli trampoline eşlemeleri CR3'ten derhal kaldırılır (`unmap_kernel_phys_range`).

### B7. `IrqRwLock` Desteği
- **Problem:** `SURFACE_REGISTRY`, `CSpace` ve dosya tanımlayıcıları gibi veri yapıları %90+ okuma, %10 yazma yüküne sahiptir. Spin kilidi okuyucuları da gereksiz yere sıralar.
- **Tasarım:**
  - Çok okuyucu / tek yazıcı destekli, kesme güvenli `IrqRwLock<T>` ilkel yapısı kullanılır.
  - Okuma sırasında kesmeler kontrol altında tutulurken birden fazla CPU aynı anda veri yapılarını eşzamanlı sorgular.

### B8. Kabul Kriterleri ve Başarım Hedefleri
1. **SMP Ölçeklenebilirlik:** 4 çekirdekli QEMU SMP ortamında hesaplama ve IPC iş yükünde `smp_scale_x >= 3.0x` doğrusal hızlanma sağlanmalıdır.
2. **IPC RTT Gecikmesi:** Çekirdekler arası mesajlaşma gidiş-dönüş süresi (`ipc_rtt_cycles`) mevcut global kilitli tabana göre en az %30 düşmelidir.
3. **Sıfır Çekirdek Heap Tahsisi:** Çalışma zamanında (boot tamamlandıktan sonra) thread değişimi, uykuya dalma ve hazır kuyruğu işlemlerinde 0 bayt dinamik heap tahsisi (`alloc::alloc`) garantilenmelidir.
4. **Regresyon Bütünlüğü:** Tüm mevcut regresyon testleri (24/24 PASS) ve 10 saniyelik deadlock stress testi sıfır kilitlenme ile geçmelidir.
