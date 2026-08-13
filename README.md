# SparkOS 1.0 - FINAL

Rust ile yazılmış, freestanding (bağımsız) bir **x86_64 işletim sistemi çekirdeği**. `no_std` + `no_main`, kendi paging/heap altyapısı, kesme yönetimi, async görev yürütücü, dosya sistemi, ATA disk sürücüsü, RTL8139 ağ sürücüsü, GUI (grafik masaüstü) alt sistemi ve tam teşekküllü Ring 3 kullanıcı alanı (Userspace) desteğiyle birlikte SparkOS 1.0 stabilizasyonuna ulaşmıştır.

Proje **QEMU + KVM** üzerinde test edilir. Önyükleme `bootloader 0.9` (crate) ve `bootimage` ile sağlanır.

## SparkOS Mimari Özeti ve L0-L12 Seviyeleri (Yol Haritası Tamamlandı)

- **L0-L2:** Bootloader, Ekran (VGA/Seri), CPU Kesmeleri (IDT, GDT), Timer ve Klavye.
- **L3-L4:** Bellek Yönetimi (Paging, Frame Allocator), Kernel Heap (`allocator.rs`).
- **L5-L6:** Çoklu Görev (Async/Await, Executor), Disk/Dosya Sistemi (Bellek İçi + ATA Disk).
- **L7-L8:** Donanım (PCI, RTL8139, Ağ Yığını), Güvenlik (Sec Mem, UID/GID).
- **L9-L10:** Kullanıcı Modu (Ring 3 `iretq`), ELF Yükleyici, Sistem Çağrıları (`int 0x80`).
- **L11-L12 (1.0 - FINAL):** SMP/ACPI hazırlığı, IPC (Tip güvenli kanallar), Kararlılık, `app.rs` / `sysapi.rs` üzerinden kullanıcı-uzayı (Userspace) API sözleşmesi ve GUI entegrasyonu.

## Kullanıcı Uygulama Sözleşmesi (Userspace API)

SparkOS 1.0, kullanıcı uygulamalarını (Ring 3) bağımsız ELF ikilileri olarak destekler:

- **Entry Point:** `#[no_mangle] pub extern "C" fn _start()`
- **Sistem Çağrıları (Syscalls):** `int 0x80` üzerinden sağlanır. (Bkz: `src/sysapi.rs`)
  - `SYS_READ (0)`
  - `SYS_EXIT (1)`
  - `SYS_OPEN (2)`
  - `SYS_CLOSE (3)`
  - `SYS_WRITE (4)`
  - `SYS_LSEEK (8)`
- **Bellek (Memory Layout):** Uygulamalara otomatik olarak 4KB'lık Yığın (Stack) tahsis edilir. Kod/Veri segmentleri ELF başlığında belirtildiği şekilde kullanıcı erişimine açık (Ring 3) haritalanır.
- **İskelet Kodu:** `src/lib_userspace.rs` dosyasında bir `no_std` kullanıcı uygulaması şablonu mevcuttur. Çekirdek içerisindeki `app::run_app` API'si ile yüklenip çalıştırılır.

## Gereksinimler & Derleme

- Rust **nightly** toolchain (`rust-toolchain.toml`)
- `bootimage` (`cargo install bootimage`)
- **QEMU** (`qemu-system-x86_64`)

Derleme ve çalıştırma:

```bash
cargo build
cargo bootimage
./run.sh
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
├── Cargo.toml           # bootloader 0.9, x86_64 0.15, vb.
├── rust-toolchain.toml  # nightly toolchain
├── .cargo/config.toml   # x86_64-unknown-none target, build-std
├── run.sh               # Derle + bootimage + QEMU başlat
├── scratch/             # Ring 3 ELF denemeleri
└── src/
    ├── main.rs          # Giriş noktası: kernel_main
    ├── app.rs           # (YENİ) Userspace App API
    ├── sysapi.rs        # (YENİ) Syscall Tablosu ve Dokümantasyonu
    ├── lib_userspace.rs # (YENİ) Örnek kullanıcı alanı programı iskeleti
    ├── syscall.rs       # Syscall dispatcher
    ├── user.rs / elf.rs # Ring 3 geçişi + ELF64 yükleyici
    └── ...              # Diğer çekirdek modülleri (bellek, gui, fs, ağ)
```
