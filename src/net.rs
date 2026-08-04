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
