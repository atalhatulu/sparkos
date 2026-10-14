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
}

impl MemObject {
    pub fn new(pages: usize, flags: u32, owner_pid: u64) -> Result<Self, &'static str> {
        let mut frames = Vec::with_capacity(pages);
        for _ in 0..pages {
            if let Some(frame) = crate::memory::user_alloc_frame() {
                frames.push(PhysAddr::new(frame.start_address().as_u64()));
            } else {
                // Tahsis başarısız olursa geri al
                for f in frames {
                    let frame_obj = PhysFrame::containing_address(f);
                    crate::memory::user_free_frame(frame_obj);
                }
                return Err("Out of physical frames");
            }
        }
        Ok(Self {
            pages,
            flags,
            owner_pid,
            frames,
            mapped_vaddrs: Vec::new(),
        })
    }
}

pub static MEM_OBJECT_STORE: IrqMutex<alloc::collections::BTreeMap<u32, MemObject>> = IrqMutex::new(alloc::collections::BTreeMap::new());

/// Yeni bir MemObject tahsis eder ve buna bağlı bir Capability Handle döner.
pub fn mem_create(pages: usize, flags: u32, pid: u64) -> cap::Result<CapHandle> {
    if pages == 0 || pages > 16384 { // maks 64 MB tek tahsis
        return Err(CapError::Invalid);
    }

    // 1. Kota denetimi
    let bytes = (pages as u64) * 4096;
    crate::task::process::try_charge_process_memory(pid, bytes)
        .map_err(|_| CapError::NoRights)?;

    // 2. Fiziksel sayfa tahsisi
    let obj = match MemObject::new(pages, flags, pid) {
        Ok(o) => o,
        Err(_) => {
            crate::task::process::uncharge_process_memory(pid, bytes);
            return Err(CapError::Exhausted);
        }
    };

    // 3. Cap oluştur
    let handle = match cap::create_object(ObjectKind::MemObject) {
        Ok(h) => h,
        Err(e) => {
            for f in &obj.frames {
                let frame_obj = PhysFrame::containing_address(*f);
                crate::memory::user_free_frame(frame_obj);
            }
            crate::task::process::uncharge_process_memory(pid, bytes);
            return Err(e);
        }
    };

    let access = cap::deref(handle, Rights::empty())?;
    let obj_idx = access.object_idx;

    MEM_OBJECT_STORE.lock().insert(obj_idx, obj);
    Ok(handle)
}

/// MemObject'i serbest bırakır ve sayfaları frame havuzuna iade eder.
pub fn mem_destroy(handle: CapHandle, pid: u64) -> cap::Result<()> {
    let access = cap::deref(handle, Rights::DESTROY)?;
    let obj_idx = access.object_idx;
    
    let mut store = MEM_OBJECT_STORE.lock();
    if let Some(obj) = store.remove(&obj_idx) {
        for f in obj.frames {
            let frame = PhysFrame::containing_address(f);
            crate::memory::user_free_frame(frame);
        }
        let bytes = (obj.pages as u64) * 4096;
        crate::task::process::uncharge_process_memory(pid, bytes);
    }
    drop(store);
    drop(access);
    cap::revoke(handle)
}
