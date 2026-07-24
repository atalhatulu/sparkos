use alloc::collections::VecDeque;
use spin::Mutex;
use core::fmt::Debug;

/// SparkOS Typed IPC
/// 
/// Geleneksel write/read byte stream yerine tip güvenli kanallar.
/// Her mesaj tipi derleme anında bilinir, asla mismatch olmaz.

/// Yetenek (Capability) tipi — bir kaynağa erişim yetkisini temsil eder
pub struct Capability<T> {
    _inner: T,
}

impl<T> Capability<T> {
    pub fn new(inner: T) -> Self {
        Capability { _inner: inner }
    }
}

/// Typed kanal — sadece belirli bir tipin geçmesine izin verir
pub struct Channel<M: Send + 'static> {
    buffer: Mutex<VecDeque<M>>,
}

impl<M: Send + 'static> Channel<M> {
    pub const fn new() -> Self {
        Channel {
            buffer: Mutex::new(VecDeque::new()),
        }
    }
    
    pub fn send(&self, msg: M) {
        self.buffer.lock().push_back(msg);
    }
    
    pub fn recv(&self) -> Option<M> {
        self.buffer.lock().pop_front()
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

// Global system IPC kanalı
pub static SYSTEM_CHAN: Channel<SystemMessage> = Channel::new();
