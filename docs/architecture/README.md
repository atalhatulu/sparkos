# Spark OS — MIMARI DURUM OZETI

> v0.2 mimari sozlesmeleri. Kodlama BASLAMADAN once durum.
> Bu dokuman, dorduncu adversarial review'in sonucunu ve kilit kararlari donduurur.

## Amac

Microkernel mimarisini kodlamaya baslamadan once mimari kararlari, cozulen konulari,
celiskileri ve acik noktalari freeze etmek. Yeni ozellik EKLEMEMEK; v0.2 sozlesmelerini
dondurmak.

## Kesinlesenler

### Capability-first guvenlik

- Sistemde kaynak erisiminin temel mekanizmasi **capability**.
- Capability resource degildir; resource'a erisim hakkini temsil eder; rights icerir;
  dogrudan fiziksel pointer olarak KULLANILMAZ.

```
Process
   ↓
Capability
   ↓
Indirection
   ↓
Kernel Object
   ↓
Resource
```

### 3 katmanli lifetime

| Katman | Mekanizma |
|---|---|
| Handle | `generation` |
| Authority | `lineage + epoch` |
| Resource | `claim / refcount` |

### Temel operasyon semantikleri

| Op | Davranis |
|---|---|
| `close()` | Sadece kendi handle'ini kapatir. Turetilmisine dokunmaz. |
| `revoke()` | Handle + tum child lineage'i oldurur. Sibling'ler etkilenmez. |
| `transfer()` | Capability'i lineage'dan koparir, yeni root yapar; recall edilemez. |
| `grant()` | Child olusturur; recall edilebilir. |

### Revocation kurallari

- `revoke` **O(1), non-blocking**.
- `revoke ≠ free` — resource, aktif referanslar biteene kadar yasar.
- `revoke ≠ cancel` — in-flight IPC/DMA/mapping claim'leri revoke sonrasi yasamaya devam eder.
- Generation hatasi → `CAP_INVALID`
- Epoch hatasi → `CAP_REVOKED`
- Rights hatasi → `CAP_NO_RIGHTS`

### IPC

- Capability transfer mesaj icinde tasinabilir.
- Capability kontrolu **dequeue/delivery sirasinda** yapilir.
- Revoked capability tasiyan mesaj **sessizce dusurulmez**.
- Genel IPC modeli: **message + shared memory + ring buffer**.

### Invariantlar (Step 1 karari)

- INV-1 `revoke` asla deallocate etmez.
- INV-2 in-flight claim'ler revoke'dan sag cikar, tamamlanir, sonra drain olur.
- INV-3 no resurrection — DEAD node bir daha LIVE olamaz.
- INV-4 recall isbirligi gerektirmez (rekursif dogrulama).
- INV-5 `close ≠ revoke`.
- INV-6 `revoke` idempotent + monotonik.
- INV-7 `grant ≠ transfer` (moved recall edilemez, granted recall edilebilir).
- INV-8 uc hata kodu asla karismaz.
- INV-9 `derived_rights ⊆ parent_rights` (haklar genisletilemez).

## 4 AI'in ortak katkisi

- **Gemini:** 3 temel contract dokumani onerdi:
  `CAPABILITY_MODEL.md`, `IPC_CONTRACT.md`, `RESOURCE_LIFETIME.md`.
  Kilit isik: capability/IPC/DMA/Hardware uc lifetime'inin kesismesinde race contract
  seviyesinde cozulmeli.
- **Kimi:** capability → lineage → object ve generation/epoch/refcount ayrimini
  guclendirdi. "No nested blocking IPC" deadlock kurali onerdi. (1ms cooldown gibi
  keyfi degerler RED — erken karar.)
- **DeepSeek:** **Reference counting** — resource lifetime'i capability lifetime'dan
  ayirdi (mapping/IPC/DMA referanslari). En guclu katki.
- **Son adversarial review:** per-handle revoke, resource-wide indirection, synchronous
  revoke ve revoke=cancel modellerini reddetti; **lineage + lazy epoch + claim/refcount**
  modelini secti.

## Hala cozulmesi gerekenler

1. ~~Grant / Transfer / Lend tam semantigi~~ → **STEP 2 ile cozuldu**
2. ~~Concurrency / Memory Ordering~~ → **STEP 3 ile cozuldu (FIX-1/2/3)**
3. Memory mapping + TLB revoke davranisi (STEP 4)
4. DMA + IOMMU + cancel modeli (STEP 5)
5. Resource lifetime implementasyon detayi (STEP 6)
6. IPC cancellation semantigi (STEP 7)
7. Priority / time donation (STEP 8)
8. Slot allocator implementasyon (STEP 6 alti)
9. Process death sonrasi capability davranisi

## Sıradaki adim

**STEP 3 Concurrency / Memory Ordering** — FROZEN (bkz. `CONCURRENCY_MODEL.md`).
Capability core'un race-free oldugu adres-adres dogrulandi.

Sonra:
**STEP 4 Mapping/TLB → STEP 5 DMA/IOMMU → STEP 6 Resource Lifetime → STEP 7 IPC Cancel → STEP 8 Scheduler**

### Ana kural

> Once mimari contractlari freeze et, sonra kernel kodla.
