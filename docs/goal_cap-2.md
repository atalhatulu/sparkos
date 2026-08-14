SparkOS Asama 2 — Syscall Yetki Kontrolu (REVIZE v2 — fcc eleştirisi işlendi)

GOREV: FROZEN capability core'unu (src/cap.rs) mevcut syscall'a bagla. Yetki
kontrolleri syscall_cap.rs koprusunde. SEN YEREL DOSYA ERISIMINE MUHTAC DEGILSIN —
tum baglam asagida. Kod blogu olarak uret.

## REVIZYon notu (fcc-claude eleştirisi)
fcc 3 BLOCKER + refactor onerdi, bunlara GORE kodla:
- **B1 stdio seed:** fd 1/2 (stdout/stderr özel dallari) SYS_OPEN ile acilmaz, cap_table'da
  OLMAZ. Bunlari check etmezsen tum user ciktisi EACCES alir, sistem kirilir. Bu yuzden
  capability tablosu HICBIR zorunlu check'i engelleyecek sekilde koseye sikistirilamaz;
  ama ASAMA 2 kapsami: SADECE fd giris yollarina gate koy, fd 1/2'nin cap_table'da
  seed'li OLDUGUNU garanti etmek icin `bootstrap_root()` icin stdio seed'ini
  `syscall_cap::init()` cagrisina ekle.
- **B2 socket provision:** sys_socket de fd uretir ve cap_table'a YAZMALI.
- **B3 fork/exec:** bootstrap_root() main.rs'te `let _root=...` ile cope atiliyor; her
  process'te root/self handle YOK. FORK'un fd argumani yok. AYRI check API'si gerek.
- **Q1 slice-pure refactor:** `/get_process_mut/` MEVCUT DEGIL (sadece `get_process(pid)
  -> Option<(u64,String,bool,ProcessState)>` var, &mut Process donmez). GLOBAL'e
  baglanma; karar mantigini DILIM uzerinde saf fonksiyon yap:
  `check_fd_access_in_table(&[(u32,CapHandle)], fd, needed)` + kernel wrapper.
- **G1:** close_fd'yi SYS_CLOSE'a bagla.

## Mevcut gercek API'ler (yalniz bunlar var)

### src/cap.rs
```rust
pub enum CapError { Invalid, Revoked, NoRights, NotFound, Exhausted, AlreadyExists }
pub type Result<T> = core::result::Result<T, CapError>;
pub struct CapHandle { pub slot: u32, pub generation: u64 }
pub struct Rights(pub u32); // READ=1 WRITE=2 MAP=4 IO=8 DMA=16 TRANSFER=32 GRANT=64 DESTROY=128 EXECUTE=256 MANAGE=512
pub enum ObjectKind { Memory, Device, Endpoint, Generic, Fd, Process }
pub fn init(); pub fn bootstrap_root() -> Result<CapHandle>;
pub fn create_object(kind: ObjectKind) -> Result<CapHandle>;
pub fn grant(parent: CapHandle, req: Rights) -> Result<CapHandle>;
pub fn transfer(cap: CapHandle, req: Rights) -> Result<CapHandle>;
pub fn lend(parent: CapHandle, req: Rights) -> Result<CapHandle>;
pub fn close(cap: CapHandle) -> Result<()>;
pub fn revoke(cap: CapHandle) -> Result<()>;
pub fn deref(cap: CapHandle, flags: Rights) -> Result<CapAccess>;
pub struct CapAccess { pub object_idx: u32 } // RAII guard
```

### src/task/process.rs — MEVCUT erisim (SADECE bunlar)
```rust
pub struct Process {
    pub pid: u64, pub name: String, pub state: ProcessState, pub is_user: bool,
    pub ctx: RegisterContext, pub kernel_stack: Box<[u8]>, pub entry: Option<extern "C" fn()>,
    pub user_cr3: u64, pub user_rsp: u64, pub user_rip: u64, pub user_ss: u16, pub user_cs: u16,
    pub kernel_rsp: u64, pub kernel_rip: u64, pub exit_code: u64, pub exited: bool,
    pub cap_table: alloc::vec::Vec<(u32, crate::cap::CapHandle)>,  // fd -> CapHandle
}
pub fn current_pid() -> u64;                        // mevcut pid
pub fn current_is_user_process() -> bool;
pub fn current_process_info() -> Option<(u64, String)>;  // (pid, name)
pub fn get_process(pid: u64) -> Option<(u64, String, bool, ProcessState)>; // SADECE snapshot, &mut YOK
// NOT: process cap_table'a erismek icin `SCHEDULER` static'ine lock acabilirsin:
//   `crate::task::process::SCHEDULER` pub mi kontrol et; degilse syscall_cap'i
//   `current_pid()` ile cap_table okuyan bir `pub fn cap_table_of(pid)->Option<&...>`
//   tutamazsan wrapper'i KERNEL tarafinda kapalii yaz — ama PURE core dilim uzerinde.
```

### src/syscall.rs (dispatcher, consts)
```rust
pub const SYS_READ: u64 = 0; pub const SYS_WRITE: u64 = 4; pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3; pub const SYS_LSEEK: u64 = 8; pub const SYS_EXIT: u64 = 1;
pub const SYS_YIELD: u64 = 9; pub const SYS_SOCKET: u64 = 10; pub const SYS_CONNECT: u64 = 11;
pub const SYS_SEND: u64 = 12; pub const SYS_RECV: u64 = 13; pub const SYS_FORK: u64 = 14;
pub const SYS_EXEC: u64 = 15;
const EFAULT: u64 = (-14i64) as u64;
pub extern "C" fn syscall_dispatcher(...) -> u64 { ... match ... }
```
`sys_write` icinde fd==1||fd==2 ozele dali vardir — gate BUNLARI da kapsar.

### src/syscall_storage.rs
```rust
pub fn sys_open(path_ptr: u64, flags: u64) -> u64;   // fd doner ya da -1
pub fn sys_read(fd: u64, buf_ptr: u64, len: u64) -> u64;
pub fn sys_write(fd: u64, buf_ptr: u64, len: u64) -> u64;
pub fn sys_close(fd: u64) -> u64;
pub fn sys_lseek(fd: u64, offset: i64, whence: u64) -> u64;
```

### src/net_socket.rs
```rust
pub fn sys_socket(domain: u64, kind: u64) -> u64;    // fd doner ya da -1
pub fn sys_connect(fd: u64, ip_packed: u64, port: u64) -> u64;
pub fn sys_send(fd: u64, buf_ptr: u64, len: u64) -> u64;
pub fn sys_recv(fd: u64, buf_ptr: u64, len: u64) -> u64;
```

## 3 dosya uret (FROZEN uyumlu)

### 1) src/syscall_cap.rs — tam dosya (YENI)
NO `#![no_std]` ve NO `extern crate alloc` icerime (modul olarak gelir; parent crate
no_std + alloc saglar — Asama 1'de birebir ayni hata yapildi ve derlenmedi).

Saf core (fcc Q1 — global'e baglanma, dilim argumani):
```rust
pub fn check_fd_access_in_table(table: &[(u32, CapHandle)], fd: u32, needed: Rights) -> cap::Result<()> {
    // tabloda fd ara, yoksa NotFound; handle'in rights'ina deref, yetmezse NoRights
}
```
Kernel wrapper'lari (current process uzerinden):
- `pub fn current_cap_table() -> Option<alloc::vec::Vec<(u32, CapHandle)>>`
  aktaracak sekilde kernel'dan tabloyu cek (SCHEDULER'a lock). Dur: SCHEDULER pub mu?
  asagida NOT. Erisilemiyorsa: `pub fn check_fd_access(fd, needed)` → `current_pid()`
  + `process::SCHEDULER` uzerinden. SCHEDULER'a erisim yoksa, `get_process` snapshot'ina
  dayan (iki asamali: `mut` gerektirmeyen bir yol bul — belki `pub fn cap_table(pid)`).
  YANLI context'te uretmektense: PURE fonksiyonu saglam yaz; kernel wrapper'i icin
  `current_pid()` + SCHEDULER'lı satirda `// KERNEL-WRAPPER` yorumu birak ve ana calisma
  dizinindeki `crate::task::process` erisimini dogru isimle doldur. Spec'te belirsizse
  PURENIN dogru oldugundan emin ol, wrapper'i minimal tut (icinde sadece table'i cek).
- `pub fn grant_fd_in_table(...)`, `pub fn close_fd_in_table(...)` (pure)
- `pub fn syscall_cap_init()` — B1: stdio seed. create_user_process/fork/exec'te bir
  kez cagrilacak: fd 0/1/2 icin `create_object(Fd)` + READ|WRITE grant + tabloya yaz.
  Ama bunu DISARIDAN cagirimasi kolay olsun: `pub fn seed_stdio(table: &mut Vec<(u32,CapHandle)>)`
  (pure, host test). Kernel, create_user_process sonunda bunu cagirir.
- `pub fn check_process_exec_for(pid)` — B3: process'in EXECUTE-right handle'i var mi?
  (root/self cap_table'inda EXECUTE veya create_object(Process)+grant(EXECUTE) ile seed.)
  Bu Asama 2'de "process'in kendisi exec edebilir mi" kuralini kurar.

FROZEN FIX-1: check + deref tek dilimde; CapAccess guard'lari scope sonu drop.

Host test (cfg(test)): PURE fonksiyonlari dogrudan düz Vec ile test et:
- tabloda fd yok -> NotFound
- rights yetmiyor -> NoRights
- seed_stdio sonrasi fd 0/1/2 -> ok
- close_fd_in_table sonrasi entry yok

### 2) src/syscall.rs — degisim
Dispatchera EACCES: `const EACCES: u64 = (-13i64) as u64;`
Her fd syscall basina gate (B1/B2 icin tablodaki handle'a bak):
- SYS_READ: gate(fd, READ) → sys_read
- SYS_WRITE: gate(fd, WRITE), fd 1/2 dallari dahil
- SYS_LSEEK: gate(fd, READ)
- SYS_SOCKET: → sys_socket (fd uretir; provision icin sys_socket icinde icrete_object+grant)
- SYS_CONNECT/SYS_SEND/SYS_RECV: gate(fd, IO)
- SYS_CLOSE: gate(fd, DESTROY) → sys_close
- SYS_FORK: check_process_exec (EXECUTE)
- SYS_EXEC: check_process_exec (EXECUTE)
Gate hata durumunda syscall'i ERKEN don; gecen islem yok.

### 3) src/syscall_storage.rs — degisim
sys_open icerisine, fd basarili acilinca (>=0 ise):
`syscall_cap` kullanarak `create_object(ObjectKind::Fd)` + tabloya grant. (B2'ye benzer
yaklasim.) Ve sys_close icinde close_fd'i bagla (G1).

### net_socket provision (B2)
sys_socket icinde fd olusturunca cap_table'a create_object(Fd)+grant.

## KISITLAR
- `main.rs`'e DOKUNMA (`pub mod syscall_cap;` HERMES ekler).
- Eski security.rs Capability(u64) KULLANMA. Uid/Gid'e DOKUNMA (o farkli concern).
- PROVISIONAL/DEFERRED kodlama. Rust 2021. Sonunda test blogu ekle.

Cikti:
```
## 1) syscall_cap.rs (tam)
<rust>
## 2) syscall.rs (degisim)
<rust>
## 3) syscall_storage.rs (degisim)
<rust>
## (istege bagli) net_socket.rs (degisim)
<rust>
## Tasarim notlari
```
