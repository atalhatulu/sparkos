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

## Tamamlanan ve Dondurulan Mimarî Adımlar (Aşama 1 - 10)

1. ✅ **Grant / Transfer / Lend Tam Semantiği** → `src/cap.rs` içinde çözüldü ve donduruldu.
2. ✅ **Concurrency & Memory Ordering** → Lock-order ve Atomic/Spinlock korumasıyla çözüldü.
3. ✅ **Memory Mapping & CR3 İzolasyonu (Aşama 2-3)** → Her süreç için bağımsız CR3 sayfa tablosu.
4. ✅ **DMA + DmaRegion + Zero-Copy Köprüsü (Aşama 6.1, 6.2, 6.3)** → `DmaRegion`, `SLOT_MAP` ve `recycle_slot_cap`.
5. ✅ **Resource Lifetime & RefCount (Aşama 1 & 10)** → `revoke != free`, RefCount==0 olunca temizlik.
6. ✅ **IPC Cancellation Semantiği (Aşama 7.1)** → `SYS_IPC_CANCEL(29)` ve `cancel_endpoint`.
7. ✅ **Lend Expiry (Aşama 7.2)** → PIT 1000Hz zamanlayıcıya bağlı `expire_lent_capabilities`.
8. ✅ **Binary-Safe VFS & BlockCache (Aşama 8.2 & 8.3)** → `FsNode::File { content: Vec<u8> }` ve 32KB LFU/LRU önbellek.
9. ✅ **Çok Çekirdekli (SMP) Aktivasyonu (Aşama 9)** → ACPI MADT, Local APIC, I/O APIC `0xFEC00000`, INIT-SIPI-SIPI.
10. ✅ **Biçimsel Güvenlik & 20 Invariant (Aşama 10)** → `CAP_INV-1..20` 57 test ile doğrulandı.
8. Slot allocator implementasyon (STEP 6 alti)
9. Process death sonrasi capability davranisi

## Sıradaki adim

**STEP 3 Concurrency / Memory Ordering** — FROZEN (bkz. `CONCURRENCY_MODEL.md`).
Capability core'un race-free oldugu adres-adres dogrulandi.

Sonra:
**STEP 4 Mapping/TLB → STEP 5 DMA/IOMMU → STEP 6 Resource Lifetime → STEP 7 IPC Cancel → STEP 8 Scheduler**

### Ana kural

> Once mimari contractlari freeze et, sonra kernel kodla.
