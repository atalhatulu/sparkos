use alloc::vec::Vec;
use spin::Mutex;
use crate::net::{
    send_udp_packet, send_tcp_syn_seq, send_tcp_ack, send_tcp_data,
    Ipv4Addr, parse_ipv4_payload, parse_tcp_header, tcp_flags,
};

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

/// A user-facing socket bound to a file descriptor. Carries enough state to run
/// a minimal Linux-1.0-style TCP client: a fixed ISN, the peer's sequence
/// number we acknowledge, and an RX queue of raw datagrams/payloads.
pub struct Socket {
    pub fd: usize,
    pub sock_type: SocketType,
    pub state: SocketState,
    pub local_port: u16,
    pub remote_addr: Option<SocketAddr>,
    pub rx_buffer: Vec<Vec<u8>>,
    /// Our next send sequence number.
    pub tcp_seq: u32,
    /// The sequence number we send as ACK (next expected byte from peer).
    pub tcp_ack: u32,
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

/// Locate a socket slot by fd. Returns its index, if any.
fn find_slot(fd: usize) -> Option<usize> {
    let table = SOCKET_TABLE.lock();
    for i in 0..64 {
        if let Some(sock) = &table.sockets[i] {
            if sock.fd == fd {
                return Some(i);
            }
        }
    }
    None
}

/// Create a new socket and return its fd (or 0 on failure / MAX on table full).
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
                tcp_seq: 0,
                tcp_ack: 0,
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

// ---------------------------------------------------------------------------
// TCP 3-way handshake
// ---------------------------------------------------------------------------

/// An arbitrary but per-connection unique initial sequence number.
fn initial_seq(last_fd: usize) -> u32 {
    0x1000_0000u32.wrapping_add((last_fd as u32) * 1000) + 1234
}

/// Drain a bounded number of frames, routing each to ARP or the IP/TCP path.
/// Used by the blocking handshake to collect the peer's SYN-ACK.
fn pump_rx(max_frames: usize) {
    unsafe {
        if let Some(ref mut dev) = crate::rtl8139::RTL8139_DEV {
            for _ in 0..max_frames {
                if let Some(packet) = dev.poll_rx() {
                    crate::net::handle_incoming_frame(&packet);
                }
            }
        }
    }
}

/// Open a TCP connection by performing a full 3-way handshake:
///   CLOSED --SYN--> SYN_SENT --SYN-ACK--> ESTABLISHED (send ACK)
/// Returns true on success, false on failure/timeout. Blocking but bounded.
fn tcp_handshake(fd: usize) -> bool {
    let (local_port, remote, iscn) = {
        let table = SOCKET_TABLE.lock();
        match find_slot(fd).and_then(|i| table.sockets[i].as_ref()) {
            Some(s) => (s.local_port, s.remote_addr.unwrap_or(SocketAddr { ip: [0; 4], port: 0 }), s.tcp_seq),
            None => return false,
        }
    };
    if remote.port == 0 {
        return false;
    }

    // Kick off the handshake: mark SYN_SENT and send our SYN.
    {
        let mut table = SOCKET_TABLE.lock();
        if let Some(i) = find_slot(fd) {
            if let Some(sock) = &mut table.sockets[i] {
                sock.state = SocketState::SynSent;
                sock.tcp_seq = iscn;
            }
        }
    }
    send_tcp_syn_seq(local_port, remote, iscn);

    // Wait for SYN-ACK (bounded polling).
    let deadline = 4_000_000usize;
    let mut waited = 0usize;
    loop {
        pump_rx(32);
        let established = {
            let table = SOCKET_TABLE.lock();
            match find_slot(fd).and_then(|i| table.sockets[i].as_ref()) {
                Some(s) => s.state == SocketState::Connected,
                None => return false,
            }
        };
        if established {
            return true;
        }
        waited += 32;
        if waited > deadline {
            return false;
        }
    }
}

// ---------------------------------------------------------------------------
// Syscall-facing API (fd based)
// ---------------------------------------------------------------------------

/// SYS_SOCKET(10): socket(domain) -> fd. `domain` is AF_INET==2. The socket
/// kind is chosen by an extended arg (arg2): 1=TCP, 2=UDP, 3=Raw.
pub fn sys_socket(domain: u64, kind: u64) -> u64 {
    crate::serial_println!("[SOCKET] sys_socket(domain={}, kind={})", domain, kind);
    if domain != 2 && domain != 0 {
        // AF_INET only (allow 0 as "don't care" for simplicity).
        crate::serial_println!("[SOCKET] Unsupported address family: {}", domain);
        return u64::MAX;
    }
    let st = match kind {
        1 => SocketType::Tcp,
        2 => SocketType::Udp,
        _ => SocketType::Raw,
    };
    match socket(st) {
        Some(fd) => fd as u64,
        None => u64::MAX,
    }
}

/// SYS_CONNECT(11): connect(fd, ip_packed, port). `ip_packed` packs the 4
/// IPv4 octets big-endian into a u32: a.b.c.d -> 0xAAAA.bbbb.cccc.dddd.
pub fn sys_connect(fd: u64, ip_packed: u64, port: u64) -> u64 {
    let addr = SocketAddr {
        ip: [
            ((ip_packed >> 24) & 0xFF) as u8,
            ((ip_packed >> 16) & 0xFF) as u8,
            ((ip_packed >> 8) & 0xFF) as u8,
            (ip_packed & 0xFF) as u8,
        ],
        port: port as u16,
    };
    crate::serial_println!(
        "[SOCKET] sys_connect(fd={}, {}.{}.{}.{}:{})",
        fd,
        addr.ip[0],
        addr.ip[1],
        addr.ip[2],
        addr.ip[3],
        addr.port
    );

    let is_tcp = {
        let table = SOCKET_TABLE.lock();
        match find_slot(fd as usize).and_then(|i| table.sockets[i].as_ref()) {
            Some(s) => s.sock_type == SocketType::Tcp,
            None => return u64::MAX,
        }
    };

    // Set local port + remote address + handshake seq before sending.
    let local_port = {
        let mut table = SOCKET_TABLE.lock();
        let need_port = match find_slot(fd as usize).and_then(|i| table.sockets[i].as_ref()) {
            Some(s) => s.local_port == 0,
            None => return u64::MAX,
        };
        // Allocate a port before taking a mutable borrow of the slot.
        let lp = if need_port {
            table.allocate_port()
        } else {
            match find_slot(fd as usize).and_then(|i| table.sockets[i].as_ref()) {
                Some(s) => s.local_port,
                None => return u64::MAX,
            }
        };
        if let Some(i) = find_slot(fd as usize) {
            let sock = table.sockets[i].as_mut().unwrap();
            sock.local_port = lp;
            sock.remote_addr = Some(addr);
            sock.tcp_seq = initial_seq(lp as usize);
            sock.tcp_ack = 0;
            if !is_tcp {
                sock.state = SocketState::Connected;
            }
        }
        lp
    };

    if is_tcp {
        if !tcp_handshake(fd as usize) {
            crate::serial_println!("[SOCKET] TCP handshake failed (fd {})", fd);
            return u64::MAX;
        }
    } else {
        send_udp_packet(local_port, addr, &[]);
    }
    0
}

/// SYS_SEND(12): send(fd, buf, len) -> bytes sent (UDP: len, TCP: sent count).
pub fn sys_send(fd: u64, buf_ptr: u64, len: u64) -> u64 {
    let data = match crate::sec_mem::validate_user_ptr(buf_ptr, len as usize) {
        Ok(d) => d,
        Err(_) => {
            crate::serial_println!("[SOCKET] send EFAULT");
            return (-14i64) as u64; // -EFAULT
        }
    };

    let (sock_type, local_port, remote, tcp_seq, tcp_ack, established) = {
        let table = SOCKET_TABLE.lock();
        match find_slot(fd as usize).and_then(|i| table.sockets[i].as_ref()) {
            Some(s) => (
                s.sock_type,
                s.local_port,
                s.remote_addr,
                s.tcp_seq,
                s.tcp_ack,
                s.state == SocketState::Connected,
            ),
            None => return u64::MAX,
        }
    };

    let Some(addr) = remote else {
        crate::serial_println!("[SOCKET] send: not connected (fd {})", fd);
        return u64::MAX;
    };

    match sock_type {
        SocketType::Udp => {
            send_udp_packet(local_port, addr, data);
            crate::serial_println!("[SOCKET] UDP sent {} bytes (fd {})", data.len(), fd);
            data.len() as u64
        }
        SocketType::Tcp => {
            if !established {
                crate::serial_println!("[SOCKET] send: TCP not established (fd {})", fd);
                return u64::MAX;
            }
            send_tcp_data(local_port, addr, tcp_seq, tcp_ack, data);
            // Advance our send sequence by the number of payload bytes.
            let n = data.len() as u32;
            let mut table = SOCKET_TABLE.lock();
            if let Some(i) = find_slot(fd as usize) {
                if let Some(sock) = &mut table.sockets[i] {
                    sock.tcp_seq = sock.tcp_seq.wrapping_add(n);
                }
            }
            crate::serial_println!("[SOCKET] TCP sent {} bytes (fd {})", data.len(), fd);
            data.len() as u64
        }
        SocketType::Raw => data.len() as u64,
    }
}

/// SYS_RECV(13): recv(fd, buf, len) -> bytes copied into buf, 0 if EAGAIN.
pub fn sys_recv(fd: u64, buf_ptr: u64, len: u64) -> u64 {
    let mut table = SOCKET_TABLE.lock();
    if let Some(i) = find_slot(fd as usize) {
        if let Some(sock) = &mut table.sockets[i] {
            if !sock.rx_buffer.is_empty() {
                let data = sock.rx_buffer.remove(0);
                let n = core::cmp::min(data.len(), len as usize);
                let out = match crate::sec_mem::validate_user_ptr_mut(buf_ptr, n) {
                    Ok(d) => d,
                    Err(_) => return (-14i64) as u64, // -EFAULT
                };
                out.copy_from_slice(&data[..n]);
                crate::serial_println!("[SOCKET] recv {} bytes (fd {})", n, fd);
                return n as u64;
            }
        }
    }
    0 // EAGAIN semantics: no data available
}

// ---------------------------------------------------------------------------
// Existing non-syscall API (kept for shell / internal callers)
// ---------------------------------------------------------------------------

pub fn connect(fd: usize, addr: SocketAddr) -> bool {
    // Mirror sys_connect but return a bool for legacy callers. Pack the IP
    // octets big-endian into a u32 for the fd-based ABI.
    let ip_packed = ((addr.ip[0] as u64) << 24)
        | ((addr.ip[1] as u64) << 16)
        | ((addr.ip[2] as u64) << 8)
        | (addr.ip[3] as u64);
    let ret = sys_connect(fd as u64, ip_packed, addr.port as u64);
    ret != u64::MAX
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
                    // Send on an established connection using current seq/ack.
                    let (seq, ack) = {
                        let t = SOCKET_TABLE.lock();
                        match find_slot(fd).and_then(|i| t.sockets[i].as_ref()) {
                            Some(s) => (s.tcp_seq, s.tcp_ack),
                            None => return false,
                        }
                    };
                    send_tcp_data(local_port, addr, seq, ack, data);
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

// ---------------------------------------------------------------------------
// Inbound packet routing (Ethernet frame -> IP -> UDP/TCP)
// ---------------------------------------------------------------------------

/// Route a full incoming Ethernet frame carrying IPv4 across to the matching
/// socket. Handles UDP datagrams and TCP data/control segments (including
/// completing the handshake by sending the final ACK).
pub fn handle_incoming_packet(packet: &[u8]) {
    let Some((src_ip, _dst_ip, proto, payload_start)) = parse_ipv4_payload(packet) else {
        return;
    };

    match proto {
        6 => handle_incoming_tcp(packet, src_ip, payload_start),
        17 => handle_incoming_udp(packet, payload_start),
        _ => {}
    }
}

fn handle_incoming_udp(packet: &[u8], udp_start: usize) {
    if packet.len() < udp_start + 8 {
        return;
    }
    let src_port = ((packet[udp_start] as u16) << 8) | packet[udp_start + 1] as u16;
    let dest_port = ((packet[udp_start + 2] as u16) << 8) | packet[udp_start + 3] as u16;
    let length = ((packet[udp_start + 4] as u16) << 8) | packet[udp_start + 5] as u16;
    if packet.len() < udp_start + length as usize {
        return;
    }
    let payload = &packet[udp_start + 8..udp_start + length as usize];
    let _ = src_port;
    deliver_packet_to_socket(SocketType::Udp, dest_port, payload);
}

fn handle_incoming_tcp(packet: &[u8], src_ip: Ipv4Addr, ip_payload_start: usize) {
    let Some(hdr) = parse_tcp_header(packet, ip_payload_start) else {
        return;
    };
    let payload_start = ip_payload_start + hdr.data_offset;
    let payload: &[u8] = if payload_start <= packet.len() {
        &packet[payload_start..]
    } else {
        &[]
    };

    // Find the destination socket by local port.
    let mut target: Option<usize> = None;
    let mut sock_state = SocketState::Closed;
    let mut sock_seq = 0u32;
    let mut sock_local_port = 0u16;
    {
        let table = SOCKET_TABLE.lock();
        for i in 0..64 {
            if let Some(sock) = &table.sockets[i] {
                if sock.sock_type == SocketType::Tcp && sock.local_port == hdr.dst_port {
                    target = Some(i);
                    sock_state = sock.state;
                    sock_seq = sock.tcp_seq;
                    sock_local_port = sock.local_port;
                    break;
                }
            }
        }
    }

    let Some(idx) = target else { return };

    let has_syn = hdr.flags & tcp_flags::SYN != 0;

    match (sock_state, has_syn, hdr.flags & tcp_flags::ACK != 0) {
        // We sent SYN; peer answered SYN-ACK. Send final ACK -> ESTABLISHED.
        (SocketState::SynSent, true, true) => {
            let remote = SocketAddr { ip: src_ip, port: hdr.src_port };
            // Our ack to send = peer_seq + 1. Track it on the socket.
            let ack_val = hdr.seq.wrapping_add(1);
            let our_seq = sock_seq; // we already advanced past SYN in the send path semantics
            send_tcp_ack(sock_local_port, remote, our_seq, ack_val);
            let mut table = SOCKET_TABLE.lock();
            if let Some(sock) = &mut table.sockets[idx] {
                sock.state = SocketState::Connected;
                // On SYN, peer consumed one of our sequence numbers; our next
                // send seq is ISN+1. tcp_ack tracks peer's next expected byte.
                sock.tcp_seq = sock.tcp_seq.wrapping_add(1);
                sock.tcp_ack = ack_val;
            }
            crate::serial_println!("[SOCKET] TCP established (local port {})", sock_local_port);
        }
        // Established receiving ACK data: deliver payload and advance ack.
        (SocketState::Connected, _, _) => {
            if !payload.is_empty() {
                deliver_packet_to_socket(SocketType::Tcp, hdr.dst_port, payload);
            }
            // Advance the ack we send: peer_seq + payload_len + (1 if SYN).
            let consumed = hdr.seq.wrapping_add(payload.len() as u32).wrapping_add(if has_syn { 1 } else { 0 });
            let mut table = SOCKET_TABLE.lock();
            if let Some(sock) = &mut table.sockets[idx] {
                sock.tcp_ack = consumed;
            }
        }
        _ => {
            // Unknown inbound segment; ignore for this minimal stack.
        }
    }
}
