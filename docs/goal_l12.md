# SparkOS Kernel — /goal (L12 SparkOS 1.0 — FINAL)

## Görev
`~/Documents/GitHub/sparkos` repo'sunu **SparkOS 1.0** seviyesine taşı. Yol haritası L0-L11 tamamlandı; şimdi **stabilizasyon + kullanıcıya açılabilirlik**. **SADECE aşağıdaki dosyalara dokun:**
- `src/app.rs` — YENI (user app API'si)
- `src/sysapi.rs` — YENI (kullanıcıya açık syscall API dokümantasyon modülü)
- `README.md` — güncelle (SparkOS 1.0 mimari özeti, seviyeler, nasıl build edilir)
- `src/lib_userspace.rs` — YENI (ilk kullanıcı alanı Rust programı örnek iskeleti, ELF olarak)

**KESiN: `src/main.rs`'e DOKUNMA.** `pub mod` eklemelerini HERMES yapacak. Kritik çekirdek dosyalarına (syscall.rs, user.rs, scheduler, memory) dokunma — stabilizasyon gerekçesiyle DOKUNMA, sadece oku.

## Proje
- Rust freestanding x86_64 `no_std` kernel, `bootloader 0.9`, `x86_64 0.15`
- Build: `cargo build` (0 error). **NOT: terminal komutlarını `env -u PYTHONPATH <cmd>` ile çalıştır.**
- L0-L11 tamlandı: boot, CPU, memory, scheduler, user mode+ELF, syscall (EXIT/WRITE/READ/OPEN/CLOSE/LSEEK), IPC+sync, storage(fd), drivers(pci/usb/display), networking(net_socket), security(uid/gid/caps/sec_mem), SMP/ACPI, klog/panic/trace. Bootimage önyüklenebilir.

## Yapılacaklar — SparkOS 1.0 stabilizasyonu
1. **`src/app.rs` (yeni):** Kullanıcı uygulamaları için API:
   - `AppInit`, `AppExit` benzeri yaşam döngüsü tipleri.
   - `run_app(elf: &[u8])` sarmalayıcı (mevcut user.rs/elf.rs'i import et, senkronize et).
   - Kullanıcı programı beklentileri: hangi syscall'lar açık, stack/kod/data layout, giriş adresi — README'de anlatılacak sözleşme.

2. **`src/sysapi.rs` (yeni):** Kullanıcı-uzayı syscall API dokümantasyonu:
   - `SyscallTable` const tablo: numara → isim (+ kısa açıklama).
   - Şu syscallları belgele: EXIT(1), READ(0), OPEN(2), CLOSE(3), WRITE(4), LSEEK(8). `syscall_table()` döndürür.
   - `SYSCALLS: &[SyscallInfo]` static — debugger/log'a basılabilir.

3. **`src/lib_userspace.rs` (yeni):** İlk kullanıcı alanı Rust programı iskeleti:
   - `#[no_mangle] pub extern "C" fn _start()` giriş noktası + `syscall(n, a1, a2, a3)` inline asm yardımcısı (`int 0x80`).
   - "Hello from SparkOS userspace" yazdıran örnek: `sys_write(1, ...)`.
   - NOT: Bu ayrı ELF olarak kullanılacak (derlenmesi ana build'i etkilemez). Sadece iskelet + comment.

4. **`README.md` güncelle:**
   - SparkOS 1.0 başlık: ne yapıyor, mimari.
   - 12 seviye yol haritası özeti (ne var ne yok).
   - Build & run: `cargo bootimage`, QEMU run.
   - Kullanıcı uygulama sözleşmesi (app.rs + sysapi.rs'ten).
   - Kısa ve net.

## Teknik
- no_std + alloc. Merhaba dünya kullanıcı programı syscall'la.
- `int 0x80` girişi SysV benzeri (mevcut syscall.rs dispatcher'ı ile uyumlu).
- Mevcut user.rs/elf.rs API'sini OKU, uyumlu import et.

## Teslim
1. Kısa analiz: mevcut user/elf API, ne eklendi.
2. `src/app.rs` + `src/sysapi.rs` + `src/lib_userspace.rs` + README.md.
3. `cargo build` çıktısı (0 error). Geçici `pub mod` ile doğrula (kalıcı ekleme yok).
4. `git diff --stat`.

AGY: "README'yi oku, mevcut durumu doğrula. Eğer görev mantıksızsa YAPMA — raporla."
