use x86_64::instructions::port::{Port, PortReadOnly};
use spin::Mutex;

pub struct AtaDrive {
    data: Port<u16>,
    #[allow(dead_code)]
    error: PortReadOnly<u8>,
    sec_count: Port<u8>,
    lba_lo: Port<u8>,
    lba_mid: Port<u8>,
    lba_hi: Port<u8>,
    drive_select: Port<u8>,
    command: Port<u8>,
    status: PortReadOnly<u8>,
    is_master: bool,
}

impl AtaDrive {
    pub const fn new(base: u16, is_master: bool) -> Self {
        AtaDrive {
            data: Port::new(base),
            error: PortReadOnly::new(base + 1),
            sec_count: Port::new(base + 2),
            lba_lo: Port::new(base + 3),
            lba_mid: Port::new(base + 4),
            lba_hi: Port::new(base + 5),
            drive_select: Port::new(base + 6),
            command: Port::new(base + 7),
            status: PortReadOnly::new(base + 7),
            is_master,
        }
    }

    fn wait_busy(&mut self) -> Result<(), &'static str> {
        // PIO busy-wait with a bounded timeout so a missing/unresponsive
        // disk can't hang the kernel forever. ~100k spins ~ plenty for real HW.
        let mut timeout = 0usize;
        unsafe {
            while self.status.read() & 0x80 != 0 {
                core::hint::spin_loop();
                timeout += 1;
                if timeout > 200_000 {
                    return Err("ATA wait_busy timeout");
                }
            }
        }
        Ok(())
    }

    fn wait_drq(&mut self) -> Result<(), &'static str> {
        let mut timeout = 0usize;
        unsafe {
            while self.status.read() & 0x08 == 0 {
                if self.status.read() & 0x21 != 0 {
                    break;
                }
                core::hint::spin_loop();
                timeout += 1;
                if timeout > 200_000 {
                    return Err("ATA wait_drq timeout");
                }
            }
        }
        Ok(())
    }

    pub fn read_sector(&mut self, lba: u32, buf: &mut [u8; 512]) -> Result<(), &'static str> {
        let drive_bit = if self.is_master { 0 } else { 1 };
        let select = 0xE0 | (drive_bit << 4) | ((lba >> 24) & 0x0F) as u8;

        unsafe {
            self.drive_select.write(select);
            self.wait_busy()?;

            self.sec_count.write(1);
            self.lba_lo.write((lba & 0xFF) as u8);
            self.lba_mid.write(((lba >> 8) & 0xFF) as u8);
            self.lba_hi.write(((lba >> 16) & 0xFF) as u8);
            self.command.write(0x20); // Okuma komutu

            self.wait_busy()?;
            self.wait_drq()?;

            if self.status.read() & 0x01 != 0 {
                return Err("ATA Disk Okuma Hatasi (ERR)");
            }

            let mut ptr = buf.as_mut_ptr() as *mut u16;
            for _ in 0..256 {
                *ptr = self.data.read();
                ptr = ptr.add(1);
            }
        }
        Ok(())
    }

    pub fn write_sector(&mut self, lba: u32, buf: &[u8; 512]) -> Result<(), &'static str> {
        let drive_bit = if self.is_master { 0 } else { 1 };
        let select = 0xE0 | (drive_bit << 4) | ((lba >> 24) & 0x0F) as u8;

        unsafe {
            self.drive_select.write(select);
            self.wait_busy()?;

            self.sec_count.write(1);
            self.lba_lo.write((lba & 0xFF) as u8);
            self.lba_mid.write(((lba >> 8) & 0xFF) as u8);
            self.lba_hi.write(((lba >> 16) & 0xFF) as u8);
            self.command.write(0x30); // Yazma komutu

            self.wait_busy()?;
            self.wait_drq()?;

            if self.status.read() & 0x01 != 0 {
                return Err("ATA Disk Yazma Hatasi (ERR)");
            }

            let mut ptr = buf.as_ptr() as *const u16;
            for _ in 0..256 {
                self.data.write(*ptr);
                ptr = ptr.add(1);
            }

            // Cache flush
            self.command.write(0xE7);
            self.wait_busy()?;
        }
        Ok(())
    }
}

// 2. disk = Primary Slave (index 1)
pub static DATA_DRIVE: spin::Lazy<Mutex<AtaDrive>> = spin::Lazy::new(|| {
    Mutex::new(AtaDrive::new(0x1F0, false))
});
