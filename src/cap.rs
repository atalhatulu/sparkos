// This module is compiled inside the kernel crate (main.rs is `#![no_std]`), so
// it inherits no_std. `extern crate alloc` is needed both in the kernel (no_std
// build-std provides `alloc`) and in the host test crate (std host also exposes
// `alloc`); declaring it here is valid in both and does not duplicate the crate.
extern crate alloc;

use alloc::vec::Vec;
use spin::Mutex;

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

#[derive(Clone, Copy, Debug)]
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
}

/// Root capability'sini olusturup dondurur. Boot'ta root process'e verilir.
/// ObjectKind::Process tipinde bir obje yaratir ve temel yetkileri (READ|WRITE|
/// MAP|EXECUTE) ile bir handle doner. Asama 2.0 (fcc ön koşulu): capability core'un
/// boot'ta aktiflesmesi icin cagrilir — main.rs'te `cap::init()` + `bootstrap_root()`.
pub fn bootstrap_root() -> Result<CapHandle> {
    let root = create_object(ObjectKind::Process)?;
    grant(root, Rights(1 | 2 | 4 | 256)) // READ|WRITE|MAP|EXECUTE
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
}