use x86_64::instructions::port::Port;
use alloc::vec::Vec;
use crate::pci::{scan_pci, pci_read_u32};

pub struct Rtl8139 {
    io_base: u16,
    mac_address: [u8; 6],
    tx_cur: u8,
    rx_buffer: Vec<u8>,
    rx_idx: usize,
}

pub static mut RTL8139_DEV: Option<Rtl8139> = None;

impl Rtl8139 {
    pub fn new(io_base: u16) -> Self {
        let mut rx_buf = Vec::with_capacity(8192 + 16 + 1500); // 8K + Wrap overhead
        rx_buf.resize(8192 + 16 + 1500, 0);
        let mut dev = Rtl8139 {
            io_base,
            mac_address: [0; 6],
            tx_cur: 0,
            rx_buffer: rx_buf,
            rx_idx: 0,
        };
        dev.init();
        dev
    }

    fn init(&mut self) {
        unsafe {
            // 1. Power on
            let mut config_1: Port<u8> = Port::new(self.io_base + 0x52);
            config_1.write(0x00);

            // 2. Software Reset
            let mut cmd: Port<u8> = Port::new(self.io_base + 0x37);
            cmd.write(0x10);
            while (cmd.read() & 0x10) != 0 {
                // Bekle
            }

            // 3. MAC Adresini Oku (0x00 offsetinden 6 byte)
            for i in 0..6 {
                let mut mac_port: Port<u8> = Port::new(self.io_base + i);
                self.mac_address[i as usize] = mac_port.read();
            }

            // 4. Rx Buffer Baslangic Adresini Ayarla (RBSTART - 0x30)
            let phys_addr = (self.rx_buffer.as_ptr() as u64) - crate::gui::PHYS_OFFSET;
            let mut rbstart: Port<u32> = Port::new(self.io_base + 0x30);
            rbstart.write(phys_addr as u32);

            // 5. IMR (Interrupt Mask Register - 0x3C)
            let mut imr: Port<u16> = Port::new(self.io_base + 0x3C);
            imr.write(0x0005); // ROK (Receive OK) ve RER (Receive Error)

            // 6. RCR (Receive Configuration Register - 0x44)
            let mut rcr: Port<u32> = Port::new(self.io_base + 0x44);
            // Accept Broadcast, Multicast, My MAC. WRAP=1
            rcr.write(0x8F); 

            // 7. Tx ve Rx'i Aktif Et (Command Register - 0x37)
            let mut cmd2: Port<u8> = Port::new(self.io_base + 0x37);
            cmd2.write(0x0C); // TE (Transmit Enable) | RE (Receive Enable)
        }
    }

    pub fn get_mac_address(&self) -> [u8; 6] {
        self.mac_address
    }

    pub fn send_packet(&mut self, data: &[u8]) {
        // 1. Sanal adresi Fiziksel adrese cevir (QEMU DMA icin)
        let phys_addr = (data.as_ptr() as u64) - unsafe { crate::gui::PHYS_OFFSET };
        
        // 2. TSADx (Transmit Start Address) - Verinin fiziksel adresini yaz
        let tsad_port = self.io_base + 0x20 + (self.tx_cur as u16 * 4);
        unsafe {
            let mut port: Port<u32> = Port::new(tsad_port);
            port.write(phys_addr as u32);
        }
        
        // 3. TSDx (Transmit Status) - Veri boyutunu yazarak gonderimi tetikle
        let tsd_port = self.io_base + 0x10 + (self.tx_cur as u16 * 4);
        unsafe {
            let mut port: Port<u32> = Port::new(tsd_port);
            let size = data.len() as u32;
            port.write(size);
        }
        
        // Sira bir sonraki Tx tamponuna gecer (Ring Buffer)
        self.tx_cur = (self.tx_cur + 1) % 4;
    }

    pub fn poll_rx(&mut self) -> Option<Vec<u8>> {
        unsafe {
            let mut cmd: Port<u8> = Port::new(self.io_base + 0x37);
            // Bit 0 = BUFE (Buffer Empty). 0 ise paket gelmistir.
            if (cmd.read() & 0x01) != 0 {
                return None; // Buffer bos
            }

            // Paket basligini (4 byte) oku: [Status: u16] [Length: u16]
            let header_ptr = self.rx_buffer.as_ptr().add(self.rx_idx) as *const u16;
            let status = core::ptr::read_volatile(header_ptr);
            let length = core::ptr::read_volatile(header_ptr.offset(1)) as usize;

            if status & 0x01 == 0 || length == 0 || length > 8192 {
                // Hata veya gecersiz paket. Sifirla.
                let mut cmd_reset: Port<u8> = Port::new(self.io_base + 0x37);
                cmd_reset.write(0x10); // Reset
                while (cmd_reset.read() & 0x10) != 0 {}
                self.init();
                return None;
            }

            // Gercek paket verisi 4 byte'lik basliktan sonradir
            let packet_start = self.rx_idx + 4;
            // Kopyala
            let mut packet = Vec::with_capacity(length - 4);
            packet.extend_from_slice(&self.rx_buffer[packet_start..packet_start + length - 4]);

            // rx_idx'i guncelle (4 byte sinirinda olmali)
            self.rx_idx = (self.rx_idx + length + 4 + 3) & !3;
            if self.rx_idx > 8192 {
                self.rx_idx -= 8192;
            }

            // Karta nereye kadar okudugumuzu bildir (CAPR)
            let mut capr: Port<u16> = Port::new(self.io_base + 0x38);
            capr.write((self.rx_idx.wrapping_sub(16)) as u16);

            // Interrupt durumunu temizle
            let mut isr: Port<u16> = Port::new(self.io_base + 0x3E);
            isr.write(0x0005); 

            Some(packet)
        }
    }
}

// PCI Command Register (Offset 0x04)
pub unsafe fn pci_write_u32(bus: u8, slot: u8, func: u8, offset: u8, value: u32) {
    let address = 0x80000000u32 
                | ((bus as u32) << 16) 
                | ((slot as u32) << 11) 
                | ((func as u32) << 8) 
                | ((offset as u32) & 0xFC);
                
    let mut config_address: Port<u32> = Port::new(0xCF8);
    let mut config_data: Port<u32> = Port::new(0xCFC);
    
    config_address.write(address);
    config_data.write(value);
}

pub fn init_network() -> Result<(), &'static str> {
    let devices = scan_pci();
    let mut rtl_dev = None;
    
    for dev in devices {
        if dev.vendor_id == 0x10EC && dev.device_id == 0x8139 {
            rtl_dev = Some(dev);
            break;
        }
    }

    let dev = match rtl_dev {
        Some(d) => d,
        None => return Err("RTL8139 Ethernet Karti Bulunamadi!"),
    };

    unsafe {
        // BAR0 Oku (Offset 0x10)
        let bar0 = pci_read_u32(dev.bus, dev.slot, dev.func, 0x10);
        let io_base = (bar0 & 0xFFFC) as u16; // Son 2 biti at

        // Bus Mastering ve IO Space aktif et (Offset 0x04 - Command Register)
        // Bit 0 = I/O Space, Bit 2 = Bus Master
        let mut cmd = pci_read_u32(dev.bus, dev.slot, dev.func, 0x04);
        cmd |= 0x05; // 0000_0101
        pci_write_u32(dev.bus, dev.slot, dev.func, 0x04, cmd);

        RTL8139_DEV = Some(Rtl8139::new(io_base));
    }

    Ok(())
}
