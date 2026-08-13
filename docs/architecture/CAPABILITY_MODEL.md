# CAPABILITY_MODEL.md — Spark OS Capability Model (FREEZE)

> v0.2. STEP 1 (Revocation state machine) + STEP 2 (Grant/Transfer/Lend) mitzvah.
> Bu dosya, capability sisteminin tek tutarli semantigini tanimlar.

## 1. Temel Kavramlar

### Capability ≠ Resource

- **Capability** — resource'a erisim hakki. Rights icerir. Fiziksel pointer degildir.
- **Resource** — gercek kernel objekturi (RAM pardesi, device, IPC endpoint, DMA buffer).
- Capability, resource ile handle arasindaki yetki bagidir; resource'un kendisi degildir.

```
Process
   ↓
Capability (handle + rights + lineage ptr)
   ↓
Indirection
   ↓
Kernel Object
   ↓
Resource
```

### Capability Bilesenleri

| Alan | Anlam |
|---|---|
| `slot` + `generation` | Handle (process-tabel). Generation, slot reuse korur. |
| `rights` | READ/WRITE/MAP/IO/DMA/TRANSFER/GRANT/DESTROY/EXECUTE/MANAGE bitmask. |
| `node` | Lineage node — parent ptr + epoch. Recall/izolasyon mekanizmasi. |
| `object` ptr | Kernel object'e referans (indirection). |
| `outstanding` claim | Refcount: mapping, in-flight IPC, DMA pin. Yalniz resource lock'ta. |

## 2. Handle & Generation

```
Handle = slot + generation
```

- `close(h)` → slot free, generation++ → gelecek deref `CAP_INVALID`.
- Eski handle ayni index'i reused eder ama generation degisir → stale handle `CAP_INVALID`.
- Generation tasmasi (u64 wrap): slot kalici emekli (fail-closed).

### Deref akis

1. `deref(h)` → handle tabel lookup, `slot.generation == h.generation` kontrolu.
2. Epoch check (lineage lazy chain walk).
3. Rights check (opsiyonel, op-specific).
4. Claim (refcount++), kontrol kisitlamalari ile atomik.

## 3. Lineage & Epoch (Recall / Revocation)

### Lineage agaci

- Her capability bir **lineage node**'a baglidir.
- `grant` → child node. `transfer` → node'u kopar, yeni root.
- Node: `{ parent: Option<NodeRef>, epoch: u64 }`.

### Revocation

```
revoke(h)
   ↓
handle → REVOKED
   ↓
node.epoch++ (O(1))
   ↓
future deref (lazy chain walk) → CAP_REVOKED
```

- **Lazy epoch:** revoke sadece node'un epoch'unu artirir (O(1)).
  Derencer, lineage'deki her node'un epoch'unu ana node'un snapshot path'i ile
  dogrular. Path'i kirilan → `CAP_REVOKED`.
- **Isbirligi gerekmez:** A, B'nin handle'ina dokunmadan A'nin node'unu revoke eder →
  B'nin chain walk'i A'nin DEAD node'una carpar → B de olur.
- **Sibling izolasyonu:** revoke(child) yalniz o child subtree'ini oldurur; ayni
  parent'tan kardesler etkilenmez.
- **Idempotent + monotonik:** bir kere REVOKED, bir daha LIVE olamaz (no resurrection).

### Neden "ağaç değil, parent-pointer zinciri"

- Tam agac + eager walk (seL4 mdb tarzı) = O(descendants) + agac kitabi.
- Lazy chain-check: revoke O(1), deref O(depth ≤ birkaç). Amortized O(1) icin
  global revoke counter + cached chain-check optimizasyonu v0.2'de SART DEGIL.
- Gozlenebilir semantik ayni: turetilmisler olur. Implementasyon farki.

## 4. Rights Model

```
derived_rights ⊆ parent_rights   (INV-9)
```

- `grant/transfer` yalnizca **daralttığında** yeni hak verebilir; genisletemez.
- `req_rights` ∩ `parent_rights` — kesişim çıkar.

| Right | Anlam |
|---|---|
| READ | okuma |
| WRITE | yazma |
| MAP | memory mapping |
| IO | port/MMIO erisimi |
| DMA | device DMA |
| TRANSFER | transfer izni (receiptor için) |
| GRANT | grant izni (parent olma) |
| DESTROY | resource destroy |
| EXECUTE | execute |
| MANAGE | policy/metadata |

### Hata kodlari (INV-8 — asla karismaz)

| Durum | Kod |
|---|---|
| generation mismatch | `CAP_INVALID` |
| epoch mismatch (linage kirildi) | `CAP_REVOKED` |
| rights yetmez | `CAP_NO_RIGHTS` |

## 5. Grant / Transfer / Lend — STEP 2 FROZEN

### GRANT (copy)

```
grant(parent, req_rights)
   → yeni child capability
   → parent aynen kalir
   → derived = req_rights ∩ parent_rights
```

- Ayni parent'tan ikinci grant → **yeni kardes** (izole).
- Child, parent'in lineage'ina bagli (recall edilebilir).
- Child public rights silinerse parent haklari etkilenmez (child cap zaten kopya).

### TRANSFER (move)

```
transfer(src, req_rights)
   → capability'yi lineage'dan kopar, yeni root yapar
   → src slot kapatilir (transfer eden artik sahip degil)
   → rights: derived = req ∩ parent
```

- **Recall edilemez:** Node bagimsiz root oldugu icin eski lineage'in revoke'undan
  ETKILENMEZ.
- TRANSFER izni, receiptor'un yeni root'u kabul etmesi icin transfer tarafinda gerekli.

### LEND (temporal)

```
lend(src, req_rights, expiry)
   → gecici child capability
   → return(cap) → otomatik revoke
   → expiry doldu → otomatik revoke
```

- **Tek seviye:** lend'den sonra cap'i grant/transfer YAPILAMAZ. Return disinda yol yok.
  > **v0.x penceresi:** Bu bir evrensel capability kurali DEGIL, **bilincli v0.x
  > kısıtı**. Sadece expire-on-return/otomatik revoke guvenligini basit tutmak icin
  > konuldu. Ileriki surumlerde lend edilen cap'in grant edilebilmesi (ust seviye
  > lend) yeniden degerlendirilebilir — ilgili STEP'in kapsaminda.
- Lend edilen cap'i process, son kullaniciya teslim eder; root degil; süresi doldugunda
  otomatik recall edilir.
- **Expiry implementasyonu ERTELENDI (v0.x):** `expiry` timer/clock altyapisina bagli.
  Semantik (expiry → otomatik revoke) şimdiden freeze; MEKANIZMA (timer, deadline
  handler) scheduler/time STEP'ine birakildi. v0.x'te lend yalnizca `return` ile sona
  erer; expiry parametresi o adima kadar pasif arayuzu tasir.

### Karsilastirma ozet

| | GRANT | TRANSFER | LEND |
|---|---|---|---|
| Kaynak hakki | Aynen kalir | Kaybolur | Gecici (expiry) |
| Yeni cap | Kopya child | Yeni root | Gecici child |
| Recall | Evet (child) | Hayir (root) | Evet (expiry/return) |
| Tekrar grant | Evet | Hayir | Hayir |

## 6. Revocation vs In-flight

### In-flight operasyonlar (IPC/DMA/mapping)

- Revoke, in-flight claim'leri DURDURMAZ. Onlar tamamlanir, sonra drain olur (INV-2).
- `revoke ≠ cancel`. Iptal ayri, kullanici-gorunur bir operasyondur (STEP 5/7).
- Resource yalnizca tum authority claim'ler + refcount dusunce FREE (INV-1).

### In-flight IPC capability transfer

- Capability dequeue aninda dogrulanir, enqueue aninda degil.
- Kuyruktaki mesaj + sender revoke → **payload teslim edilir, cap slot `CAP_REVOKED`**.
- Sessiz drop YASAK (receiver sonsuz blok olur = DoS). Error mesaji teslim edilir.
- Teslim edilen cap ayni node'u paylastigi icin revoke ikisini de oldurur (retroaktivite yok).

## 7. Resource Lifetime (DeepSeek refcount modeli)

```
Capability revoke
   ↓
access lost
   ↓
resource still may exist
   ↓
refcount / active references (mapping, in-flight IPC, DMA)
   ↓
final release
   ↓
resource free
```

- **Capability lifetime ≠ Resource lifetime** (INV-1).
- Resource yalnizca `refcount == 0` oldugunda free.
- Refcount, authority claim'ler (cap) + in-flight claim'ler (map/ipc/dma) toplami.

### Object state machine

`ALIVE → (son authority claim dustu) → DYING → (refcount=0) → FREE`

| Event | Handle | Node | Object |
|---|---|---|---|
| `close(h)` | slot free, gen++ | ref-- (cocuk varsa yasar) | — |
| `revoke(h)` | REVOKED | DEAD (epoch++, O(1)) | authority claim duser; DYING'e gecebilir |
| `deref(h)` | gen check | lazy chain walk | claim (refcount++ atomik) |
| `release(claim)` | — | — | refcount--; 0 → FREE, object gen++ |

## 8. Contract (Freeze kurallari)

1. Capability sistemdeki tek erisim vektoru.
2. `derived_rights ⊆ parent_rights` her zaman.
3. `revoke` O(1), non-blocking, monotonik, idempotent.
4. `revoke ≠ free`; `revoke ≠ cancel`; `close ≠ revoke`.
5. Transfer edilen cap recall edilemez; grant edilen recall edilebilir; lend otomatik expiry.
6. Her deref, generate → epoch → rights sirasiyla dogrular (tek kritik bolge).
7. In-flight claim'ler revoke'dan sag cikar; iptal ayri operasyon.
8. Hata kodlari ayri: `CAP_INVALID` / `CAP_REVOKED` / `CAP_NO_RIGHTS`.
