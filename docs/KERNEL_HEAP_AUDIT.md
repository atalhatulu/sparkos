# SparkOS Kernel Heap Audit (Faz 3.1)

Bu denetim raporu, kullanıcı (Ring-3) eylemleri veya sistem çağrıları (syscalls) sonucunda dinamik olarak kernel heap'inde büyüyebilen veri yapılarını, bunları tetikleyen mekanizmaları, olası sızıntı/tükenme risklerini ve getirilmesi gereken üst sınır (kota/havuz) politikalarını tanımlar.

---

## 1. Kullanıcı Tetiklemeli Büyüyebilen Veri Yapıları Envanteri

| Veri Yapısı | Bulunduğu Modül | Tetikleyen Syscall / İşlem | Mevcut Tip | Üst Sınır Var mı? | İlişkili PID | Risk & DoS Senaryosu |
|-------------|-----------------|-----------------------------|------------|-------------------|--------------|----------------------|
| **Cap Slots & Objects** | [`kernel/src/cap.rs`](file:///home/teha/Documents/GitHub/sparkos/kernel/src/cap.rs) (`CoreState`) | `SYS_CAP_CREATE` (20), `SYS_CAP_GRANT` (22), `SYS_CAP_LEND` | `Vec<CapSlot>`, `Vec<CapNode>`, `Vec<CapObject>` | **YOK** (sınırsız `push`) | Çağıran PID | Döngüde milyonlarca capability üreterek kernel heap'ini tüketme (OOM panik). |
| **Process Cap Table** | [`kernel/src/task/process.rs`](file:///home/teha/Documents/GitHub/sparkos/kernel/src/task/process.rs) (`Process.cap_table`) | `seed_new_process`, `SYS_CAP_GRANT`, `SYS_IPC_CREATE_ENDPOINT` | `Vec<(u32, CapHandle)>` | **YOK** (sınırsız `push`) | Hedef Process PID | Bir süreç kendi fd/cap tablosuna sınırsız handle doldurabilir. |
| **IPC Endpoints & Queues** | [`kernel/src/ipc.rs`](file:///home/teha/Documents/GitHub/sparkos/kernel/src/ipc.rs) (`ENDPOINTS`) | `SYS_IPC_CREATE_ENDPOINT` (24) | `BTreeMap<u32, RawCapChannel>` | **YOK** | Oluşturan PID | Sınırsız endpoint oluşturup heap tüketme. |
| **IPC Mesaj Kuyrukları** | [`kernel/src/ipc.rs`](file:///home/teha/Documents/GitHub/sparkos/kernel/src/ipc.rs) (`RawCapChannel.messages`) | `SYS_IPC_SEND` | `VecDeque<CapMsg>` | Kapasite parametresi var ama toplam kanal sayısı sınırsız | Gönderen/Alıcı PID | Çok sayıda endpoint'e mesaj yığma. |
| **Process Table (PCB)** | [`kernel/src/task/process.rs`](file:///home/teha/Documents/GitHub/sparkos/kernel/src/task/process.rs) (`Scheduler.table`) | `SYS_SPAWN`, `spawn_user_process`, `fork` | `BTreeMap<u64, Process>` | **YOK** | Ebeveyn PID | Fork-bomb veya spawn döngüsü ile PCB ve kernel stack (16 KB) tüketimi. |
| **Process List & Killed List** | [`kernel/src/task/mod.rs`](file:///home/teha/Documents/GitHub/sparkos/kernel/src/task/mod.rs) | Process oluşturma ve sonlandırma | `BTreeMap<u64, String>`, `Vec<u64>` | **YOK** | Global | Her süreç oluşturulduğunda isim String'i ve PID heap'e eklenir; reaped edilmeyenler birikir. |
| **Surfaces & Framebuffer Slots** | [`kernel/src/surface.rs`](file:///home/teha/Documents/GitHub/sparkos/kernel/src/surface.rs) | `SYS_CREATE_SURFACE` (40) | `BTreeMap<u64, Surface>` | Kısmi kota kontrolü var | Sahip PID | Büyük çözünürlüklü yüzeyler tahsis edilerek fiziksel bellek ve heap tüketimi. |
| **Window List** | [`kernel/src/wm.rs`](file:///home/teha/Documents/GitHub/sparkos/kernel/src/wm.rs) (`WindowManager.windows`) | `SYS_CREATE_WINDOW` (41) | `Vec<Window>` | **YOK** | Sahip PID | Sınırsız sayıda pencere oluşturarak WM çizim döngüsünü ve heap'i boğma. |
| **Input Event Queues** | [`kernel/src/input.rs`](file:///home/teha/Documents/GitHub/sparkos/kernel/src/input.rs) (`INPUT_QUEUES`) | Pencere açılması ve klavye/fare girişleri | `BTreeMap<u64, EventQueue>` | Kuyruk başı 64 olay var ama pencere başına 1 kuyruk | Pencere sahibi PID | Okunmayan pencere kuyruklarında birikme. |
| **COW Mapping Registry** | [`kernel/src/memory.rs`](file:///home/teha/Documents/GitHub/sparkos/kernel/src/memory.rs) (`COW_PAGES`) | `fork` veya COW bellek eşleme | `BTreeMap<(u64, u64), PhysAddr>` | **YOK** | İlgili PID | Her sayfa kopyasında BTreeMap düğümü tahsis edilmesi. |

---

## 2. Faz 3 Düzeltme Stratejisi

1. **Sabit Boyutlu Havuzlar (Slab / Fixed Array):**
   - `cap.rs`: `CoreState` içindeki `Vec<CapSlot>`, `Vec<CapNode>`, `Vec<CapObject>` sabit boyutlu statik/slab dizilere (`MAX_CAP_SLOTS = 2048`, `MAX_CAP_OBJECTS = 1024`) taşınacak. Kapasite dolduğunda panik yerine `CapError::OutOfSlots` dönecektir.
   - `ipc.rs`: `ENDPOINTS` tablosu `MAX_ENDPOINTS = 256` sabit array/slab yapısına dönüştürülecektir.
   - `process.rs`: `MAX_PROCESSES = 64` üst sınırı uygulanacaktır.
2. **Process Resource Limits (Hiyerarşik Kota):**
   - Her `Process` struct'ı `ResourceLimits { max_kernel_bytes, max_caps, max_endpoints, max_surfaces }` ile sınırlanacak.
   - Her syscall'da ilgili kaynağın kotası `try_charge_*` ile denetlenecek, aşıldığında `EQUOTA` (veya `ENOMEM`) dönülecektir.
3. **MemObject Soyutlaması:**
   - Dağınık `surface`, `dma_region`, ELF segment ve kullanıcı stack eşlemeleri tek tip `MemObject` cap altına alınarak kota doğrudan fiziksel sayfa havuzuna bağlanacaktır.
4. **POSIX Temizliği:**
   - Kullanılmayan `SYS_FORK` devreden çıkarılacak, `COW_PAGES` karmaşası kaldırılacaktır.
