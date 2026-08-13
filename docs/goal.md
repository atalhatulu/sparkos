# SparkOS Kernel — /goal

## Görev
SparkOS'u "gerçek güçlü kernel" seviyesine taşımak için **L5 (IPC & Synchronization)** katmanını sağlam, üretim kalitesinde implement et. Mevcut çalışan sistemi bozma; mevcut yarım işleri tamamla/temizle. Build doğrulaması zorunlu.

## Proje Konumu
- Repo: `~/Documents/GitHub/sparkos` (git: github.com/atalhatulu/sparkos)
- Rust freestanding x86_64 `no_std` kernel, `bootloader 0.9`, `x86_64 0.15`, `spin 0.9`, linked_list_allocator, crossbeam-queue
- Nightly toolchain (`rust-toolchain.toml`), entry `kernel_main` (main.rs)
- Build: `cargo build` → `NOT_working` warning'leri kalabilir, **0 error olmalı**

## Mevcut Durum (dün L4 tamamlandı)
- ✅ L0-4: boot, output, GDT/IDT/interrupts/timer, paging/heap/allocator, async Task+SimpleExecutor, Ring3 (user.rs)+ELF loader, syscall dispatcher (syscall.rs)
- ⚠️ **`src/scheduler.rs` (66 satır)** eski, fonksiyon-pointer tabanlı (pre-heap) ve ARTIK KULLANILMIYOR — `main.rs` sadece `task::simple_executor::SimpleExecutor` kullanıyor. Bu eski dosya kafa karışıklığı yaratıyor.
- ⚠️ **`src/ipc.rs` (60 satır)** çok zayıf: sadece `Channel<M>` (spin::Mutex + VecDeque) tip-güvenli kanal, `send`'de bloklama yok, `recv` polling (kondisyon değişkeni/signal yok). `SYSTEM_CHAN` global var. **KRİTİK: `ipc` modülü `main.rs`'te `pub mod ipc;` olarak TANIMLI DEĞİL ve hiçbir yerde kullanılmıyor — yani `ipc.rs` şu an ölü kod, derlemeye dahil edilmiyor.** L5'te bunu hayata döndürüp `main.rs`'e bağlayacaksın. **Mutex/spinlock/semaphore/condvar primitive'leri YOK**.

## Hedef: L5 — IPC & Synchronization katmanı

Yeni bir **`src/sync.rs`** modülü kur ve şu primitive'leri, `no_std` + alloc, interrupt-safe olacak şekilde implement et:

1. **`Spinlock<T>`** — raw CPU spinlock (atomic swap/AcqRel), kritik bölge guard'ı (RAII), interrupt'ları güvenle kapatıp açan versiyon (IRQ-safe variant). Debug assertion ile karşılıklı kilit algılama (opsiyonel).
2. **`Mutex<T>`** — `spin::Mutex` üzerine inşa edilmiş veya kendi blocking kilidin; kullanımı `std::sync::Mutex` gibi (lock() → Guard, try_lock()).
3. **`Semaphore`** — sayısal semafor (wait/try_wait/signal), atomic sayaç + wakelist (askıya alınmış awaiters).
4. **`Condvar`** — koşul değişkeni, `spin::MutexGuard` ile uyumlu wait/notify_one/notify_all.
5. **`BlockingChannel<M>`** — mevcut `Channel<M>`'i iyileştir: `send` bloklayabilsin (doluysa bekle) veya `try_send`, `recv` boşsa bekle (condvar ile wake-up), birden çok producer/consumer desteklesin.

Sonra **`src/ipc.rs`'i güçlendir**:
- Mevcut `Channel<M>`'i `BlockingChannel` olarak tip-güvenli koru (veya Condvar tabanlı dönüştür).
- `SYSTEM_CHAN`'i koru, üzerine `dbus` benzeri ama basit route edilebilir mesaj dağıtımı katmanı ekle (opsiyonel).
- En az 1 example göster: `kernel_main`'de senkron test (örn. 2 producer + 1 consumer channel üzerinden sayı akışı, bu sırada scheduler'ın kitlenmediğini göster).

**Eski `scheduler.rs`:**
- Ya tamamen sil (kullanılmıyor) **veya** `simple_executor`'a işaret eden bir "deprecated" notuna çevir. Kullanılmıyorsa silmek daha temiz — main.rs'te import yok, doğrula ve kaldır. Eğer bir yerde kullanılıyorsa (grep et) bırak ama modernize et.

## Teknik Kısıtlar
- **Interrupt-safe**: Kernel timer (1000 Hz) ve klavye/mouse IRQ'ları kritik bölge sırasında oluşabilir → spinlock/kısa kritik bölgelerde interrupt'ları disable et, uzun (mutex/io) bölgelerde değil.
- **Deadlock yok**: Kilit sıralaması tutarlı; asla kilit tutarken kilit bekleme (aynı thread'de).
- `no_std`: `core::sync::atomic` kullan, `std` yok.
- `alloc` serbest: `alloc::collections::VecDeque`, `alloc::sync::Arc` kullanılabilir.
- Performans: kendi spinlock'in `xchg`/atomic `swap` ile yaz, busy-wait yerine `pause`/`hlt` ipucu kullan.

## Teslim
1. Kısa analiz: L5 neden kritik, ne eklendi.
2. `src/sync.rs` (yeni) + `src/main.rs` `pub mod sync;` + kernel_main'de senkron test çağrısı.
3. `src/ipc.rs` güçlendirilmiş hali.
4. `scheduler.rs` ne oldu (silindi mi / korundu mu).
5. `cargo build` çıktısı (0 error; warning'leri `cargo fix --allow-dirty` ile temizlemeyi dene).
6. `git diff --stat`.

AGY: "Eğer bir görev mantıksızsa, zaten çözülmüşse veya gerekli değilse YAPMA — neden gerekmediğini raporla."
Kodu yaz, build et, doğrula, bitir.
