//! SparkOS Desktop V1.23 — Decoupled Network Service (`network_service`)
//!
//! Coordinates UDP socket operations, network capability enforcement,
//! buffer overflow protection, and IPC socket contracts over the microkernel.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use spin::Mutex;
use crate::net::Ipv4Addr;
use crate::permission::AppPermission;

pub const MAX_UDP_PAYLOAD_SIZE: usize = 1472; // Standard Ethernet MTU 1500 - 20 (IPv4) - 8 (UDP)

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

pub enum NetIpcRequest {
    CreateUdpSocket { local_port: u16 },
    SendUdp { socket_id: u32, dst_ip: Ipv4Addr, dst_port: u16, data: Vec<u8> },
    RecvUdp { socket_id: u32 },
    CloseSocket { socket_id: u32 },
}

#[derive(Debug, PartialEq, Eq)]
pub enum NetIpcResponse {
    SocketCreated { socket_id: u32, local_port: u16 },
    Sent { bytes_sent: usize },
    Received { src_ip: Ipv4Addr, src_port: u16, data: Vec<u8> },
    Closed,
    Error(&'static str),
}

pub struct NetworkService {
    pub sockets: BTreeMap<u32, UdpSocketRecord>,
    pub next_socket_id: u32,
    pub next_ephemeral_port: u16,
}

impl NetworkService {
    pub const fn new() -> Self {
        Self {
            sockets: BTreeMap::new(),
            next_socket_id: 1,
            next_ephemeral_port: 49152,
        }
    }

    /// Handles an incoming IPC socket request from a client process
    pub fn handle_request(&mut self, caller_pid: u64, req: NetIpcRequest) -> NetIpcResponse {
        // 1. Mandatory Network Capability check
        if crate::permission::PERMISSION_MANAGER.lock().check_permission(caller_pid, AppPermission::NetworkAccess).is_err() {
            crate::serial_println!("[NET-SERVICE] Access denied: PID {} lacks Network capability", caller_pid);
            return NetIpcResponse::Error("PermissionDenied");
        }

        match req {
            NetIpcRequest::CreateUdpSocket { local_port } => {
                let port = if local_port == 0 {
                    let p = self.next_ephemeral_port;
                    self.next_ephemeral_port = self.next_ephemeral_port.wrapping_add(1);
                    if self.next_ephemeral_port < 49152 { self.next_ephemeral_port = 49152; }
                    p
                } else {
                    local_port
                };

                let sock_id = self.next_socket_id;
                self.next_socket_id += 1;

                self.sockets.insert(sock_id, UdpSocketRecord {
                    socket_id: sock_id,
                    owner_pid: caller_pid,
                    local_port: port,
                    rx_queue: Vec::new(),
                });

                crate::serial_println!("[NET-SERVICE] Created UDP socket {} for PID {} on port {}", sock_id, caller_pid, port);
                NetIpcResponse::SocketCreated { socket_id: sock_id, local_port: port }
            }
            NetIpcRequest::SendUdp { socket_id, dst_ip, dst_port, data } => {
                // Buffer overflow protection
                if data.len() > MAX_UDP_PAYLOAD_SIZE {
                    crate::serial_println!("[NET-SERVICE] Error: payload size {} exceeds MTU limit {}", data.len(), MAX_UDP_PAYLOAD_SIZE);
                    return NetIpcResponse::Error("BufferOverflow");
                }

                if let Some(sock) = self.sockets.get(&socket_id) {
                    if sock.owner_pid != caller_pid {
                        return NetIpcResponse::Error("SocketOwnershipViolation");
                    }
                    let bytes_len = data.len();
                    // Dispatch via kernel RTL8139 / net pipeline
                    let dest_addr = crate::net_socket::SocketAddr {
                        ip: dst_ip,
                        port: dst_port,
                    };
                    crate::net::send_udp_packet(sock.local_port, dest_addr, &data);
                    crate::serial_println!("[NET-SERVICE] PID {} sent {} bytes via UDP socket {} to {:?}:{}", caller_pid, bytes_len, socket_id, dst_ip, dst_port);
                    NetIpcResponse::Sent { bytes_sent: bytes_len }
                } else {
                    NetIpcResponse::Error("SocketNotFound")
                }
            }
            NetIpcRequest::RecvUdp { socket_id } => {
                if let Some(sock) = self.sockets.get_mut(&socket_id) {
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
                if let Some(sock) = self.sockets.get(&socket_id) {
                    if sock.owner_pid != caller_pid {
                        return NetIpcResponse::Error("SocketOwnershipViolation");
                    }
                    self.sockets.remove(&socket_id);
                    crate::serial_println!("[NET-SERVICE] Closed UDP socket {} for PID {}", socket_id, caller_pid);
                    NetIpcResponse::Closed
                } else {
                    NetIpcResponse::Error("SocketNotFound")
                }
            }
        }
    }

    /// Feeds incoming UDP packets from RTL8139 driver to bound sockets
    pub fn dispatch_incoming_packet(&mut self, src_ip: Ipv4Addr, src_port: u16, dst_port: u16, payload: &[u8]) {
        for sock in self.sockets.values_mut() {
            if sock.local_port == dst_port {
                if sock.rx_queue.len() < 32 {
                    sock.rx_queue.push(UdpDatagram {
                        src_ip,
                        src_port,
                        dst_ip: [10, 0, 2, 15],
                        dst_port,
                        payload: payload.to_vec(),
                    });
                }
            }
        }
    }
}

pub static NETWORK_SERVICE: Mutex<NetworkService> = Mutex::new(NetworkService::new());
