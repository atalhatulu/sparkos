# SPARKOS EVRIM — 5 Asamali Detayli Plan (v2)

> Oncelik: MIGRATION_PLAN.md. Bu dokuman, 5 asamanin uygulanabilir planidir.
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

## Aşama 2 — Syscall Yetki Kontrolu (capability → syscall koprusu)

**Hedef:** capability core'unu mevcut syscall dispatcher'a bagla; her syscall,
cagiran process'in capability cizelgesini kontrol etsin. `security.rs`'teki eski
`Capability(u64)` bitmask'i, `cap.rs`'in handle-tabanli modeline kopru yap.

**Surum:** Mevcut `Capability(u64)` bitmask (uid/gid tabanli) KORUNUR ama artik
kaputalizm icin tek yetki kaynagi DEGILDIR; `cap.rs`-tabanli per-process cap table
devreye girer. Kopru modulu: `src/syscall_cap.rs`.

**Kapsam (syscall basina):**
- `SYS_READ`/`SYS_WRITE`: fd'ye erisim icin capability (READ/WRITE rights)
- `SYS_OPEN`: yeni capability olustur (object → fd mapping)
- `SYS_CLOSE`: capability'yi close (handle free)
- `SYS_EXEC`/`SYS_FORK`: process baslatmak icin EXECUTE right (cap table of parent)
- `SYS_SOCKET`/`SYS_CONNECT`/`SYS_SEND`/`SYS_RECV`: NET/IO right
- `SYS_EXIT`/`SYS_YIELD`: yetki gerektirmez (goren islem)

**Tasarim detayi (kopru):**
- `Process` struct'ina `cap_table: CapTable` alani ekle (fd → CapHandle map).
- `syscall_dispatcher` her syscall'da once caller'ın cap table'ina bakar.
- Eksik capability → `-EACCES` (ecurity) sonucu, syscall islem yapmaz.
- ROOT (uid 0) process icin bir "bootstrap" capability seti init edilir.

**Test (host):**
- syscall_cap kopru testleri: fd acma capability'siz → EACCES, capability'li → OK.
- capability revoke sonrasi syscall → EACCES.
- QEMU boot regression (mevcut deneme uygulamalari hala calisir olmali).

**Worker:** AGY. Dosya(ler): `src/syscall_cap.rs` (copru), `src/cap.rs` (gerekli
public API genisletilmeleri). `main.rs`'e `pub mod syscall_cap;` Hermes ekler.

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
- Message payload yaninda `CapHandle` tasinabilir (cap transfer).

**Capability transfer kurallari (FROZEN: dequeue aninda dogrula):**
- Mesaj kuyruktayken sender revoke ettiyse → cap slot `Revoked` isaretlenir.
- Sessiz drop YASAK.

**Test:**
- Channel capability'si olmayan process send dener → EACCES.
- capability transfer + revoke → `Revoked` state.
- QEMU boot regression (IPC demo hala calisir).

**Worker:** AGY. Dosyalar: `src/ipc.rs`, `src/cap.rs` (transfer API).

## Aşama 5 — Driver/service ayrıştırması → microkernel

**Hedef:** Monolitikten microkernel'a kademeli gecis. Driver'lar user-space'e
tasimaya basla (fault isolation).

**Surum:** Bu en buyuk ve en riskli asama. Dogrulama standardi yukselir. Asama 4
tamamlanmadan Asama 5'e girilmez (IPC capability tabanli olmali).

**Kapsam (ilk adim — kapsami dar tut):**
- En basit driver'i (rtl8139? yok; ps2 mouse? serial) user-space servise tasi.
- capability tabanli IPC ile kernel <-> driver arasinda protokol.
- Driver crash → kernel etkilenmeden driver yeni baslatilabilir (fault recovery).

**Test:**
- Driver user-space'te calisir, kernel'e syscall ile erisir.
- Driver'i force-crash et → kernel cokmez, driver restart.
- QEMU boot regression.

**Worker:** AGY (paralel kanal icin fcc ile veya yalniz). En kapsamli.
Bu asama buyuk; ayrıca alt adimlara bolunebilir.

## Uygulama sirasi (bu plan icin)

```
Asama 2 plan + fcc eleştiri → AGY kod → dogrula → commit
         ↓
Asama 3 plan + fcc eleştiri → AGY kod → dogrula → commit
         ↓
Asama 4 plan + fcc eleştiri → AGY kod → dogrula → commit
         ↓
Asama 5 plan + fcc eleştiri → AGY kod → dogrula → commit
```

## Riskler ve acik sorular (fcc inceleyecek)

1. Asama 2'de mevcut bitmask `Capability(u64)` ile yeni `cap.rs` handle modeli
   yan yana mi yasamali, yoksa tamamen mi degistirilmeli? (Uyumluluk vs tamlik.)
2. Asama 3'te per-process CR3 gecisinin boot'a etkisi — mevcut single address
   space varsayimi kirilinca hangi kod patlar?
3. Asama 4'te `BlockingChannel`'a capability eklemek, mevcut IPC demo'sunu
   bozar mi? Geriye donuk uyumluluk stratejisi ne olmali?
4. Asama 5'te hangi driver en dogru ilk aday? (rtl8139 QEMU'da yok.)
5. capability table per-process mi global mi olmali? (Asama 2'de `Process`'e
   alan eklemek mi, yoksa global map mi?)
