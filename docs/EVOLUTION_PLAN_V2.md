# SPARKOS EVRIM — 5 Asamali Detayli Plan (v3)

> Oncelik: MIGRATION_PLAN.md. Bu dokuman, 5 asamanin uygulanabilir planidir.
> **v3 (fcc-claude eleştirisi sonrasi revizyon):** Asama 2↔3 sirasi degisti.
> fcc: "somut exploit edilebilir delik (SYS_EXEC) Asama 3'te; capability gating,
> korunacak bir sinir yokken dusuk deger." Yeni sure: once `cap::init` boot wiring +
> caller kimliği + SYS_EXEC exploit kapat → Asama 2 (capability gating) → 4 → 5.
> Asama 3'un izolasyon kismi Asama 2 ile paralel gider.
> Is bolumu: Kodlar AGY'ye yazdirilir. Eleştiri/fikir: fcc-claude (read-only).
> Dogrulama: Hermes (build + test + QEMU). Hermes kod YAZMAZ (orkestrator),
> sadece iskelet/glue + dogrulama + commit yapar.

## Genel kurallar (tum asamalar)

- **`src/main.rs`'e dokunma** (Hermes harici). `pub mod` eklemelerini HERMES yapar.
- **FROZEN semantigi kodlar**, PROVISIONAL/DEFERRED'i ASLA kodlama (mapping/TLB detayi,
  DMA/IOMMU, IPC cancellation, lend expiry, NUMA, formal verification).
- **Celiski cikarsa:** worker durur, rapor eder. Hermes mimari karari TEK BASINA
  degistirmez; fcc'den eleştiri alir, kullanici onayiyla ilerler.
- **Worker GUI**: `env -u PYTHONPATH` (izole venv, PYTHONPATH kirletmesin).
- **Mevcut davranis bozma**: her asamada QEMU boot regression.
- **Dogrulama standardi**: `cargo build` 0 error + host test + QEMU boot.

## Aşama 1 — Capability Core (izole) ✅ TAMAMLANDI (85c14a1)

`src/cap.rs` (577 satır, AGY uretti). FROZEN Step1-3 semantigi:
grant/transfer/lend/revoke/deref, lineage+epoch+refcount, tek spinlock.
9/9 invariant testi gecti. Build 0 error. QEMU boot ayni.

## Aşama 2.0 — Ön koşullar (fcc eleştirisiyle eklendi) 🔴

**Neden:** fcc: "capability gating, korunacak bir sınır yokken düşük değer.
Somut exploit edilebilir delik (SYS_EXEC) önce kapatılmalı." Bir köprü kurmadan
önce köprünün iki ucu hazır olmalı: (a) capability core aktif (boot'ta init),
(b) her syscall'ın hangi process'ten geldiği biliniyor (caller kimliği).

**Kapsam:**
1. **`cap::init()` boot'a bağla** — `main.rs` kernel başlangıcında, capability
   core'u init et. Boot'ta ROOT process'e bootstrap capability seti ver. Asama 1'de
   modül sadece `pub mod cap;` ile import edildi, hiçbir syscall onu kullanmıyor —
   bu bağlantıyı şimdi kur.
2. **Caller kimliği tanımla** — her syscall'ın hangi process'ten geldiğini
   `syscall_dispatcher`'a ver (current_pid + process cap table). capability
   per-process olacak (fcc: "process exit'te sahibin tüm cap'lerini temizleme;
   caller'ın kendi tablosunda olmayan handle'ı reddetme — yoksa A, B'nin handle
   değerini bilirse kullanabilir; generation zorlaştırır, engellemez"). →
   `Process.cap_table: CapTable` (fd → CapHandle map).
3. **SYS_EXEC exploit kapat** — `syscall.rs:78`'teki `from_raw_parts`'i
   `validate_user_ptr` ile koru (canonical + user half + her page user-mapped).
   Bu somut delik: user, kernel adresi verip kernel'ın o adresi ELF olarak
   okumasını sağlayabilir.

**Test:** SYS_EXEC'e kernel adresi verme → -EFAULT. `cap::init` boot logu çıkar.
QEMU boot regression.

**Worker:** AGY. Dosyalar: `src/cap.rs` (init + Process entegrasyon helper),
`src/syscall.rs` (SYS_EXEC fix), `src/task/process.rs` (cap_table alanı).
`main.rs`'e `cap::init()` çağrısını Hermes ekler.

## Aşama 2 — Syscall Yetki Kontrolu (capability → syscall koprusu)

**Hedef:** capability core'unu syscall dispatcher'a bagla; her syscall, cagiran
process'in capability cizelgesini kontrol etsin. `security.rs`'teki eski
`Capability(u64)` bitmask'i tamamen `cap.rs`'in `Rights`'ına cevrildi (fcc: "tek
right sözlüğü — eşleme katmanını minimumda tut").
```
security.rs eski Capability(u64)  →  cap.rs Rights(u32)
```
Yani bitmask kanunu kaldırılır; yetki kaynağı tek: `cap.rs` handle modeli.

**Kopru:** `src/syscall_cap.rs` — `Process.cap_table` (fd → CapHandle) üzerinden
doğrulama yapar.

**Kapsam (syscall basina):**
- `SYS_READ`/`SYS_WRITE`: fd'ye erişim için capability (READ/WRITE rights)
- `SYS_OPEN`: yeni capability oluştur (object → fd mapping)
- `SYS_CLOSE`: capability'yi close (handle free)
- `SYS_EXEC`/`SYS_FORK`: ELF yükleme + process başlatma için EXECUTE/SYS_ADMIN right
- `SYS_SOCKET`/`SYS_CONNECT`/`SYS_SEND`/`SYS_RECV`: IO/NET right
- `SYS_EXIT`/`SYS_YIELD`: yetki gerektirmez (gören işlem)

**Tasarim detayi (kopru):**
- `syscall_dispatcher` her syscall'da once caller kimliğini alır (current_pid),
  sonra `Process.cap_table`'ına bakar.
- Eksik capability → `-EACCES`, syscall islemi yapmaz.
- `CapObject` artik **gerçek kaynağa bağlıdır** (fcc: "köprünün kalbi — object →
  fd mapping): her açılan fd bir `CapObject(fd)` olur, `Process.cap_table[fd]`
  o objenin handle'ın taşır.

**Test (host):**
- syscall_cap kopru testleri: fd açma capability'siz → EACCES, capability'li → OK.
- capability revoke sonrasi syscall → EACCES.
- Caller'ın kendi tablosunda olmayan handle'ı syscall'da kullanma → red.
- QEMU boot regression (mevcut deneme uygulamaları hala calisir olmali).

**Worker:** AGY. Dosya(ler): `src/syscall_cap.rs` (kopru), `src/cap.rs` (gerekli
API genisletmeleri), `src/syscall.rs` (dispatcher gating). `main.rs`'e `pub mod
syscall_cap;` Hermes ekler.

## Aşama 3 — Memory/pointer güvenliği (per-process CR3 + user ptr zorlaması)

**Hedef:** `sec_mem.rs`'teki mevcut "user ptr doğrulaması deliğini" kapat; capability
tabanlı memory izolasyonu kur. Su an `validate_user_ptr` bir yorumla "doğrulanmadan
kullanılıyor" diye itiraf ediyor.

**Surum:** Mevcut `sec_mem::validate_user_ptr` KORUNUR ve TUM syscall'larda ZORUNLU
kilinir (syscall_storage, net_socket). `cap.rs`'in MAP right'ı, bir kaputalizmli
memory mapping'in iznidir. Per-process CR3 "ready for later" durumunda — Asama 3'te
devreye alinir (gerekiyorsa).

**Kapsam:**
- `sec_mem.rs`: `validate_user_ptr`'i eksiksiz uygula (canonical, user half, her
  page user-mapped). Eksik ise tamamla.
- `syscall_storage.rs` + `net_socket.rs`: tum `from_raw_parts` kullanimlarina
  `validate_user_ptr` oncesi check ekle.
- `process.rs`: susturulan/per-process CR3 destegi (single address space varsayimi
  kirilir → her process ayri page table).

**Test:**
- Kernel adresine write denemesi → -EFAULT (kernel ve user adresleri).
- BOZUK pointer (user half disinda) → -EFAULT.
- Per-process CR3: process A'nin memory'sine process B erismeyi dener → fault.
- QEMU boot regression.

**Worker:** AGY. Dosyalar: `src/sec_mem.rs`, `src/syscall_storage.rs`,
`src/net_socket.rs`, `src/task/process.rs` (CR3). Disjoint file set — fcc paralel
analiz edebilir ama authoring AGY'de.

## Aşama 4 — IPC'yi capability modeline taşı

**Hedef:** Mevcut `ipc.rs`'teki typed `BlockingChannel`'i, capability tabanli hale
getir; capability`de mesaj icinde tasinabilir olsun.

**Surum:** Mevcut `BlockingChannel` API KORUNUR (uygulamalar bozulmasin) ama
icerisinde capability check + capability transfer destegi eklenir.

**Kapsam:**
- `ipc.rs`: channel capability'si (bir process'in channel'a send/recv hakk vardir).
- Message payload yaninda `CapHandle` tasinabilir.

**Capability transfer kurallari (fcc dagitimi — FROZEN uyumlu):**
- **Kalici devir** → `transfer` (callback edilemez, sahiplik aktarimi).
- **Revoke-edilebilir gecici yetki** → `lend` (FROZEN "dequeue'da dogrula" kurali
  LEND uzerinden tanimlanir): mesaj kuyruktayken sender lend'i iptal ederse → cap
  slot `Revoked` isaretlenir, payload yine teslim edilir.
- Sessiz drop YASAK.

**Test:**
- Channel capability'si olmayan process send dener → EACCES.
- **Lend:** lend sonrasi sender iptal → mesaj payload teslim, cap slot `Revoked`.
- **Transfer:** transfer devir → revoke-edilemez, kalici.
- QEMU boot regression (IPC demo hala calisir).

**Worker:** AGY. Dosyalar: `src/ipc.rs`, `src/cap.rs` (transfer API).

## Aşama 5 — Driver/service ayrıştırması → microkernel

**Hedef:** Monolitikten microkernel'a kademeli gecis. Driver'lar user-space'e
tasimaya basla (fault isolation).

**Surum:** Bu en buyuk ve en riskli asama. Dogrulama standardi yukselir. Asama 4
tamamlanmadan Asama 5'e girilmez (IPC capability tabanli olmali).

**Kapsam (fcc: donanimsız servis önce, ilk donanım adayı serial):**
- **Donanımsız servis** ile başla: örn. `keyboard` veya `fb_query` gibi bir user-space
  servis, capability tabanlı IPC ile kernel'e erişir. Bu, servis mimarisini driver
  karmaşıklığı olmadan doğrular.
- İlk donanım adayı **serial** (rtl8139 QEMU'da yok; serial en basit, IOMMU gerektirmez).
- Driver/servis crash → kernel etkilenmeden restart (fault recovery).

**Test:**
- Servis user-space'te calisir, kernel'e syscall/IPC ile erisir.
- Servisi force-crash et → kernel cokmez, servis restart.
- QEMU boot regression.

**Worker:** AGY (paralel kanal icin fcc ile veya yalniz). En kapsamli.
Bu asama buyuk; ayrıca alt adimlara bolunebilir.

## Uygulama sirasi (fcc revizyonu — bu plan icin)

```
Asama 2.0 (ön koşul) + fcc eleştiri → AGY kod → dogrula → commit
         ↓
Asama 2 (capability gating) + fcc eleştiri → AGY kod → dogrula → commit
         ↓
Asama 4 (IPC capability) → Asama 5 (driver/servis)
         ↓
(Asama 3 izolasyon kismi — per-process CR3 — Asama 2 ile paralel gider; dogrusal
 yapilacaksa Asama 4'ten sonra.)
```

## Riskler ve acik sorular (fcc eleştirisinden)

fcc-claude eleştirisi (2026-son, /tmp/fcc_plan_review_out.txt) işlendi:
1. ✅ **Öncelik hatası düzeltildi** — Asama 2.0 eklendi (cap::init boot wiring +
   caller kimliği + SYS_EXEC exploit kapat). "capability gating korunacak sınır
   yokken düşük değer."
2. ✅ **CapObject gerçek kaynağa bağlandı** — object→fd mapping köprünün kalbi.
3. ✅ **Tek right sözlüğü** — eski `Capability(u64)` tamamen `cap.rs Rights`'ına
   çevrildi, eşleme katmanı min.
4. ✅ **Per-process cap_table** — Process'e `cap_table` (fd→CapHandle); caller'ın
   kendi tablosunda olmayan handle reddi; process exit'te temizlik.
5. ✅ **Asama 4: transfer→lend** ayrımı — kalıcı devir `transfer`, geçici yetki
   `lend`; FROZEN "dequeue'da doğrula" lend üzerinden.
6. ✅ **Asama 5: donanımsız servis önce, ilk donanım adayı serial.**
7. ⚠️ **SMP tek global spinlock** — DEFERRED kapsamında; deref lock'u erişim
   boyunca tutmuyor (CapAccess salınıyor) — tasarım doğru, korunacak.
8. ⚠️ **QEMU boot regression güvenlik için yetersiz** — negatif testler (cap yok →
   EACCES) zorunlu; boot hata vermese de yetki boş olabilir.

**Açık karar:** Eski `security.rs Capability(u64)` tamamen kaldırılacak (fcc önerisi)
— ama uid/gid temeli (Uid/Gid) KORUNUR; sadece bitmask capability katmanı
`cap.rs Rights`'ına devredilir.
