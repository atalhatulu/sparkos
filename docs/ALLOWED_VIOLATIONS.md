# İzin Verilen Ters Bağımlılıklar Envanteri (Faz 1.6)

Bu belge, SparkOS mikroçekirdek katmanında (Grup A - Core Kernel), sürücü (Grup B), servis (Grup C) ve masaüstü (Grup D) modüllerine yapılan geçici doğrudan çağrıları (ters bağımlılıkları) listeler.

Bu liste CI kontrol scripti (`tools/check_deps.sh`) tarafından beyaz liste (whitelist) olarak kullanılır. Her faz sonunda bu liste küçülecek ve Faz 5 (Servislerin Ring 3'e taşınması) tamamlandığında tamamen boşalacaktır.

---

## 1. Grup A -> Grup B (Sürücüler) İzinli Bağımlılıklar

| Kaynak Dosya | Hedef Modül | Gerekçe / Mevcut Durum | Çözüleceği Faz |
|---|---|---|---|
| `kernel/src/interrupts.rs` | `crate::mouse` | PS/2 mouse kesme işleyicisi doğrudan sürücüyü çağırıyor | Faz 4 / Faz 5 (User-space input driver) |
| `kernel/src/task/keyboard.rs` | `crate::keyboard` | PS/2 klavye kuyruğu ve tuş durumu sorgulaması | Faz 4 / Faz 5 (User-space input driver) |
| `kernel/src/task/keyboard.rs` | `crate::vga_buffer` | GUI / CLI moduna göre ekran çıktısı yönlendirmesi | Faz 5 (User-space display server) |
| `kernel/src/task/process.rs` | `crate::pci` | Sürücü başlatma sırasında PCI tarama | Faz 5 (PCI bus driver service) |
| `kernel/src/iommu.rs` | `crate::pci` | IOMMU donanım aygıt eşleme için PCI tarama | Faz 2 (Driver isolation) |
| `kernel/src/klog.rs` | `crate::vga_buffer` | Erken çekirdek panik/log ekran çıktısı | Faz 2 (Early console buffer) |
| `kernel/src/syscall.rs` | `crate::vga_buffer` | `SYS_WRITE` stdout (fd 1/2) VGA tamponuna yazdırma | Faz 3 (VFS / FD abstraction) |
| `kernel/src/syscall.rs` | `crate::rtl8139` | `SYS_NET_SEND`/`SYS_NET_RECV` ham paket çağrıları | Faz 5 (Netstack service) |

---

## 2. Grup A -> Grup C (Sistem Servisleri) İzinli Bağımlılıklar

| Kaynak Dosya | Hedef Modül | Gerekçe / Mevcut Durum | Çözüleceği Faz |
|---|---|---|---|
| `kernel/src/syscall.rs` | `crate::net_socket` | `SYS_SOCKET`, `SYS_CONNECT`, `SYS_SEND`, `SYS_RECV` | Faz 5 (IPC socket service) |
| `kernel/src/syscall_storage.rs` | `crate::fs` | SPFS dosya sistemi doğrudan syscall köprüsünde | Faz 3 (VFS & FS service) |
| `kernel/src/fd.rs` | `crate::fs` | Dosya tanımlayıcı tablosu doğrudan FS inode/path çağırıyor | Faz 3 (VFS abstraction) |

---

## 3. Grup A -> Grup D (Masaüstü ve UI) İzinli Bağımlılıklar

| Kaynak Dosya | Hedef Modül | Gerekçe / Mevcut Durum | Çözüleceği Faz |
|---|---|---|---|
| `kernel/src/task/process.rs` | `crate::surface` | Süreç sonlanmasında ve `spawn_live_demo_app`'ta yüzey temizleme/oluşturma | Faz 5 (Compositor service) |
| `kernel/src/task/process.rs` | `crate::wm` | Süreç pencere temizleme ve pencere oluşturma | Faz 5 (Window manager service) |
| `kernel/src/task/process.rs` | `crate::input` | Süreç giriş kuyruğu temizleme | Faz 5 (Input server) |
| `kernel/src/task/process.rs` | `crate::terminal_app` | `spawn_desktop_v1_apps` içinde terminal başlatma | Faz 5 (Ring 3 user apps) |
| `kernel/src/task/process.rs` | `crate::files_app` | `spawn_desktop_v1_apps` içinde dosya yöneticisi başlatma | Faz 5 (Ring 3 user apps) |
| `kernel/src/task/keyboard.rs` | `crate::input` | Klavye giriş olaylarını doğrudan compositor input kuyruğuna iletme | Faz 5 (Input delivery IPC) |
| `kernel/src/syscall.rs` | `crate::surface` | `SYS_CREATE_SURFACE`, `SYS_PRESENT_SURFACE`, `SYS_DESTROY_SURFACE` | Faz 5 (Compositor IPC) |
| `kernel/src/syscall.rs` | `crate::wm` | `SYS_CREATE_WINDOW`, `SYS_DESTROY_WINDOW`, `SYS_MOVE_WINDOW` vb. | Faz 5 (Window manager IPC) |
| `kernel/src/syscall.rs` | `crate::input` | `SYS_POLL_EVENT` doğrudan input kuyruğundan okuma | Faz 5 (Input IPC) |
| `kernel/src/perf.rs` | `crate::wm` | Compositor piksel/pencere çizim sayaçları benchmarkı | Faz 5 (Service metrics IPC) |

---

## 4. Kabul Kriteri
Bu belgede yer almayan yeni bir Grup A -> Grup B/C/D bağımlılığı eklenmesi kesinlikle yasaktır ve `tools/check_deps.sh` tarafından engellenecektir.
