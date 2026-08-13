//! L9 Security: Kullanıcı / Grup / İzin / Capability sistemi.
//!
//! Mevcut durumda syscall'lar yetki kontrolü YAPMADAN istenen her şeyi yapabilir
//! (bkz. EKSİK analizi). Bu modül, HERMES'in syscall.rs'e bağlayacağı
//! yetkilendirme altyapısını sağlar:
//!   - `Uid` / `Gid` (newtype, Copy)
//!   - `Capability` (u64 bitmask)
//!   - `Credentials` (uid, gid, gruplar, capability maskesi)
//!   - `SecurityManager`: kullanıcı sorgulama, `check_permission`, `syscall_capable`
//!
//! Built-in kullanıcılar: `root` (uid 0, tüm yetkiler), `user` (uid 1000, sınırlı),
//! `guest` (uid 65534, yetkisiz).

use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;

use crate::serial_println;

// ---------------------------------------------------------------------------
// Uid / Gid — newtype deseni (Copy)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uid(u32);

impl Uid {
    pub const ROOT: Uid = Uid(0);
    pub const USER: Uid = Uid(1000);
    pub const GUEST: Uid = Uid(65534);

    pub const fn new(v: u32) -> Uid {
        Uid(v)
    }

    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

impl From<u32> for Uid {
    fn from(v: u32) -> Uid {
        Uid(v)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Gid(u32);

impl Gid {
    pub const ROOT: Gid = Gid(0);
    pub const USER: Gid = Gid(1000);
    pub const GUEST: Gid = Gid(65534);

    pub const fn new(v: u32) -> Gid {
        Gid(v)
    }

    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

impl From<u32> for Gid {
    fn from(v: u32) -> Gid {
        Gid(v)
    }
}

// ---------------------------------------------------------------------------
// Capability — u64 bitmask
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability(u64);

impl Capability {
    pub const NONE: Capability = Capability(0);
    /// Sistem yönetimi (mount, device, vs.)
    pub const SYS_ADMIN: Capability = Capability(1 << 0);
    /// Dosya okuma
    pub const FILE_READ: Capability = Capability(1 << 1);
    /// Dosya yazma
    pub const FILE_WRITE: Capability = Capability(1 << 2);
    /// Ağ (socket, paket gönderme/alma)
    pub const NET: Capability = Capability(1 << 3);
    /// Sistem saati değiştirme
    pub const SYS_TIME: Capability = Capability(1 << 4);
    /// Ham I/O (port, DMA)
    pub const SYS_RAWIO: Capability = Capability(1 << 5);
    /// Kernel modülü yükleme
    pub const SYS_MODULE: Capability = Capability(1 << 6);
    /// Başka process'i sonlandırma
    pub const PROCESS_KILL: Capability = Capability(1 << 7);
    /// Tüm yetkiler
    pub const ALL: Capability = Capability(u64::MAX);

    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// `self` maskesi `other`'ın tüm bitlerini içeriyor mu?
    pub const fn contains(self, other: Capability) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn add(self, other: Capability) -> Capability {
        Capability(self.0 | other.0)
    }

    pub const fn remove(self, other: Capability) -> Capability {
        Capability(self.0 & !other.0)
    }
}

impl core::ops::BitOr for Capability {
    type Output = Capability;
    fn bitor(self, rhs: Capability) -> Capability {
        self.add(rhs)
    }
}

impl core::ops::BitOrAssign for Capability {
    fn bitor_assign(&mut self, rhs: Capability) {
        *self = self.add(rhs);
    }
}

// ---------------------------------------------------------------------------
// Permission — yüksek seviye operasyon → gerektirdiği capability
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Admin,
    ReadFile,
    WriteFile,
    Network,
    RawIo,
    SetTime,
    LoadModule,
    KillProcess,
}

impl Permission {
    pub const fn required_cap(self) -> Capability {
        match self {
            Permission::Admin => Capability::SYS_ADMIN,
            Permission::ReadFile => Capability::FILE_READ,
            Permission::WriteFile => Capability::FILE_WRITE,
            Permission::Network => Capability::NET,
            Permission::RawIo => Capability::SYS_RAWIO,
            Permission::SetTime => Capability::SYS_TIME,
            Permission::LoadModule => Capability::SYS_MODULE,
            Permission::KillProcess => Capability::PROCESS_KILL,
        }
    }
}

// ---------------------------------------------------------------------------
// Dosya izin bitleri (Unix 9-bit mode)
// ---------------------------------------------------------------------------

pub const MODE_OWNER_READ: u16 = 0o400;
pub const MODE_OWNER_WRITE: u16 = 0o200;
pub const MODE_OWNER_EXEC: u16 = 0o100;
pub const MODE_GROUP_READ: u16 = 0o040;
pub const MODE_GROUP_WRITE: u16 = 0o020;
pub const MODE_GROUP_EXEC: u16 = 0o010;
pub const MODE_OTHER_READ: u16 = 0o004;
pub const MODE_OTHER_WRITE: u16 = 0o002;
pub const MODE_OTHER_EXEC: u16 = 0o001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAccess {
    Read,
    Write,
    Execute,
}

// ---------------------------------------------------------------------------
// User / Group
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct User {
    pub uid: Uid,
    pub gid: Gid,
    pub name: &'static str,
    pub caps: Capability,
}

#[derive(Debug, Clone, Copy)]
pub struct Group {
    pub gid: Gid,
    pub name: &'static str,
}

// ---------------------------------------------------------------------------
// Credentials — bir process'in kimliği
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Credentials {
    pub uid: Uid,
    pub gid: Gid,
    pub groups: Vec<Gid>,
    pub caps: Capability,
}

impl Credentials {
    /// Varsayılan: yetkisiz kimlik (güvenli varsayılan — hiçbir cap yok).
    pub fn new(uid: Uid, gid: Gid) -> Credentials {
        Credentials {
            uid,
            gid,
            groups: Vec::new(),
            caps: Capability::NONE,
        }
    }

    /// root (uid 0): tüm yetkiler.
    pub fn root() -> Credentials {
        Credentials {
            uid: Uid::ROOT,
            gid: Gid::ROOT,
            groups: Vec::new(),
            caps: Capability::ALL,
        }
    }

    pub fn add_group(&mut self, gid: Gid) {
        if !self.groups.contains(&gid) {
            self.groups.push(gid);
        }
    }

    pub fn add_cap(&mut self, cap: Capability) {
        self.caps |= cap;
    }

    pub fn has_cap(&self, cap: Capability) -> bool {
        self.caps.contains(cap)
    }

    pub fn is_root(&self) -> bool {
        self.uid == Uid::ROOT
    }
}

// ---------------------------------------------------------------------------
// SecurityManager
// ---------------------------------------------------------------------------

pub struct SecurityManager {
    users: Vec<User>,
    groups: Vec<Group>,
}

impl SecurityManager {
    pub fn new() -> SecurityManager {
        let users = vec![
            User {
                uid: Uid::ROOT,
                gid: Gid::ROOT,
                name: "root",
                caps: Capability::ALL,
            },
            User {
                uid: Uid::USER,
                gid: Gid::USER,
                name: "user",
                caps: Capability::FILE_READ | Capability::FILE_WRITE | Capability::NET,
            },
            User {
                uid: Uid::GUEST,
                gid: Gid::GUEST,
                name: "guest",
                caps: Capability::NONE,
            },
        ];
        let groups = vec![
            Group { gid: Gid::ROOT, name: "root" },
            Group { gid: Gid::USER, name: "user" },
            Group { gid: Gid::GUEST, name: "nogroup" },
        ];
        SecurityManager { users, groups }
    }

    pub fn get_user(&self, uid: Uid) -> Option<&User> {
        self.users.iter().find(|u| u.uid == uid)
    }

    pub fn lookup_user(&self, name: &str) -> Option<&User> {
        self.users.iter().find(|u| u.name == name)
    }

    pub fn get_group(&self, gid: Gid) -> Option<&Group> {
        self.groups.iter().find(|g| g.gid == gid)
    }

    /// Bir UID için `Credentials` üretir: kullanıcının capability'lerini yükler
    /// ve birincil grubunu gruplar listesine ekler.
    pub fn credentials_for(&self, uid: Uid) -> Option<Credentials> {
        let user = self.get_user(uid)?;
        let mut creds = Credentials::new(user.uid, user.gid);
        creds.caps = user.caps;
        if let Some(g) = self.get_group(user.gid) {
            creds.add_group(g.gid);
        }
        Some(creds)
    }

    /// Yüksek seviye izin kontrolü: `required` operasyon, capability'ye eşlenir.
    pub fn check_permission(&self, creds: &Credentials, required: Permission) -> bool {
        self.syscall_capable(creds, required.required_cap())
    }

    /// Capability kontrolü. root (uid 0) her zaman tüm yetkilere sahiptir.
    pub fn syscall_capable(&self, creds: &Credentials, cap: Capability) -> bool {
        if creds.is_root() {
            return true;
        }
        creds.caps.contains(cap)
    }

    /// Unix 9-bit dosya izin kontrolü (owner/group/other rwx).
    pub fn check_file_mode(
        &self,
        creds: &Credentials,
        access: FileAccess,
        owner: Uid,
        group: Gid,
        mode: u16,
    ) -> bool {
        // Unix kuralı: root dosya izinlerinden bağımsız erişir.
        if creds.is_root() {
            return true;
        }

        let bits = match access {
            FileAccess::Read => (MODE_OWNER_READ, MODE_GROUP_READ, MODE_OTHER_READ),
            FileAccess::Write => (MODE_OWNER_WRITE, MODE_GROUP_WRITE, MODE_OTHER_WRITE),
            FileAccess::Execute => (MODE_OWNER_EXEC, MODE_GROUP_EXEC, MODE_OTHER_EXEC),
        };

        if creds.uid == owner {
            mode & bits.0 != 0
        } else if creds.gid == group || creds.groups.contains(&group) {
            mode & bits.1 != 0
        } else {
            mode & bits.2 != 0
        }
    }
}

// ---------------------------------------------------------------------------
// Global SecurityManager (Mutex'li)
// ---------------------------------------------------------------------------

pub static SECURITY: spin::Lazy<Mutex<SecurityManager>> =
    spin::Lazy::new(|| Mutex::new(SecurityManager::new()));

/// L9 demosu: `check_permission`, `syscall_capable` ve `check_file_mode`
/// örnekleri. (syscall.rs'e bağlama HERMES'in işi.)
pub fn demo() {
    let man = SECURITY.lock();

    let root = Credentials::root();
    let user = man.credentials_for(Uid::USER).unwrap_or_else(|| Credentials::new(Uid::USER, Gid::USER));
    let guest = man.credentials_for(Uid::GUEST).unwrap_or_else(|| Credentials::new(Uid::GUEST, Gid::GUEST));

    serial_println!("[SEC] -- capability demo --");
    serial_println!("[SEC] root  CAP_SYS_ADMIN : {}", man.syscall_capable(&root, Capability::SYS_ADMIN));
    serial_println!("[SEC] user  CAP_FILE_WRITE: {}", man.syscall_capable(&user, Capability::FILE_WRITE));
    serial_println!("[SEC] user  CAP_SYS_ADMIN : {}", man.syscall_capable(&user, Capability::SYS_ADMIN));
    serial_println!("[SEC] guest CAP_FILE_WRITE: {}", man.syscall_capable(&guest, Capability::FILE_WRITE));
    serial_println!("[SEC] guest CAP_NET      : {}", man.syscall_capable(&guest, Capability::NET));

    serial_println!("[SEC] -- permission demo (check_permission) --");
    serial_println!("[SEC] root  WriteFile: {}", man.check_permission(&root, Permission::WriteFile));
    serial_println!("[SEC] user  ReadFile : {}", man.check_permission(&user, Permission::ReadFile));
    serial_println!("[SEC] user  Admin    : {}", man.check_permission(&user, Permission::Admin));
    serial_println!("[SEC] guest WriteFile: {}", man.check_permission(&guest, Permission::WriteFile));

    serial_println!("[SEC] -- file mode demo (check_file_mode, root:rw- group:r-- other:r--) --");
    let mode = 0o644;
    serial_println!("[SEC] root  read  : {}", man.check_file_mode(&root, FileAccess::Read, Uid::ROOT, Gid::ROOT, mode));
    serial_println!("[SEC] root  write : {}", man.check_file_mode(&root, FileAccess::Write, Uid::ROOT, Gid::ROOT, mode));
    serial_println!("[SEC] user  read  : {}", man.check_file_mode(&user, FileAccess::Read, Uid::ROOT, Gid::ROOT, mode));
    serial_println!("[SEC] user  write : {}", man.check_file_mode(&user, FileAccess::Write, Uid::ROOT, Gid::ROOT, mode));
    serial_println!("[SEC] guest read  : {}", man.check_file_mode(&guest, FileAccess::Read, Uid::ROOT, Gid::ROOT, mode));
    serial_println!("[SEC] guest write : {}", man.check_file_mode(&guest, FileAccess::Write, Uid::ROOT, Gid::ROOT, mode));
}
