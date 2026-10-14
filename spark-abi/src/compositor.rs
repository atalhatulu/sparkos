//! SparkOS Compositor & Window Manager IPC Protocol (Faz 5.4)
//!
//! Provides ABI structures for Ring-3 Compositor, Surface Shared Memory Sharing (MemObject),
//! Damage Tracking Presentation, and Window Management IPC.

pub const COMP_OP_CREATE_SURFACE: u32 = 1;
pub const COMP_OP_PRESENT_SURFACE: u32 = 2;
pub const COMP_OP_DESTROY_SURFACE: u32 = 3;
pub const COMP_OP_CREATE_WINDOW: u32 = 4;
pub const COMP_OP_DESTROY_WINDOW: u32 = 5;
pub const COMP_OP_MOVE_WINDOW: u32 = 6;
pub const COMP_OP_FOCUS_WINDOW: u32 = 7;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompositorRequest {
    pub opcode: u32,
    pub client_pid: u64,
    pub surface_id: u32,
    pub window_id: u32,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub damage_x: u32,
    pub damage_y: u32,
    pub damage_w: u32,
    pub damage_h: u32,
}

impl CompositorRequest {
    pub const fn empty() -> Self {
        Self {
            opcode: 0,
            client_pid: 0,
            surface_id: 0,
            window_id: 0,
            width: 0,
            height: 0,
            x: 0,
            y: 0,
            damage_x: 0,
            damage_y: 0,
            damage_w: 0,
            damage_h: 0,
        }
    }

    pub fn new_create_surface(client_pid: u64, width: u32, height: u32) -> Self {
        let mut req = Self::empty();
        req.opcode = COMP_OP_CREATE_SURFACE;
        req.client_pid = client_pid;
        req.width = width;
        req.height = height;
        req
    }

    pub fn new_present(client_pid: u64, surface_id: u32, dx: u32, dy: u32, dw: u32, dh: u32) -> Self {
        let mut req = Self::empty();
        req.opcode = COMP_OP_PRESENT_SURFACE;
        req.client_pid = client_pid;
        req.surface_id = surface_id;
        req.damage_x = dx;
        req.damage_y = dy;
        req.damage_w = dw;
        req.damage_h = dh;
        req
    }

    pub fn new_create_window(client_pid: u64, surface_id: u32, x: i32, y: i32, w: u32, h: u32) -> Self {
        let mut req = Self::empty();
        req.opcode = COMP_OP_CREATE_WINDOW;
        req.client_pid = client_pid;
        req.surface_id = surface_id;
        req.x = x;
        req.y = y;
        req.width = w;
        req.height = h;
        req
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompositorResponse {
    pub status: u32,
    pub surface_id: u32,
    pub window_id: u32,
    pub shmem_vaddr: u64,
    pub shmem_size: u64,
}

impl CompositorResponse {
    pub const fn ok_surface(surface_id: u32, vaddr: u64, size: u64) -> Self {
        Self {
            status: 0,
            surface_id,
            window_id: 0,
            shmem_vaddr: vaddr,
            shmem_size: size,
        }
    }

    pub const fn ok_window(window_id: u32) -> Self {
        Self {
            status: 0,
            surface_id: 0,
            window_id,
            shmem_vaddr: 0,
            shmem_size: 0,
        }
    }

    pub const fn err(errno_val: u32) -> Self {
        Self {
            status: errno_val,
            surface_id: 0,
            window_id: 0,
            shmem_vaddr: 0,
            shmem_size: 0,
        }
    }

    pub fn is_ok(&self) -> bool {
        self.status == 0
    }
}
