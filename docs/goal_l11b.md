# SparkOS /goal — L11 Kernel Infra

Repo: ~/Documents/GitHub/sparkos (no_std x86_64 Rust kernel).

SADECE yeni dosyalar olustur, VAR OLANA DOKUNMA:
- src/klog.rs  (seviyeli log sistemi)
- src/panic.rs (crash dump + panic handler sisesi)
- src/ktrace.rs (trace ring buffer + backtrace iskeleti)
main.rs'e dokunma, HERMES ekler.

KISITLAMA: Terminal komutlarini `env -u PYTHONPATH <cmd>` ile calistir. Build 0 error olmali (mevcut 7 warning kabul).

Mevcut durum: main.rs'te basit #[panic_handler] var (serial_println + hlt loop). serial.rs QEMU seri port, vga_buffer.rs VGA. Kullanilabilir.

YAPILACAKLAR (3 dosya):

1. src/klog.rs — kernel loglama:
   - LogLevel enum: Error>Warn>Info>Debug>Trace
   - KLogger static (Mutex), set_level(), log(level, msg)
   - Macro: klog_info!("..."), klog_error!, klog_warn!, klog_debug!, klog_trace!
   - seviye filtreleme, hedef serial + VGA (cift). cikti "TIME [LEVEL] msg" formatinda (TIME=sabit 0 tick kullan).

2. src/panic.rs — panic + crash dump:
   - crash_dump(info): panic mesaji + CR2 (adres) + RSP/RBP okuma (asm) + mevcut 8 register döku
   - halt_loop() / abort() yardimcilari

3. src/ktrace.rs — izleme:
   - TraceEvent {id, level, tick, data}
   - TRACE_RING: sabit 512 event ring buffer (Mutex)
   - trace!(...) macro, print_trace() son N olayi dizer
   - walk_stack(): RBP zinciriyle return adresleri topla (backtrace, adres basta yeterli)

Teknik: no_std + alloc. asm! ile register okuma. Ring buffer sabit dizi.

TESLIM:
1. Kisa analiz (2 satir)
2. Build ciktisi (0 error)
3. git diff --stat (3 yeni dosya)

once dosyalari OKU dogrula, sonra yaz."
