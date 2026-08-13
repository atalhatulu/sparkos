# SparkOS Kernel — /goal

## Görev
SparkOS'u "gerçek güçlü kernel" seviyesine taşımak için yol haritasındaki **L6 (Storage)** seviyesini tamamla. Mevcut çalışan sistemi bozma; mevcut yarım işleri tamamla/temizle. Build doğrulaması zorunlu.

## Proje Konumu
- Repo: `~/Documents/GitHub/sparkos` (git: github.com/atalhatulu/sparkos)
- Rust freestanding x86_64 `no_std` kernel, `bootloader 0.9`, `x86_64 0.15`
- Nightly toolchain, entry `kernel_main` (main.rs)
- Build: `cargo build` → **0 error olmalı** (mevcut 5 warning kabul)
- **ÖNEMLİ:** Bu ortamda Python tooling'i çalıştırırken `PYTHONPATH` kirli olabilir → terminal komutlarını `env -u PYTHONPATH <cmd>` ile çalıştır.

## Mevcut Durum (L0-L5 tamamlandı)
- ✅ L0-3: boot, output, GDT/IDT/interrupts/timer, paging/heap/allocator, async Task+SimpleExecutor
- ✅ L4: Ring3 (user.rs), ELF loader (elf.rs), syscall dispatcher (syscall.rs — SYS_EXIT, SYS_WRITE)
- ✅ L5: `src/sync.rs` (Spinlock, IrqSafeSpinlock, Mutex, Semaphore, Condvar, BlockingChannel), `src/ipc.rs` (Channel=BlockingChannel, SYSTEM_CHAN)
- ⚠️ **Storage seviyesi (L6) KISMEN var:**
  - `src/ata.rs` (120 satır) — ATA PIO disk sürücüsü var
  - `src/fs.rs` (440 satır) — bellek içi VFS (`FsNode` = File/Directory), `load_from_disk()`, `edit` komutu
  - **EKŞİK:** File descriptor (fd) sistemi YOK — open/read/write/close syscall'ları YOK. VFS soyutlama katmanı, inode kavramı, gerçek blok dizinleme, dosya modları YOK. Syscall tablosuna storage syscall'ları eklenmemiş.

## Hedef: L6 — Storage katmanı

**1. File Descriptor (fd) Sistemi** — yeni `src/fd.rs`:
- `FileDescriptor` yapısı: fd numarası, açık dosya node referansı, read/write offset (position), access flags (O_RDONLY/O_WRONLY/O_RDWR), referans sayısı.
- `FdTable` — per-process açık dosya tablosu (max fd 256 gibi), `open`, `close`, `read`, `write`, `lseek` operasyonları.
- Global fd tablosu (şimdilik tüm kernel paylaşımlı) + `static` olarak korunabilir.

**2. VFS Soyutlama** — `src/fs.rs`'i güçlendir:
- `VfsNode` trait veya enum: File / Directory / Device ayrımı, `read`, `write`, `open`, `close`, `size`, `name` operasyonları.
- Mevcut `FsNode`'u bu soyutlamaya entegre et (kırma, uyumlu bırak).
- Klasör gezinme: `ls`, `cd`, `cat`, `mkdir` benzeri komutların fd tabanlı çalışması.

**3. Syscall Entegrasyonu** — `src/syscall.rs`'e storage syscall'ları ekle:
- `SYS_OPEN` (fd aç), `SYS_READ`, `SYS_WRITE` (dosyaya, fd=1/2 stdout/stderr ayrı), `SYS_CLOSE`, `SYS_LSEEK`.
- Ring 3'ten çağrılabilir olmalı (user.rs/elf.rs ile uyumlu).
- Mevcut `sys_write`'i fd lerine göre genişlet (fd=1/2 terminal, fd>=3 dosya).

**4. ATA/Blok Katmanı İyileştirme** (opsiyonel ama önerilen):
- `ata.rs`'e okuma/yazma tamponlama, LBA28→LBA48, kesme (IRQ) tabanlı I/O yerine polled I/O doğrulaması.
- Blok cache (basit) — tekrar okumaları hızlandır.

**5. Shell Entegrasyonu** — `shell.rs`'te mevcut dosya komutlarının fd tabanlı çalıştığını göster, örn. `cat <file>`, `write <file> <data>`.

**NOT:** syscall tablosuna yeni syscall eklerken, mevcut `syscall_dispatcher`'ın match yapısını koru, yeni sabitleri ekle. `sys_write` için fd<3 terminal, fd>=3 dosya yazımı olacak şekilde dallandır.

## Teknik Kısıtlar
- `no_std`: alloc + spin kullan, `std` yok.
- Dosya sistemi bellek içi (RAM disk) — disk I/O'su ATA üzerinden `load_from_disk` ile. Gerçek disk formatı aramıyoruz, mevcut bellek-içi modeli fd sistemiyle bütünleştir.
- Syscall'lar interrupt-safe, fd tablosu `sync::Mutex`/`Spinlock` ile korunmalı.
- Deadlock yok; fd tablosu kilidini kısa tut.

## Teslim
1. Kısa analiz: mevcut fs.ktiği, ne eklendi.
2. `src/fd.rs` (yeni) + syscall storage entegrasyonu.
3. `src/fs.rs` VFS güçlendirmesi.
4. `src/syscall.rs` genişletilmiş dispatcher.
5. `cargo build` çıktısı (0 error; mevcut 5 warning kalabilir).
6. `git diff --stat`.

AGY: "Eğer bir görev mantıksızsa, zaten çözülmüşse veya gerekli değilse YAPMA — neden gerekmediğini raporla. Yaptığın iddiaları önce doğrula (dosyayı oku), varsayımla 'acil/bitti' deme."
