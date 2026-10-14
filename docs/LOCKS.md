# SparkOS Kilit Envanteri ve Kilit Disiplini

Bu doküman, çekirdekteki tüm statik eşzamanlılık kilitlerinin (`IrqMutex`, `Spinlock`, `RwLock`) envanterini, IRQ erişim durumlarını ve kilit hiyerarşisi seviyelerini belgeler.

## 1. Kilit Sırası Hiyerarşisi

Deadlock oluşumunu engellemek için kilitler yalnızca hiyerarşide yukarıdan aşağıya (düşük seviye numarasından yüksek seviye numarasına) doğru alınabilir. Ters sırada kilit edinimi KESİNLİKLE YASAKTIR.

**ÖNEMLİ KURAL (SCHEDULER YAPRAK KİLİT):**
`SCHEDULER` kilit hiyerarşisinde en dipteki **YAPRAK (LEAF)** kilittir (Seviye 7). `SCHEDULER` kilidi tutulurken başka hiçbir kilit (`IrqMutex`, `SpinMutex`, vb.) ALINAMAZ. `schedule()` çağrısı girişinde bu invaryant `debug_assert_eq!(irq_locks_held(), 0)` ile donanımsal/yazılımsal olarak denetlenir.

```
Seviye 1: Günlükleme ve Çıktı (Serial / VGA Write Lock / Logs)
   │
   ▼
Seviye 2: Masaüstü ve Pencere Yöneticisi (WM / Surfaces / Apps)
   │
   ▼
Seviye 3: Bellek, Dosya Sistemi ve Donanım (Memory / FramePool / VFS / Drivers / Mouse)
   │
   ▼
Seviye 4: İletişim Kanalları ve Girdi Dağıtımı (IPC Endpoints / Input Queues / Sockets / FD)
   │
   ▼
Seviye 5: Yetki Tabloları ve Güvenlik (Cap Core State / Lend Expiries / Security)
   │
   ▼
Seviye 6: İşlemci Durumu ve SMP İş Kuyrukları (CPU State / RunQueue)
   │
   ▼
Seviye 7: Çizelgeleyici ve İş Parçacığı Tablosu (SCHEDULER - YAPRAK KİLİT)
          [En son alınır; elde tutulurken hiçbir kilit alınamaz!]
```

## 2. Kilit Envanter Tablosu

| Kilit Adı | Dosya | Tür | IRQ Erişimi (Doğrudan/Dolaylı) | Alındığı Bağlamlar | Sıra Seviyesi | IrqMutex Durumu |
|---|---|---|---|---|:---:|:---:|
| `WRITE_LOCK` | `src/vga_buffer.rs` | `IrqMutex<VgaWriter>` | Evet (Panic, Clock task, VGA log) | Ekrana yazı yazma | 1 | **ZORUNLU (IrqMutex)** |
| `SERIAL` | `src/serial.rs` | `IrqMutex<Option<SerialPort>>`| Evet (Panic dump, debug logları) | COM1 seri port çıktısı | 1 | **ZORUNLU (IrqMutex)** |
| `TRACE_RING` | `src/ktrace.rs` | `IrqMutex<RingBuffer>` | Evet (User-fault loglama) | Çekirdek izleme halkası | 1 | **ZORUNLU (IrqMutex)** |
| `CRASH_REPORTER` | `src/crash_reporter.rs` | `IrqMutex<CrashReporter>` | Evet (Panic crash dump) | Çökme kaydı | 1 | **ZORUNLU (IrqMutex)** |
| `WM` | `src/wm.rs` | `IrqMutex<WindowManager>` | Evet (Mouse click, desktop damage) | Pencere yönetimi, kompozisyon | 2 | **ZORUNLU (IrqMutex)** |
| `SURFACE_REGISTRY`| `src/surface.rs` | `RwLock<Vec<SurfaceInfo>>` | Hayır | Yüzey tahsisi, serbest bırakma | 2 | Standart |
| `GPU_MANAGER` | `src/gpu.rs` | `IrqMutex<GpuManager>` | Hayır | GPU arayüzü | 2 | Standart |
| `VIRTIO_GPU` | `src/gpu_virtio.rs` | `IrqMutex<VirtioGpuDevice>` | Hayır | Virtio GPU MMIO/PCI | 2 | Standart |
| `APP_MANAGER` | `src/app_manager.rs` | `IrqMutex<ApplicationManager>`| Hayır | Uygulama yaşam döngüsü | 2 | Standart |
| `USER_FRAME_ALLOC`| `src/memory.rs` | `IrqMutex<Option<FramePool>>`| Evet (Page fault CoW işleme) | User frame alloc, free, stats | 3 | **ZORUNLU (IrqMutex)** |
| `COW_PAGES` | `src/memory.rs` | `IrqMutex<BTreeMap<..>>` | Evet (Page fault handler CoW yolu) | CoW fault, register_cow_page | 3 | **ZORUNLU (IrqMutex)** |
| `VFS` | `src/fs.rs` | `IrqMutex<FsNode>` | Hayır | Dosya sistemi işlemleri | 3 | Standart |
| `BLOCK_CACHE` | `src/fs.rs` | `IrqMutex<BlockCache>` | Hayır | Disk blok önbelleği | 3 | Standart |
| `DATA_DRIVE` | `src/ata.rs` | `IrqMutex<AtaDrive>` | Hayır | ATA PIO disk okuma/yazma | 3 | Standart |
| `MOUSE` | `src/mouse.rs` | `IrqMutex<MouseState>` | Evet (Mouse IRQ 12) | Mouse interrupt işleme | 3 | **ZORUNLU (IrqMutex)** |
| `CURSOR` | `src/cursor.rs` | `IrqMutex<CursorState>` | Evet (Mouse IRQ 12) | Mouse imleç koordinat güncellemesi | 3 | **ZORUNLU (IrqMutex)** |
| `ENDPOINTS` | `src/ipc.rs` | `IrqMutex<BTreeMap<..>>` | Evet (Syscall / IRQ notify) | IPC send, recv, endpoint create | 4 | **ZORUNLU (IrqMutex)** |
| `ENDPOINT_OWNERS` | `src/ipc.rs` | `IrqMutex<BTreeMap<..>>` | Hayır | Endpoint transfer, mülkiyet sorgusu | 4 | **ZORUNLU (IrqMutex)** |
| `IRQ_BINDINGS` | `src/ipc.rs` | `IrqMutex<BTreeMap<..>>` | Hayır (IRQ tetiklemesi `IRQ_EVENTS` atomik kuyruğunda) | `bind_irq`, `unbind_irq` | 4 | **ZORUNLU (IrqMutex)** |
| `INPUT_QUEUES` | `src/input.rs` | `IrqMutex<BTreeMap<..>>` | Evet (Mouse/Keyboard yönlendirme) | Girdi olay dağıtımı | 4 | **ZORUNLU (IrqMutex)** |
| `KERNEL_INPUT_OWNERS`| `src/input.rs` | `IrqMutex<BTreeSet<..>>` | Evet (Input dispatch) | Giriş odağı | 4 | **ZORUNLU (IrqMutex)** |
| `FD_TABLE` | `src/fd.rs` | `IrqMutex<FdTable>` | Hayır | Syscall open/close/read/write | 4 | Standart |
| `SOCKET_TABLE` | `src/net_socket.rs` | `IrqMutex<SocketTable>` | Hayır | Ağ soket syscall'ları | 4 | Standart |
| `STATE` (Cap Core)| `src/cap.rs` | `IrqMutex<Option<CoreState>>`| Evet (Timer `expire_lent_capabilities` -> `revoke`) | Cap create, transfer, lend, grant, revoke, syscall | 5 | **ZORUNLU (IrqMutex)** |
| `LEND_EXPIRIES` | `src/cap.rs` | `IrqMutex<BTreeMap<..>>` | Evet (Timer IRQ) | `lend`, `expire_lent_capabilities` | 5 | **ZORUNLU (IrqMutex)** |
| `RUN_QUEUES` | `src/smp.rs` | `[IrqMutex<RunQueue>; 8]` | Evet (Reschedule IPI/Timer) | SMP work stealing, enqueue/dequeue | 6 | **ZORUNLU (IrqMutex)** |
| `CPU_STATES` | `src/smp.rs` | `IrqMutex<[PerCpuState]>` | Evet (Timer/Watchdog) | Heartbeat, APIC keşfi | 6 | **ZORUNLU (IrqMutex)** |
| `PER_CPU_SCHEDULER`| `src/task/process.rs`| `IrqMutex<[..]>` | Evet (Timer IRQ) | SMP çizelgeleme, tick | 6 | **ZORUNLU (IrqMutex)** |
| `SCHEDULER` | `src/task/process.rs` | `IrqMutex<Scheduler>` | Evet (Timer IRQ `timer_tick`) | Thread switch, process spawn/exit, sleep queue | 7 | **ZORUNLU (IrqMutex - YAPRAK)** |

## 3. Bulgular ve Güvenlik İlkeleri

1. **SCHEDULER Yaprak Kilit İlkesi:**
   `SCHEDULER` kilidi tutulduğu anda başka hiçbir alt sistem kilidi alınamaz. Bu nedenle `exit_current()` içindeki kaynak temizleme işlemleri (`destroy_process_cspace`, `hangup_channel_for_pid`, adres alanı imhası) `SCHEDULER` kilidi bırakıldıktan sonra çağrılmalıdır.
2. **Kayıp Uyanma ve Kilit Sıralaması:**
   Koşul kilitleri (`IrqMutex`) tutulurken `SCHEDULER.lock()` alınarak süreç `WaitQueue`'ya kaydedilir (`prepare_wait`), ardından önce `SCHEDULER`, sonra koşul kilidi bırakılır ve `schedule()` çağrılır. Bu sıra hiyerarşiye (Seviye 2/3/4 -> Seviye 7) tam uyumludur.
3. **Assert Doğrulaması:**
   `schedule()` fonksiyonuna giriş yapıldığında `irq_locks_held() == 0` olduğu assert edilerek yaprak kilit kuralının çiğnenmediği her context switch'te matematiksel olarak doğrulanır.
