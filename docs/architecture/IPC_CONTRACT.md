# IPC_CONTRACT.md — Spark OS IPC Modeli

> DURUM: **KISMEN FROZEN.** Genel hicbrid model ve capability-in-message kesinlesti.
> IPC cancellation (STEP 7) + Priority donation (STEP 8) konulari ACIK — ilgili
> STEP'lerde kapatilip bu dosya guncellenecek.

## 1. Genel Model (FROZEN)

Hibrit IPC:

| Veri boyutu | Mekanizma |
|---|---|
| Kucuk | Message IPC: Send / Receive / Call / Reply |
| Buyuk | Shared memory: Map / Lend / Streaming |
| Zaman-esasli | Ring buffer: Producer → Ring Buffer → Consumer |

## 2. Message IPC (FROZEN kismen)

### Operasyonlar

- **Send** — ilet, bekleme yok.
- **Receive** — gelene kadar blok.
- **Call** — send + reply bekle (sync request/response).
- **Reply** — call'a cevap.

### Capability-in-message (FROZEN)

IPC yalnizca veri tasimaz; capability de mesaj icinde tasinabilir:

```
Message
 ├── Payload (data)
 └── Capability transfer (yetki)
```

**Kritik kural:** Capability, **dequeue/delivery** sirasinda dogrulanir — enqueue
sirasinda degil. `revoked` capability tasiyan mesaj **sessizce dusurulmez**;
receiver'a error bildirilir (sonsuz blok onlenir = DoS karsi).

| Durum | Davranis |
|---|---|
| Mesaj kuyrukta + sender revoke | payload teslim edilir, cap slot `CAP_REVOKED` |
| Sessiz drop | **YASAK** |
| Teslim edilen cap | ayni node'u paylasir → revoke ikisini de oldurur |

## 3. Timeout / Cancellation (ACIK — STEP 7)

Blocking IPC'de timeout ve cancellation gerekli.

**Kritik sinir:** Kernel, user-space server'in yaptigi hesabi otomatik rollback edemez.

Dogru model (aday):

```
Client timeout
   ↓
Request cancelled (kernel-side)
   ↓
Server cancellation notification
   ↓
Server cleanup
```

Acil sorular:
- Kernel server thread'i oldurecek mi, yoksa cancellation flag mi koyacak?
- Server'a IPC cancellation notification mi gonderilecek?
- Server cleanup garantisi nasil saglanacak?
- Cancel edilmis client request'inin server digest'i ne olacak?

## 4. Priority Donation (ACIK — STEP 8)

IPC'de priority inversion cozumu. Aday model:

```
Client priority
   ↓
Server priority donation
   ↓
Server runs at elevated priority
   ↓
IPC complete
   ↓
priority restored
```

Acil sorular:
- Donation suresi
- Nested IPC / chained donation
- Timeout konusma
- Malicious priority abuse (sosuza yuksek oncelik donorler)

Not: Kimi'nin "no nested blocking IPC" deadlock onleme kurali gozden gecirilecek —
buyuk IPC sistemlerinde kistliyici olabilir; ayrinti STEP 8'de.

## 5. Hata Kodlari (taslak — STEP 7'de kisilanacak)

| Kod | Anlam |
|---|---|
| IPC_OK | basarili |
| IPC_TIMEOUT | sure doldu |
| IPC_CANCELLED | client iptal etti |
| IPC_NO_SERVER | hedef yok |
| IPC_QUEUE_FULL | kuyruk dolu |
| IPC_INVALID_CAP | tasinan cap gecersiz |

Hata kodu seti STEP 7'de freez edilecek; simdilik kesin degil.
