//! USB host controller iskeleti (L7).
//!
//! Gerçek donanım kodu burada yoktur — bu modül şunları sağlar:
//!   - USB host controller mimarisi özeti (UHCI/EHCI/xHCI register map'leri)
//!   - `UsbHostController` trait'i (sürücü yazacaklara sabit arayüz)
//!   - PCI taraması: USB host controller'ları bulma (`probe_usb_controllers`)
//!   - Cihaz numaralandırma penceresi: descriptor parsing iskeleti + state machine
//!
//! Tak-çalıştır BEKLENMEZ: bu adımda sağlam iskelet + temel yapılar yeterlidir.
//! Gerçek transferler (UHCI frame list / EHCI async & periodic schedules /
//! xHCI TRB ring'leri) ayrı bir katman gerektirir.

use alloc::vec::Vec;
use alloc::string::String;
use alloc::format;

use crate::pci::{PciDevice, PciBar};

// ---------------------------------------------------------------------------
// PCI kimlikleri
// ---------------------------------------------------------------------------

/// Serial bus controller (class 0x0C) / USB (subclass 0x03).
pub const PCI_CLASS_SERIAL_BUS: u8 = 0x0C;
pub const PCI_SUBCLASS_USB: u8 = 0x03;

// ---------------------------------------------------------------------------
// Controller tipleri
// ---------------------------------------------------------------------------

/// USB host controller donanım mimarisi.
///
/// Register map özetleri:
///
/// **UHCI (Intel, PCI prog-if 0x00, I/O BAR)**
/// ```text
/// +0x00 USBCMD    Command (Run, Host Reset, Global Reset...)
/// +0x02 USBSTS    Status (HCHalted, HSE, ...)
/// +0x04 USBINTR   Interrupt Enable
/// +0x06 FRNUM     Frame Number
/// +0x08 FRBASEADD Frame List Base Address (fiziksel, 4096-aligned)
/// +0x0C SOFMOD    Start of Frame Modify
/// +0x10.. PORTSC0..PORTSCn  Port Status/Control (x2 port)
/// ```
/// Transfer modeli: 1024'lük frame list → Queue Head → Transfer Descriptor (TD)
/// zincirleri; isochronous dahil tüm aktarımlar frame schedule üzerinden.
///
/// **OHCI (Compaq, prog-if 0x10, MMIO BAR)**
/// ```text
/// +0x00 HcRevision, +0x04 HcControl, +0x08 HcCommandStatus
/// +0x0C HcInterruptStatus, +0x10 HcInterruptEnable
/// +0x14 HcInterruptDisable, +0x18 HcHCCA (Host Controller Comm. Area, fiziksel)
/// +0x1C HcPeriodCurrentED, +0x20 HcControlHeadED, +0x24 HcControlCurrentED
/// +0x28 HcBulkHeadED, +0x2C HcBulkCurrentED, +0x30 HcDoneHead
/// +0x34 HcFmInterval, +0x38 HcFmRemaining, +0x3C HcFmNumber
/// +0x40 HcPeriodicStart, +0x44 HcLSThreshold
/// +0x48.. HcRhDescriptorA/B, HcRhStatus, HcRhPortStatus[0..n]
/// ```
///
/// **EHCI (USB 2.0, prog-if 0x20, MMIO BAR)**
/// ```text
/// +0x00 CAPLENGTH/CAPBASE, +0x04 HCIVERSION
/// +0x08 HCSPARAMS  (N_PORTS, PPC, N_CC, N_PCC...)
/// +0x0C HCCPARAMS  (64-bit addressing, Async/Periodic scheduling cap)
/// +0x10.. USBCMD, USBSTS, USBINTR, FRINDEX, CTRLDSSEGMENT
/// +0x20.. PERIODICLISTBASE, ASYNCLISTADDR
/// +0x50.. CONFIGFLAG, PORTSC[n]
/// ```
/// Transfer modeli: async list (bulk/control Queue Heads) + periodic list
/// (isochronous/interrupt). 64-bit çalışmak için `CTRLDSSEGMENT` kullanılır.
///
/// **xHCI (USB 3.x, prog-if 0x30, MMIO BAR)**
/// ```text
/// +0x00  CAPLENGTH/HCIVERSION
/// +0x04  HCSPARAMS1 (MaxSlots, MaxIntrs, MaxPorts)
/// +0x08  HCSPARAMS2
/// +0x0C  HCSPARAMS3
/// +0x10  HCCPARAMS1  (64-bit addressing, xECP pointer)
/// +0x14  DBOFF, +0x18 RTSOFF
/// +0x00+CAPLENGTH..  OPERATIONAL: USBCMD, USBSTS, USBINTR, ...
/// ```
/// Transfer modeli: Event Ring + Command Ring + Transfer Ring'ler (TRB'ler).
/// Slot/Endpoint başına ring'ler; en karmaşık mimari budur.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbControllerType {
    Uhci,
    Ohci,
    Ehci,
    Xhci,
    Other,
}

impl UsbControllerType {
    /// PCI prog-if (Programming Interface) değerinden tür çözümler.
    pub fn from_prog_if(prog_if: u8) -> Self {
        match prog_if {
            0x00 => UsbControllerType::Uhci,
            0x10 => UsbControllerType::Ohci,
            0x20 => UsbControllerType::Ehci,
            0x30 => UsbControllerType::Xhci,
            _ => UsbControllerType::Other,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            UsbControllerType::Uhci => "UHCI (USB 1.1, I/O mapped)",
            UsbControllerType::Ohci => "OHCI (USB 1.1, MMIO)",
            UsbControllerType::Ehci => "EHCI (USB 2.0)",
            UsbControllerType::Xhci => "xHCI (USB 3.x)",
            UsbControllerType::Other => "Unknown USB controller",
        }
    }

    /// MMIO mu yoksa I/O port üzerinden mi erişilir?
    pub fn is_mmio(&self) -> bool {
        !matches!(self, UsbControllerType::Uhci)
    }
}

// ---------------------------------------------------------------------------
// USB hızları / cihaz tipleri
// ---------------------------------------------------------------------------

/// USB aygıt hızı (port üzerinden algılanır).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbSpeed {
    Low,   // 1.5 Mbps
    Full,  // 12 Mbps
    High,  // 480 Mbps
    Super, // 5 Gbps
}

impl UsbSpeed {
    pub fn name(&self) -> &'static str {
        match self {
            UsbSpeed::Low => "Low (1.5 Mbps)",
            UsbSpeed::Full => "Full (12 Mbps)",
            UsbSpeed::High => "High (480 Mbps)",
            UsbSpeed::Super => "Super (5 Gbps)",
        }
    }
}

/// USB device class sabitleri (bDeviceClass / bInterfaceClass).
pub mod device_class {
    pub const PER_INTERFACE: u8 = 0x00;
    pub const AUDIO: u8 = 0x01;
    pub const HID: u8 = 0x03;
    pub const MASS_STORAGE: u8 = 0x08;
    pub const HUB: u8 = 0x09;
    pub const VENDOR_SPECIFIC: u8 = 0xFF;
}

/// USB device state (numaralandırma ilerlemesi).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbDeviceState {
    /// Port reset bekleniyor / yapıldı, adres atanmadı.
    Reset,
    /// SET_ADDRESS ile adres atandı.
    Addressed,
    /// SET_CONFIGURATION ile yapılandırıldı.
    Configured,
}

// ---------------------------------------------------------------------------
// Control transfer istekleri
// ---------------------------------------------------------------------------

/// USB control transfer setup packet'i (8 byte, bmRequestType..wLength).
#[derive(Clone, Copy, Debug, Default)]
pub struct UsbControlRequest {
    pub bm_request_type: u8,
    pub b_request: u8,
    pub w_value: u16,
    pub w_index: u16,
    pub w_length: u16,
}

// Request type bitleri (bmRequestType)
pub const REQ_TYPE_OUT: u8 = 0x00; // Host → Device
pub const REQ_TYPE_IN: u8 = 0x80; // Device → Host
pub const REQ_TYPE_STANDARD: u8 = 0x00;
pub const REQ_TYPE_CLASS: u8 = 0x20;
pub const REQ_TYPE_VENDOR: u8 = 0x40;
pub const REQ_TYPE_DEVICE: u8 = 0x00;
pub const REQ_TYPE_INTERFACE: u8 = 0x01;
pub const REQ_TYPE_ENDPOINT: u8 = 0x02;

// Standard request'ler
pub const REQ_GET_STATUS: u8 = 0x00;
pub const REQ_CLEAR_FEATURE: u8 = 0x01;
pub const REQ_SET_FEATURE: u8 = 0x03;
pub const REQ_SET_ADDRESS: u8 = 0x05;
pub const REQ_GET_DESCRIPTOR: u8 = 0x06;
pub const REQ_SET_DESCRIPTOR: u8 = 0x07;
pub const REQ_GET_CONFIGURATION: u8 = 0x08;
pub const REQ_SET_CONFIGURATION: u8 = 0x09;

impl UsbControlRequest {
    /// GET_DESCRIPTOR isteği kurar.
    pub const fn get_descriptor(desc_type: u8, index: u8, lang_id: u16, length: u16) -> Self {
        UsbControlRequest {
            bm_request_type: REQ_TYPE_IN | REQ_TYPE_STANDARD | REQ_TYPE_DEVICE,
            b_request: REQ_GET_DESCRIPTOR,
            w_value: ((desc_type as u16) << 8) | index as u16,
            w_index: lang_id,
            w_length: length,
        }
    }

    /// SET_ADDRESS isteği kurar.
    pub const fn set_address(addr: u8) -> Self {
        UsbControlRequest {
            bm_request_type: REQ_TYPE_OUT | REQ_TYPE_STANDARD | REQ_TYPE_DEVICE,
            b_request: REQ_SET_ADDRESS,
            w_value: addr as u16,
            w_index: 0,
            w_length: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Descriptor parsing iskeleti
// ---------------------------------------------------------------------------

/// USB descriptor type sabitleri.
pub mod descriptor_type {
    pub const DEVICE: u8 = 1;
    pub const CONFIG: u8 = 2;
    pub const STRING: u8 = 3;
    pub const INTERFACE: u8 = 4;
    pub const ENDPOINT: u8 = 5;
}

/// Device descriptor (18 byte). `bMaxPacketSize0` ep0 için kritik.
#[derive(Clone, Copy, Debug)]
pub struct UsbDeviceDescriptor {
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_subclass: u8,
    pub b_device_protocol: u8,
    pub b_max_packet_size0: u8,
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial_number: u8,
    pub b_num_configurations: u8,
}

/// Config descriptor (9 byte + interface/endpoint'ler).
#[derive(Clone, Copy, Debug)]
pub struct UsbConfigDescriptor {
    pub w_total_length: u16,
    pub b_num_interfaces: u8,
    pub b_configuration_value: u8,
    pub i_configuration: u8,
    pub bm_attributes: u8,
    pub b_max_power: u8,
}

/// Interface descriptor (9 byte).
#[derive(Clone, Copy, Debug)]
pub struct UsbInterfaceDescriptor {
    pub b_interface_number: u8,
    pub b_alternate_setting: u8,
    pub b_num_endpoints: u8,
    pub b_interface_class: u8,
    pub b_interface_subclass: u8,
    pub b_interface_protocol: u8,
    pub i_interface: u8,
}

/// Endpoint descriptor (7 byte).
#[derive(Clone, Copy, Debug)]
pub struct UsbEndpointDescriptor {
    pub b_endpoint_address: u8,
    pub bm_attributes: u8,
    pub w_max_packet_size: u16,
    pub b_interval: u8,
}

/// `buf` içindeki 18-byte device descriptor'ı parse eder.
pub fn parse_device_descriptor(buf: &[u8]) -> Option<UsbDeviceDescriptor> {
    if buf.len() < 18 || buf[1] != descriptor_type::DEVICE {
        return None;
    }
    Some(UsbDeviceDescriptor {
        bcd_usb: u16::from_le_bytes([buf[2], buf[3]]),
        b_device_class: buf[4],
        b_device_subclass: buf[5],
        b_device_protocol: buf[6],
        b_max_packet_size0: buf[7],
        id_vendor: u16::from_le_bytes([buf[8], buf[9]]),
        id_product: u16::from_le_bytes([buf[10], buf[11]]),
        bcd_device: u16::from_le_bytes([buf[12], buf[13]]),
        i_manufacturer: buf[14],
        i_product: buf[15],
        i_serial_number: buf[16],
        b_num_configurations: buf[17],
    })
}

/// `buf` içindeki 9-byte config descriptor'ı parse eder.
pub fn parse_config_descriptor(buf: &[u8]) -> Option<UsbConfigDescriptor> {
    if buf.len() < 9 || buf[1] != descriptor_type::CONFIG {
        return None;
    }
    Some(UsbConfigDescriptor {
        w_total_length: u16::from_le_bytes([buf[2], buf[3]]),
        b_num_interfaces: buf[4],
        b_configuration_value: buf[5],
        i_configuration: buf[6],
        bm_attributes: buf[7],
        b_max_power: buf[8],
    })
}

/// Interface descriptor'ı parse eder.
pub fn parse_interface_descriptor(buf: &[u8]) -> Option<UsbInterfaceDescriptor> {
    if buf.len() < 9 || buf[1] != descriptor_type::INTERFACE {
        return None;
    }
    Some(UsbInterfaceDescriptor {
        b_interface_number: buf[2],
        b_alternate_setting: buf[3],
        b_num_endpoints: buf[4],
        b_interface_class: buf[5],
        b_interface_subclass: buf[6],
        b_interface_protocol: buf[7],
        i_interface: buf[8],
    })
}

/// Endpoint descriptor'ı parse eder.
pub fn parse_endpoint_descriptor(buf: &[u8]) -> Option<UsbEndpointDescriptor> {
    if buf.len() < 7 || buf[1] != descriptor_type::ENDPOINT {
        return None;
    }
    Some(UsbEndpointDescriptor {
        b_endpoint_address: buf[2],
        bm_attributes: buf[3],
        w_max_packet_size: u16::from_le_bytes([buf[4], buf[5]]),
        b_interval: buf[6],
    })
}

/// Bir config descriptor kümesindeki interface'leri çıkarır.
///
/// Config verisi: config descriptor + (interface descriptor + endpoint'ler) *
/// `b_num_interfaces` kez. Bu fonksiyon interface descriptor'ları ayıklar.
pub fn parse_configuration(buf: &[u8]) -> Vec<UsbInterfaceDescriptor> {
    let mut interfaces = Vec::new();
    let mut off = 0usize;
    while off + 2 <= buf.len() {
        let len = buf[off] as usize;
        let kind = buf[off + 1];
        if len == 0 {
            break; // bozuk veriye karşı koruma
        }
        if kind == descriptor_type::INTERFACE {
            if let Some(intf) = parse_interface_descriptor(&buf[off..]) {
                interfaces.push(intf);
            }
        }
        off += len;
    }
    interfaces
}

// ---------------------------------------------------------------------------
// Host controller trait
// ---------------------------------------------------------------------------

/// USB host controller sürücü arayüzü.
///
/// Gerçek sürücüler bu trait'i uygular; üst katman (numaralandırma, sınıf
/// sürücüleri) yalnızca bu arayüzü görür.
pub trait UsbHostController {
    /// Bu controller'ın mimarisi (UHCI/EHCI/xHCI...).
    fn controller_type(&self) -> UsbControllerType;

    /// Controller'ı sıfırlar (Host Reset). Hazır olduğunda `Ok(())`.
    fn reset(&mut self) -> Result<(), &'static str>;

    /// Controller'ı çalıştırır (Run bit, schedule'ları başlat).
    fn start(&mut self) -> Result<(), &'static str>;

    /// Controller'ı durdurur (güvenli kapanma).
    fn stop(&mut self);

    /// Bağlı port sayısı.
    fn port_count(&self) -> u8;

    /// `port` üzerindeki cihaz hızı; cihaz yoksa `None`.
    fn device_speed(&self, port: u8) -> Option<UsbSpeed>;

    /// `port`'taki cihazı reset'ler (SE0 ~50ms). Başarılıysa `Ok(())`.
    fn reset_port(&mut self, port: u8) -> Result<(), &'static str>;

    /// Control transferi (setup + data + status phase'leri) yürütür.
    ///
    /// `dev_addr` 0 olabilir (numaralandırma öncesi, default address).
    /// `buf` data phase için kullanılır; IN ise dolu döner, OUT ise içeriği gönderilir.
    fn submit_control(
        &mut self,
        dev_addr: u8,
        request: &UsbControlRequest,
        buf: &mut [u8],
    ) -> Result<usize, &'static str>;
}

// ---------------------------------------------------------------------------
// PCI üzerinden controller bulma
// ---------------------------------------------------------------------------

/// PCI'da bulunan bir USB host controller'ın kaynakları.
#[derive(Clone, Debug)]
pub struct UsbControllerInfo {
    pub device: PciDevice,
    pub ctype: UsbControllerType,
    /// MMIO controller'lar için BAR'dan gelen fiziksel taban adres.
    /// Kernel bu adresi sayfa tablosuna haritalamalıdır (`PHYS_OFFSET` + base).
    pub mmio_base: u64,
    pub mmio_size: u64,
    /// UHCI gibi I/O controller'lar için I/O port tabanı.
    pub io_base: u16,
    /// PCI interrupt line (255 = yok).
    pub irq: u8,
    pub bar: Option<PciBar>,
}

impl UsbControllerInfo {
    /// Controller'ın kısa tanımı (log amaçlı).
    pub fn describe(&self) -> String {
        let res = if self.ctype.is_mmio() {
            format!("mmio {:#x} ({} bytes)", self.mmio_base, self.mmio_size)
        } else {
            format!("io 0x{:04x}", self.io_base)
        };
        format!(
            "{}: {} @ {} irq={}",
            self.ctype.name(),
            res,
            self.device.get_name(),
            self.irq
        )
    }
}

/// PCI cihaz listesinden USB host controller'ları bulur.
pub fn probe_usb_controllers(devices: &[PciDevice]) -> Vec<UsbControllerInfo> {
    let mut controllers = Vec::new();
    for dev in devices {
        if dev.class == PCI_CLASS_SERIAL_BUS && dev.subclass == PCI_SUBCLASS_USB {
            let ctype = UsbControllerType::from_prog_if(dev.prog_if);
            let mut d = dev.clone();
            let bar = d.bar(0);
            let (mmio_base, mmio_size, io_base) = match bar {
                Some(b) if b.is_io => (0u64, 0u64, b.base as u16),
                Some(b) => (b.base, b.size, 0u16),
                None => (0u64, 0u64, 0u16),
            };
            controllers.push(UsbControllerInfo {
                device: dev.clone(),
                ctype,
                mmio_base,
                mmio_size,
                io_base,
                irq: dev.interrupt_line(),
                bar,
            });
        }
    }
    controllers
}

// ---------------------------------------------------------------------------
// Numaralandırma iskeleti
// ---------------------------------------------------------------------------

/// Numaralandırılmış bir USB cihazının özeti.
#[derive(Clone, Debug)]
pub struct UsbDevice {
    /// Numaralandırma sırasında atanan adres (1..=127).
    pub address: u8,
    /// Bağlı olduğu controller portu.
    pub port: u8,
    pub speed: UsbSpeed,
    pub state: UsbDeviceState,
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    /// Endpoint 0 max packet size (device descriptor'dan).
    pub max_packet_size0: u8,
    pub num_configurations: u8,
}

/// Numaralandırma state machine'i.
///
/// Gerçek akış (tak-çalıştır için tamamlanması gerekenler):
/// 1. `reset_port` → cihaz default state'e geçer (addr 0).
/// 2. `set_address` (control, addr 0) → cihaz adresi atanır.
/// 3. `get_descriptor(DEVICE)` → `bMaxPacketSize0` öğrenilir.
/// 4. `get_descriptor(CONFIG)` → config seti çekilir, sınıf sürücüsü seçilir.
/// 5. `set_configuration` → cihaz `Configured` olur.
pub struct UsbEnumerator<HC: UsbHostController> {
    controller: HC,
    next_address: u8,
}

impl<HC: UsbHostController> UsbEnumerator<HC> {
    pub fn new(controller: HC) -> Self {
        UsbEnumerator {
            controller,
            next_address: 1,
        }
    }

    /// Tüm portları tarayıp cihazları numaralandırır (iskelet).
    pub fn enumerate_all(&mut self) -> Vec<UsbDevice> {
        let mut devices = Vec::new();
        for port in 0..self.controller.port_count() {
            if let Some(speed) = self.controller.device_speed(port) {
                if let Some(dev) = self.enumerate_port(port, speed) {
                    devices.push(dev);
                }
            }
        }
        devices
    }

    /// Tek bir portu numaralandırır.
    pub fn enumerate_port(&mut self, port: u8, speed: UsbSpeed) -> Option<UsbDevice> {
        // 1) Portu resetle → cihaz default address (0) ile konuşur.
        self.controller.reset_port(port).ok()?;

        // 2) İlk 8 byte device descriptor: sadece bMaxPacketSize0 için.
        let mut head = [0u8; 8];
        let req = UsbControlRequest::get_descriptor(descriptor_type::DEVICE, 0, 0, 8);
        let n = self.controller.submit_control(0, &req, &mut head).ok()?;
        if n < 8 {
            return None;
        }
        // head[7] = bMaxPacketSize0 — gerçek sürücü bu değeri ep0 window'u için kullanır.

        // 3) Yeni adres ata (SET_ADDRESS, addr 0 üzerinden).
        let address = self.next_address;
        self.next_address = self.next_address.saturating_add(1);
        let set_addr = UsbControlRequest::set_address(address);
        self.controller
            .submit_control(0, &set_addr, &mut [])
            .ok()?;

        // 4) Tam device descriptor'ı çek (18 byte).
        let mut full = [0u8; 18];
        let req = UsbControlRequest::get_descriptor(descriptor_type::DEVICE, 0, 0, 18);
        let n = self.controller.submit_control(address, &req, &mut full).ok()?;
        let desc = parse_device_descriptor(&full[..n])?;

        Some(UsbDevice {
            address,
            port,
            speed,
            state: UsbDeviceState::Addressed,
            vendor_id: desc.id_vendor,
            product_id: desc.id_product,
            device_class: desc.b_device_class,
            device_subclass: desc.b_device_subclass,
            max_packet_size0: desc.b_max_packet_size0,
            num_configurations: desc.b_num_configurations,
        })
    }
}
