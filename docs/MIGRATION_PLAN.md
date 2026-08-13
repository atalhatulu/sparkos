# MIGRATION_PLAN.md — SparkOS: Monolitik → Capability Microkernel (Evrim)

> HEDEF: STEP 1-3 mimarisi (docs/architecture/) hedef sozlesme. Mevcut src/ (~9900 LOC)
> monolitik kernel. Bu dokuman, korunacak / degisecek / yeniden yazilacak parcaları
> isaretler ve capability core'un mevcut davranisi BOZMADAN nasil entegre edilecegini
> tanimlar.

## Karar

**EVRIM.** Mevcut SparkOS temel alinir, capability core adim adim entegre edilir.
Dokumanlar cope gitmez — hedef sozlesme olarak kalir.

```
Mevcut SparkOS
   ↓
[1] Capability Core
   ↓
[2] Syscall'lari capability tabanli yap
   ↓
[3] Memory/pointer guvenlik aciklarini kapat
   ↓
[4] IPC'yi capability'lere tasi
   ↓
[5] Driver'ları servisleştir (user-space)
   ↓
Microkernel
```

## Mevcut durum tespiti (gercek kod incelemasi)

| Dosya/Modul | Rotu | Durum | Aciklama |
|---|---|---|---|
| `src/security.rs` | MODIFY | capability yok | uid/gid + u64 bitmask. Bitmask'i capability core'a cevir. |
| `src/sec_mem.rs` | MODIFY/REWRITE | guvenlik deligi | user ptr'si `from_raw_parts` ile dogrulanmadan. Capability deref claim ile birlesecek. |
| `src/syscall.rs` | MODIFY | yetki kontrolu YOK | dispatcher'a capability check eklenecek. |
| `src/syscall_storage.rs`, `net_socket.rs` | MODIFY | fd/socket syscall | capability-gated yapilacak. |
| `src/ipc.rs` | REWRITE | `Capability<T>` bos wrapper | typed channel var; capability-in-message plana gececek. |
| `src/security.rs` cap mask | REMOVE (degismesi) | u64 bitmask | capability object'lere geciyor. |
| `src/task/process.rs` | MODIFY (minimal) | tek adres alani varsayimi | per-process CR3 "ready for later". Cap table process'e baglancak. |
| `src/allocator.rs`, `memory.rs` | KEEP | — | allocator/paging sağlam kalir. |
| `src/fd.rs`, `fs.rs`, `ata.rs` | KEEP (MODIFY gated) | — | capability-per-fd eklenir, ic mantik korunur. |
| `src/net*.rs`, `rtl8139.rs`, `usb.rs`, `pci.rs`, `display.rs` | KEEP | — | driverlar ileride servisleşecek (STEP 5). |
| `src/shell.rs`, `gui.rs`, `editor.rs` | KEEP | — | kullanici tarafı. |
| `src/sync.rs`, `smp.rs`, `interrupts.rs` | KEEP | — | altyapi. |

## Asama 1 — Capability Core (ilk implementasyon)

**Kapsam:** Bagimsiz, minimal, mevcut davranisi BOZMAYAN capability subsystem.
Mevcut syscall'lara henuz dokuNMAZ.

### Yeni moduller (src/ altina)

- `src/cap.rs` — capability core:
  - `CapHandle { slot: u32, generation: u32 }`
  - `Rights` bitmask (READ/WRITE/MAP/DMA/TRANSFER/GRANT/DESTROY/EXECUTE/MANAGE)
  - `CapNode { parent: Option<u32>, epoch: u64 }`
  - Cap table: slot bitmap + generation + node tree.
  - `grant` / `transfer` / `revoke` / `close` / `deref` (STEP 2-3 semantikleri).
- `src/cap_lock.rs` (veya cap.rs icine) — STEP 3: tek spinlock cap mutasyonlari,
  deref claim = epoch+gen+refcount++ tek dilim.

### Referans semantigi

- v0.1 capability core: `Rights` mevcut bitmask'i kapsayacak sekilde; ama **mevcut
  syscall'lara baglanmaz** (yeni syscall numaralari reserve edilir, dispatcher'a
  dokunulmaz). Boylece mevcut davranis %100 korunur.

### Dogrulama (QEMU)

- Unit test: grant/transfer/revoke/lend semantikleri (STEP 2 invariantlar).
- Concurrency smoke test: cap mutasyonlari tek lock, deref claim atomik.
- QEMU boot: mevcut kernel ayni davranisla acilir (regression yok).

## Asama 2 — Syscall'lari capability-gated yap

- `security.rs`'teki u64 bitmask'i `cap.rs` Rights'ina bagla.
- `syscalls.rs` dispatcher'ina capability check: her process'in cap table'inda syscall
  icin gerekli right.
- `SYS_EXEC` / `SYS_FORK`: yeni process'in cap table'i bos baslar, parent'tan devralir.
- Regression: tum mevcut testler + kernel boot.

## Asama 3 — Memory/pointer guvenlik acigi kapat

- `sec_mem.rs` `validate_user_ptr(_mut)` capability core'a baglanir:
  pointer + capability right (MAP) + paging dogrulamasi.
- Kernel adresi / null / overflow rejection korunur; capability check eklenir.
- `syscall.rs` SYS_READ/WRITE/OPEN/LSEEK `sec_mem` uzerinden gecirilir.

## Asama 4 — IPC'yi capability'lere tasi

- `ipc.rs` `Capability<T>` bos wrapper'i gercek capability transfer'e bagla:
  mesaj icinde `CapHandle` tasinsin, dequeue aninda dogrula (IPC_CONTRACT.md).
- BlockingChannel'i koru, capability-in-message ekle (uzun mesajlar icin ring/shared
  memory plani ileride).

## Asama 5 — Driver servislesitirme (microkernel gecisi)

- Ayri proje/asama. Driverlar user-space servis haline gelir, IPC uzerinden erisilir.
- Fault isolation + revoke devreye girer.
- IOMMU/DMA (STEP 5), cancellation (STEP 7), donation (STEP 8).

## Acik konular (STEP 4 sonrasi benimsenmis ama buradaki not)

- Mapping/TLB (STEP 4): capability core'un deref claim'i mapping'e baglaninca
  concurrency modeli (CONCURRENCY_MODEL.md) genisler; TLB shootdown ordering eklenir.
- Per-process CR3: su an tek adres alani varsayimi var. Capability memory izolasyonu
  icin per-process address space gerekecek — Asama 3'te devreye girer (tek adres
  alani su anki sınır; capability ilk once sadece fd/syscall yetkisi icin).

## Guvenli limitler (Kodlama kurallari)

- `src/main.rs`'e DOKUNMA. `pub mod` eklemelerini Hermes yapar.
- Asama 1'de mevcut syscall'lara DOKUNMA (yeni modul + reserve syscall no + test).
- Her asama: cargo build 0 hata + QEMU boot regression. `env -u PYTHONPATH cargo build`.
- Mevcut tip güvenli kanallar/allocator/paging modileri yeniden yazma.
