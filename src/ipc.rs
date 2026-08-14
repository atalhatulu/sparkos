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
    /// Taşınan capability varsa belirtilen TransferMode kurallarına göre işlenir.
    pub fn send(
        &self,
        sender_cap: CapHandle,
        msg: M,
        attached_cap: Option<CapHandle>,
        mode: TransferMode,
    ) -> cap::Result<()> {
        // Kanal Endpoint WRITE yetki kontrolü
        cap::check_rights(sender_cap, Rights::WRITE)?;

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
        // Kanal Endpoint READ yetki kontrolü
        cap::check_rights(receiver_cap, Rights::READ)?;
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
        cap::check_rights(sender_cap, Rights::WRITE)?;

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
        cap::check_rights(receiver_cap, Rights::READ)?;
        Ok(self.inner.try_recv())
    }
}

// -----------------------------------------------------------------------------
// Global Endpoint Registry & Syscall Bridge (Ring 3 <-> Microkernel IPC)
// -----------------------------------------------------------------------------

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use spin::Mutex;

pub type RawCapChannel = CapChannel<Vec<u8>>;

pub static ENDPOINTS: Mutex<BTreeMap<u32, RawCapChannel>> = Mutex::new(BTreeMap::new());

/// Yeni bir mikroçekirdek IPC Endpoint'i oluşturur ve kayıt eder.
pub fn create_raw_endpoint(capacity: usize) -> cap::Result<(u32, CapHandle)> {
    let (channel, handle) = RawCapChannel::new(capacity)?;
    let ep_id = handle.slot;
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

/// Syscall için ham bayt IPC alımı
pub fn raw_ipc_recv(
    ep_id: u32,
    receiver_cap: CapHandle,
) -> cap::Result<CapMessage<Vec<u8>>> {
    let guard = ENDPOINTS.lock();
    let channel = guard.get(&ep_id).ok_or(CapError::NotFound)?;
    channel.recv(receiver_cap)
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
}
