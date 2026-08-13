# RESOURCE_LIFETIME.md — Spark OS Resource Lifetime

> DURUM: **KISMEN FROZEN.** Capability vs Resource ayrimi ve claim/refcount modeli
> kesinlesti. Mapping/TLB (STEP 4) ve DMA/IOMMU (STEP 5) konulari ACIK — ilgili
> STEP'lerde kapatilip bu dosya guncellenecek.

## 1. Temel Ayrım (FROZEN)

```
Capability lifetime ≠ Resource lifetime
```

- **Capability** — yetki. `revoke` ile olur. Resource'u free etmez.
- **Resource** — kernel object. `refcount == 0` oldugunda free olur.
- Bir capability revoke edilse bile, resource — in-flight mapping, IPC, DMA referanslar
  yuzunden — yasamaya devam edebilir.

```
revoke(cap)
   ↓
capability dead
   ↓
resource still alive (refcount > 0)
   ↓
DMA / mapping / in-flight IPC biter
   ↓
refcount == 0
   ↓
resource free
```

## 2. Claim / Refcount Modeli (FROZEN)

Bir resource'un yasam suresi, aktif referans sayisina baglidir:

| Referans turu | Refcount etkisi |
|---|---|
| Capability (authority claim) | her slot +1 |
| Memory mapping (MAP) | +1 |
| In-flight IPC (queue'daki mesaj) | +1 |
| DMA pin (ioMMU mapping) | +1 |

- Resource yalnizca **tum referanslar dustugunde** (refcount = 0) free.
- Iptal edilen referansların release'i icin caller garantisi gerekir.

### Object state machine

`ALIVE → (son authority claim dustu) → DYING → (refcount=0) → FREE`

- **DYING**: yeni claim kabul edilmez; mevcut in-flight claim'ler yasamaya devam eder.
- **FREE**: object gen++ (slot reuse korunur), bellek geri donebilir.

| Event | Handle | Node | Object |
|---|---|---|---|
| `close(h)` | slot free, gen++ | ref-- | — |
| `revoke(h)` | REVOKED | DEAD (epoch++) | authority claim duser |
| `deref(h)` | gen check | lazy chain walk | claim (refcount++) |
| `release(claim)` | — | — | refcount--; 0 → FREE |

## 3. Mapping + TLB Revoke (ACIK — STEP 4)

Henuz cozulmedi. Gereken state machine:

```
Capability revoke
   ↓
Mapping
   ↓
Page Table
   ↓
TLB
```

Acil sorular:
- Page table update siralama
- TLB invalidation
- TLB shootdown (SMP)
- Concurrent access / CPU race

**On tohum (karar degil):** mapping bir claim'dir; revoke shootdown "yapmaz",
epoch refill/fault'ta yeniden kontrol edilir. STEP 4'te kesinlesecek.

## 4. DMA + IOMMU (ACIK — STEP 5)

En zor acik konu. DMA devam ederken `revoke(buffer)` olursa:

Adaylar:
- **Async revoke** — DMA bitene kadar resource tutulur; DMA complete → FREE.
- **Forced cancellation** — DMA durdurulur.
- **IOMMU revoke** — device mapping kesilir, DMA hata alir.
- **Device reset** — guvenli iptal mumkun degilse device resetlenir.

Hangi durumda hangisi uygulanacagi henuz kesin degil (STEP 5).

## 5. Process Death (ACIK)

Delegator oldugunde capability'lere ne olur? (STEP 8 oncesi not)
- varsayilan: tum handle'lar `close` (revoke degil) → delegation delegator'un
  olumunden sag cikar (crash cascade engelle). "Revoke-on-death" ileride policy
  katmani olarak eklenebilir, default degil.

## 6. Quotas / Emergency Reserve (ERKEN — RED)

Kimi'nin "1ms cooldown", "generation + 1 epoch" ve "OOM emergency reserve" gibi
onerileri bu asamada KESIN mimari karar DEGIL. STEP'lere gelince degerlendirilecek.
