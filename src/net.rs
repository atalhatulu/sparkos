use alloc::vec::Vec;

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

pub fn create_ping_packet(src_mac: [u8; 6], sequence_num: u16) -> Vec<u8> {
    let mut packet = Vec::with_capacity(74); // 14 (Eth) + 20 (IP) + 40 (ICMP)
    
    // 1. ETHERNET HEADER (14 bytes)
    // Dest MAC: QEMU Gateway (52:54:00:12:34:56)
    packet.extend_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
    // Src MAC: RTL8139 MAC
    packet.extend_from_slice(&src_mac);
    // EtherType: IPv4 (0x0800)
    packet.extend_from_slice(&[0x08, 0x00]);

    // 2. IP HEADER (20 bytes)
    let ip_start = packet.len();
    packet.push(0x45); // Version 4, IHL 5
    packet.push(0x00); // DSCP/ECN
    // Total Length (20 IP + 40 ICMP = 60 bytes = 0x003C)
    packet.extend_from_slice(&[0x00, 0x3C]); 
    // Identification
    packet.extend_from_slice(&[0x12, 0x34]);
    // Flags & Fragment Offset
    packet.extend_from_slice(&[0x00, 0x00]);
    // TTL
    packet.push(0x40); // 64
    // Protocol (1 = ICMP)
    packet.push(0x01);
    // Checksum placeholder
    packet.extend_from_slice(&[0x00, 0x00]);
    // Source IP: 10.0.2.15 (QEMU default guest IP)
    packet.extend_from_slice(&[10, 0, 2, 15]);
    // Dest IP: 8.8.8.8 (Google)
    packet.extend_from_slice(&[8, 8, 8, 8]);
    
    // Calculate IP Checksum
    let ip_checksum = calculate_checksum(&packet[ip_start..packet.len()]);
    packet[ip_start + 10] = (ip_checksum >> 8) as u8;
    packet[ip_start + 11] = (ip_checksum & 0xFF) as u8;

    // 3. ICMP HEADER (8 bytes) + DATA (32 bytes)
    let icmp_start = packet.len();
    packet.push(0x08); // Type: Echo Request (8)
    packet.push(0x00); // Code: 0
    // Checksum placeholder
    packet.extend_from_slice(&[0x00, 0x00]);
    // Identifier
    packet.extend_from_slice(&[0x00, 0x01]);
    // Sequence
    packet.extend_from_slice(&[(sequence_num >> 8) as u8, (sequence_num & 0xFF) as u8]);
    
    // ICMP Data (32 bytes of 'A')
    for _ in 0..32 {
        packet.push(b'A');
    }
    
    // Calculate ICMP Checksum
    let icmp_checksum = calculate_checksum(&packet[icmp_start..packet.len()]);
    packet[icmp_start + 2] = (icmp_checksum >> 8) as u8;
    packet[icmp_start + 3] = (icmp_checksum & 0xFF) as u8;

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
    let udp_data_len = 12 + qname.len() + 4; // DNS Header (12) + QNAME + QTYPE(2) + QCLASS(2)
    let total_len = 14 + 20 + 8 + udp_data_len;
    
    let mut packet = Vec::with_capacity(total_len);
    
    // 1. ETHERNET HEADER (14 bytes)
    packet.extend_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x56]); // QEMU Gateway MAC
    packet.extend_from_slice(&src_mac);
    packet.extend_from_slice(&[0x08, 0x00]); // IPv4
    
    // 2. IP HEADER (20 bytes)
    let ip_start = packet.len();
    packet.push(0x45); // IPv4
    packet.push(0x00);
    packet.extend_from_slice(&((20 + 8 + udp_data_len as u16).to_be_bytes())); // Total Length
    packet.extend_from_slice(&[0x43, 0x21]); // ID
    packet.extend_from_slice(&[0x00, 0x00]); // Flags/Offset
    packet.push(0x40); // TTL 64
    packet.push(0x11); // Protocol = 17 (UDP)
    packet.extend_from_slice(&[0x00, 0x00]); // Checksum (placeholder)
    packet.extend_from_slice(&[10, 0, 2, 15]); // Src IP
    packet.extend_from_slice(&[8, 8, 8, 8]); // Dest IP (Google DNS)
    
    let ip_checksum = calculate_checksum(&packet[ip_start..packet.len()]);
    packet[ip_start + 10] = (ip_checksum >> 8) as u8;
    packet[ip_start + 11] = (ip_checksum & 0xFF) as u8;
    
    // 3. UDP HEADER (8 bytes)
    let udp_len = (8 + udp_data_len) as u16;
    packet.extend_from_slice(&[0xCA, 0xFE]); // Src Port: 51966
    packet.extend_from_slice(&[0x00, 0x35]); // Dest Port: 53 (DNS)
    packet.extend_from_slice(&udp_len.to_be_bytes()); // Length
    packet.extend_from_slice(&[0x00, 0x00]); // Checksum (optional in IPv4, set to 0)
    
    // 4. DNS HEADER (12 bytes)
    packet.extend_from_slice(&transaction_id.to_be_bytes()); // ID
    packet.extend_from_slice(&[0x01, 0x00]); // Flags: Standard query, Recursion desired
    packet.extend_from_slice(&[0x00, 0x01]); // Questions: 1
    packet.extend_from_slice(&[0x00, 0x00]); // Answer RRs: 0
    packet.extend_from_slice(&[0x00, 0x00]); // Authority RRs: 0
    packet.extend_from_slice(&[0x00, 0x00]); // Additional RRs: 0
    
    // 5. DNS QUERY
    packet.extend_from_slice(&qname);
    packet.extend_from_slice(&[0x00, 0x01]); // Type A (Host Address)
    packet.extend_from_slice(&[0x00, 0x01]); // Class IN
    
    packet
}

pub fn parse_dns_response(packet: &[u8], expected_tx_id: u16) -> Option<Vec<[u8; 4]>> {
    // Min size for Eth (14) + IP (20) + UDP (8) + DNS Header (12)
    if packet.len() < 54 { return None; }
    
    // IP Protocol = 17 (UDP)
    if packet[23] != 0x11 { return None; }
    
    // Check UDP Dest Port (0xCAFE = 51966)
    let dest_port = ((packet[36] as u16) << 8) | (packet[37] as u16);
    if dest_port != 0xCAFE { return None; }
    
    // Check DNS Tx ID
    let tx_id = ((packet[42] as u16) << 8) | (packet[43] as u16);
    if tx_id != expected_tx_id { return None; }
    
    // Flags: QR (bit 15) must be 1 (Response)
    let flags = ((packet[44] as u16) << 8) | (packet[45] as u16);
    if (flags & 0x8000) == 0 { return None; } // Not a response
    
    let q_count = ((packet[46] as usize) << 8) | (packet[47] as usize);
    let a_count = ((packet[48] as usize) << 8) | (packet[49] as usize);
    
    if a_count == 0 { return Some(Vec::new()); } // No answers
    
    // Skip Headers
    let mut offset = 54;
    
    // Skip Questions
    for _ in 0..q_count {
        // Skip QNAME
        while offset < packet.len() && packet[offset] != 0 {
            let len = packet[offset] as usize;
            if (len & 0xC0) == 0xC0 {
                // Pointer!
                offset += 1;
                break;
            }
            offset += len + 1;
        }
        offset += 1; // Skip null byte or second byte of pointer
        offset += 4; // Skip QTYPE and QCLASS
    }
    
    let mut ips = Vec::new();
    
    // Parse Answers
    for _ in 0..a_count {
        if offset >= packet.len() { break; }
        
        // Skip NAME
        if (packet[offset] & 0xC0) == 0xC0 {
            offset += 2; // Pointer is 2 bytes
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
        // TTL is 4 bytes at offset+4
        let rdlength = ((packet[offset + 8] as usize) << 8) | (packet[offset + 9] as usize);
        offset += 10;
        
        if offset + rdlength > packet.len() { break; }
        
        // Type A (1), Class IN (1)
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
    
    Some(ips)
}
