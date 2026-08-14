# SparkOS

Rust ile sıfırdan (`no_std`, `no_main`) yazılmış, **Capability-Based (CSpace)** güvenlik modeline, **Per-Process CR3 İzolasyonuna**, **Per-CPU TSS/GDT Donanım Desteğine** ve **Mikroçekirdek Servis Mimarisine** sahip modern bir **x86_64 İşletim Sistemi**.

> **Mevcut Sürüm:** **v0.1.0 (Stabil Çekirdek & Kullanıcı Alanı Platformu)**  
> **Doğrulama:** **119 / 119 Host-Side Invariant Testi Başarılı (%100 Geçiş)** · **QEMU `-smp 2` ve `-smp 4` Çok Çekirdekli Boot Doğrulandı**  
> **Yeni Strateji:** *"Minik ama Gerçek SparkOS"* — Karmaşık ve devasa mimari hedefler yerine; küçük, anlaşılır, stabil ve uçtan uca kullanılabilir bir işletim sistemi.

---

## 🎯 Proje Hedefi: "Minik Ama Gerçek SparkOS"

SparkOS'un temel hedefi modern Linux/Windows'a rakip olmak değil; **küçük, anlaşılır, sıfır panikli ve gerçekten kullanılabilir** bir mikroçekirdek masaüstü sistemi oluşturmaktır.

### Temel Kullanıcı Deneyimi Döngüsü:
$$\text{BOOT} \longrightarrow \text{SHELL} \longrightarrow \text{FILESYSTEM} \longrightarrow \text{PROGRAM} \longrightarrow \text{NETWORK} \longrightarrow \text{GUI}$$

Kullanıcının yapabildikleri:
1. Sistemi anında boot etmek (BIOS -> 64-bit Long Mode).
2. VGA Metin Shell'i ve Ring 3 kullanıcı kabuğunu (`sh`) kullanmak.
3. Dosya ve klasör oluşturmak, okumak, yazmak, silmek ve düzenlemek (`cat`, `edit`, `ls`, `touch`, `mkdir`, `rm`).
4. Bağımsız ELF kullanıcı programlarını çalıştırmak (`/bin/hello`, `/bin/echo`, `/bin/cat`, `/bin/ls`).
5. Ağ bağlantısı kurmak ve paket alıp vermek (`ping`, `ifconfig`, `netsvc`).
6. 1080p grafik arayüz (GUI) kullanmak, pencereleri açmak, kapatmak ve yönetmek (`displaysvc`, `wm`).
7. Masaüstü duvar kağıdını ve temel ayarları değiştirmek.
8. Sistemi güvenli şekilde yeniden başlatmak (`reboot`) veya kapatmak (`shutdown`).

---

## 🗺️ Tamamlanan Fazlar ve Mimari Durum (Faz 1 – 12)

| Faz | Kapsam | İlgili Dosyalar | Durum |
|:---:|---|---|:---:|
| **Faz 1** | Kernel Hardening (Preemption, Fault Recovery, CSpace/CR3 Teardown) | `src/task/process.rs`, `src/interrupts.rs` | ✅ **Donduruldu** |
| **Faz 2** | Multi-Segment ELF Loader (`PT_LOAD`, `.bss` zero-fill, `ELF_INV-1..5`) | `src/elf.rs`, `src/user.rs` | ✅ **Donduruldu** |
| **Faz 3** | IPC Hardening (Per-Client İzolasyonu, Zero-Leak Hangup, Cancel) | `src/ipc.rs` | ✅ **Donduruldu** |
| **Faz 4** | Network & Socket E2E (RTL8139 Ring 3 DMA, `netsvc`, Zero-Copy SlotCap) | `src/dma_region.rs`, `src/net_socket.rs` | ✅ **Donduruldu** |
| **Faz 5** | Filesystem Isolation (`disksvc` ATA PIO vs `fssvc` SPFS, Port Confinement) | `src/task/process.rs` | ✅ **Donduruldu** |
| **Faz 6** | Terminal & Userspace Shell (STDIO Model, Ring-3 `sh`, Child Exec & Fault) | `src/task/process.rs`, `src/main.rs` | ✅ **Donduruldu** |
| **Faz 7** | Userspace ABI & Bağımsız ELF (`lib/sysapi`, `int 0x80`, `/bin/hello`) | `src/sysapi.rs`, `src/fs.rs` | ✅ **Donduruldu** |
| **Faz 8** | Process Lifecycle & Senkronizasyon (`waitpid`, State Machine, Reap) | `src/task/process.rs` | ✅ **Donduruldu** |
| **Faz 9** | Userspace Araçları & Minimal Runtime (`/bin/echo`, `/bin/cat`, `/bin/ls`) | `src/fs.rs`, `scratch/` | ✅ **Donduruldu** |
| **Faz 10** | Developer SDK & Toolchain (`libspark`, `spark` CLI, SDK Kılavuzu) | `sdk/libspark`, `sdk/spark` | ✅ **Donduruldu** |
| **Faz 11** | Display Server & Shmem Surface Compositor (`displaysvc`, Linear FB 0xA0000) | `src/task/process.rs` | ✅ **Donduruldu** |
| **Faz 12** | Window Manager & Compositor (`wm`, Z-Order, Focus Elevation, Input Routing)| `src/task/process.rs` | ✅ **Donduruldu** |
| **SMP Altyapısı**| Per-CPU State, Per-CPU GDT, Per-CPU TSS & Bağımsız `RSP0` Stackleri | `src/smp.rs`, `src/gdt.rs` | ✅ **Donduruldu** |

---

## 🛠️ Sıradaki Odak: Tekil Küçük Görevler (Single-Task Model)

- **Task A:** Userspace Dosya Araçları (`/bin/touch`, `/bin/mkdir`, `/bin/rm`)
- **Task B:** Userspace Ağ Aracı (`/bin/ping`, `/bin/fetch`)
- **Task C:** Etkileşimli GUI Pencere Kapatma/Açma (X Butonu)
- **Task D:** Masaüstü Duvar Kağıdı & Renk Teması Değiştirici
