use alloc::vec::Vec;
use spin::Mutex;

pub type Ipv4Addr = [u8; 4];

/// Internet checksum (RFC 1071): one's-complement sum over 16-bit words.
pub fn calculate_checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    for i in (0..data.len()).step_by(2) {
        let word = if i + 1 < data.len() {
            ((data[i] as u32) << 8) | (data[i + 1] as u32)
        } else {
            (data[i] as u32) << 8
        };
        sum += word;
    }
    while (sum >> 16) > 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

// ---------------------------------------------------------------------------
// Real ARP resolution: discover + cache instead of a hardcoded gateway MAC.
// ---------------------------------------------------------------------------

/// A single learned IP -> MAC mapping in the ARP cache.
#[derive(Debug, Clone, Copy)]
struct ArpEntry {
    ip: Ipv4Addr,
    mac: [u8; 6],
}

/// Static ARP cache. Small, bounded table of recently resolved hosts.
static ARP_CACHE: Mutex<Vec<ArpEntry>> = Mutex::new(Vec::new());

/// MAC broadcast address used for ARP requests and unknown-IP fallback.
const MAC_BROADCAST: [u8; 6] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

/// Source IP of this host (QEMU default user-net address).
const MY_IP: Ipv4Addr = [10, 0, 2, 15];

fn arp_cache_lookup(ip: &Ipv4Addr) -> Option<[u8; 6]> {
    let cache = ARP_CACHE.lock();
    for e in cache.iter() {
        if &e.ip == ip {
            return Some(e.mac);
        }
    }
    None
}

fn arp_cache_insert(ip: Ipv4Addr, mac: [u8; 6]) {
    let mut cache = ARP_CACHE.lock();
    // Refresh an existing entry or append a new one (evict oldest if full).
    for e in cache.iter_mut() {
        if e.ip == ip {
            e.mac = mac;
            return;
        }
    }
    if cache.len() >= 32 {
        cache.remove(0);
    }
    cache.push(ArpEntry { ip, mac });
}

/// Build and send an ARP request frame for `target_ip`, broadcast so any host
/// owning that IP will reply with its hardware address.
pub fn send_arp_request(target_ip: Ipv4Addr) {
    let my_mac = get_my_mac();
    let mut frame = Vec::with_capacity(14 + 28);

    frame.extend_from_slice(&MAC_BROADCAST); // dest MAC
    frame.extend_from_slice(&my_mac);        // src MAC
    frame.extend_from_slice(&[0x08, 0x06]);  // EtherType: ARP

    frame.extend_from_slice(&[0x00, 0x01]); // Hardware type: Ethernet
    frame.extend_from_slice(&[0x08, 0x00]); // Protocol type: IPv4
    frame.push(6);                           // Hardware address length
    frame.push(4);                           // Protocol address length
    frame.extend_from_slice(&[0x00, 0x01]); // Operation: 1 = request
    frame.extend_from_slice(&my_mac);        // Sender hardware address
    frame.extend_from_slice(&MY_IP);         // Sender protocol address
    frame.extend_from_slice(&[0; 6]);        // Target hardware address (unknown)
    frame.extend_from_slice(&target_ip);     // Target protocol address

    send_ethernet_packet_raw(frame);
}

fn send_ethernet_packet_raw(frame: Vec<u8>) {
    unsafe {
        if let Some(ref mut dev) = crate::rtl8139::RTL8139_DEV {
            dev.send_packet(&frame);
        }
    }
}

/// Build and send an ARP reply announcing `MY_IP -> our MAC` to the requester.
pub fn send_arp_reply(target_ip: Ipv4Addr, target_mac: [u8; 6]) {
    let my_mac = get_my_mac();
    let mut frame = Vec::with_capacity(14 + 28);

    frame.extend_from_slice(&target_mac);
    frame.extend_from_slice(&my_mac);
    frame.extend_from_slice(&[0x08, 0x06]);

    frame.extend_from_slice(&[0x00, 0x01]);
    frame.extend_from_slice(&[0x08, 0x00]);
    frame.push(6);
    frame.push(4);
    frame.extend_from_slice(&[0x00, 0x02]); // Operation: 2 = reply
    frame.extend_from_slice(&my_mac);
    frame.extend_from_slice(&MY_IP);
    frame.extend_from_slice(&target_mac);
    frame.extend_from_slice(&target_ip);

    send_ethernet_packet_raw(frame);
}

/// Parse an ARP frame (full Ethernet frame). Returns `(sender_ip, sender_mac, op)`.
fn parse_arp(packet: &[u8]) -> Option<(Ipv4Addr, [u8; 6], u16)> {
    if packet.len() < 14 + 28 {
        return None;
    }
    let eth_type = ((packet[12] as u16) << 8) | packet[13] as u16;
    if eth_type != 0x0806 {
        return None;
    }
    let op = ((packet[20] as u16) << 8) | packet[21] as u16;
    let mut sender_mac = [0u8; 6];
    sender_mac.copy_from_slice(&packet[22..28]);
    let mut sender_ip = [0u8; 4];
    sender_ip.copy_from_slice(&packet[28..32]);
    Some((sender_ip, sender_mac, op))
}

/// Process an incoming ARP frame. Learns replies into the cache and answers
/// requests addressed to our IP. Returns whether the frame was handled as ARP.
pub fn handle_incoming_arp(packet: &[u8]) -> bool {
    if let Some((sender_ip, sender_mac, op)) = parse_arp(packet) {
        let hlen = packet[18] as usize;
        let ptype = packet[19] as usize;
        match op {
            1 => {
                // Request: if it targets our IP, answer with our MAC.
                let off = 14 + 8 + 2 * hlen + 2 * ptype;
                if off + 4 <= packet.len() {
                    let mut target_ip = [0u8; 4];
                    target_ip.copy_from_slice(&packet[off..off + 4]);
                    if target_ip == MY_IP {
                        send_arp_reply(sender_ip, sender_mac);
                    }
                }
            }
            2 => {
                // Reply: learn the mapping (never trust a claim to our own IP).
                if sender_ip != MY_IP {
                    arp_cache_insert(sender_ip, sender_mac);
                }
            }
            _ => {}
        }
        return true;
    }
    false
}

/// Drain the RX ring looking for an ARP reply for `ip` (bounded). Non-ARP
/// frames are forwarded to the IP/TCP path so handshake/data traffic is not
/// dropped while we wait. Returns whether a reply was learned.
fn poll_arp_reply(ip: Ipv4Addr, max_tries: usize) -> bool {
    for _ in 0..max_tries {
        unsafe {
            if let Some(ref mut dev) = crate::rtl8139::RTL8139_DEV {
                if let Some(packet) = dev.poll_rx() {
                    if !handle_incoming_arp(&packet) {
                        crate::net_socket::handle_incoming_packet(&packet);
                    }
                }
            }
        }
        if arp_cache_lookup(&ip).is_some() {
            return true;
        }
    }
    false
}

/// Resolve an IP to a MAC. Sends ARP requests and polls for a reply when the
/// mapping is not cached yet. Falls back to broadcast if the host never
/// answers (so traffic still leaves the NIC and reaches QEMU's gateway).
pub fn resolve_ip(ip: Ipv4Addr) -> [u8; 6] {
    if let Some(mac) = arp_cache_lookup(&ip) {
        return mac;
    }
    for _ in 0..3 {
        send_arp_request(ip);
        if poll_arp_reply(ip, 50) {
            break;
        }
    }
    arp_cache_lookup(&ip).unwrap_or(MAC_BROADCAST)
}

// ---------------------------------------------------------------------------
// Legacy IP -> MAC entry point. Kept for backwards compatibility with shell.rs.
// Now backed by the real ARP discovery/cache instead of a hardcoded MAC.
// ---------------------------------------------------------------------------
pub fn get_mac_for_ip(ip: Ipv4Addr) -> [u8; 6] {
    resolve_ip(ip)
}

pub fn get_my_mac() -> [u8; 6] {
    unsafe {
        if let Some(ref dev) = crate::rtl8139::RTL8139_DEV {
            return dev.get_mac_address();
        }
    }
    [0; 6]
}

pub fn send_ethernet_packet(dest_mac: [u8; 6], ethertype: u16, payload: &[u8]) {
    let src_mac = get_my_mac();
    let mut packet = Vec::with_capacity(14 + payload.len());
    packet.extend_from_slice(&dest_mac);
    packet.extend_from_slice(&src_mac);
    packet.extend_from_slice(&ethertype.to_be_bytes());
    packet.extend_from_slice(payload);
    
    unsafe {
        if let Some(ref mut dev) = crate::rtl8139::RTL8139_DEV {
            dev.send_packet(&packet);
        }
    }
}

pub fn build_ipv4_packet(dest_ip: Ipv4Addr, protocol: u8, payload: &[u8]) -> Vec<u8> {
    let mut ip_header = Vec::with_capacity(20 + payload.len());
    
    ip_header.push(0x45); // IPv4, IHL 5
    ip_header.push(0x00); // DSCP/ECN
    let total_len = (20 + payload.len()) as u16;
    ip_header.extend_from_slice(&total_len.to_be_bytes());
    ip_header.extend_from_slice(&[0x12, 0x34]); // ID
    ip_header.extend_from_slice(&[0x00, 0x00]); // Flags/Offset
    ip_header.push(64); // TTL
    ip_header.push(protocol); // Protocol
    ip_header.extend_from_slice(&[0x00, 0x00]); // Checksum placeholder
    
    // Source IP: 10.0.2.15
    ip_header.extend_from_slice(&[10, 0, 2, 15]);
    ip_header.extend_from_slice(&dest_ip);
    
    let checksum = calculate_checksum(&ip_header[0..20]);
    ip_header[10] = (checksum >> 8) as u8;
    ip_header[11] = (checksum & 0xFF) as u8;
    
    ip_header.extend_from_slice(payload);
    ip_header
}

pub fn send_ipv4_packet(dest_ip: Ipv4Addr, protocol: u8, payload: &[u8]) {
    let dest_mac = get_mac_for_ip(dest_ip);
    let ip_packet = build_ipv4_packet(dest_ip, protocol, payload);
    send_ethernet_packet(dest_mac, 0x0800, &ip_packet);
}

pub fn build_udp_packet(src_port: u16, dest_port: u16, data: &[u8]) -> Vec<u8> {
    let mut udp_packet = Vec::with_capacity(8 + data.len());
    udp_packet.extend_from_slice(&src_port.to_be_bytes());
    udp_packet.extend_from_slice(&dest_port.to_be_bytes());
    let length = (8 + data.len()) as u16;
    udp_packet.extend_from_slice(&length.to_be_bytes());
    udp_packet.extend_from_slice(&[0x00, 0x00]); // Checksum optional in IPv4
    udp_packet.extend_from_slice(data);
    udp_packet
}

pub fn send_udp_packet(src_port: u16, dest: crate::net_socket::SocketAddr, data: &[u8]) {
    let udp_packet = build_udp_packet(src_port, dest.port, data);
    send_ipv4_packet(dest.ip, 17, &udp_packet);
}

/// TCP flag bits (subset used by this minimal stack).
pub mod tcp_flags {
    pub const FIN: u8 = 0x01;
    pub const SYN: u8 = 0x02;
    pub const RST: u8 = 0x04;
    pub const PSH: u8 = 0x08;
    pub const ACK: u8 = 0x10;
}

/// Build a TCP segment (header + payload) with a correct pseudo-header checksum.
fn build_tcp_packet(
    src_port: u16,
    dest_port: u16,
    dest_ip: Ipv4Addr,
    seq: u32,
    ack: u32,
    flags: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut tcp = Vec::with_capacity(20 + payload.len());
    tcp.extend_from_slice(&src_port.to_be_bytes());
    tcp.extend_from_slice(&dest_port.to_be_bytes());
    tcp.extend_from_slice(&seq.to_be_bytes());
    tcp.extend_from_slice(&ack.to_be_bytes());
    tcp.push(0x50); // Data offset: 5 words (20 bytes)
    tcp.push(flags);
    tcp.extend_from_slice(&8192u16.to_be_bytes()); // Window
    tcp.extend_from_slice(&[0x00, 0x00]); // Checksum placeholder
    tcp.extend_from_slice(&[0x00, 0x00]); // Urgent pointer
    tcp.extend_from_slice(payload);

    // Pseudo header for checksum: src IP, dst IP, zero, proto TCP, TCP length.
    let mut pseudo = Vec::new();
    pseudo.extend_from_slice(&MY_IP);
    pseudo.extend_from_slice(&dest_ip);
    pseudo.push(0);
    pseudo.push(6);
    pseudo.extend_from_slice(&(tcp.len() as u16).to_be_bytes());
    pseudo.extend_from_slice(&tcp);

    let checksum = calculate_checksum(&pseudo);
    tcp[16] = (checksum >> 8) as u8;
    tcp[17] = (checksum & 0xFF) as u8;
    tcp
}

/// Send an IPv4 TCP segment to `dest`.
fn send_tcp_segment(dest: crate::net_socket::SocketAddr, tcp: &[u8]) {
    send_ipv4_packet(dest.ip, 6, tcp);
}

pub fn send_tcp_syn(src_port: u16, dest: crate::net_socket::SocketAddr) {
    send_tcp_syn_seq(src_port, dest, 12345678);
}

/// Send a TCP SYN with an explicit initial sequence number. Used by the
/// handshake so the ISN stored on the socket matches the wire value.
pub fn send_tcp_syn_seq(src_port: u16, dest: crate::net_socket::SocketAddr, iscn: u32) {
    let tcp = build_tcp_packet(src_port, dest.port, dest.ip, iscn, 0, tcp_flags::SYN, &[]);
    send_tcp_segment(dest, &tcp);
}

/// Send the final ACK of the 3-way handshake.
pub fn send_tcp_ack(src_port: u16, dest: crate::net_socket::SocketAddr, seq: u32, ack: u32) {
    let tcp = build_tcp_packet(src_port, dest.port, dest.ip, seq, ack, tcp_flags::ACK, &[]);
    send_tcp_segment(dest, &tcp);
}

/// Send TCP data (PSH+ACK) on an established connection.
pub fn send_tcp_data(
    src_port: u16,
    dest: crate::net_socket::SocketAddr,
    seq: u32,
    ack: u32,
    data: &[u8],
) {
    let tcp = build_tcp_packet(
        src_port,
        dest.port,
        dest.ip,
        seq,
        ack,
        tcp_flags::PSH | tcp_flags::ACK,
        data,
    );
    send_tcp_segment(dest, &tcp);
}

/// Parsed view of a TCP segment (offsets are relative to the TCP header start).
pub struct TcpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub data_offset: usize,
}

/// Parse the TCP header of an IPv4 packet. `tcp_start` is the byte offset of
/// the TCP header within the IP payload. Returns header fields plus the offset
/// of the payload data.
pub fn parse_tcp_header(packet: &[u8], tcp_start: usize) -> Option<TcpHeader> {
    if packet.len() < tcp_start + 20 {
        return None;
    }
    let src_port = ((packet[tcp_start] as u16) << 8) | packet[tcp_start + 1] as u16;
    let dst_port = ((packet[tcp_start + 2] as u16) << 8) | packet[tcp_start + 3] as u16;
    let seq = u32::from_be_bytes([
        packet[tcp_start + 4],
        packet[tcp_start + 5],
        packet[tcp_start + 6],
        packet[tcp_start + 7],
    ]);
    let ack = u32::from_be_bytes([
        packet[tcp_start + 8],
        packet[tcp_start + 9],
        packet[tcp_start + 10],
        packet[tcp_start + 11],
    ]);
    let data_offset = ((packet[tcp_start + 12] >> 4) as usize) * 4;
    let flags = packet[tcp_start + 13];
    Some(TcpHeader {
        src_port,
        dst_port,
        seq,
        ack,
        flags,
        data_offset,
    })
}

/// Parse the IPv4 header of a full Ethernet frame and return
/// `(src_ip, dst_ip, protocol, payload_start)` where `payload_start` is the
/// byte offset of the IP payload (i.e. `14 + ihl`).
pub fn parse_ipv4_payload(packet: &[u8]) -> Option<(Ipv4Addr, Ipv4Addr, u8, usize)> {
    if packet.len() < 34 {
        return None;
    }
    if (packet[12] as u16) << 8 | packet[13] as u16 != 0x0800 {
        return None;
    }
    if let Some((src, dst, proto, ihl)) = parse_ipv4_header(&packet[14..]) {
        return Some((src, dst, proto, 14 + ihl));
    }
    None
}

/// Route a full incoming Ethernet frame to the ARP or IP/TCP path.
pub fn handle_incoming_frame(packet: &[u8]) -> bool {
    if packet.len() < 14 {
        return false;
    }
    let eth_type = ((packet[12] as u16) << 8) | packet[13] as u16;
    match eth_type {
        0x0806 => handle_incoming_arp(packet),
        0x0800 => {
            crate::net_socket::handle_incoming_packet(packet);
            true
        }
        _ => false,
    }
}

// IP packet validation abstraction
pub fn parse_ipv4_header(packet: &[u8]) -> Option<(Ipv4Addr, Ipv4Addr, u8, usize)> {
    if packet.len() < 20 { return None; }
    let version = packet[0] >> 4;
    if version != 4 { return None; }
    
    let ihl = (packet[0] & 0x0F) as usize * 4;
    if packet.len() < ihl { return None; }
    
    let total_len = ((packet[2] as u16) << 8) | (packet[3] as u16);
    if packet.len() < total_len as usize { return None; }
    
    let checksum = calculate_checksum(&packet[0..ihl]);
    if checksum != 0 {
        // Warning: Bad checksum
        // return None; // Relaxed for now
    }
    
    let protocol = packet[9];
    let mut src = [0; 4];
    let mut dst = [0; 4];
    src.copy_from_slice(&packet[12..16]);
    dst.copy_from_slice(&packet[16..20]);
    
    Some((src, dst, protocol, ihl))
}

// Legacy functions for shell.rs to continue working (using same signatures)

pub fn create_ping_packet(src_mac: [u8; 6], sequence_num: u16) -> Vec<u8> {
    let mut packet = Vec::with_capacity(74); // 14 (Eth) + 20 (IP) + 40 (ICMP)
    
    // 1. ETHERNET HEADER
    let dest_mac = get_mac_for_ip([8, 8, 8, 8]);
    packet.extend_from_slice(&dest_mac);
    packet.extend_from_slice(&src_mac);
    packet.extend_from_slice(&[0x08, 0x00]);

    // 2. IP HEADER
    let mut ip_payload = Vec::new();
    
    // 3. ICMP HEADER (8 bytes) + DATA (32 bytes)
    ip_payload.push(0x08); // Type: Echo Request (8)
    ip_payload.push(0x00); // Code: 0
    ip_payload.extend_from_slice(&[0x00, 0x00]); // Checksum placeholder
    ip_payload.extend_from_slice(&[0x00, 0x01]); // Identifier
    ip_payload.extend_from_slice(&[(sequence_num >> 8) as u8, (sequence_num & 0xFF) as u8]);
    for _ in 0..32 {
        ip_payload.push(b'A');
    }
    
    let icmp_checksum = calculate_checksum(&ip_payload);
    ip_payload[2] = (icmp_checksum >> 8) as u8;
    ip_payload[3] = (icmp_checksum & 0xFF) as u8;

    let ip_header = build_ipv4_packet([8, 8, 8, 8], 1, &ip_payload);
    packet.extend_from_slice(&ip_header);

    packet
}

fn encode_domain_name(domain: &str) -> Vec<u8> {
    let mut encoded = Vec::new();
    for part in domain.split('.') {
        encoded.push(part.len() as u8);
        encoded.extend_from_slice(part.as_bytes());
    }
    encoded.push(0); // Root
    encoded
}

pub fn create_dns_query_packet(src_mac: [u8; 6], domain: &str, transaction_id: u16) -> Vec<u8> {
    let qname = encode_domain_name(domain);
    let mut dns_data = Vec::new();
    
    // DNS HEADER
    dns_data.extend_from_slice(&transaction_id.to_be_bytes());
    dns_data.extend_from_slice(&[0x01, 0x00]);
    dns_data.extend_from_slice(&[0x00, 0x01]);
    dns_data.extend_from_slice(&[0x00, 0x00]);
    dns_data.extend_from_slice(&[0x00, 0x00]);
    dns_data.extend_from_slice(&[0x00, 0x00]);
    
    // DNS QUERY
    dns_data.extend_from_slice(&qname);
    dns_data.extend_from_slice(&[0x00, 0x01]); // Type A
    dns_data.extend_from_slice(&[0x00, 0x01]); // Class IN

    let udp_packet = build_udp_packet(0xCAFE, 53, &dns_data);
    let ip_packet = build_ipv4_packet([8, 8, 8, 8], 17, &udp_packet);
    
    let dest_mac = get_mac_for_ip([8, 8, 8, 8]);
    let mut packet = Vec::with_capacity(14 + ip_packet.len());
    packet.extend_from_slice(&dest_mac);
    packet.extend_from_slice(&src_mac);
    packet.extend_from_slice(&[0x08, 0x00]);
    packet.extend_from_slice(&ip_packet);
    
    packet
}

pub fn parse_dns_response(packet: &[u8], expected_tx_id: u16) -> Option<Vec<[u8; 4]>> {
    if packet.len() < 54 { return None; }
    
    // Use the new parser for IP layer validation
    if let Some((_src, _dst, proto, ihl)) = parse_ipv4_header(&packet[14..]) {
        if proto != 17 { return None; }
        
        let udp_offset = 14 + ihl;
        if packet.len() < udp_offset + 8 { return None; }
        
        let dest_port = ((packet[udp_offset + 2] as u16) << 8) | (packet[udp_offset + 3] as u16);
        if dest_port != 0xCAFE { return None; }
        
        let dns_offset = udp_offset + 8;
        if packet.len() < dns_offset + 12 { return None; }
        
        let tx_id = ((packet[dns_offset] as u16) << 8) | (packet[dns_offset + 1] as u16);
        if tx_id != expected_tx_id { return None; }
        
        let flags = ((packet[dns_offset + 2] as u16) << 8) | (packet[dns_offset + 3] as u16);
        if (flags & 0x8000) == 0 { return None; } // Not a response
        
        let q_count = ((packet[dns_offset + 4] as usize) << 8) | (packet[dns_offset + 5] as usize);
        let a_count = ((packet[dns_offset + 6] as usize) << 8) | (packet[dns_offset + 7] as usize);
        
        if a_count == 0 { return Some(Vec::new()); }
        
        let mut offset = dns_offset + 12;
        
        for _ in 0..q_count {
            while offset < packet.len() && packet[offset] != 0 {
                let len = packet[offset] as usize;
                if (len & 0xC0) == 0xC0 {
                    offset += 1;
                    break;
                }
                offset += len + 1;
            }
            offset += 1;
            offset += 4;
        }
        
        let mut ips = Vec::new();
        
        for _ in 0..a_count {
            if offset >= packet.len() { break; }
            
            if (packet[offset] & 0xC0) == 0xC0 {
                offset += 2;
            } else {
                while offset < packet.len() && packet[offset] != 0 {
                    let len = packet[offset] as usize;
                    offset += len + 1;
                }
                offset += 1;
            }
            
            if offset + 10 > packet.len() { break; }
            
            let atype = ((packet[offset] as u16) << 8) | (packet[offset + 1] as u16);
            let aclass = ((packet[offset + 2] as u16) << 8) | (packet[offset + 3] as u16);
            let rdlength = ((packet[offset + 8] as usize) << 8) | (packet[offset + 9] as usize);
            offset += 10;
            
            if offset + rdlength > packet.len() { break; }
            
            if atype == 1 && aclass == 1 && rdlength == 4 {
                let ip = [
                    packet[offset],
                    packet[offset + 1],
                    packet[offset + 2],
                    packet[offset + 3]
                ];
                ips.push(ip);
            }
            
            offset += rdlength;
        }
        
        return Some(ips);
    }
    
    None
}
