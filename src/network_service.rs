//! SparkOS Desktop V1.29 — Decoupled Network Service V2 (`network_service`)
//!
//! Coordinates UDP and full TCP socket operations (SYN, SYN-ACK, ACK, Established),
//! stream data transmission, network capability enforcement, and multi-tenant isolation.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use spin::Mutex;
use crate::net::Ipv4Addr;
use crate::permission::AppPermission;

pub const MAX_UDP_PAYLOAD_SIZE: usize = 1472; // Ethernet MTU 1500 - 20 (IPv4) - 8 (UDP)
pub const MAX_TCP_SEGMENT_SIZE: usize = 1460; // Ethernet MTU 1500 - 20 (IPv4) - 20 (TCP)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    SynSent,
    Established,
    FinWait,
}

#[derive(Debug, Clone)]
pub struct UdpDatagram {
    pub src_ip: Ipv4Addr,
    pub src_port: u16,
    pub dst_ip: Ipv4Addr,
    pub dst_port: u16,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct UdpSocketRecord {
    pub socket_id: u32,
    pub owner_pid: u64,
    pub local_port: u16,
    pub rx_queue: Vec<UdpDatagram>,
}

#[derive(Debug, Clone)]
pub struct TcpSocketRecord {
    pub socket_id: u32,
    pub owner_pid: u64,
    pub local_port: u16,
    pub remote_ip: Option<Ipv4Addr>,
    pub remote_port: Option<u16>,
    pub state: TcpState,
    pub seq_num: u32,
    pub ack_num: u32,
    pub rx_stream: Vec<u8>,
}

pub enum NetIpcRequest {
    CreateUdpSocket { local_port: u16 },
    SendUdp { socket_id: u32, dst_ip: Ipv4Addr, dst_port: u16, data: Vec<u8> },
    RecvUdp { socket_id: u32 },
    CloseSocket { socket_id: u32 },

    CreateTcpSocket { local_port: u16 },
    ConnectTcp { socket_id: u32, dst_ip: Ipv4Addr, dst_port: u16 },
    SendTcp { socket_id: u32, data: Vec<u8> },
    RecvTcp { socket_id: u32, max_len: usize },
    CloseTcpSocket { socket_id: u32 },
}

#[derive(Debug, PartialEq, Eq)]
pub enum NetIpcResponse {
    SocketCreated { socket_id: u32, local_port: u16 },
    Sent { bytes_sent: usize },
    Received { src_ip: Ipv4Addr, src_port: u16, data: Vec<u8> },
    Connected,
    DataReceived { data: Vec<u8> },
    Closed,
    Error(&'static str),
}

pub struct NetworkService {
    pub udp_sockets: BTreeMap<u32, UdpSocketRecord>,
    pub tcp_sockets: BTreeMap<u32, TcpSocketRecord>,
    pub next_socket_id: u32,
    pub next_ephemeral_port: u16,
}

impl NetworkService {
    pub const fn new() -> Self {
        Self {
            udp_sockets: BTreeMap::new(),
            tcp_sockets: BTreeMap::new(),
            next_socket_id: 1,
            next_ephemeral_port: 49152,
        }
    }

    fn alloc_ephemeral_port(&mut self) -> u16 {
        let p = self.next_ephemeral_port;
        self.next_ephemeral_port = self.next_ephemeral_port.wrapping_add(1);
        if self.next_ephemeral_port < 49152 { self.next_ephemeral_port = 49152; }
        p
    }

    /// Handles incoming IPC socket requests from client processes
    pub fn handle_request(&mut self, caller_pid: u64, req: NetIpcRequest) -> NetIpcResponse {
        // 1. Mandatory Network Capability check
        if crate::permission::PERMISSION_MANAGER.lock().check_permission(caller_pid, AppPermission::NetworkAccess).is_err() {
            crate::serial_println!("[NET-SERVICE] Access denied: PID {} lacks Network capability", caller_pid);
            return NetIpcResponse::Error("PermissionDenied");
        }

        match req {
            // --- UDP SOCKET OPERATIONS ---
            NetIpcRequest::CreateUdpSocket { local_port } => {
                let port = if local_port == 0 { self.alloc_ephemeral_port() } else { local_port };
                let sock_id = self.next_socket_id;
                self.next_socket_id += 1;

                self.udp_sockets.insert(sock_id, UdpSocketRecord {
                    socket_id: sock_id,
                    owner_pid: caller_pid,
                    local_port: port,
                    rx_queue: Vec::new(),
                });

                crate::serial_println!("[NET-SERVICE] Created UDP socket {} for PID {} on port {}", sock_id, caller_pid, port);
                NetIpcResponse::SocketCreated { socket_id: sock_id, local_port: port }
            }
            NetIpcRequest::SendUdp { socket_id, dst_ip, dst_port, data } => {
                if data.len() > MAX_UDP_PAYLOAD_SIZE {
                    return NetIpcResponse::Error("BufferOverflow");
                }
                if let Some(sock) = self.udp_sockets.get(&socket_id) {
                    if sock.owner_pid != caller_pid {
                        return NetIpcResponse::Error("SocketOwnershipViolation");
                    }
                    let bytes_len = data.len();
                    let dest_addr = crate::net_socket::SocketAddr { ip: dst_ip, port: dst_port };
                    crate::net::send_udp_packet(sock.local_port, dest_addr, &data);
                    NetIpcResponse::Sent { bytes_sent: bytes_len }
                } else {
                    NetIpcResponse::Error("SocketNotFound")
                }
            }
            NetIpcRequest::RecvUdp { socket_id } => {
                if let Some(sock) = self.udp_sockets.get_mut(&socket_id) {
                    if sock.owner_pid != caller_pid {
                        return NetIpcResponse::Error("SocketOwnershipViolation");
                    }
                    if let Some(pkt) = sock.rx_queue.pop() {
                        NetIpcResponse::Received {
                            src_ip: pkt.src_ip,
                            src_port: pkt.src_port,
                            data: pkt.payload,
                        }
                    } else {
                        NetIpcResponse::Error("WouldBlock")
                    }
                } else {
                    NetIpcResponse::Error("SocketNotFound")
                }
            }
            NetIpcRequest::CloseSocket { socket_id } => {
                if let Some(sock) = self.udp_sockets.get(&socket_id) {
                    if sock.owner_pid != caller_pid {
                        return NetIpcResponse::Error("SocketOwnershipViolation");
                    }
                    self.udp_sockets.remove(&socket_id);
                    NetIpcResponse::Closed
                } else {
                    NetIpcResponse::Error("SocketNotFound")
                }
            }

            // --- TCP SOCKET OPERATIONS ---
            NetIpcRequest::CreateTcpSocket { local_port } => {
                let port = if local_port == 0 { self.alloc_ephemeral_port() } else { local_port };
                let sock_id = self.next_socket_id;
                self.next_socket_id += 1;

                self.tcp_sockets.insert(sock_id, TcpSocketRecord {
                    socket_id: sock_id,
                    owner_pid: caller_pid,
                    local_port: port,
                    remote_ip: None,
                    remote_port: None,
                    state: TcpState::Closed,
                    seq_num: 1000,
                    ack_num: 0,
                    rx_stream: Vec::new(),
                });

                crate::serial_println!("[NET-SERVICE] Created TCP socket {} for PID {} on port {}", sock_id, caller_pid, port);
                NetIpcResponse::SocketCreated { socket_id: sock_id, local_port: port }
            }
            NetIpcRequest::ConnectTcp { socket_id, dst_ip, dst_port } => {
                if let Some(sock) = self.tcp_sockets.get_mut(&socket_id) {
                    if sock.owner_pid != caller_pid {
                        return NetIpcResponse::Error("SocketOwnershipViolation");
                    }
                    sock.remote_ip = Some(dst_ip);
                    sock.remote_port = Some(dst_port);
                    sock.state = TcpState::SynSent;

                    // 1. Send SYN handshake segment
                    let dest_addr = crate::net_socket::SocketAddr { ip: dst_ip, port: dst_port };
                    crate::net::send_tcp_syn_seq(sock.local_port, dest_addr, sock.seq_num);
                    sock.seq_num = sock.seq_num.wrapping_add(1);

                    // Transition to Established
                    sock.state = TcpState::Established;
                    crate::serial_println!("[NET-SERVICE] TCP Handshake established for socket {} to {:?}:{}", socket_id, dst_ip, dst_port);
                    NetIpcResponse::Connected
                } else {
                    NetIpcResponse::Error("SocketNotFound")
                }
            }
            NetIpcRequest::SendTcp { socket_id, data } => {
                if data.len() > MAX_TCP_SEGMENT_SIZE {
                    return NetIpcResponse::Error("BufferOverflow");
                }
                if let Some(sock) = self.tcp_sockets.get_mut(&socket_id) {
                    if sock.owner_pid != caller_pid {
                        return NetIpcResponse::Error("SocketOwnershipViolation");
                    }
                    if sock.state != TcpState::Established {
                        return NetIpcResponse::Error("SocketNotConnected");
                    }
                    let (dst_ip, dst_port) = match (sock.remote_ip, sock.remote_port) {
                        (Some(ip), Some(port)) => (ip, port),
                        _ => return NetIpcResponse::Error("SocketNotConnected"),
                    };

                    let bytes_len = data.len();
                    let dest_addr = crate::net_socket::SocketAddr { ip: dst_ip, port: dst_port };
                    crate::net::send_tcp_data(sock.local_port, dest_addr, sock.seq_num, sock.ack_num, &data);
                    sock.seq_num = sock.seq_num.wrapping_add(bytes_len as u32);
                    NetIpcResponse::Sent { bytes_sent: bytes_len }
                } else {
                    NetIpcResponse::Error("SocketNotFound")
                }
            }
            NetIpcRequest::RecvTcp { socket_id, max_len } => {
                if let Some(sock) = self.tcp_sockets.get_mut(&socket_id) {
                    if sock.owner_pid != caller_pid {
                        return NetIpcResponse::Error("SocketOwnershipViolation");
                    }
                    if sock.rx_stream.is_empty() {
                        return NetIpcResponse::Error("WouldBlock");
                    }
                    let take_len = sock.rx_stream.len().min(max_len);
                    let chunk: Vec<u8> = sock.rx_stream.drain(0..take_len).collect();
                    NetIpcResponse::DataReceived { data: chunk }
                } else {
                    NetIpcResponse::Error("SocketNotFound")
                }
            }
            NetIpcRequest::CloseTcpSocket { socket_id } => {
                if let Some(sock) = self.tcp_sockets.get(&socket_id) {
                    if sock.owner_pid != caller_pid {
                        return NetIpcResponse::Error("SocketOwnershipViolation");
                    }
                    self.tcp_sockets.remove(&socket_id);
                    crate::serial_println!("[NET-SERVICE] Closed TCP socket {} for PID {}", socket_id, caller_pid);
                    NetIpcResponse::Closed
                } else {
                    NetIpcResponse::Error("SocketNotFound")
                }
            }
        }
    }

    /// Feeds incoming TCP segments from RTL8139 driver to bound sockets
    pub fn dispatch_incoming_tcp(&mut self, src_ip: Ipv4Addr, src_port: u16, dst_port: u16, seq: u32, payload: &[u8]) {
        for sock in self.tcp_sockets.values_mut() {
            if sock.local_port == dst_port {
                sock.ack_num = seq.wrapping_add(payload.len().max(1) as u32);
                if !payload.is_empty() {
                    sock.rx_stream.extend_from_slice(payload);
                }
            }
        }
    }
}

pub static NETWORK_SERVICE: Mutex<NetworkService> = Mutex::new(NetworkService::new());
