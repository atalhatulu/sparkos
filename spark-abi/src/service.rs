//! SparkOS Service Manager & Discovery IPC Protocol (Faz 5.0)
//!
//! Provides ABI structures and opcodes for Ring-3 Service Manager,
//! Service Discovery (Name -> Endpoint Capability), Readiness Handshake,
//! Heartbeat Monitoring, and Reconnect Protocol after crash recovery.


pub const MAX_SERVICE_NAME_LEN: usize = 32;

pub const SVC_OP_REGISTER: u32 = 1;
pub const SVC_OP_LOOKUP: u32 = 2;
pub const SVC_OP_READY: u32 = 3;
pub const SVC_OP_HEARTBEAT: u32 = 4;
pub const SVC_OP_STATUS: u32 = 5;
pub const SVC_OP_RECONNECT: u32 = 6;
pub const SVC_OP_STOP: u32 = 7;

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartPolicy {
    Always = 0,
    OnFailure = 1,
    Never = 2,
}

impl RestartPolicy {
    pub fn from_u32(val: u32) -> Self {
        match val {
            0 => RestartPolicy::Always,
            1 => RestartPolicy::OnFailure,
            _ => RestartPolicy::Never,
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceState {
    Stopped = 0,
    Starting = 1,
    Running = 2,
    Restarting = 3,
    Failed = 4,
}

impl ServiceState {
    pub fn from_u32(val: u32) -> Self {
        match val {
            0 => ServiceState::Stopped,
            1 => ServiceState::Starting,
            2 => ServiceState::Running,
            3 => ServiceState::Restarting,
            _ => ServiceState::Failed,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceRequest {
    pub opcode: u32,
    pub client_pid: u64,
    pub service_name: [u8; MAX_SERVICE_NAME_LEN],
    pub name_len: u32,
    pub extra: u64,
}

impl ServiceRequest {
    pub const fn empty() -> Self {
        Self {
            opcode: 0,
            client_pid: 0,
            service_name: [0u8; MAX_SERVICE_NAME_LEN],
            name_len: 0,
            extra: 0,
        }
    }

    pub fn new_register(name: &str, client_pid: u64) -> Self {
        let mut req = Self::empty();
        req.opcode = SVC_OP_REGISTER;
        req.client_pid = client_pid;
        req.set_name(name);
        req
    }

    pub fn new_lookup(name: &str, client_pid: u64) -> Self {
        let mut req = Self::empty();
        req.opcode = SVC_OP_LOOKUP;
        req.client_pid = client_pid;
        req.set_name(name);
        req
    }

    pub fn new_ready(service_pid: u64, status: u32) -> Self {
        let mut req = Self::empty();
        req.opcode = SVC_OP_READY;
        req.client_pid = service_pid;
        req.extra = status as u64;
        req
    }

    pub fn new_heartbeat(service_pid: u64) -> Self {
        let mut req = Self::empty();
        req.opcode = SVC_OP_HEARTBEAT;
        req.client_pid = service_pid;
        req
    }

    pub fn new_reconnect(name: &str, client_pid: u64) -> Self {
        let mut req = Self::empty();
        req.opcode = SVC_OP_RECONNECT;
        req.client_pid = client_pid;
        req.set_name(name);
        req
    }

    pub fn set_name(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(MAX_SERVICE_NAME_LEN);
        self.service_name = [0u8; MAX_SERVICE_NAME_LEN];
        self.service_name[..len].copy_from_slice(&bytes[..len]);
        self.name_len = len as u32;
    }

    pub fn name_str(&self) -> &str {
        let len = (self.name_len as usize).min(MAX_SERVICE_NAME_LEN);
        core::str::from_utf8(&self.service_name[..len]).unwrap_or("")
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceResponse {
    pub status: u32,
    pub endpoint_id: u32,
    pub service_pid: u64,
    pub generation: u32,
    pub restart_count: u32,
}

impl ServiceResponse {
    pub const fn ok(endpoint_id: u32, service_pid: u64, generation: u32, restart_count: u32) -> Self {
        Self {
            status: 0,
            endpoint_id,
            service_pid,
            generation,
            restart_count,
        }
    }

    pub const fn err(errno_val: u32) -> Self {
        Self {
            status: errno_val,
            endpoint_id: 0,
            service_pid: 0,
            generation: 0,
            restart_count: 0,
        }
    }

    pub fn is_ok(&self) -> bool {
        self.status == 0
    }
}
