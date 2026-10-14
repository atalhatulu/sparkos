//! SparkOS Network Service IPC Protocol (Faz 5.3)
//!
//! Provides ABI structures for Ring-3 Network Service, Zero-Copy Packet Rings,
//! and Socket IPC Operations (Socket/Bind/Listen/Connect/Send/Recv/Close).

pub const NET_OP_SOCKET: u32 = 1;
pub const NET_OP_BIND: u32 = 2;
pub const NET_OP_LISTEN: u32 = 3;
pub const NET_OP_CONNECT: u32 = 4;
pub const NET_OP_SEND: u32 = 5;
pub const NET_OP_RECV: u32 = 6;
pub const NET_OP_CLOSE: u32 = 7;

pub const IPPROTO_TCP: u32 = 6;
pub const IPPROTO_UDP: u32 = 17;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetRequest {
    pub opcode: u32,
    pub socket_id: u32,
    pub proto: u32,
    pub ip: [u8; 4],
    pub port: u16,
    pub _padding: u16,
    pub payload_len: u32,
}

impl NetRequest {
    pub const fn empty() -> Self {
        Self {
            opcode: 0,
            socket_id: 0,
            proto: 0,
            ip: [0; 4],
            port: 0,
            _padding: 0,
            payload_len: 0,
        }
    }

    pub fn new_socket(proto: u32) -> Self {
        let mut req = Self::empty();
        req.opcode = NET_OP_SOCKET;
        req.proto = proto;
        req
    }

    pub fn new_connect(socket_id: u32, ip: [u8; 4], port: u16) -> Self {
        let mut req = Self::empty();
        req.opcode = NET_OP_CONNECT;
        req.socket_id = socket_id;
        req.ip = ip;
        req.port = port;
        req
    }

    pub fn new_send(socket_id: u32, len: u32) -> Self {
        let mut req = Self::empty();
        req.opcode = NET_OP_SEND;
        req.socket_id = socket_id;
        req.payload_len = len;
        req
    }

    pub fn new_recv(socket_id: u32, max_len: u32) -> Self {
        let mut req = Self::empty();
        req.opcode = NET_OP_RECV;
        req.socket_id = socket_id;
        req.payload_len = max_len;
        req
    }

    pub fn new_close(socket_id: u32) -> Self {
        let mut req = Self::empty();
        req.opcode = NET_OP_CLOSE;
        req.socket_id = socket_id;
        req
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetResponse {
    pub status: u32,
    pub socket_id: u32,
    pub bytes_transferred: u32,
}

impl NetResponse {
    pub const fn ok(socket_id: u32, bytes: u32) -> Self {
        Self {
            status: 0,
            socket_id,
            bytes_transferred: bytes,
        }
    }

    pub const fn err(errno_val: u32) -> Self {
        Self {
            status: errno_val,
            socket_id: 0,
            bytes_transferred: 0,
        }
    }

    pub fn is_ok(&self) -> bool {
        self.status == 0
    }
}
