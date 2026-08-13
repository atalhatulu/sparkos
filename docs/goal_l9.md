# SparkOS Kernel — /goal (L9 Security)

## Görev
`~/Documents/GitHub/sparkos` repo'sunda **L9 (Security)** seviyesini implement et. **SADECE aşağıdaki dosyalara dokun:**
- `src/security.rs` — YENI dosya (kullanıcı/grup/izin/capability sistemi)
- `src/sec_mem.rs` — YENI dosya (bellek koruma / güvenli syscall yardımcıları)

**KESiN: `src/main.rs`, `src/syscall.rs`'e DOKUNMA.** `pub mod` eklemelerini HERMES yapacak. Mevcut `src/user.rs`, `src/elf.rs`, `src/memory.rs`'e de dokunma (sadece okuma için eriş, değiştirme).

## Proje
- Rust freestanding x86_64 `no_std` kernel, `bootloader 0.9`, `x86_64 0.15`
- Build: `cargo build` (0 error). **NOT: terminal komutlarını `env -u PYTHONPATH <cmd>` ile çalıştır.**
- L0-L8 tamam (net hâlâ geliştiriliyor, ama ondan bagımsız çalış).

## Mevcut L9 Durumu
- `src/user.rs` (142) — Ring3 geçişi (`iretq`), makine kodu test, ELF ile kullanıcı programı. KERNEL_RSP/KERNEL_RIP restore.
- `src/elf.rs` (108) — ELF64 yükleyici (`run_app` scratch/hello.elf yükler).
- `src/syscall.rs` — syscall dispatcher, SYS_EXIT/WRITE/READ/OPEN/CLOSE/LSEEK.
- `src/memory.rs` — paging, `set_user_accessible`.
- **EKŞİK:** Kullanıcı/grup modeli YOK (uid/gid). Permission/izin kontrolları YOK. Capability sistemi YOK. Syscall'lar yetki kontrolü YAPMADAN istediğini yapabilir. Bellek koruma: user process'in kernel belleğine erişimini denetleyen mekanizma YOK (syscall buffer pointer geçerliliği doğrulanmıyor).

## Yapılacaklar (güvenlik açısından sağlam ve no_std)
1. **`src/security.rs` (yeni):**
   - `Uid`, `Gid` tipleri (u32 wrapper), `User`, `Group` yapıları.
   - `Credentials`: uid, gid, groups listesi, capability maskesi.
   - `SecurityManager`: kullanıcı/izin sorgulama, `check_permission(creds, required)`, `syscall_capable(cap)`.
   - Basit in-memory kullanıcı listesi: `root` (uid 0, tüm yetkiler), `user` (uid 1000, sınırlı), `guest` (uid 65534).
   - **Capability modeli:** bitmask `Capability` tipi, `Cap::SYS_ADMIN`, `Cap::FILE_READ`, `Cap::FILE_WRITE`, `Cap::NET`, vb.

2. **`src/sec_mem.rs` (yeni) — güvenli syscall bellek erişimi:**
   - `validate_user_ptr(buf_ptr, len) -> Result<&[u8], E>` — kullanıcı pointer'ının geçerli kullanıcı adres alanında olduğunu paging ile doğrular (0x0000_0000_0000_4000 benzeri user-space aralığı), kernel adreslerini ve null'u reddeder.
   - `validate_user_ptr_mut` (yazma için).
   - adres aralığı / overflow kontrolü (ptr + len wrapping engelle).
   - NOT: Bu, mevcut syscall.rs'teki `from_raw_parts` güvensiz çağrılarını gelecekte değiştirmek için temel oluşturur — syscall.rs'e DOKUNMA, sadece bu fonksiyonları hazırla.

3. **Entegrasyon işaretleri:** security.rs'i kullanan örnek fonksiyonlar, `SecurityManager` static instance (Mutex'li), `check_permission` kullanan demo. (syscall.rs'e bağlamak HERMES'in işi.)

## Teknik
- `no_std` + alloc. `spin::Mutex` kullan.
- UID/GID wrapper'ları Newtype deseni (Copy).
- Capability bit işlemleri `u64` bitmask üzerinde.
- Bellek doğrulama: `x86_64` crate'in `VirtAddr`'i, paging erişim biti (present/user/access) kontrolü — `PageTableEntry` flags oku.

## Teslim
1. Kısa analiz: mevcut güvenlik açıkları, ne eklendi.
2. `src/security.rs` + `src/sec_mem.rs`.
3. `cargo build` çıktısı (0 error). Geçici `pub mod` ile doğrula (main.rs'e kalıcı ekleme yok).
4. `git diff --stat`.

Claude: "Eğer görev mantıksızsa YAPMA — raporla. İddialarını önce dosyayı okuyarak doğrula."
