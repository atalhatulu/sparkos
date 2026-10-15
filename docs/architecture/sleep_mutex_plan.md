# SparkOS SleepMutex Geçiş Planı (WM ve VFS)

## 1. Giriş ve Gerekçe
Mevcut mimaride `WM` (`WindowManager`), VESA/backbuffer işlemlerini `IrqMutex` altında gerçekleştirmektedir. Bu durum CPU kesmelerini uzun süre kapalı tutarak (`max_irq_off_cycles` zirveleri ~28-34 milyon döngü) sistem gecikmesini (latency) artırmaktadır.
`VFS` ve `FD_TABLE` ise hibrit `Mutex` kullanmakta olup meşgul kilitlerde `schedule()` çağırmaktadır; ancak uyuyan bir `WaitQueue` yerine spin+yield yaklaşımı uygulamaktadır.

`SleepMutex<T>` mekanizması, kilit elde edilemediğinde çağıran iş parçacığını doğrudan `WaitQueue`'ya ekleyip `ProcessState::Blocked(BlockReason::Mutex)` durumuna geçirir ve lost-wakeup koruması sağlar.

## 2. Invariant'lar ve Giriş Assert'leri
`SleepMutex::lock` ve `sync::Mutex::lock` çağrı noktalarında şu şartlar mutlak olarak sağlanmalıdır:
1. `IF == 1` (`x86_64::instructions::interrupts::are_enabled()`): Kilit beklerken uykuya geçileceği için kesmeler açık olmalıdır.
2. `irq_locks_held() == 0`: Hiyerarşi kuralı uyarınca hiçbir `IrqMutex` (SCHEDULER, VgaWriter, vb.) tutulurken uyuyan kilit alınamaz.
3. Donanım kesme (IRQ/ISR) bağlamında çağrılamaz.

## 3. Çağrı Yerleri Analizi
| Alt Sistem | Mevcut Kilit Tipi | Tipik Tutma Süresi | Hedef Kilit Tipi | Dönüşüm Riski / Önlem |
|---|---|---|---|---|
| **WM (Window Manager)** | `IrqMutex<WindowManager>` | 5 ms - 15 ms (Full Composite) | `SleepMutex<WindowManager>` | IRQ işleyicilerinden (mouse/keyboard IRQ) doğrudan `WM.lock()` çağrılmamalı; giriş olayları lockless/IrqMutex halka kuyruğa yazılıp compositor thread'i tarafından `SleepMutex` ile çekilmelidir. |
| **VFS (Sanal Dosya Sistemi)** | `sync::Mutex<FsNode>` | 100 µs - 2 ms (Disk I/O) | `SleepMutex<FsNode>` | ATA disk okumaları sırasında thread'in bloklanması I/O verimini ve yanıt süresini iyileştirir. |
| **FD_TABLE** | `sync::Mutex<FdTable>` | < 10 µs | `SleepMutex<FdTable>` | Kısa süreli tutulduğundan mevcut `Mutex` veya `SleepMutex` ile uyumludur. |

## 4. Uygulama Adımları
1. **WM Composite İzolasyonu:** Ekran çizimi ve dirty-rect kopyalaması backbuffer üzerinde WM kilidinden bağımsız veya yerel çift tampon ile yürütülür.
2. **Event Kuyruğu Ayrımı:** Fare ve klavye kesmeleri `INPUT_QUEUES` (`IrqMutex`) içine atomik basılır; GUI/Compositor thread'i bu kuyruğu tüketirken `SleepMutex` kullanır.
3. **VFS WaitQueue Entegrasyonu:** Disk blok I/O beklemesi doğrudan disk interrupt bildirimine (`ipc::irq_event`) bağlanarak uyuyan kilit mimarisine kavuşturulur.
