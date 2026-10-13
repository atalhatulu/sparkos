# SparkOS Kilit Envanteri ve Kilit Disiplini

Bu doküman, Faz 0.3 kapsamında çekirdekteki tüm statik eşzamanlılık kilitlerinin (`Mutex`, `RwLock`, `Spinlock`) envanterini, IRQ erişim durumlarını ve kilit hiyerarşisi seviyelerini belgeler.

## 1. Kilit Sırası Hiyerarşisi

Deadlock oluşumunu engellemek için kilitler yalnızca hiyerarşide yukarıdan aşağıya (düşük seviye numarasından yüksek seviye numarasına) doğru alınabilir. Ters sırada kilit edinimi KESİNLİKLE YASAKTIR.

```
Seviye 1: Süreç ve Çizelgeleyici (ProcessTable / Scheduler)
   │
   ▼
Seviye 2: İş Kuyrukları ve İşlemci Durumu (RunQueue / CPU State)
   │
   ▼
Seviye 3: Yetki Tabloları ve Güvenlik (CapTable / Security)
   │
   ▼
Seviye 4: İletişim Kanalları ve Tutamaçlar (IPC / FD / Sockets)
   │
   ▼
Seviye 5: Bellek, Dosya Sistemi ve Ham Donanım (Memory / VFS / Drivers)
   │
   ▼
Seviye 6: Masaüstü ve Pencere Yöneticisi (WM / Surfaces / Apps)
   │
   ▼
Seviye 7: Seri Port ve Günlükleme (Serial / VGA / Logs)
```

## 2. Kilit Envanter Tablosu

| Kilit Adı | Dosya | Tür | IRQ Erişimi (Doğrudan/Dolaylı) | Alındığı Bağlamlar | Sıra Seviyesi | IrqMutex Gereksinimi |
|---|---|---|---|---|:---:|:---:|
| `SCHEDULER` | `src/task/process.rs` | `Mutex<Scheduler>` | Evet (Timer IRQ `timer_tick`) | Timer, Syscall, Thread switch, Process spawn/exit | 1 | **ZORUNLU** |
| `PER_CPU_SCHEDULER` | `src/task/process.rs` | `spin::Mutex<[..]>` | Evet (Timer IRQ) | SMP çizelgeleme, tick | 1 | **ZORUNLU** |
| `EXECUTOR_RESUME` | `src/task/process.rs` | `Mutex<Option<Regs>>` | Evet (Fault/Timer recovery) | Servis exit, fault kurtarma | 1 | **ZORUNLU** |
| `PROCESS_LIST` | `src/task/mod.rs` | `Mutex<BTreeMap<..>>` | Hayır | Syscall, Task yöneticisi | 1 | Opsiyonel |
| `KILLED_PROCESSES` | `src/task/mod.rs` | `Mutex<Vec<u64>>` | Evet (Fault recovery) | `exit_current`, fault recovery | 1 | **ZORUNLU** |
| `RUN_QUEUES` | `src/smp.rs` | `[Mutex<RunQueue>; 8]` | Evet (Reschedule IPI/Timer) | SMP work stealing, enqueue/dequeue | 2 | **ZORUNLU** |
| `CPU_STATES` | `src/smp.rs` | `Mutex<[PerCpuState]>` | Evet (Timer/Watchdog) | Heartbeat, APIC keşfi | 2 | **ZORUNLU** |
| `TLB_SHOOTDOWN_LOCK`| `src/smp.rs` | `Mutex<()>` | Evet (TLB IPI) | Sayfa unmap, adres alanı temizliği | 2 | **ZORUNLU** |
| `BSP_LAPIC` | `src/smp.rs` | `Mutex<Option<LocalApic>>`| Hayır | Boot init, timer kalibrasyonu | 2 | Standart |
| `STATE` (Cap Core)| `src/cap.rs` | `Mutex<Option<CoreState>>`| Evet (Timer `expire_lent_capabilities` -> `revoke`) | Cap create, transfer, lend, grant, revoke, syscall | 3 | **ZORUNLU** |
| `LEND_EXPIRIES` | `src/cap.rs` | `Mutex<BTreeMap<..>>` | Evet (Timer IRQ) | `lend`, `expire_lent_capabilities` | 3 | **ZORUNLU** |
| `DEVICE_PORT_BINDINGS`| `src/cap.rs` | `Mutex<BTreeMap<..>>` | Hayır | `create_device_ports`, `sys_ioperm` | 3 | Standart |
| `DMA_REGIONS` | `src/dma_region.rs` | `Mutex<BTreeMap<..>>` | Hayır | DMA alloc, free, IOMMU map | 3 | Standart |
| `SLOT_MAP` | `src/dma_region.rs` | `Mutex<BTreeMap<..>>` | Hayır | DMA yetki denetimi | 3 | Standart |
| `SECURITY` | `src/security.rs` | `Mutex<SecurityManager>` | Hayır | Güvenlik politikaları | 3 | Standart |
| `AUTH_MANAGER` | `src/auth.rs` | `Mutex<AuthManager>` | Hayır | Kullanıcı kimlik doğrulama | 3 | Standart |
| `PERMISSION_MANAGER`| `src/permission.rs` | `Mutex<PermissionManager>` | Hayır | Uygulama izinleri | 3 | Standart |
| `ENDPOINTS` | `src/ipc.rs` | `Mutex<BTreeMap<..>>` | Hayır (Lock-free ArrayQueue kullanılır) | IPC send, recv, endpoint create | 4 | Standart |
| `ENDPOINT_OWNERS` | `src/ipc.rs` | `Mutex<BTreeMap<..>>` | Hayır | Endpoint transfer, mülkiyet sorgusu | 4 | Standart |
| `IRQ_BINDINGS` | `src/ipc.rs` | `Mutex<BTreeMap<..>>` | Hayır (IRQ tetiklemesi `IRQ_EVENTS` atomik kuyruğunda) | `bind_irq`, `unbind_irq` | 4 | Standart |
| `FD_TABLE` | `src/fd.rs` | `Mutex<FdTable>` | Hayır | Syscall open/close/read/write | 4 | Standart |
| `SOCKET_TABLE` | `src/net_socket.rs` | `Mutex<SocketTable>` | Hayır | Ağ soket syscall'ları | 4 | Standart |
| `INPUT_QUEUES` | `src/input.rs` | `Mutex<BTreeMap<..>>` | Evet (Mouse/Keyboard yönlendirme) | Girdi olay dağıtımı | 4 | **ZORUNLU** |
| `KERNEL_INPUT_OWNERS`| `src/input.rs` | `Mutex<BTreeSet<..>>` | Evet (Input dispatch) | Giriş odağı | 4 | **ZORUNLU** |
| `EVENT_BUS` | `src/event.rs` | `Mutex<EventBus>` | Hayır | Sistem olay abonelikleri | 4 | Standart |
| `USER_FRAME_ALLOC`| `src/memory.rs` | `Mutex<Option<FramePool>>`| Evet (Page fault CoW işleme) | User frame alloc, free, stats | 5 | **ZORUNLU** |
| `COW_PAGES` | `src/memory.rs` | `spin::Mutex<BTreeMap<..>>`| Evet (Page fault handler CoW yolu) | CoW fault, register_cow_page | 5 | **ZORUNLU** |
| `KERNEL_HEAP_RANGE`| `src/allocator.rs` | `spin::Mutex<(u64, u64)>` | Hayır | Boot heap init | 5 | Standart |
| `VFS` | `src/fs.rs` | `Mutex<FsNode>` | Hayır | Dosya sistemi işlemleri | 5 | Standart |
| `BLOCK_CACHE` | `src/fs.rs` | `Mutex<BlockCache>` | Hayır | Disk blok önbelleği | 5 | Standart |
| `BLOB_STORE` | `src/fs.rs` | `Mutex<Vec<..>>` | Hayır | Basit ikili depolama | 5 | Standart |
| `DATA_DRIVE` | `src/ata.rs` | `Mutex<AtaDrive>` | Hayır | ATA PIO disk okuma/yazma | 5 | Standart |
| `IOMMU_DOMAIN1_PML4`| `src/iommu.rs` | `Mutex<Option<u64>>` | Hayır | IOMMU sayfa tablosu | 5 | Standart |
| `ARP_CACHE` | `src/net.rs` | `Mutex<Vec<ArpEntry>>` | Hayır | ARP çözümleme | 5 | Standart |
| `KEYBOARD` | `src/keyboard.rs` | `Mutex<Keyboard>` | Hayır (Klavye IRQ'su ArrayQueue'ye yazar) | Kullanıcı alanı okuma | 5 | Standart |
| `MOUSE` | `src/mouse.rs` | `Mutex<MouseState>` | Evet (Mouse IRQ 12) | Mouse interrupt işleme | 5 | **ZORUNLU** |
| `CURSOR` | `src/cursor.rs` | `Mutex<CursorState>` | Evet (Mouse IRQ 12) | Mouse imleç koordinat güncellemesi | 5 | **ZORUNLU** |
| `SURFACE_REGISTRY`| `src/surface.rs` | `RwLock<Vec<SurfaceInfo>>` | Hayır | Yüzey tahsisi, serbest bırakma | 6 | Standart |
| `WM` | `src/wm.rs` | `Mutex<WindowManager>` | Evet (Clock task / Hud damage / Mouse click) | Pencere yönetimi, kompozisyon | 6 | **ZORUNLU** |
| `GPU_MANAGER` | `src/gpu.rs` | `Mutex<GpuManager>` | Hayır | GPU arayüzü | 6 | Standart |
| `VIRTIO_GPU` | `src/gpu_virtio.rs` | `Mutex<VirtioGpuDevice>` | Hayır | Virtio GPU MMIO/PCI | 6 | Standart |
| `APP_MANAGER` | `src/app_manager.rs` | `Mutex<ApplicationManager>`| Hayır | Uygulama yaşam döngüsü | 6 | Standart |
| `NOTIFICATION_MANAGER`| `src/notification.rs` | `Mutex<NotificationManager>`| Hayır | Bildirim servisi | 6 | Standart |
| `CLIPBOARD` | `src/clipboard.rs` | `Mutex<Clipboard>` | Hayır | Pano verisi | 6 | Standart |
| `THEME_MANAGER` | `src/theme.rs` | `Mutex<ThemeManager>` | Hayır | Masaüstü teması | 6 | Standart |
| `FONT_MANAGER` | `src/font.rs` | `Mutex<FontManager>` | Hayır | Yazı tipi render | 6 | Standart |
| Uygulama Durumları (`*_INSTANCES`)| `src/*_app.rs` | `Mutex<BTreeMap<..>>` | Hayır | Uygulama yerel durumları | 6 | Standart |
| `SYSTEM_BAR` | `src/system_bar.rs` | `Mutex<SystemBar>` | Hayır | Sistem çubuğu render | 6 | Standart |
| `DESKTOP_ENV` | `src/desktop.rs` | `Mutex<DesktopEnvironment>` | Hayır | Masaüstü durumu | 6 | Standart |
| `WRITE_LOCK` | `src/vga_buffer.rs` | `spin::Mutex<VgaWriter>` | Evet (Panic, Clock task, VGA log) | Ekrana yazı yazma | 7 | **ZORUNLU** |
| `SERIAL` | `src/serial.rs` | `Mutex<Option<SerialPort>>`| Evet (Panic dump, debug logları) | COM1 seri port çıktısı | 7 | **ZORUNLU** |
| `TRACE_RING` | `src/ktrace.rs` | `Mutex<RingBuffer>` | Evet (User-fault loglama) | Çekirdek izleme halkası | 7 | **ZORUNLU** |
| `CRASH_REPORTER` | `src/crash_reporter.rs` | `Mutex<CrashReporter>` | Evet (Panic crash dump) | Çökme kaydı | 7 | **ZORUNLU** |
| `STATS` (Perf) | `src/perf.rs` | `spin::Mutex<[Histogram]>` | Hayır | F9/F10 performans profilleme | 7 | Standart |

## 3. Bulgular ve Risk Değerlendirmesi

1. **Deadlock Riski Yüksek Olan Kilitler:**
   - `SCHEDULER`, `STATE` (Cap Core), `LEND_EXPIRIES`: Timer IRQ doğrudan bu kilitleri alır. Preemption açıkken veya çekirdek iş parçacığı bu kilidi tutarken kesme gelmesi doğrudan deadlock üretir.
   - `MOUSE` ve `CURSOR`: IRQ 12 her fare paketinde doğrudan `MOUSE.lock()` ve `CURSOR.lock()` çağırır.
   - `WRITE_LOCK` ve `SERIAL`: Çekirdek panik veya fault anında IRQ içinden seri port veya VGA kilidine eriştiğinde, ana iş parçacığı log yazarken kesilirse deadlock olur.
2. **Sonuç:**
   - Tabloda "ZORUNLU" olarak işaretlenen tüm kilitler, Faz 0.4'te yazılacak `IrqMutex<T>` ile korunmalıdır.
