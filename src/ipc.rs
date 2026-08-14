use crate::sync::BlockingChannel;
use crate::cap::{self, CapHandle, CapError, Rights, ObjectKind};
use core::fmt::Debug;

/// SparkOS Capability-Based & Typed IPC
/// 
/// Geleneksel tip güvenli BlockingChannel üzerine inşa edilmiş,
/// yetki ve kaynak aktarımı (Capability Transfer / Lend) destekleyen
/// mikroçekirdek IPC katmanı.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMode {
    None,
    /// Sahiplik aktarımı: kaynak eski lineage'dan tamamen koparılır, alıcı yeni root olur.
    Transfer,
    /// Geçici ödünç: gönderici mesaj kuyruktayken yetkiyi geri alabilir (revoke).
    Lend,
}

/// Capability taşıyabilen mesaj zarfı
#[derive(Debug, Clone)]
pub struct CapMessage<M> {
    pub payload: M,
    pub capability: Option<CapHandle>,
    pub transfer_mode: TransferMode,
}

impl<M> CapMessage<M> {
    pub fn new_plain(payload: M) -> Self {
        CapMessage {
            payload,
            capability: None,
            transfer_mode: TransferMode::None,
        }
    }

    pub fn new_with_cap(payload: M, cap: CapHandle, mode: TransferMode) -> Self {
        CapMessage {
            payload,
            capability: Some(cap),
            transfer_mode: mode,
        }
    }
}

/// Capability-gated Typed IPC Kanalı
pub struct CapChannel<M> {
    endpoint_cap: CapHandle,
    inner: BlockingChannel<CapMessage<M>>,
}

impl<M> CapChannel<M> {
    /// Yeni bir capability-gated kanal ve ilişkili Endpoint capability'sini oluşturur.
    pub fn new(capacity: usize) -> cap::Result<(Self, CapHandle)> {
        let endpoint_obj = cap::create_object(ObjectKind::Endpoint)?;
        let channel = CapChannel {
            endpoint_cap: endpoint_obj,
            inner: BlockingChannel::new(capacity),
        };
        Ok((channel, endpoint_obj))
    }

    /// Kanalın kök Endpoint capability handle'ını döndürür.
    pub fn endpoint(&self) -> CapHandle {
        self.endpoint_cap
    }

    /// Mesaj gönderimi: Gönderenin kanal üzerinde WRITE (2) yetkisi doğrulanır.
    /// Gönderilen capability'nin bu spesifik endpoint nesnesine ait olduğu denetlenir.
    pub fn send(
        &self,
        sender_cap: CapHandle,
        msg: M,
        attached_cap: Option<CapHandle>,
        mode: TransferMode,
    ) -> cap::Result<()> {
        // Kanal Endpoint WRITE ve nesne kimliği kontrolü (Confused Deputy koruması)
        cap::check_rights_for_object(sender_cap, self.endpoint_cap, Rights::WRITE)?;

        let final_cap = match (attached_cap, mode) {
            (Some(cap), TransferMode::Transfer) => {
                // Kalıcı devir: lineage'dan kopar
                Some(cap::transfer(cap, Rights::all())?)
            }
            (Some(cap), TransferMode::Lend) => {
                // Geçici devir: canlılık doğrula, handle aynen iletilir
                cap::check_rights(cap, Rights::empty())?;
                Some(cap)
            }
            (Some(cap), TransferMode::None) => Some(cap),
            (None, _) => None,
        };

        let envelope = CapMessage {
            payload: msg,
            capability: final_cap,
            transfer_mode: mode,
        };

        self.inner.send(envelope);
        Ok(())
    }

    /// Mesaj alımı: Alıcının kanal üzerinde READ (1) yetkisi doğrulanır.
    pub fn recv(&self, receiver_cap: CapHandle) -> cap::Result<CapMessage<M>> {
        // Kanal Endpoint READ ve nesne kimliği kontrolü
        cap::check_rights_for_object(receiver_cap, self.endpoint_cap, Rights::READ)?;
        Ok(self.inner.recv())
    }

    /// Non-blocking deneme ile gönderim
    pub fn try_send(
        &self,
        sender_cap: CapHandle,
        msg: M,
        attached_cap: Option<CapHandle>,
        mode: TransferMode,
    ) -> cap::Result<()> {
        cap::check_rights_for_object(sender_cap, self.endpoint_cap, Rights::WRITE)?;

        let final_cap = match (attached_cap, mode) {
            (Some(cap), TransferMode::Transfer) => Some(cap::transfer(cap, Rights::all())?),
            (Some(cap), TransferMode::Lend) => {
                cap::check_rights(cap, Rights::empty())?;
                Some(cap)
            }
            (Some(cap), TransferMode::None) => Some(cap),
            (None, _) => None,
        };

        let envelope = CapMessage {
            payload: msg,
            capability: final_cap,
            transfer_mode: mode,
        };

        self.inner.try_send(envelope).map_err(|_| CapError::Exhausted)
    }

    /// Non-blocking deneme ile alım
    pub fn try_recv(&self, receiver_cap: CapHandle) -> cap::Result<Option<CapMessage<M>>> {
        cap::check_rights_for_object(receiver_cap, self.endpoint_cap, Rights::READ)?;
        Ok(self.inner.try_recv())
    }
}

// -----------------------------------------------------------------------------
// Global Endpoint Registry & Syscall Bridge (Ring 3 <-> Microkernel IPC)
// -----------------------------------------------------------------------------

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use crossbeam_queue::ArrayQueue;
use spin::{Mutex, Once};

pub type RawCapChannel = CapChannel<Vec<u8>>;

pub static ENDPOINTS: Mutex<BTreeMap<u32, RawCapChannel>> = Mutex::new(BTreeMap::new());

/// Monotonik (asla yeniden kullanılmayan) endpoint ID kaynağı.
///
/// `ep_id` iki rolde birden çalışır: (1) ENDPOINTS kayıt defterinin anahtarı ve
/// (2) syscall köprüsünün per-process cap_table'ındaki fd değeri (bkz.
/// syscall.rs `sys_ipc_create_endpoint` ve syscall_cap.rs `find_fd_in_table`).
/// Bu yüzden ep_id, capability object store'un `handle.slot`'undan — serbest
/// listeyle yeniden kullanılabilen dahili bir indeks — TÜRETİLEMEZ: slot'un
/// yeniden kullanımı, çoktan sonlanmış bir endpoint'in ep_id'sinin canlı bir
/// endpoint'e atanmasına yol açardı ve fd çakışması / yanlış kuyruk erişimi
/// üretirdi. Bu sayaç, object store slot'larından tamamen bağımsız, monotonik
/// ve uygulama ömrü boyunca asla çakışmayan bir ep_id ad alanı sağlar.
///
/// Başlangıç değeri: stdio fd'leri (0,1,2) ile service/exec sentinellerinden
/// (process.rs: `SERVICE_DEVICE_FD = u32::MAX - 1`, exec = u32::MAX) uzak
/// tutar; sarılma (wrap) pratikte imkânsızdır (u32 aralığı).
static NEXT_EP_ID: AtomicU32 = AtomicU32::new(8);

/// Yeni bir mikroçekirdek IPC Endpoint'i oluşturur ve kayıt eder.
///
/// Dönen `ep_id` monotonik ad alanından gelir (`NEXT_EP_ID`) — capability
/// object store'un serbest-listeyle yeniden kullanılabilen slot'undan değil.
/// `ep_id` aynı zamanda syscall'un cap_table'a fd olarak yerleştirdiği değerdir;
/// arama kuralı (`find_fd_in_table`) ep_id == fd denkliğini koruduğu sürece
/// değişmez.
pub fn create_raw_endpoint(capacity: usize) -> cap::Result<(u32, CapHandle)> {
    let (channel, handle) = RawCapChannel::new(capacity)?;
    let ep_id = NEXT_EP_ID.fetch_add(1, Ordering::Relaxed);
    ENDPOINTS.lock().insert(ep_id, channel);
    Ok((ep_id, handle))
}

/// Syscall için ham bayt IPC gönderimi
pub fn raw_ipc_send(
    ep_id: u32,
    sender_cap: CapHandle,
    data: &[u8],
    attached_cap: Option<CapHandle>,
    mode: TransferMode,
) -> cap::Result<()> {
    let guard = ENDPOINTS.lock();
    let channel = guard.get(&ep_id).ok_or(CapError::NotFound)?;
    channel.send(sender_cap, data.to_vec(), attached_cap, mode)
}

/// Syscall için ham bayt IPC alımı — BLOCKING.
///
/// ÖNEMLİ (single-CPU freeze düzeltmesi): ENDPOINTS kilidi bekleme boyunca
/// TUTULMAZ. Her denemede kısa süre kilitle, `try_recv` dene, kilidi bırak;
/// kuyruk boşsa kilit AÇIKKEN `hlt` ile bekle. Kilidi beklerken tutmak, gönderen
/// process'in `ENDPOINTS.lock()` üzerinde sonsuz spin yapmasına ve single-CPU'da
/// deadlock'a yol açardı. Preemptive scheduler aktifse hlt sırasında timer IRQ
/// gönderici process'i çalıştırır; biz döndüğümüzde kuyrukta mesaj buluruz.
/// Kuyruk boşken kısa sürelerle hlt (power-save) ile dönmek CPU'yu yakmaz.
///
/// `#[cfg(target_os = "none")]`: bu yol kernel hedefine (`x86_64-unknown-none`)
/// özgüdür — `enable_and_hlt` host'ta anlamsız/tehlikelidir ve host unit testi
/// (scratch/cap_test) blocking bekleme yolunu zaten test edemez (gerçek condvar
/// yok). Yalnızca kernel syscall köprüsü (SYS_IPC_RECV) tarafından çağrılır.
#[cfg(target_os = "none")]
pub fn raw_ipc_recv(
    ep_id: u32,
    receiver_cap: CapHandle,
) -> cap::Result<CapMessage<Vec<u8>>> {
    loop {
        let msg = {
            let guard = ENDPOINTS.lock();
            let channel = guard.get(&ep_id).ok_or(CapError::NotFound)?;
            channel.try_recv(receiver_cap)?
        };
        if let Some(msg) = msg {
            return Ok(msg);
        }
        // Kilit bırakıldı — gönderen process kilidi alıp send yapabilir.
        x86_64::instructions::interrupts::enable_and_hlt();
    }
}

/// Syscall için ham bayt IPC alımı — NON-BLOCKING.
/// Kuyrukta mesaj yoksa `Ok(None)` döner, CPU beklemez. User-space servislerin
/// kilitleme riski olmadan endpoint'leri polleyebilmesi için tasarlandı.
pub fn raw_ipc_try_recv(
    ep_id: u32,
    receiver_cap: CapHandle,
) -> cap::Result<Option<CapMessage<Vec<u8>>>> {
    let guard = ENDPOINTS.lock();
    let channel = guard.get(&ep_id).ok_or(CapError::NotFound)?;
    channel.try_recv(receiver_cap)
}

/// Syscall / IRQ teslimi için ham bayt IPC gönderimi — NON-BLOCKING.
/// Kanal doluysa `Err(CapError::Exhausted)` döner, arayanı asla bloke etmez.
/// IRQ teslim yolu (`deliver_pending_irqs`) bunu kullanır: `BlockingChannel::send`'in
/// condvar beklemesine takılıp executor'ı kilitlemek felaket olurdu (single-CPU).
pub fn raw_ipc_try_send(
    ep_id: u32,
    sender_cap: CapHandle,
    data: &[u8],
    attached_cap: Option<CapHandle>,
    mode: TransferMode,
) -> cap::Result<()> {
    let guard = ENDPOINTS.lock();
    let channel = guard.get(&ep_id).ok_or(CapError::NotFound)?;
    channel.try_send(sender_cap, data.to_vec(), attached_cap, mode)
}

// -----------------------------------------------------------------------------
// IRQ Notification Endpoint (Aşama 5.1: driver → user-space servis olay akışı)
// -----------------------------------------------------------------------------
//
// Kısıt: IRQ handler interrupt context'inde çalışır; `ENDPOINTS` spinlock'u orada
// ALINAMAZ (kesilen context aynı kilidi tutuyor olabilir → single-CPU deadlock).
// Bu yüzden `add_scancode` ile aynı kanıtlanmış lock-free desen kullanılır:
//
//   IRQ handler ──lock-free push──▶ IRQ_EVENTS ──interrupt dışı drain──▶ bağlı Endpoint
//       (irq, payload)             (crossbeam ArrayQueue)             (raw_ipc_try_send)
//
// Bağlı IRQ yokken her IRQ'da kuyruğa boş push yapmamak için `IRQ_BINDINGS_NONEMPTY`
// atomic bayrağı, IRQ handler'ın spinlock almadan kontrol edebildiği ucuz bir kapıdır.

/// Bir IRQ'nun olaylarının iletileceği hedef (bağlı Endpoint).
#[derive(Debug, Clone, Copy)]
pub struct IrqBinding {
    ep_id: u32,
    /// Teslimde kullanılan, endpoint üzerinde WRITE yetkisi taşıyan handle.
    writer_cap: CapHandle,
}

/// Bağlı IRQ → hedef eşlemesi. IRQ numarasına (PIC 0..15) anahtarlı.
/// CapObject payload taşımadığı için (Task #4 deseni) bu kayıt kernel tarafında tutulur.
static IRQ_BINDINGS: Mutex<BTreeMap<u8, IrqBinding>> = Mutex::new(BTreeMap::new());
/// IRQ handler'ın spinlock almadan (relaxed atomic load) kontrol ettiği "bağ var mı"
/// kapısı. Bağ yokken her IRQ olayı tek atomic load ile no-op'tur.
static IRQ_BINDINGS_NONEMPTY: AtomicBool = AtomicBool::new(false);
/// (irq, payload) çiftleri — lock-free, interrupt-safe. Doluysa düşer
/// (add_scancode ile aynı dayanıklılık kuralı: olay kaybı IPC kilitlenmesinden iyidir).
static IRQ_EVENTS: Once<ArrayQueue<(u8, u8)>> = Once::new();

/// Interrupt'lar açılmadan önce çağrılmalı (main.rs, `task::keyboard::init()` yanında).
pub fn init_irq_notify() {
    IRQ_EVENTS.call_once(|| ArrayQueue::new(256));
}

/// `irq` kesmesini `ep_id` endpoint'ine bağlar. Gate (Task #4 ile tutarlı):
/// `device_cap` canlı + MANAGE yetkili + ObjectKind::Device olmalı — socket gibi
/// IO yetkili ama cihaz olmayan nesneler IRQ bağlayamaz (confused deputy).
/// `writer_cap`, teslimde kullanılacak, endpoint üzerinde WRITE yetkisi taşıyan
/// handle'dır. Bind anında ep_id'nin kayıtlı bir endpoint olduğu ve writer_cap'in
/// ona yazabildiği doğrulanır — kötü ep_id / yetkisiz handle erken yakalanır.
pub fn bind_irq(
    device_cap: CapHandle,
    irq: u8,
    ep_id: u32,
    writer_cap: CapHandle,
) -> cap::Result<()> {
    cap::check_rights(device_cap, Rights::MANAGE)?;
    let (kind, _object_idx) = cap::object_identity(device_cap)?;
    if kind != ObjectKind::Device {
        return Err(CapError::NoRights);
    }
    if irq > 15 {
        return Err(CapError::Invalid); // PIC yalnızca IRQ 0..15 üretir
    }
    // Endpoint'in kayıtlı olduğunu ve writer_cap'in ona yazabildiğini bind anında doğrula.
    let endpoint_root = {
        let guard = ENDPOINTS.lock();
        guard.get(&ep_id).ok_or(CapError::NotFound)?.endpoint()
    };
    cap::check_rights_for_object(writer_cap, endpoint_root, Rights::WRITE)?;

    IRQ_BINDINGS
        .lock()
        .insert(irq, IrqBinding { ep_id, writer_cap });
    IRQ_BINDINGS_NONEMPTY.store(true, Ordering::Relaxed);
    Ok(())
}

/// IRQ bağını kaldırır. Aynı MANAGE + Device gate'i gerekir.
pub fn unbind_irq(device_cap: CapHandle, irq: u8) -> cap::Result<()> {
    cap::check_rights(device_cap, Rights::MANAGE)?;
    let (kind, _object_idx) = cap::object_identity(device_cap)?;
    if kind != ObjectKind::Device {
        return Err(CapError::NoRights);
    }
    let removed = IRQ_BINDINGS.lock().remove(&irq).is_some();
    if IRQ_BINDINGS.lock().is_empty() {
        IRQ_BINDINGS_NONEMPTY.store(false, Ordering::Relaxed);
    }
    if !removed {
        return Err(CapError::NotFound);
    }
    Ok(())
}

/// IRQ handler'dan çağrılır (interrupt context). Lock-free: spinlock alınmaz;
/// yalnızca relaxed atomic load + lock-free queue push. Bağlı IRQ yoksa hızlıca döner.
pub(crate) fn irq_event(irq: u8, payload: u8) {
    if !IRQ_BINDINGS_NONEMPTY.load(Ordering::Relaxed) {
        return;
    }
    if let Some(queue) = IRQ_EVENTS.get() {
        if queue.push((irq, payload)).is_err() {
            #[cfg(target_os = "none")]
            crate::serial_println!("WARNING: IRQ event queue full; dropping irq {}", irq);
        }
    }
}

/// Interrupt DIŞI bağlamda çağrılır (executor task / syscall): `IRQ_EVENTS`'i
/// boşaltır, her olayı bağlı endpoint'e try_send ile teslim eder. Dolu kanalda
/// olay düşürülür (executor'ı bloke etmez); bağı olmayan IRQ olayları da düşer
/// (unbind sonrası kuyrukta kalanlar).
pub fn deliver_pending_irqs() {
    let mut drained: Vec<(u8, u8)> = Vec::new();
    if let Some(queue) = IRQ_EVENTS.get() {
        while let Some(event) = queue.pop() {
            drained.push(event);
        }
    }
    for (irq, payload) in drained {
        let binding = {
            let guard = IRQ_BINDINGS.lock();
            guard
                .get(&irq)
                .map(|b| IrqBinding { ep_id: b.ep_id, writer_cap: b.writer_cap })
        };
        if let Some(b) = binding {
            let _ = raw_ipc_try_send(b.ep_id, b.writer_cap, &[payload], None, TransferMode::None);
        }
    }
}

// -----------------------------------------------------------------------------
// Legacy API & System Message Definitions (Geriye Dönük Uyumluluk)
// -----------------------------------------------------------------------------

/// Typed kanal — sadece belirli bir tipin geçmesine izin verir (legacy alias)
pub type Channel<M> = BlockingChannel<M>;

/// Yetenek (Capability) tipi — legacy generic wrapper
pub struct Capability<T> {
    _inner: T,
}

impl<T> Capability<T> {
    pub fn new(inner: T) -> Self {
        Capability { _inner: inner }
    }
}

/// Sistem mesaj tipleri — her biri kendi alanına sahip
#[derive(Debug)]
pub enum SystemMessage {
    OpenFile { path: &'static str, flags: FileFlags },
    ReadSector { device: u8, lba: u64, len: u64 },
    WriteSector { device: u8, lba: u64, data: &'static [u8] },
    SpawnTask { name: &'static str },
    Shutdown,
}

#[derive(Debug, Clone, Copy)]
pub enum FileFlags {
    ReadOnly,
    ReadWrite,
    Create,
}

// Global system IPC kanalı (legacy uyumluluğu korunur)
pub static SYSTEM_CHAN: Channel<SystemMessage> = Channel::new(128);

// -----------------------------------------------------------------------------
// PURE Host Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cap::{self, Rights, ObjectKind, CapError};

    #[test]
    fn test_cap_channel_send_recv_permissions() {
        cap::init();
        let (channel, endpoint_root) = CapChannel::<u32>::new(10).unwrap();

        // Writer ve Reader handle'ları türet
        let writer_cap = cap::grant(endpoint_root, Rights::WRITE).unwrap();
        let reader_cap = cap::grant(endpoint_root, Rights::READ).unwrap();
        let unpriv_cap = cap::grant(endpoint_root, Rights::empty()).unwrap();

        // Yetkisiz gönderim -> NoRights
        assert_eq!(
            channel.send(unpriv_cap, 42, None, TransferMode::None).err(),
            Some(CapError::NoRights)
        );

        // Doğru yetkiyle gönderim -> OK
        assert!(channel.send(writer_cap, 42, None, TransferMode::None).is_ok());

        // Yetkisiz alım -> NoRights
        assert_eq!(
            channel.recv(unpriv_cap).err(),
            Some(CapError::NoRights)
        );

        // Doğru yetkiyle alım -> OK
        let msg = channel.recv(reader_cap).unwrap();
        assert_eq!(msg.payload, 42);
        assert!(msg.capability.is_none());
    }

    #[test]
    fn test_cap_channel_transfer_and_lend() {
        cap::init();
        let (channel, endpoint_root) = CapChannel::<&'static str>::new(10).unwrap();
        let writer_cap = cap::grant(endpoint_root, Rights::WRITE).unwrap();
        let reader_cap = cap::grant(endpoint_root, Rights::READ).unwrap();

        // 1. Transfer testi: Kaynak obje oluştur ve transfer et
        let mem_obj = cap::create_object(ObjectKind::Memory).unwrap();
        channel.send(writer_cap, "transfer_test", Some(mem_obj), TransferMode::Transfer).unwrap();

        let received = channel.recv(reader_cap).unwrap();
        assert_eq!(received.payload, "transfer_test");
        let transferred_cap = received.capability.unwrap();
        // Aktarılan capability hala geçerlidir
        assert!(cap::check_rights(transferred_cap, Rights::READ).is_ok());

        // 2. Lend testi: Ödünç ver ve geri al (revoke)
        let parent_obj = cap::create_object(ObjectKind::Memory).unwrap();
        let lent_cap = cap::lend(parent_obj, Rights::READ).unwrap();
        channel.send(writer_cap, "lend_test", Some(lent_cap), TransferMode::Lend).unwrap();

        // Gönderici ödünç verdiği yetkiyi geri alıyor
        cap::revoke(parent_obj).unwrap();

        let received_lend = channel.recv(reader_cap).unwrap();
        assert_eq!(received_lend.payload, "lend_test"); // Payload başarıyla teslim edilir
        let cap_in_msg = received_lend.capability.unwrap();
        // Ancak ödünç alınan yetki artık Revoked durumundadır
        assert_eq!(cap::check_rights(cap_in_msg, Rights::READ).err(), Some(CapError::Revoked));
    }

    #[test]
    fn test_cap_channel_endpoint_isolation() {
        cap::init();
        let (channel_a, ep_a) = CapChannel::<u32>::new(10).unwrap();
        let (channel_b, _ep_b) = CapChannel::<u32>::new(10).unwrap();

        let writer_cap_a = cap::grant(ep_a, Rights::WRITE).unwrap();

        // writer_cap_a, Channel A'ya yazabilir
        assert!(channel_a.send(writer_cap_a, 100, None, TransferMode::None).is_ok());

        // writer_cap_a, Channel B'ye yazamaz (farklı endpoint nesnesi) -> NoRights (Confused Deputy engellendi)
        assert_eq!(
            channel_b.send(writer_cap_a, 200, None, TransferMode::None).err(),
            Some(CapError::NoRights)
        );
    }

    #[test]
    fn test_irq_bind_requires_manage_device() {
        cap::init();
        init_irq_notify();
        let (ep_id, ep_root) = create_raw_endpoint(16).unwrap();

        // MANAGE'sız Device (yalnız IO) → NoRights
        let dev_io = cap::create_object(ObjectKind::Device).unwrap();
        let dev_io = cap::grant(dev_io, Rights(8)).unwrap();
        assert_eq!(bind_irq(dev_io, 1, ep_id, ep_root).err(), Some(CapError::NoRights));

        // Device değil ama MANAGE'li (socket fd) → NoRights (confused deputy)
        let sock = cap::create_object(ObjectKind::Fd).unwrap();
        let sock_manage = cap::grant(sock, Rights(512)).unwrap();
        assert_eq!(bind_irq(sock_manage, 1, ep_id, ep_root).err(), Some(CapError::NoRights));

        // Doğru: Device + MANAGE (create_object tüm yetkileri verir) → OK
        let dev = cap::create_object(ObjectKind::Device).unwrap();
        assert!(bind_irq(dev, 1, ep_id, ep_root).is_ok());

        // PIC dışı IRQ → Invalid
        assert_eq!(bind_irq(dev, 16, ep_id, ep_root).err(), Some(CapError::Invalid));

        // Kayıtsız endpoint → NotFound (bind anında erken yakalanır)
        assert_eq!(bind_irq(dev, 2, 999, ep_root).err(), Some(CapError::NotFound));

        unbind_irq(dev, 1).unwrap();
    }

    #[test]
    fn test_irq_unbind_removes_binding() {
        cap::init();
        init_irq_notify();
        let (ep_id, ep_root) = create_raw_endpoint(16).unwrap();
        let dev = cap::create_object(ObjectKind::Device).unwrap();

        bind_irq(dev, 3, ep_id, ep_root).unwrap();
        unbind_irq(dev, 3).unwrap();
        // İkinci unbind → NotFound
        assert_eq!(unbind_irq(dev, 3).err(), Some(CapError::NotFound));
    }

    #[test]
    fn test_irq_delivery_chain() {
        cap::init();
        init_irq_notify();
        // Önceki testlerden kalan olayları temizle.
        deliver_pending_irqs();

        let (ep_id, ep_root) = create_raw_endpoint(16).unwrap();
        let dev = cap::create_object(ObjectKind::Device).unwrap();
        bind_irq(dev, 0, ep_id, ep_root).unwrap();

        // IRQ handler simülasyonu: lock-free push (interrupt-safe yol).
        irq_event(0, 0x4b);
        deliver_pending_irqs();
        let msg = raw_ipc_try_recv(ep_id, ep_root).unwrap().unwrap();
        assert_eq!(msg.payload, [0x4b]);

        // Bağlı olmayan IRQ olayı → düşürülür, teslim yok.
        irq_event(4, 0x99);
        deliver_pending_irqs();
        assert!(raw_ipc_try_recv(ep_id, ep_root).unwrap().is_none());

        unbind_irq(dev, 0).unwrap();
    }
}
