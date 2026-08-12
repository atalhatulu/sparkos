# SparkOS

Rust ile yazılmış, freestanding (bağımsız) bir **x86_64 işletim sistemi çekirdeği**. `no_std` + `no_main`, kendi paging/heap altyapısı, kesme yönetimi, async görev yürütücü, dosya sistemi, ATA disk sürücüsü, RTL8139 ağ sürücüsü, basit ağ yığını ve kendi **GUI (grafik masaüstü)** alt sistemiyle birlikte gelir.

Proje **QEMU + KVM** üzerinde test edilir. Önyükleme `bootloader 0.9` (crate) ve `bootimage` ile sağlanır.

## Özellikler

- **Freestanding çekirdek**: Rust nightly, `#![no_std]`, `#![no_main]`, `#![feature(abi_x86_interrupt)]`.
- **Önyükleme**: `bootloader 0.9` + `cargo bootimage`. Entry noktası `src/main.rs` → `kernel_main(boot_info)` (`entry_point!` makrosu).
- **Bellek yönetimi**:
  - Paging (sanal bellek) ve `BootInfoFrameAllocator` (fiziksel çerçeve tahsisi).
  - `linked_list_allocator` tabanlı kernel heap (`allocator.rs`).
  - Kullanıcı moduna açılabilir bellek (`set_user_accessible`).
- **Kesme ve CPU altyapısı**:
  - GDT + TSS (Ring 3 selector'ları dahil), IDT, PIC, 1000 Hz programlanabilir zamanlayıcı (timer tick).
  - PS/2 klavye ve fare (mouse) sürücüleri.
- **Görevler / Multitasking**:
  - Async `Task` + `SimpleExecutor` (cooperative), `yield_now`, klavye scancode kuyruğu.
  - `PROCESS_LIST` ve `KILLED_PROCESSES` ile süreç yönetimi (`ps`, `kill`).
- **Dosya sistemi**: Bellek içi VFS (`FsNode` = `File`/`Directory`) + **ATA** sabit disk sürücüsü; dosya sistemi açılışta ATA diskinden yüklenir (`fs::load_from_disk`).
- **Ağ**: **RTL8139** PCI ağ kartı sürücüsü, PCI tarama (`lspci`), ICMP ping (`8.8.8.8`) ve UDP üzerinden DNS çözümleme (`host <domain>`, `8.8.8.8:53`).
- **Kullanıcı Modu (Ring 3)**: `iretq` ile Ring 3'e geçiş, makine kodu test (`int 0x80`) ve **ELF64** yükleyici (`elf.rs`) — `run_app` komutu `scratch/hello.elf` dosyasını yükler.
- **Tip güvenli IPC**: `Channel<T>` ve `Capability<T>` ile tip güvenli mesaj kanalları (`ipc.rs`).
- **GUI / Masaüstü (Ek)**: VBE/VESA framebuffer (1920x1080, 32 BPP), backbuffer, clip-rect, 4 pencereli masaüstü (**Terminal, Files, Notepad, TaskMgr**) ve widget alt sistemi (`ui/` → `Widget`, `Button`, `Label`, `apps`). Sistem açılışta otomatik olarak GUI modunda başlar.
- **Kabuk**: Zengin yerleşik komut seti (aşağıya bakınız), tam ekran metin editörü (`edit`).

## Gereksinimler

- Rust **nightly** toolchain (`rust-toolchain.toml`) ve şu bileşenler: `rust-src`, `llvm-tools-preview`, `rustc-dev`.
- `bootimage` alt komutu:
  ```bash
  rustup component add llvm-tools-preview
  cargo install bootimage
  ```
- **QEMU** (`qemu-system-x86_64`) ve KVM desteği.
- (İsteğe bağlı) VNC — masaüstünü görüntülemek için `vncviewer` ve boş bir `:5900` portu.

## Derleme ve Çalıştırma

Kök dizindeki `run.sh` betiği derleme, önyükleme görüntüsü oluşturma ve QEMU'da başlatmayı sırasıyla yapar:

```bash
./run.sh
```

Betik şunları yapar:

1. `cargo bootimage` ile çekirdeği derler.
2. Varsa `disk.img` bir kez oluşturur (10MB kalıcı sanal disk, `dd if=/dev/zero ...`). Yoksa ATA disk olarak kullanılır — dosya sistemi buradan yüklenir.
3. QEMU'yu başlatır:
   - `-drive format=raw,file=<bootimage>,index=0,media=disk`
   - `-drive format=raw,file=disk.img,index=1,media=disk`
   - `-serial stdio` (seri konsol çıktısı)
   - `-vga std`, `-m 256M`
   - `-netdev user` + `-device rtl8139` (ağ)
4. `:5900` portu boşsa VNC (`-vnc :0`) yaynı açar ve varsa `vncviewer` ile tam ekran bağlanır.
5. QEMU 60 saniye timeout ile çalışır (`timeout --foreground 60`).

Derleme doğrulaması için doğrudan:

```bash
cargo build
cargo bootimage
```

## Kabuk Komutları

| Komut | Açıklama |
|-------|----------|
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

## Proje Yapısı

```
sparkos/
├── Cargo.toml           # bootloader 0.9, x86_64 0.15, spin, crossbeam-queue, linked_list_allocator, uart_16550
├── rust-toolchain.toml  # nightly toolchain
├── .cargo/config.toml   # x86_64-unknown-none target, build-std (core/alloc/compiler_builtins)
├── run.sh               # Derle + bootimage + QEMU başlat
├── font.h / gen_font.py # GUI yazı tipi verileri (python ile üretim)
├── fix_shell.py / patch_gui.py / rewrite_mouse*.py / upgrade.py  # yardımcı scripting araçları
├── docs/goal.md         # geliştirme yol haritası / hedef dokümanı
├── scratch/             # Ring 3 ELF denemeleri (hello.S → hello.elf)
└── src/
    ├── main.rs          # Giriş noktası: kernel_main
    ├── serial.rs        # Seri (COM1) konsol çıktısı
    ├── vga_buffer.rs    # VGA text buffer + GUI_MOD
    ├── gdt.rs           # GDT + TSS (Ring 3 selector'ları)
    ├── interrupts.rs    # IDT, exceptions, PIC, timer (1000 Hz)
    ├── memory.rs        # Paging, frame allocator, set_user_accessible
    ├── allocator.rs     # Kernel heap
    ├── alloc            # (alloc crate kullanımı)
    ├── keyboard.rs / mouse.rs
    ├── shell.rs / editor.rs
    ├── fs.rs / ata.rs   # Bellek içi VFS + ATA sürücüsü
    ├── task/            # Async Task, SimpleExecutor, yield_now, klavye kuyruğu
    ├── scheduler.rs     # eski fonksiyon tabanlı scheduler
    ├── pci.rs / rtl8139.rs / net.rs
    ├── user.rs / elf.rs # Ring 3 geçişi + ELF64 yükleyici
    ├── ipc.rs           # Tip güvenli kanallar
    ├── font.rs / gui.rs # Font + VBE/VESA GUI / backbuffer
    └── ui/              # Widget, Button, Label, apps (window içerikleri)
```

## Notlar

- GUI framebuffer'ı **1920x1080, 32 BPP** olarak yapılandırılır (`gui.rs`, VBE portlar 0x01CE/0x01CF).
- `run_app` komutu `src/../scratch/hello.elf` dosyasını `include_bytes!` ile gömülü olarak yükler; bu dosya `scratch/hello.S` kaynağıyla üretilir.
- Bu depo geliştirme aşamasındadır; `docs/goal.md` mevcut durum ve bir sonraki hedef için yol haritası sunar.
