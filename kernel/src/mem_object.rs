//! SparkOS MemObject (VMO Benzeri Paylaşımlı Bellek Soyutlaması)
//!
//! Fiziksel bellek tahsislerini capability güvencesiyle temsil eder.
//! Eager sayfa tahsisi, hak denetimi (READ, WRITE, MAP, DESTROY) ve
//! süreç bellek kotasıyla doğrudan entegredir.

use alloc::vec::Vec;
use x86_64::PhysAddr;
use x86_64::structures::paging::PhysFrame;
use crate::sync::IrqMutex;
use crate::cap::{self, CapError, CapHandle, ObjectKind, Rights};

pub const MEM_FLAG_READ: u32 = 1 << 0;
pub const MEM_FLAG_WRITE: u32 = 1 << 1;
pub const MEM_FLAG_EXEC: u32 = 1 << 2;
pub const MEM_FLAG_DMA: u32 = 1 << 3;

pub struct MemObject {
    pub pages: usize,
    pub flags: u32,
    pub owner_pid: u64,
    pub frames: Vec<PhysAddr>,
    pub mapped_vaddrs: Vec<(u64, u64)>, // (pid, vaddr)
    pub ref_count: usize,
    pub destroyed: bool,
}

impl Drop for MemObject {
    fn drop(&mut self) {
        let bytes = (self.pages as u64) * 4096;
        crate::task::process::uncharge_process_memory(self.owner_pid, bytes);
        for f in self.frames.drain(..) {
            let frame_obj = PhysFrame::containing_address(f);
            crate::memory::user_free_frame(frame_obj);
        }
    }
}

impl MemObject {
    pub fn new(pages: usize, flags: u32, owner_pid: u64) -> Result<Self, &'static str> {
        let mut frames = Vec::with_capacity(pages);
        for _ in 0..pages {
            if let Some(frame) = crate::memory::user_alloc_frame() {
                frames.push(PhysAddr::new(frame.start_address().as_u64()));
            } else {
                return Err("Out of physical frames");
            }
        }
        Ok(Self {
            pages,
            flags,
            owner_pid,
            frames,
            mapped_vaddrs: Vec::new(),
            ref_count: 1,
            destroyed: false,
        })
    }
}

pub static MEM_OBJECT_STORE: IrqMutex<alloc::collections::BTreeMap<u32, MemObject>> = IrqMutex::new(alloc::collections::BTreeMap::new());

/// Yeni bir MemObject tahsis eder ve buna bağlı bir Capability Handle döner.
pub fn mem_create(pages: usize, flags: u32, pid: u64) -> cap::Result<CapHandle> {
    if pages == 0 || pages > 16384 { // maks 64 MB tek tahsis
        return Err(CapError::Invalid);
    }

    // 1. Kota denetimi (kota hatası EQUOTA / QuotaExceeded olarak döner)
    let bytes = (pages as u64) * 4096;
    crate::task::process::try_charge_process_memory(pid, bytes)
        .map_err(|_| CapError::QuotaExceeded)?;

    // 2. Fiziksel sayfa tahsisi
    let mut obj = match MemObject::new(pages, flags, pid) {
        Ok(o) => o,
        Err(_) => {
            crate::task::process::uncharge_process_memory(pid, bytes);
            return Err(CapError::Exhausted);
        }
    };

    // 3. (e) create_object ile store.insert arası pencereyi kapat:
    // Store kilidi altında cap oluşturulup store'a eklenir.
    let mut store = MEM_OBJECT_STORE.lock();
    let handle = match cap::create_object(ObjectKind::MemObject) {
        Ok(h) => h,
        Err(e) => {
            crate::task::process::uncharge_process_memory(pid, bytes);
            // obj drop ile ayrılan sayfaları otomatik iade eder
            return Err(e);
        }
    };

    let (_, obj_idx) = match cap::object_identity(handle) {
        Ok(info) => info,
        Err(e) => {
            crate::task::process::uncharge_process_memory(pid, bytes);
            let _ = cap::revoke(handle);
            let _ = cap::close(handle);
            return Err(e);
        }
    };

    obj.ref_count = 1;
    store.insert(obj_idx, obj);
    drop(store);

    Ok(handle)
}

/// MemObject'i serbest bırakır: kotayı owner_pid'e iade eder ve aktif eşleme yoksa sayfaları serbest bırakır.
pub fn mem_destroy(handle: CapHandle, _caller_pid: u64) -> cap::Result<()> {
    cap::check_rights(handle, Rights::DESTROY)?;
    let (_, obj_idx) = cap::object_identity(handle)?;

    let mut store = MEM_OBJECT_STORE.lock();
    let obj = store.get_mut(&obj_idx).ok_or(CapError::NotFound)?;

    if !obj.destroyed {
        obj.destroyed = true;
    }

    if obj.mapped_vaddrs.is_empty() && obj.ref_count <= 1 {
        store.remove(&obj_idx); // Drop çalışır ve frames serbest kalır
        let _ = cap::revoke(handle);
    } else if obj.ref_count > 0 {
        obj.ref_count -= 1;
    }
    drop(store);

    let _ = cap::close(handle);
    Ok(())
}

/// MemObject capability'sini çoğaltır ve referans sayımını artırır.
pub fn mem_dup(handle: CapHandle, rights: Rights) -> cap::Result<CapHandle> {
    cap::check_rights(handle, Rights::GRANT)?;
    let (_, obj_idx) = cap::object_identity(handle)?;

    let mut store = MEM_OBJECT_STORE.lock();
    let obj = store.get_mut(&obj_idx).ok_or(CapError::NotFound)?;
    if obj.destroyed {
        return Err(CapError::Revoked);
    }

    let new_handle = cap::grant(handle, rights)?;
    obj.ref_count = obj.ref_count.saturating_add(1);
    Ok(new_handle)
}

/// MemObject'i hedef sürecin adres uzayına eşler.
pub fn mem_map(handle: CapHandle, target_pid: u64, target_vaddr: u64, writable: bool) -> cap::Result<u64> {
    cap::check_rights(handle, Rights::MAP)?;
    if writable {
        cap::check_rights(handle, Rights::WRITE)?;
    }
    let (_, obj_idx) = cap::object_identity(handle)?;

    let mut store = MEM_OBJECT_STORE.lock();
    let obj = store.get_mut(&obj_idx).ok_or(CapError::NotFound)?;
    if obj.destroyed {
        return Err(CapError::Revoked);
    }

    let target_cr3 = {
        let mut sched = crate::task::process::SCHEDULER.lock();
        let proc = sched.get_process_mut(target_pid).ok_or(CapError::NotFound)?;
        if proc.exited || proc.reaped || proc.user_cr3 == 0 {
            return Err(CapError::Invalid);
        }
        proc.user_cr3
    };

    let mut mapped_count = 0;
    for (i, frame_addr) in obj.frames.iter().enumerate() {
        let v = target_vaddr + (i as u64) * 4096;
        let p = *frame_addr;
        if let Err(_) = crate::memory::map_user_phys_range_in_cr3(target_cr3, v, p, 1, writable) {
            for j in 0..mapped_count {
                let unmap_v = target_vaddr + (j as u64) * 4096;
                let _ = crate::memory::unmap_user_phys_range_in_cr3(target_cr3, unmap_v, 1);
            }
            return Err(CapError::Exhausted);
        }
        mapped_count += 1;
    }

    obj.mapped_vaddrs.push((target_pid, target_vaddr));
    obj.ref_count = obj.ref_count.saturating_add(1);
    Ok(target_vaddr)
}

/// MemObject eşlemesini kaldırır. Eşleme kalmamışsa ve nesne yok edilmişse çerçeveler serbest bırakılır.
pub fn mem_unmap(handle: CapHandle, target_pid: u64, target_vaddr: u64) -> cap::Result<()> {
    cap::check_rights(handle, Rights::MAP)?;
    let (_, obj_idx) = cap::object_identity(handle)?;

    let mut store = MEM_OBJECT_STORE.lock();
    let obj = store.get_mut(&obj_idx).ok_or(CapError::NotFound)?;

    let pos = obj.mapped_vaddrs.iter().position(|&(p, v)| p == target_pid && v == target_vaddr)
        .ok_or(CapError::NotFound)?;
    let pages = obj.pages;
    obj.mapped_vaddrs.remove(pos);

    if obj.ref_count > 0 {
        obj.ref_count -= 1;
    }

    if let Some(target_cr3) = crate::task::process::SCHEDULER.lock().get_process(target_pid).map(|p| p.user_cr3) {
        if target_cr3 != 0 {
            for i in 0..pages {
                let v = target_vaddr + (i as u64) * 4096;
                let _ = crate::memory::unmap_user_phys_range_in_cr3(target_cr3, v, 1);
            }
        }
    }

    if obj.destroyed && obj.mapped_vaddrs.is_empty() && obj.ref_count <= 1 {
        store.remove(&obj_idx);
    }

    Ok(())
}

