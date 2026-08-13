# SparkOS Kernel — /goal (L11 Kernel Infrastructure)

## Görev
`~/Documents/GitHub/sparkos` repo'sunda **L11 (Kernel Infrastructure)** seviyesini implement et. **SADECE aşağıdaki dosyalara dokun:**
- `src/klog.rs` — YENI dosya (kernel logging sistemi, seviyeli log)
- `src/panic.rs` — YENI dosya (gelişmiş panic handler + çökmüş durum dökümü)
- `src/ktrace.rs` — YENI dosya (izleme/tracing iskeleti)

**KESiN: `src/main.rs`'e DOKUNMA.** `pub mod` eklemelerini HERMES yapacak. Mevcut `src/serial.rs`, `src/vga_buffer.rs`, `src/shell.rs`'e dokunma (sadece oku). Gerekiyorsa `src/main.rs`deki mevcut `#[panic_handler]`'i değiştirmeyeceksin — sadece modüllerini hazırla, HERMES bağlar.

## Proje
- Rust freestanding x86_64 `no_std` kernel, `bootloader 0.9`, `x86_64 0.15`
- Build: `cargo build` (0 error). **NOT: terminal komutlarını `env -u PYTHONPATH <cmd>` ile çalıştır.**
- L0-L10 tamam (smp/acpi paralel). Mevcut `main.rs`'te basit bir `#[panic_handler]` var (serial_println + hlt loop).

## Mevcut L11 Durumu
- `main.rs`'te `#[panic_handler]` — basit: "KERNEL PANIC: {info}" serial'e basar, sonsuz `hlt` loop. Seviyeli loglama, crash dump, backtrace, izleme YOK.
- `src/serial.rs` — QEMU seri port (`serial_println!`).
- `src/shell.rs` — komut satırı (düzenli).

## Yapılacaklar
1. **`src/klog.rs` (yeni) — seviyeli kernel loglama:**
   - `LogLevel` enum: Error, Warn, Info, Debug, Trace (öncelik sıralı).
   - `KLogger`: küresel `static` (Mutex'li), `set_level(max_level)`, `log(level, args)`.
   - Macro'lar: `klog_error!`, `klog_warn!`, `klog_info!`, `klog_debug!`, `klog_trace!` — seviye filtreleme + zaman damgası (timer tick sayacı, mevcut `ticks` varsa).
   - Seri port + isteğe bağlı VGA çift hedef.
   - Kısayol: `KLOG` static, `logger.lock()` üzerinden.

2. **`src/panic.rs` (yeni) — gelişmiş panic + crash dump:**
   - `crash_dump(info: &PanicInfo)`: seviyeli panic mesajı, `RSP`/`RBP` (stack pointer), `CR2` (page fault adresi), `CR3`, mevcut `CPU` bilgisi, son N log kaydını (ring buffer tutuluyorsa) dök.
   - Kayıt defteri döküm iskeleti: `RAX/RBX/RCX/RDX/RSI/RDI/R8-R15` (inline asm ile okuma).
   - `abort()` / `halt_loop()` yardımcıları.
   - `#[panic_handler]` için hazır `panic_impl` fonksiyon şişesi (main.rs'e DOKUNMA, HERMES bağlar).

3. **`src/ktrace.rs` (yeni) — izleme iskeleti:**
   - `TracePoint`, `TraceEvent` (id, seviye, zaman, veri).
   - Ring buffer `TRACE_RING` (sabit N event, örn. 512).
   - `trace!(...)` macro — önemli olayları kaydeder.
   - Basit `print_trace()` — son event'leri döker (shell'den erişilebilir olması için).
   - **Backtrace:** RBP zinciri üzerinden kayıt yürüyücü (`walk_stack`) — mevcut stack frame'lerinden return adresleri toplama (isKelet, gerçek symbol bilinmez ama adresler basılır).

## Teknik
- `no_std` + alloc. Ring buffer sabit dizi.
- Register okuma `core::arch::asm!` ile.
- `serial_println!` mevcut macrosunu kullan (klog.rs içinde çift hedef).
- Konuşkan olma; sağlam ve derlenebilir.

## Teslim
1. Kısa analiz: mevcut panic/log durumu, ne eklendi.
2. `src/klog.rs` + `src/panic.rs` + `src/ktrace.rs`.
3. `cargo build` çıktısı (0 error). Geçici `pub mod` ile doğrula (main.rs'e kalıcı ekleme yok).
4. `git diff --stat`.

Claude: "Eğer görev mantıksızsa YAPMA — raporla. İddialarını önce dosyayı okuyarak doğrula."
