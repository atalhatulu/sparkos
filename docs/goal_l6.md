# SparkOS Kernel — /goal (L6 Storage)

## Görev
`~/Documents/GitHub/sparkos` repo'sunda **L6 (Storage)** seviyesini implement et. **SADECE aşağıdaki dosyalara dokun:**
- `src/fd.rs` — YENI dosya oluştur (file descriptor sistemi)
- `src/fs.rs` — güçlendir (VFS soyutlama)
- `src/syscall_storage.rs` — YENI dosya (storage syscall'ları ayrı modül)

**KESiN: `src/main.rs`'e DOKUNMA.** `pub mod` eklemelerini HERMES yapacak. Mevcut `src/syscall.rs`, `src/sync.rs`, `src/ipc.rs`'e dokunma.

## Proje
- Rust freestanding x86_64 `no_std` kernel, `bootloader 0.9`, `x86_64 0.15`, `spin 0.9`
- Build: `cargo build` (0 error gerekir, 5 warning kabul). **NOT: terminal komutlarını `env -u PYTHONPATH <cmd>` ile çalıştır.**
- Mevcut L0-L5 tamam: syscall.rs (SYS_EXIT=1, SYS_WRITE=4), sync.rs (BlockingChannel), ipc.rs

## Mevcut L6 Durumu
- `src/ata.rs` (120 satır) — ATA PIO disk sürücüsü var
- `src/fs.rs` (440 satır) — bellek içi VFS (`FsNode`, `load_from_disk()`, dosya/dizin gezinme)
- **EKŞİK:** File descriptor sistemi yok (fd numarası, open/read/write/close/lseek), VFS soyutlama katmanı eksik, storage syscall'ları syscall tablosuna eklenmemiş.

## Yapılacaklar
1. **`src/fd.rs` (yeni):**
   - `FileDescriptor` yapısı: fd numarası, node referansı/pointer, pozisyon (offset), access flags (R/O/W/RW), ref count.
   - `FdTable`: global açık dosya tablosu (max 256 fd), `open(path, flags) -> Result<fd,Err>`, `close(fd)`, `read(fd, buf, len)`, `write(fd, buf, len)`, `lseek(fd, offset, whence)`.
   - `sync::Mutex` veya `Spinlock` ile korunan `static FD_TABLE`.
   - Mevcut `fs.rs`'in node yapısına bağlan (FsNode). İmzaları fs.rs'teki gerçek API'ye göre uyarla — ÖNCE `src/fs.rs`'i oku.

2. **`src/fs.rs` güçlendir:** VFS soyutlama (File/Directory/Device ayrımı), dokunulabilir açık node API'si ekle. Mevcut yapıyı kırma.

3. **`src/syscall_storage.rs` (yeni):** Storage syscall yazmaçları + dispatcher fonksiyonları:
   - `SYS_OPEN`, `SYS_READ`, `SYS_CLOSE`, `SYS_LSEEK`, ve `SYS_WRITE`'ı dosyaya genişlet.
   - `sys_open(path_ptr, flags)`, `sys_read(fd, buf_ptr, len)`, `sys_write(fd, buf_ptr, len)`, `sys_close(fd)`, `sys_lseek(fd, off, whence)`.
   - Bu fonksiyonları `fd.rs`'in FdTable'ına bağla. (syscall.rs'e SİZ ekleme — Hermes dispatcher'a bağlayacak.)

## Teknik
- `no_std` + alloc, `std` yok.
- fd tablosu interrupt-safe (spin/Mutex), deadlock yok, kilit kısa.
- Path çözümleme: mevcut fs.rs'teki gezinme fonksiyonlarını kullan.

## Teslim
1. Kısa analiz: fs.rs mevcut yapısı, ne eklendi, fd sistemi nasıl bağlandı.
2. `src/fd.rs` + `src/fs.rs` + `src/syscall_storage.rs`.
3. `cargo build` çıktısı (0 error). NOT: yeni modüller main.rs'e bağlı olmadığı için `cargo build` onları derlemeyecek — bunu `rustc --edition 2021 --crate-type lib` ile veya geçici `main.rs` tekil `pub mod` ekleyerek DOĞRULA. (Sonunda main.rs'e ekleme, sadece test amaçlı geçici yap.)
4. `git diff --stat`.

AGY: "Eğer görev mantıksızsa YAPMA — raporla. İddialarını önce dosyayı okuyarak doğrula."
