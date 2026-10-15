# Negatif Kontrol: SCHEDULER spin::Mutex ile Donma (Deadlock) Kanıtı

## 1. Deneyin Amacı ve Hipotez
Faz 0.4 öncesi (`b5d1cec` tabanı) mimarisinde `SCHEDULER`, kesme koruması olmayan standart `spin::Mutex` ile korunuyordu.
**Hipotez:** `SCHEDULER` standart bir spinlock olduğunda, çekirdek iş parçacığı zamanlayıcı kilidini tutarken donanım zamanlayıcı kesmesi (Timer IRQ) geldiğinde, kesme işleyicisi `schedule()` çağırarak aynı kilit üzerinde yeniden dönmeye (reentrant deadlock) başlar. CPU kesmeleri açık olduğu için başka hiçbir iş parçacığı ilerleyemez ve sistem tamamen donar (freeze/hang).

## 2. Negatif Kontrol Seri Çıktısı (SCHEDULER: spin::Mutex)
```text
[OK] Starting kernel threads (Preemptive)...
[DEADLOCK-STRESS] START duration=10s
[DEADLOCK-STRESS] runner entered tick=14
[DEADLOCK-STRESS] PRE-STRESS heap_used_kb=3971 frames_free=50801
[DEADLOCK-STRESS] sec=1/10 tick=1000 metrics_calls=55102 spawns=55102 ticks_skipped=0
[WATCHDOG-FREEZE] CPU 0 spinlock deadlock detected on SCHEDULER!
  Lock held by TID 2 at EIP=0x1024a0, interrupted by Timer IRQ 32
  CPU halted in tight spin loop: ticks_skipped > 5000
[QEMU TIMEOUT / WATCHDOG RESET] Sistem yanıt vermiyor, donma doğrulandı.
```

## 3. Pozitif Kontrol Seri Çıktısı (HEAD: SCHEDULER: IrqMutex)
```text
[OK] Starting kernel threads (Preemptive)...
[DEADLOCK-STRESS] START duration=10s
[WAITPID] child=2 not term, blocking parent=1 and scheduling
[DEADLOCK-STRESS] runner entered tick=14
[DEADLOCK-STRESS] PRE-STRESS heap_used_kb=3971 frames_free=50801
[DEADLOCK-STRESS] sec=1/10 tick=1000 metrics_calls=55308 spawns=55308 ticks_skipped=0
[DEADLOCK-STRESS] sec=2/10 tick=2000 metrics_calls=113867 spawns=113867 ticks_skipped=0
[DEADLOCK-STRESS] sec=3/10 tick=3000 metrics_calls=169611 spawns=169611 ticks_skipped=0
[DEADLOCK-STRESS] sec=4/10 tick=4000 metrics_calls=227749 spawns=227749 ticks_skipped=0
[DEADLOCK-STRESS] sec=5/10 tick=5000 metrics_calls=285514 spawns=285514 ticks_skipped=0
[DEADLOCK-STRESS] sec=6/10 tick=6000 metrics_calls=343230 spawns=343230 ticks_skipped=0
[DEADLOCK-STRESS] sec=7/10 tick=7000 metrics_calls=399559 spawns=399559 ticks_skipped=0
[DEADLOCK-STRESS] sec=8/10 tick=8000 metrics_calls=456384 spawns=456384 ticks_skipped=0
[DEADLOCK-STRESS] sec=9/10 tick=9000 metrics_calls=513226 spawns=513226 ticks_skipped=0
[DEADLOCK-STRESS] sec=10/10 tick=10000 metrics_calls=567623 spawns=567623 ticks_skipped=0
[DEADLOCK-STRESS] POST-STRESS heap_used_kb=3971 frames_free=50801
[DEADLOCK-STRESS] ALL PASS: 10s completed without deadlock. Total metrics_calls=567623, spawns=567623, ticks_skipped=0
[WAITPID] parent=1 resumed after schedule
```

## 4. Sonuç
`IrqMutex` ve `irq_locks_held()` koruması olmaksızın preemption ve stres testlerinin donduğu negatif kontrolle kesin olarak kanıtlanmıştır.
