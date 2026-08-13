# CONCURRENCY_MODEL.md — Spark OS Concurrency & Memory Ordering

> DURUM: **FROZEN** (STEP 3). Capability core'unun race-free oldugunu garanti eden
> concurrency modeli ve memory ordering kurallari. Mapping/TLB (STEP 4) ve DMA (STEP 5)
> bu modelin uzerine insa edilecek.

## 1. Amac

`epoch + claim + revoke/deref` modelinin gercekten race-condition-free oldugunu
adres-by-adres dogrulamak. Bu cozulmeden Mapping/DMA'ya gecmek erken olurdu.

## 2. Yaris ciftleri listesi

| A op | B op | Ortak state |
|---|---|---|
| `deref(h)` claim (refcount++) | `revoke(h)` (epoch++) | node.epoch |
| `deref(h)` claim | drain (refcount-- → FREE) | object.refcount |
| `close(h)` (gen++) | `deref(h)` (gen check) | slot.generation |
| `close(h)` slot free | `grant/transfer` (slot alloc) | slot bitmap |
| `grant(parent)` child ekle | `revoke(parent)` epoch++ | parent node + child ptr |
| `transfer` node re-root | `revoke(eski lineage)` | node.parent |
| `deref(h)` | `deref(h')` (iki process) | object.refcount |

Her satir, asagidaki kurallar ile cozulur.

## 3. Temel kural: deref = atomik kritik dilim

**`deref` claim mutlaka tek kritik bolgede:** epoch check + generation check +
refcount++ bir arada, atomik olarak.

```
deref(h):
  lock(CAP_TABLE):
    hdr = header_arena[h.slot]           # segment/cap header
    if h.slot.generation != h.generation: return CAP_INVALID
    if !chain_valid(h.node):             # lazy epoch walk
        return CAP_REVOKED
    if !rights_ok(op):                   # op-specific rights
        return CAP_NO_RIGHTS
    obj.refcount++                       # claim
```

Eger bu uc adim ayrilsaydi:
- deref epoch check gecer → context switch → revoke epoch++ → deref refcount++ → **DELIK**.
- revoked capability claim yapmis olur (mevcut linearizasyon ihlali).

**Sonuc:** epoch + gen check + refcount++ ayni kilit blogunda. (FIX-1)

## 4. Object header kalıcı arena + generation (TOCTOU-free)

En tehlikeli yaris: `refcount==0` sonrası object FREE edilip pointer dangling olsa,
deref free'lenmis object'e claim yapabilir.

**Cozum:** object `header`'lari kalici bir arena'da yasar; FREE header'i SİLMEZ, sadece
yeniden baslatir (mark + gen++). Fiziksel kaynak (RAM pardesi, device) serbest birakilir,
ama header struct'i arena'da kalir.

```
FREE:
  mark header oku
  obj.refcount == 0
  obj.generation++          # stale pointer artik gecersiz
  header'i "free" basin ca()na isaretle
```

Bu, deref'in pointer'inin her zaman gecerli bir header'a bakmasini garantiler; stale
olup olmadigi header.generation ile CAS dogrulanir. (FIX-2)

## 5. Linearizasyon noktalari

| Op | Linearizasyon noktasi |
|---|---|
| `revoke(h)` | `node.epoch++` yayini |
| `deref(h)` | claim commit (refcount++ + check ayni blogunda) |
| `close(h)` | slot free + gen++ |
| `grant` | child link + parent epoch ayni blogu |

revoke ile deref yarisi siralidir:
- deref commit < revoke epoch++ → claim gecer, revoke sonra (cap revoked olur ama o an claim yapmisti)
- revoke < deref commit → deref `CAP_REVOKED`
- Araya ucuncu hal yok (FIX-1 sayesinde).

## 6. grant/transfer vs revoke(parent)

grant, parent'a child pointer yazarken parent revoke edilebilir.

**Kural:** parent node state (epoch + child ptr) ayni lock altinda. grant, parent epoch'u
okur; DEAD ise `CAP_REVOKED` dondur, child ekleme. grant commit < revoke ise child eklenir
ve revoke onu da oldurur (lazy walk tutarli).

Eger ayriysa: grant child ekler → revoke epoch++ (child yeni eklendi, walk gormeyebilir).
**Bu yuzden parent epoch read + child link write ayni kritik bolge. (FIX-3)**

## 7. Memory ordering ozet

| Veri yapisi | Erisim | Ordering |
|---|---|---|
| Capability/lineage/object header | tek işlemci (scheduler+IPC) cogu zaman | Spinlock (iceri relaxed), slot icin AcqRel |
| In-flight IPC queue / ring | farkli process, gercek SMP | SeqCst + fence |

### Kod kurallari (Rust/Bare-metal)

1. Capability mutasyonlari (grant/transfer/revoke/close): **tek spinlock** altinda —
   her islem atomik bir dilim.
2. `deref` claim: epoch + gen check + refcount++ **ayni dilimde**.
3. Revocation epoch: `Ordering::AcqRel` ile yayinlanir/okunur. REVOKED durumu tum
   gozlemciler tarafindan gorulur.
4. Crossover-process IPC: `SeqCst` ring. Reordering mesaj reorder yaratmamali.
5. Slot allocator (STEP 6 mekanizma, ama ordering simdi): `freeGeneration` AcqRel ile
   artirilir, yeni allocator stale slot'a dusmez.
6. Asla "once epoch check, sonra refcount++" seklinde ayri atomik islem YOK.

## 8. Neden bu (adieralin)

- **Kilit-corridor mi, lock-free mi?** Capability core icin lock-free gereksiz: tek
  islemci weight'li, erisim nadir, IPC/driver icinde. Lock basit ve dogrulanabilir.
  Lock-free (SMP cap parcali) implementasyon cok daha zor, v0.x gereksiz risk.
  Crossover-process IPC kuyruklari gerekirse lock-free (SeqCst), ama kapsami sinirli.
- **Object arena kaliciligi:** pointer erisim guvenligini YAPI bazinda cozer (compiler
  UB/use-after-free yok). Bellek allocator'dan ayri; cap header'lari asla move edilmez.
- **Tek buyuk kilit mi, per-object kilit mi?** v0.x tek cap table lock yeterli
  (cozumleme basta). Per-object lock, paralel deref'lerde kazdir ama deadlock riski
  (nested object lock) getirir. v0.x TEK LOCK. Ileride olcersek bol.

## 9. Contract (Freeze)

1. Capability mutasyonlari atomik dilim (tek lock). Ici relaxed.
2. `deref` claim = gen + epoch + refcount++ tek dilim. (FIX-1)
3. Object header kalici arena; FREE header'i silmez, gen++ yapar. (FIX-2)
4. grant child link + parent epoch ayni dilim. (FIX-3)
5. revoke linearizasyon = epoch++ yayini; deref = claim commit.
6. Revocation epoch `AcqRel`; crossover-process ring `SeqCst`.
7. Slot allocator free `AcqRel` (STEP 6 mec., kurall simdi).
8. v0.x tek cap table lock; per-object lock ileride.

## 10. STEP 4/5'e etkisi

- **Mapping (STEP 4):** deref claim'i mapping'i de kapsar — ayni dilimde mapping refcount
  claim. TLB shootdown ordering'i bu modelin uzerine eklenir.
- **DMA (STEP 5):** DMA pin claim + ioMMU mapping ayni dilimde. IOMMU revoke farkli
  islemci tarafinda calisiyor olabilir → SeqCst fence gerekecek, STEP 5'te.

Bu iki bozulma STEP 4/5'te ilgili dokumanlara islenecek. Bu dokuman cap table ve
lifetime core'unu kapatir.
