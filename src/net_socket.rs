use alloc::vec::Vec;
use spin::Mutex;
use crate::net::{send_udp_packet, send_tcp_syn, Ipv4Addr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketType {
    Udp,
    Tcp,
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    Closed,
    Listening,
    SynSent,
    SynReceived,
    Connected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketAddr {
    pub ip: Ipv4Addr,
    pub port: u16,
}

pub struct Socket {
    pub fd: usize,
    pub sock_type: SocketType,
    pub state: SocketState,
    pub local_port: u16,
    pub remote_addr: Option<SocketAddr>,
    pub rx_buffer: Vec<Vec<u8>>,
}

pub struct SocketTable {
    sockets: [Option<Socket>; 64],
    next_fd: usize,
    next_port: u16,
}

impl SocketTable {
    pub const fn new() -> Self {
        const INIT: Option<Socket> = None;
        SocketTable {
            sockets: [INIT; 64],
            next_fd: 0,
            next_port: 49152,
        }
    }

    pub fn allocate_port(&mut self) -> u16 {
        let port = self.next_port;
        self.next_port = if self.next_port == 65535 { 49152 } else { self.next_port + 1 };
        port
    }
}

pub static SOCKET_TABLE: Mutex<SocketTable> = Mutex::new(SocketTable::new());

pub fn socket(sock_type: SocketType) -> Option<usize> {
    let mut table = SOCKET_TABLE.lock();
    for i in 0..64 {
        if table.sockets[i].is_none() {
            let fd = table.next_fd;
            table.next_fd += 1;
            table.sockets[i] = Some(Socket {
                fd,
                sock_type,
                state: SocketState::Closed,
                local_port: 0,
                remote_addr: None,
                rx_buffer: Vec::new(),
            });
            return Some(fd);
        }
    }
    None
}

pub fn bind(fd: usize, port: u16) -> bool {
    let mut table = SOCKET_TABLE.lock();
    
    for i in 0..64 {
        if let Some(sock) = &table.sockets[i] {
            if sock.local_port == port && port != 0 {
                return false;
            }
        }
    }

    let mut target_idx = None;
    for i in 0..64 {
        if let Some(sock) = &table.sockets[i] {
            if sock.fd == fd {
                target_idx = Some(i);
                break;
            }
        }
    }

    if let Some(i) = target_idx {
        let port_to_use = if port == 0 { table.allocate_port() } else { port };
        if let Some(sock) = &mut table.sockets[i] {
            sock.local_port = port_to_use;
            return true;
        }
    }
    false
}

pub fn listen(fd: usize) -> bool {
    let mut table = SOCKET_TABLE.lock();
    for i in 0..64 {
        if let Some(sock) = &mut table.sockets[i] {
            if sock.fd == fd {
                if sock.sock_type == SocketType::Tcp {
                    sock.state = SocketState::Listening;
                    return true;
                }
                return false;
            }
        }
    }
    false
}

pub fn connect(fd: usize, addr: SocketAddr) -> bool {
    let mut table = SOCKET_TABLE.lock();
    let mut target_idx = None;
    for i in 0..64 {
        if let Some(sock) = &table.sockets[i] {
            if sock.fd == fd {
                target_idx = Some(i);
                break;
            }
        }
    }
    
    if let Some(i) = target_idx {
        let is_tcp = if let Some(sock) = &table.sockets[i] { sock.sock_type == SocketType::Tcp } else { false };
        let mut local_port = if let Some(sock) = &table.sockets[i] { sock.local_port } else { 0 };
        
        if local_port == 0 {
            local_port = table.allocate_port();
        }
        
        if let Some(sock) = &mut table.sockets[i] {
            sock.local_port = local_port;
            sock.remote_addr = Some(addr);
            if is_tcp {
                sock.state = SocketState::SynSent;
            } else {
                sock.state = SocketState::Connected;
            }
        }
        
        if is_tcp {
            drop(table);
            send_tcp_syn(local_port, addr);
        }
        return true;
    }
    false
}

pub fn send(fd: usize, data: &[u8]) -> bool {
    let table = SOCKET_TABLE.lock();
    let mut sock_info = None;
    
    for i in 0..64 {
        if let Some(sock) = &table.sockets[i] {
            if sock.fd == fd {
                sock_info = Some((sock.sock_type, sock.local_port, sock.remote_addr));
                break;
            }
        }
    }
    
    drop(table);

    if let Some((sock_type, local_port, remote_addr)) = sock_info {
        if let Some(addr) = remote_addr {
            match sock_type {
                SocketType::Udp => {
                    send_udp_packet(local_port, addr, data);
                    return true;
                }
                SocketType::Tcp => {
                    // TCP send implementation (placeholder for full stack)
                    return true;
                }
                SocketType::Raw => {
                    return true;
                }
            }
        }
    }
    false
}

pub fn recv(fd: usize) -> Option<Vec<u8>> {
    let mut table = SOCKET_TABLE.lock();
    for i in 0..64 {
        if let Some(sock) = &mut table.sockets[i] {
            if sock.fd == fd {
                if !sock.rx_buffer.is_empty() {
                    return Some(sock.rx_buffer.remove(0));
                }
            }
        }
    }
    None
}

pub fn close(fd: usize) {
    let mut table = SOCKET_TABLE.lock();
    for i in 0..64 {
        if let Some(sock) = &mut table.sockets[i] {
            if sock.fd == fd {
                table.sockets[i] = None;
                return;
            }
        }
    }
}

pub fn deliver_packet_to_socket(sock_type: SocketType, dest_port: u16, data: &[u8]) {
    let mut table = SOCKET_TABLE.lock();
    for i in 0..64 {
        if let Some(sock) = &mut table.sockets[i] {
            if sock.sock_type == sock_type && sock.local_port == dest_port {
                if sock.rx_buffer.len() < 16 { // Max 16 packets per socket
                    sock.rx_buffer.push(data.to_vec());
                }
                return;
            }
        }
    }
}
