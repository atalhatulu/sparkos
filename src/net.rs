use alloc::vec::Vec;



pub type Ipv4Addr = [u8; 4];

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

// Dummy ARP resolution (IP -> MAC)
pub fn get_mac_for_ip(_ip: Ipv4Addr) -> [u8; 6] {
    // Hardcoded QEMU Gateway MAC for now
    [0x52, 0x54, 0x00, 0x12, 0x34, 0x56]
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

pub fn send_tcp_syn(src_port: u16, dest: crate::net_socket::SocketAddr) {
    let mut tcp_packet = Vec::with_capacity(20);
    tcp_packet.extend_from_slice(&src_port.to_be_bytes());
    tcp_packet.extend_from_slice(&dest.port.to_be_bytes());
    tcp_packet.extend_from_slice(&12345678u32.to_be_bytes()); // Seq Number
    tcp_packet.extend_from_slice(&0u32.to_be_bytes()); // Ack Number
    tcp_packet.push(0x50); // Data offset (5 words)
    tcp_packet.push(0x02); // Flags: SYN
    tcp_packet.extend_from_slice(&8192u16.to_be_bytes()); // Window
    tcp_packet.extend_from_slice(&[0x00, 0x00]); // Checksum placeholder
    tcp_packet.extend_from_slice(&[0x00, 0x00]); // Urgent pointer
    
    let mut pseudo = Vec::new();
    pseudo.extend_from_slice(&[10, 0, 2, 15]);
    pseudo.extend_from_slice(&dest.ip);
    pseudo.push(0);
    pseudo.push(6); // TCP protocol
    pseudo.extend_from_slice(&(tcp_packet.len() as u16).to_be_bytes());
    pseudo.extend_from_slice(&tcp_packet);
    
    let checksum = calculate_checksum(&pseudo);
    tcp_packet[16] = (checksum >> 8) as u8;
    tcp_packet[17] = (checksum & 0xFF) as u8;
    
    send_ipv4_packet(dest.ip, 6, &tcp_packet);
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
