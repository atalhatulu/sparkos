SparkOS Asama 2.0 — Capability core boot entegrasyonu + SYS_EXEC exploit fix

GOREV: Mevcut (frozen) capability core'u kernel'e aktiflestir. SEN YEREL
DOSYA ERISIMINE MUHTAC DEGILSIN — tum baglam asagida verildi. Kod/blog degisiklerini
tek markdown cevabinda, her dosya icin ```kod-cevabi``` bloğunda ver. Yerel dosyaya
yazma, sadece kod uret.

## Baglam — mevcut src/cap.rs public API
```rust
pub enum CapError { Invalid, Revoked, NoRights, NotFound, Exhausted, AlreadyExists }
pub type Result<T> = core::result::Result<T, CapError>;
pub struct CapHandle { pub slot: u32, pub generation: u64 }
pub struct Rights(pub u32); // READ=1<<0 WRITE=1<<1 MAP=1<<2 IO=1<<3 DMA=1<<4 TRANSFER=1<<5 GRANT=1<<6 DESTROY=1<<7 EXECUTE=1<<8 MANAGE=1<<9
pub enum ObjectKind { Memory }  // ileride Fd, Socket, Process eklenecek
pub struct CapNode { pub parent: Option<u32>, pub epoch: u64 }
pub struct CapObject { pub kind, pub refcount: u32, pub valid: bool, pub generation: u64 }
pub fn init() -> () // STATE.lock() = Some(CoreState{ empty vecs })
pub fn create_object(kind: ObjectKind) -> Result<CapHandle>
pub fn grant(parent: CapHandle, req: Rights) -> Result<CapHandle>
pub fn transfer(src: CapHandle, req: Rights) -> Result<CapHandle>
pub fn lend(parent: CapHandle, req: Rights) -> Result<CapHandle>
pub fn reclaim(cap: CapHandle) -> Result<()>
pub fn close(cap: CapHandle) -> Result<()>
pub fn revoke(cap: CapHandle) -> Result<()>
pub fn deref(cap: CapHandle, flags: Rights) -> Result<CapAccess>
pub struct CapAccess { pub object_idx: u32 } // RAII guard
```
Not: `init()` su ana kadar sadece STATE'i bos CoreState ile baslatiyor ve HIC
bir yerde cagrilmiyor (main.rs'te `cap::` yok).

## Baglam — mevcut src/task/process.rs Process struct (PCB)
```rust
pub struct Process {
    pub pid: u64,
    pub name: String,
    pub state: ProcessState,
    pub is_user: bool,
    pub ctx: RegisterContext,         // cr3 uyesi var (0 == shared kernel table)
    pub kernel_stack: Box<[u8]>,
    pub entry: Option<extern "C" fn()>,
    pub user_cr3: u64,                // per-process address space (0 == shared)
    pub user_rsp: u64, pub user_rip: u64,
    pub user_ss: u16, pub user_cs: u16,
    pub kernel_rsp: u64, pub kernel_rip: u64,
    pub exit_code: u64, pub exited: bool,
}
```

## Baglam — mevcut src/syscall.rs SYS_EXEC dalı
```rust
SYS_EXEC => {
    serial_println!("[SYSCALL] SYS_EXEC ({:#x}, {}) from Ring 3", arg1, arg2);
    match crate::task::process::exec_elf_proc(
        "execd",
        unsafe { core::slice::from_raw_parts(arg1 as *const u8, arg2 as usize) }, // DELIK
    ) { ... }
}
```
Ve mevcut bir yardimci var: `fn check_user_read(buf_ptr: u64, len: usize) -> Result<(), u64>` -> sec_mem::validate_user_ptr cagirir, hata ise Err(EFAULT).

## Uretilmesi gereken 3 degisiklik (FROZEN uyumlu, PROVISIONAL YOK)

### 1) src/cap.rs — bootstrap capability + ROOT init
`cap.rs`'e soyle bir sey ekle (mevcut init() yerine/yanina):
- `init()`: STATE'i baslat (mevcut gibi) VE bir `ObjectKind::Process` (veya en temiz
  yeni ObjectKind) icin bir **ROOT capability** olustur, `bootstrap_root()` adli
  helper'dan dondur. Cagri: `let root = cap::bootstrap_root().unwrap();` her
  process fork'unda yeni process'in cap_table'ina `create_object(ObjectKind::Process)`
  + `grant` ile temel yetkiler verilir.
- Tasarim: SCOPE dar tut. Sadece `cap::init()` + `cap::create_object` + `cap::grant`
  kullan. Yeni karmaşık API ekleme.
- `ObjectKind`'a `Fd` ve `Process` varyantlarini EKLE (ileride Asama 2'de kullanilacak;
  boylece CapObject gercek kaynaga hazir). `cap.rs`'teki match'leri guncelle.

### 2) src/task/process.rs — Process'e cap_table alani
`Process` struct'ina ekle:
```rust
/// Caller'ın capability handle'larının tutuldugu tablo (fd -> CapHandle).
/// Per-process; process exit'te temizlenir (tum handle'lar close).
pub cap_table: CrateFdCapTable,   // ya da daha basit: Option<CapHandle> + fd map
```
Herhangi bir type kullanabilirsin ama alloc ergonomisi: mevcut kod `std::collections`
kullanamaz (no_std). En basit uyumlu yaklasim: `cap_table` alani tipini TANIMLA,
ne kullanacagini net soyle (Vec<(fd, CapHandle)> yeterli; Asama 2'de syscall_cap
onu kullanir). Process constructor'larini (fork/exec) cap_table'i bos + bir ROOT cap
ile baslatacak sekilde guncelle.
- `current_process()` yardimcisi: mevcut process'in &mut kapisinin cap_table'ini
  cagiriciya dondur. (Eger process.rs'te lock-guard deseni varsa onu kullan.)

### 3) src/syscall.rs — SYS_EXEC exploit fix
`SYS_EXEC` dalinda, `from_raw_parts` ONCESINE:
```rust
if check_user_read(arg1, arg2 as usize).is_err() {
    serial_println!("[SYSCALL] SYS_EXEC Error: invalid user buffer (EFAULT)");
    return EFAULT;   // mevcut EFAULT const'i kullan
}
```
Boylece user, kernel adresi vererek kernel'ın o adresi ELF olarak okumasini ENGELLER.

## KISITLAR
- SADECE bu 3 dosya icin kod uret. `main.rs`'e DOKUNMA (cap::init cagrisini HERMES ekler).
- PROVISIONAL/DEFERRED (mapping/TLB, DMA/IOMMU, IPC cancel, lend expiry) KODLAMA.
- no_std, alloc kullanilabilir (Vec). `spin::Mutex` var.
- Rust 2021 edition.
- Kod urettikten sonra: her degisiklik icin minimum 1 test oner (host test bloklari,
  #![cfg(test)] module).

Cikti formati:
```
## 1) cap.rs
<rust kod blogu — sadece EKLENEN/DEGISTIRILEN kismi, tamamini degil>
## 2) process.rs
<rust kod blogu>
## 3) syscall.rs
<rust kod blogu>
## Tasarim notlari
<kisa: cap_table tipi ne, ROOT cap nasil veriliyor, fork/exec constructor degisikligi>
```