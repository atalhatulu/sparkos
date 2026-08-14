// This module is compiled inside the kernel crate (main.rs is `#![no_std]`), so
// it inherits no_std. `extern crate alloc` is needed both in the kernel (no_std
// build-std provides `alloc`) and in the host test crate (std host also exposes
// `alloc`); declaring it here is valid in both and does not duplicate the crate.
extern crate alloc;

use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use spin::{Mutex, Once};

// --- FROZEN DATA STRUCTURES ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapError {
    Invalid,
    Revoked,
    NoRights,
    NotFound,
    Exhausted,
    AlreadyExists,
}

pub type Result<T> = core::result::Result<T, CapError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapHandle {
    pub slot: u32,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rights(pub u32);

impl Rights {
    pub const READ: Rights     = Rights(1 << 0);
    pub const WRITE: Rights    = Rights(1 << 1);
    pub const MAP: Rights      = Rights(1 << 2);
    pub const IO: Rights       = Rights(1 << 3);
    pub const DMA: Rights      = Rights(1 << 4);
    pub const TRANSFER: Rights = Rights(1 << 5);
    pub const GRANT: Rights    = Rights(1 << 6);
    pub const DESTROY: Rights  = Rights(1 << 7);
    pub const EXECUTE: Rights  = Rights(1 << 8);
    pub const MANAGE: Rights   = Rights(1 << 9);

    pub fn empty() -> Self { Rights(0) }
    pub fn all() -> Self { Rights(0x3FF) }

    pub fn contains(&self, other: Rights) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl core::ops::BitAnd for Rights {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Rights(self.0 & rhs.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Memory,
    Device,
    Endpoint,
    Generic,
    Fd,
    Process,
}

pub struct CapNode {
    pub parent: Option<u32>,
    pub epoch: u64,
}

pub struct CapObject {
    pub kind: ObjectKind,
    pub refcount: u64,
    pub generation: u64,
    pub valid: bool,
}

// --- INTERNAL STATE ---

struct CapSlot {
    node_idx: u32,
    object_idx: u32,
    rights: Rights,
    generation: u32,
    free: bool,
    is_lent: bool,
}

struct CoreState {
    slots: Vec<CapSlot>,
    nodes: Vec<CapNode>,
    objects: Vec<CapObject>,
}

// Tüm mutasyonlar TEK spinlock altındadır.
static STATE: Mutex<Option<CoreState>> = Mutex::new(None);

/// Boot'ta `bootstrap_root` ile oluşturulan root capability'nin handle'ı.
///
/// Kernel-resident tutulur: hiçbir process'e verilmez, ama capability
/// hiyerarşisinin kök yetkisi olarak izlenebilir kalır (Aşama 5: kernel-resident
/// device registry / root process, türetmeler buradan başlar). `Once` — root
/// tam olarak bir kez, boot sırasında ve tek thread'de kurulur; sonra okunur.
pub static ROOT_CAP: Once<CapHandle> = Once::new();

/// Kayıtlı root capability handle'ını döner; `bootstrap_root` henüz çalışmadıysa
/// `None`. Capability sisteminin kök yetkisine erişmek isteyen kernel bileşenleri
/// (device registry, root process) bunu kullanır.
pub fn root_cap() -> Option<CapHandle> {
    ROOT_CAP.get().copied()
}

// --- HELPER FUNCTIONS ---

fn allocate_slot(state: &mut CoreState) -> usize {
    for (i, slot) in state.slots.iter_mut().enumerate() {
        if slot.free {
            return i;
        }
    }
    let idx = state.slots.len();
    state.slots.push(CapSlot {
        node_idx: 0,
        object_idx: 0,
        rights: Rights::empty(),
        generation: 1, // 0'dan farklı başlatıyoruz
        free: true,
        is_lent: false,
    });
    idx
}

fn is_revoked(state: &CoreState, node_idx: u32) -> bool {
    let mut curr = node_idx as usize;
    loop {
        // Lineage zinciri boyunca her node'un epoch'unu doğrula. Kırık -> Revoked
        if state.nodes[curr].epoch > 0 {
            return true;
        }
        if let Some(p) = state.nodes[curr].parent {
            curr = p as usize;
        } else {
            break;
        }
    }
    false
}

// --- PUBLIC API ---

pub fn init() {
    *STATE.lock() = Some(CoreState {
        slots: Vec::new(),
        nodes: Vec::new(),
        objects: Vec::new(),
    });
    // Device port bağlama kaydını da sıfırla — init() capability dünyasını
    // tazelediğinde eski bağlamalar (object_idx yeniden numaralanır) geçersizdir.
    DEVICE_PORT_BINDINGS.lock().clear();
}

/// Root capability'sini olusturup dondurur. Boot'ta root process'e verilir.
/// ObjectKind::Process tipinde bir obje yaratir ve temel yetkileri (READ|WRITE|
/// MAP|EXECUTE) ile bir handle doner. Asama 2.0 (fcc ön koşulu): capability core'un
/// boot'ta aktiflesmesi icin cagrilir — main.rs'te `cap::init()` + `bootstrap_root()`.
///
/// Dönen handle yalnizca izlenebilirlik icin ayni zamanda `ROOT_CAP` statigine
/// kaydedilir; kernel-resident kalir ve `root_cap()` ile erisilir. Cagiran
/// process yoktur — bu handle kernel'in kendi kok yetkisidir.
pub fn bootstrap_root() -> Result<CapHandle> {
    let root = create_object(ObjectKind::Process)?;
    let handle = grant(root, Rights(1 | 2 | 4 | 256))?; // READ|WRITE|MAP|EXECUTE
    ROOT_CAP.call_once(|| handle);
    Ok(handle)
}

pub fn create_object(kind: ObjectKind) -> Result<CapHandle> {
    let mut state_guard = STATE.lock();
    let state = state_guard.as_mut().ok_or(CapError::Invalid)?;

    let obj_idx = state.objects.len();
    state.objects.push(CapObject {
        kind,
        refcount: 0,
        generation: 1,
        valid: true,
    });

    let node_idx = state.nodes.len() as u32;
    state.nodes.push(CapNode {
        parent: None,
        epoch: 0,
    });

    let slot_idx = allocate_slot(state);
    let gen = state.slots[slot_idx].generation;
    state.slots[slot_idx] = CapSlot {
        node_idx,
        object_idx: obj_idx as u32,
        rights: Rights::all(),
        generation: gen,
        free: false,
        is_lent: false,
    };

    Ok(CapHandle {
        slot: slot_idx as u32,
        generation: gen,
    })
}

pub fn grant(parent: CapHandle, req: Rights) -> Result<CapHandle> {
    let mut state_guard = STATE.lock();
    let state = state_guard.as_mut().ok_or(CapError::Invalid)?;

    let p_slot_idx = parent.slot as usize;
    if p_slot_idx >= state.slots.len() { return Err(CapError::Invalid); }
    
    let p_slot = &state.slots[p_slot_idx];
    if p_slot.free || p_slot.generation != parent.generation {
        return Err(CapError::Invalid);
    }
    
    // INV-9: derived_rights <= parent_rights
    if !p_slot.rights.contains(req) {
        return Err(CapError::NoRights);
    }
    
    // Lend kısıtlaması
    if p_slot.is_lent {
        return Err(CapError::NoRights);
    }

    if is_revoked(state, p_slot.node_idx) {
        return Err(CapError::Revoked);
    }

    let p_node_idx = p_slot.node_idx;
    let obj_idx = p_slot.object_idx;
    let rights = p_slot.rights & req;

    let new_node_idx = state.nodes.len() as u32;
    state.nodes.push(CapNode {
        parent: Some(p_node_idx),
        epoch: 0,
    });

    let new_slot_idx = allocate_slot(state);
    let new_gen = state.slots[new_slot_idx].generation;
    state.slots[new_slot_idx] = CapSlot {
        node_idx: new_node_idx,
        object_idx: obj_idx,
        rights,
        generation: new_gen,
        free: false,
        is_lent: false,
    };

    Ok(CapHandle {
        slot: new_slot_idx as u32,
        generation: new_gen,
    })
}

pub fn transfer(src: CapHandle, req: Rights) -> Result<CapHandle> {
    let mut state_guard = STATE.lock();
    let state = state_guard.as_mut().ok_or(CapError::Invalid)?;

    let src_slot_idx = src.slot as usize;
    if src_slot_idx >= state.slots.len() { return Err(CapError::Invalid); }
    
    let src_slot = &state.slots[src_slot_idx];
    if src_slot.free || src_slot.generation != src.generation {
        return Err(CapError::Invalid);
    }
    
    if !src_slot.rights.contains(req) {
        return Err(CapError::NoRights);
    }
    
    if src_slot.is_lent {
        return Err(CapError::NoRights);
    }

    if is_revoked(state, src_slot.node_idx) {
        return Err(CapError::Revoked);
    }

    let obj_idx = src_slot.object_idx;
    let rights = src_slot.rights & req;

    // Yeni root yap (parent: None). Eski lineage revoke'undan etkilenmez.
    let new_node_idx = state.nodes.len() as u32;
    state.nodes.push(CapNode {
        parent: None,
        epoch: 0,
    });

    let new_slot_idx = allocate_slot(state);
    let new_gen = state.slots[new_slot_idx].generation;
    state.slots[new_slot_idx] = CapSlot {
        node_idx: new_node_idx,
        object_idx: obj_idx,
        rights,
        generation: new_gen,
        free: false,
        is_lent: false,
    };

    // Src handle kapatılır
    state.slots[src_slot_idx].free = true;
    state.slots[src_slot_idx].generation += 1;

    Ok(CapHandle {
        slot: new_slot_idx as u32,
        generation: new_gen,
    })
}

pub fn lend(parent: CapHandle, req: Rights) -> Result<CapHandle> {
    let mut state_guard = STATE.lock();
    let state = state_guard.as_mut().ok_or(CapError::Invalid)?;

    let p_slot_idx = parent.slot as usize;
    if p_slot_idx >= state.slots.len() { return Err(CapError::Invalid); }
    
    let p_slot = &state.slots[p_slot_idx];
    if p_slot.free || p_slot.generation != parent.generation {
        return Err(CapError::Invalid);
    }
    
    if !p_slot.rights.contains(req) {
        return Err(CapError::NoRights);
    }
    
    if p_slot.is_lent {
        return Err(CapError::NoRights);
    }

    if is_revoked(state, p_slot.node_idx) {
        return Err(CapError::Revoked);
    }

    let p_node_idx = p_slot.node_idx;
    let obj_idx = p_slot.object_idx;
    let rights = p_slot.rights & req;

    let new_node_idx = state.nodes.len() as u32;
    state.nodes.push(CapNode {
        parent: Some(p_node_idx),
        epoch: 0,
    });

    let new_slot_idx = allocate_slot(state);
    let new_gen = state.slots[new_slot_idx].generation;
    state.slots[new_slot_idx] = CapSlot {
        node_idx: new_node_idx,
        object_idx: obj_idx,
        rights,
        generation: new_gen,
        free: false,
        is_lent: true, // Tek seviye kısıtlaması
    };

    Ok(CapHandle {
        slot: new_slot_idx as u32,
        generation: new_gen,
    })
}

pub fn reclaim(cap: CapHandle) -> Result<()> {
    revoke(cap) // reclaim işlemi revoke ile özdeştir
}

pub fn close(cap: CapHandle) -> Result<()> {
    let mut state_guard = STATE.lock();
    let state = state_guard.as_mut().ok_or(CapError::Invalid)?;

    let slot_idx = cap.slot as usize;
    if slot_idx >= state.slots.len() { return Err(CapError::Invalid); }
    
    let slot = &mut state.slots[slot_idx];
    if slot.free || slot.generation != cap.generation {
        return Err(CapError::Invalid);
    }
    
    // Sadece handle free edilir, türetilmiş node'lara dokunmaz.
    slot.free = true;
    slot.generation += 1;
    
    Ok(())
}

pub fn revoke(cap: CapHandle) -> Result<()> {
    let mut state_guard = STATE.lock();
    let state = state_guard.as_mut().ok_or(CapError::Invalid)?;

    let slot_idx = cap.slot as usize;
    if slot_idx >= state.slots.len() { return Err(CapError::Invalid); }
    
    let slot = &state.slots[slot_idx];
    if slot.free || slot.generation != cap.generation {
        return Err(CapError::Invalid);
    }
    
    let node_idx = slot.node_idx as usize;
    state.nodes[node_idx].epoch = state.nodes[node_idx].epoch.saturating_add(1);
    
    Ok(())
}

/// Sürecin sahip olduğu bir capability'yi ve ona bağlı tüm türetilmiş soy ağacını (lineage)
/// deterministik olarak temizler (CAP_INV-13).
/// - `revoke(cap)` ile tüm alt dal geçersiz kılınır.
/// - `close(cap)` ile kendi handle slotu serbest bırakılır.
pub fn destroy_owned(cap: CapHandle) -> Result<()> {
    let _ = revoke(cap);
    close(cap)
}

/// Süreç sonlandığında (exit_current) CSpace tablosundaki tüm handle'ları temizler.
pub fn destroy_process_cspace(table: &mut alloc::vec::Vec<(u32, CapHandle)>) {
    for (_, handle) in table.iter() {
        let _ = destroy_owned(*handle);
    }
    table.clear();
}

/// Pasif erişim kontrolü: capability `needed` rights'ı içeriyor mu ve hâlâ canlı mı?
/// `deref`'ten farkı: refcount ARTTIRMAZ, CapAccess guard üretmez, object'i FREE
/// ETMEZ. fd-capability gibi KALICI kaynaklar için kullanılır — "check" bir syscall
/// gate'inde object ömrünü tüketmemelidir (deref+drop refcount'u 0'a düşürüp
/// obje'yi valid=false yapar; ikinci check Invalid verirdi).
pub fn check_rights(cap: CapHandle, needed: Rights) -> Result<()> {
    let state_guard = STATE.lock();
    let state = state_guard.as_ref().ok_or(CapError::Invalid)?;
    let slot_idx = cap.slot as usize;
    if slot_idx >= state.slots.len() {
        return Err(CapError::Invalid);
    }
    let slot = &state.slots[slot_idx];
    if slot.free || slot.generation != cap.generation {
        return Err(CapError::Invalid);
    }
    if !slot.rights.contains(needed) {
        return Err(CapError::NoRights);
    }
    if is_revoked(state, slot.node_idx) {
        return Err(CapError::Revoked);
    }
    let obj_idx = slot.object_idx as usize;
    if !state.objects[obj_idx].valid {
        return Err(CapError::Invalid);
    }
    // refcount'a dokunmaz, free etmez — sadece yetki ve canlılık.
    Ok(())
}

/// Belirli bir hedef nesneye (target) ait yetkiyi doğrular.
/// `cap`'in işaret ettiği CapObject ile `target` nesnesinin aynı olduğunu
/// denetleyerek yetki sızıntısını ve Confused Deputy açıklarını önler.
pub fn check_rights_for_object(cap: CapHandle, target: CapHandle, needed: Rights) -> Result<()> {
    let state_guard = STATE.lock();
    let state = state_guard.as_ref().ok_or(CapError::Invalid)?;
    
    let slot_idx = cap.slot as usize;
    let target_idx = target.slot as usize;
    if slot_idx >= state.slots.len() || target_idx >= state.slots.len() {
        return Err(CapError::Invalid);
    }
    let slot = &state.slots[slot_idx];
    let target_slot = &state.slots[target_idx];
    if slot.free || slot.generation != cap.generation || target_slot.free || target_slot.generation != target.generation {
        return Err(CapError::Invalid);
    }
    // Nesne kimliği doğrulaması (Capability-Resource eşleşmesi)
    if slot.object_idx != target_slot.object_idx {
        return Err(CapError::NoRights);
    }
    if !slot.rights.contains(needed) {
        return Err(CapError::NoRights);
    }
    if is_revoked(state, slot.node_idx) {
        return Err(CapError::Revoked);
    }
    let obj_idx = slot.object_idx as usize;
    if !state.objects[obj_idx].valid {
        return Err(CapError::Invalid);
    }
    Ok(())
}

/// Handle'ın işaret ettiği nesnenin tipini ve object_idx'ini doğrulanmış olarak
/// döndürür. `check_rights` gibi pasiftir — refcount'a dokunmaz, obje'yi free etmez.
/// Device port bağlama kaydı (Asama 4/5) object_idx'i anahtar olarak kullanır:
/// CapObject payload taşımadığı için cihaz→port aralığı eşlemesi kernel tarafında
/// bu anahtarla tutulur.
pub fn object_identity(cap: CapHandle) -> Result<(ObjectKind, u32)> {
    let state_guard = STATE.lock();
    let state = state_guard.as_ref().ok_or(CapError::Invalid)?;
    let slot_idx = cap.slot as usize;
    if slot_idx >= state.slots.len() {
        return Err(CapError::Invalid);
    }
    let slot = &state.slots[slot_idx];
    if slot.free || slot.generation != cap.generation {
        return Err(CapError::Invalid);
    }
    if is_revoked(state, slot.node_idx) {
        return Err(CapError::Revoked);
    }
    let obj_idx = slot.object_idx as usize;
    if !state.objects[obj_idx].valid {
        return Err(CapError::Invalid);
    }
    Ok((state.objects[obj_idx].kind, slot.object_idx))
}

// -----------------------------------------------------------------------------
// Device Port Binding Registry (Asama 4/5: per-device port I/O)
// -----------------------------------------------------------------------------
//
// CapObject payload taşımadığı için, bir Device nesnesinin yetkili olduğu port
// aralığı `object_idx` anahtarıyla kernel tarafında tutulur. sys_ioperm bu kaydı
// kullanarak istenen [start..=end] aralığının, process'in elinde tuttuğu Device
// capability'sine bağlı aralığın alt kümesi olduğunu doğrular.
//
// ÖNEMLİ (güvenlik düzeltmesi): sys_ioperm'deki eski boolean gate (`Rights::IO`
// taşıyan HERHANGİ bir handle yeterliydi) bir ayrıcalık yükseltme açığıydı — socket
// fd'leri de `Rights::IO` (8) taşır (SYS_CONNECT/SEND/RECV gate'i). Ağ erişimli bir
// process, hiçbir cihaza bağlı olmadan TÜM port aralıklarına erişim isteyebilirdi.
// Bu kayıt port erişimini yalnızca bağlı Device nesnelerine kısıtlar.

static DEVICE_PORT_BINDINGS: Mutex<BTreeMap<u32, (u16, u16)>> = Mutex::new(BTreeMap::new());

/// Yeni bir Device capability'si oluşturur ve `[start..=end_inclusive]` port
/// aralığına bağlar. Dönen handle MANAGE|IO yetkileri taşır; kernel bu handle'ı
/// `add_fd_to_current` ile ilgili driver process'ine grant eder (Asama 5.3).
pub fn create_device_ports(start: u16, end_inclusive: u16) -> Result<CapHandle> {
    let dev = create_object(ObjectKind::Device)?;
    let (kind, object_idx) = object_identity(dev)?;
    debug_assert_eq!(kind, ObjectKind::Device);
    DEVICE_PORT_BINDINGS.lock().insert(object_idx, (start, end_inclusive));
    // MANAGE (512): provisioning yönetimi; IO (8): sys_ioperm erişim gate'i.
    grant(dev, Rights(8 | 512))
}

/// `cap`'in işaret ettiği Device nesnesinin bağlı port aralığını doğrular:
/// cap canlı + IO yetkili + ObjectKind::Device olmalı ve kayıtlı aralığı olmalı.
/// Socket fd gibi IO yetkili ama Device olmayan nesneler NoRights ile reddedilir
/// (confused deputy: ağ fd'si port erişimi vermemelidir).
pub fn device_io_range(cap: CapHandle) -> Result<(u16, u16)> {
    check_rights(cap, Rights::IO)?;
    let (kind, object_idx) = object_identity(cap)?;
    if kind != ObjectKind::Device {
        return Err(CapError::NoRights);
    }
    DEVICE_PORT_BINDINGS
        .lock()
        .get(&object_idx)
        .copied()
        .ok_or(CapError::NoRights)
}

/// İstenen `[start..=end_inclusive]` aralığı, `cap`'in bağlı cihaz aralığının alt
/// kümesi mi? Değilse NoRights — sınır dışı port erişimi reddedilir.
pub fn port_range_allowed(cap: CapHandle, start: u16, end_inclusive: u16) -> Result<()> {
    let (bound_start, bound_end) = device_io_range(cap)?;
    if start < bound_start || end_inclusive > bound_end {
        return Err(CapError::NoRights);
    }
    Ok(())
}

// FIX-1: deref claim = generation check + epoch check + refcount++ AYNI KRİTİK DİLİMDE
pub fn deref(cap: CapHandle, flags: Rights) -> Result<CapAccess> {
    let mut state_guard = STATE.lock(); // Atomik dilim başlangıcı
    let state = state_guard.as_mut().ok_or(CapError::Invalid)?;

    let slot_idx = cap.slot as usize;
    if slot_idx >= state.slots.len() { return Err(CapError::Invalid); }
    
    let slot = &state.slots[slot_idx];
    if slot.free || slot.generation != cap.generation {
        return Err(CapError::Invalid); // 1. Generation Check
    }
    
    if !slot.rights.contains(flags) {
        return Err(CapError::NoRights);
    }
    
    if is_revoked(state, slot.node_idx) {
        return Err(CapError::Revoked); // 2. Epoch Check (chain doğrulaması)
    }
    
    let obj_idx = slot.object_idx as usize;
    let obj = &mut state.objects[obj_idx];
    
    if !obj.valid {
        return Err(CapError::Invalid); 
    }
    
    obj.refcount += 1; // 3. refcount++
    
    Ok(CapAccess {
        object_idx: obj_idx as u32,
    }) // Atomik dilim sonu (Drop ile kilit bırakılır)
}

pub struct CapAccess {
    object_idx: u32,
}

impl Drop for CapAccess {
    fn drop(&mut self) {
        if let Some(state) = STATE.lock().as_mut() {
            let obj = &mut state.objects[self.object_idx as usize];
            if obj.valid && obj.refcount > 0 {
                obj.refcount -= 1;
                // Son deref drop'unda refcount 0 -> FREE
                if obj.refcount == 0 {
                    obj.valid = false;
                    obj.generation += 1;
                }
            }
        }
    }
}

// --- UNIT TESTS ---
#[cfg(test)]
mod tests {
    use super::*;

    fn setup() {
        init();
    }

    #[test]
    fn test_inv1_grant_rights() {
        setup();
        let root = create_object(ObjectKind::Memory).unwrap();
        let restricted = grant(root, Rights::READ).unwrap();
        
        let res = grant(restricted, Rights::WRITE);
        assert_eq!(res.unwrap_err(), CapError::NoRights);
        
        let valid = grant(restricted, Rights::READ);
        assert!(valid.is_ok());
    }

    #[test]
    fn test_inv2_revoke_child_sibling_unaffected() {
        setup();
        let root = create_object(ObjectKind::Memory).unwrap();
        let child1 = grant(root, Rights::READ).unwrap();
        let child2 = grant(root, Rights::READ).unwrap();
        
        revoke(child1).unwrap();
        
        assert_eq!(deref(child1, Rights::READ).err(), Some(CapError::Revoked));
        assert!(deref(child2, Rights::READ).is_ok());
    }

    #[test]
    fn test_inv3_revoke_parent_kills_lineage() {
        setup();
        let root = create_object(ObjectKind::Memory).unwrap();
        let child = grant(root, Rights::READ).unwrap();
        let grandchild = grant(child, Rights::READ).unwrap();
        
        revoke(root).unwrap();
        
        assert_eq!(deref(child, Rights::READ).err(), Some(CapError::Revoked));
        assert_eq!(deref(grandchild, Rights::READ).err(), Some(CapError::Revoked));
    }

    #[test]
    fn test_inv4_transfer_escapes_revocation() {
        setup();
        let root = create_object(ObjectKind::Memory).unwrap();
        let child = grant(root, Rights::READ).unwrap();
        
        let transferred = transfer(child, Rights::READ).unwrap();
        
        revoke(root).unwrap();
        
        assert!(deref(transferred, Rights::READ).is_ok());
    }

    #[test]
    fn test_inv5_close_only_affects_handle() {
        setup();
        let root = create_object(ObjectKind::Memory).unwrap();
        let child = grant(root, Rights::READ).unwrap();
        
        close(root).unwrap();
        
        assert_eq!(deref(root, Rights::READ).err(), Some(CapError::Invalid));
        assert!(deref(child, Rights::READ).is_ok());
    }

    #[test]
    fn test_inv6_lend_semantics() {
        setup();
        let root = create_object(ObjectKind::Memory).unwrap();
        let lent = lend(root, Rights::READ).unwrap();
        
        assert_eq!(grant(lent, Rights::READ).err(), Some(CapError::NoRights));
        assert_eq!(transfer(lent, Rights::READ).err(), Some(CapError::NoRights));
        
        reclaim(lent).unwrap();
        assert_eq!(deref(lent, Rights::READ).err(), Some(CapError::Revoked));
    }

    #[test]
    fn test_inv7_generation_mismatch() {
        setup();
        let root = create_object(ObjectKind::Memory).unwrap();
        close(root).unwrap();
        assert_eq!(deref(root, Rights::all()).err(), Some(CapError::Invalid));
    }

    #[test]
    fn test_inv8_no_resurrection() {
        setup();
        let root = create_object(ObjectKind::Memory).unwrap();
        revoke(root).unwrap();
        assert_eq!(deref(root, Rights::all()).err(), Some(CapError::Revoked));
    }

    #[test]
    fn test_inv9_and_10_deref_refcount_and_free() {
        setup();
        let root = create_object(ObjectKind::Memory).unwrap();
        
        {
            let _access1 = deref(root, Rights::READ).unwrap();
            let _access2 = deref(root, Rights::READ).unwrap();
            
            {
                let mut state_guard = STATE.lock();
                let state = state_guard.as_mut().unwrap();
                let obj = &state.objects[0];
                assert_eq!(obj.refcount, 2);
                assert_eq!(obj.valid, true);
            } // Lock bırakıldı
        } // İki erişim RAII guard drop edildi
        
        let mut state_guard = STATE.lock();
        let state = state_guard.as_mut().unwrap();
        let obj = &state.objects[0];
        
        // refcount == 0 olduğunda object FREE durumuna düşer (header kalır, gen++)
        assert_eq!(obj.refcount, 0);
        assert_eq!(obj.valid, false); 
        assert_eq!(obj.generation, 2); 
        
        drop(state_guard);
        
        assert_eq!(deref(root, Rights::READ).err(), Some(CapError::Invalid));
    }

    // --- Asama 4: Device port binding registry ---

    #[test]
    fn test_device_port_binding_subset_check() {
        setup();
        // Serial: 0x3F8..=0x3FF
        let serial = create_device_ports(0x3F8, 0x3FF).unwrap();
        // Alt küme: geçerli
        assert!(port_range_allowed(serial, 0x3F8, 0x3FF).is_ok());
        assert!(port_range_allowed(serial, 0x3F9, 0x3FA).is_ok());
        // Sınır dışı: reddedilir (NoRights)
        assert_eq!(port_range_allowed(serial, 0x3F0, 0x3FF).err(), Some(CapError::NoRights));
        assert_eq!(port_range_allowed(serial, 0x3F8, 0x400).err(), Some(CapError::NoRights));
    }

    #[test]
    fn test_device_io_range_rejects_non_device() {
        setup();
        // Socket fd simülasyonu: IO (8) yetkili ama Device olmayan bir nesne.
        // Eski boolean gate bunu geçirirdi (ayrıcalık yükseltmesi); artık reddedilmeli.
        let sock = create_object(ObjectKind::Fd).unwrap();
        let sock_io = grant(sock, Rights(8)).unwrap();
        assert_eq!(device_io_range(sock_io).err(), Some(CapError::NoRights));
        assert_eq!(port_range_allowed(sock_io, 0x40, 0x43).err(), Some(CapError::NoRights));
    }

    #[test]
    fn test_device_port_binding_revoked_on_revoke() {
        setup();
        let dev = create_device_ports(0x3F8, 0x3FF).unwrap();
        assert!(device_io_range(dev).is_ok());
        revoke(dev).unwrap();
        // Kesilen lineage → Revoked (bağlama erişimi de ölür)
        assert_eq!(device_io_range(dev).err(), Some(CapError::Revoked));
        assert_eq!(port_range_allowed(dev, 0x3F8, 0x3FF).err(), Some(CapError::Revoked));
    }

    // --- Asama 5: root capability kernel-resident tutulur (Task #10) ---

    #[test]
    fn test_bootstrap_root_records_root_cap() {
        setup();
        let handle = bootstrap_root().unwrap();
        // bootstrap_root dönen handle'ı ROOT_CAP'e kaydeder — çöpe atılmaz.
        assert_eq!(root_cap(), Some(handle));
        // Handle hala geçerli bir Process objesine işaret eder (deref ok).
        assert!(deref(handle, Rights::READ).is_ok());
    }

    #[test]
    fn test_destroy_owned_cleans_subtree_and_handle() {
        setup();
        let root = create_object(ObjectKind::Memory).unwrap();
        let child = grant(root, Rights::READ).unwrap();
        let grandchild = grant(child, Rights::READ).unwrap();

        // child üzerinde destroy_owned çağrılır
        assert!(destroy_owned(child).is_ok());

        // child handle'ı artık geçersizdir (closed/invalid)
        assert_eq!(deref(child, Rights::READ).err(), Some(CapError::Invalid));

        // grandchild soy ağacı öldürülmüştür (revoked)
        assert_eq!(deref(grandchild, Rights::READ).err(), Some(CapError::Revoked));

        // root capability etkilenmez
        assert!(deref(root, Rights::all()).is_ok());
    }
}