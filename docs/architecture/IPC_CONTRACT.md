# IPC_CONTRACT.md — Spark OS IPC Modeli

> DURUM: **BÜYÜK ÖLÇÜDE FROZEN.** Genel hibrit model + capability-in-message kesinleşti.
> IPC cancellation temel modeli (cooperative) ve IPC error code seti **FROZEN** (2026-08-14 kararı).
> Priority donation (STEP 8) ve cancellation mekanizma ayrıntıları **DEFERRED** — ilgili STEP'lerde kapatılıp bu dosya güncellenecek.

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

## 3. Timeout / Cancellation (FROZEN temel — mekanizma DEFERRED)

**Karar (2026-08-14): Cooperative cancellation.** Temel model FROZEN:
Kernel, server'ın yaptığı hesabı otomatik rollback edemez; iptal, server'ın
işbirliğiyle yürür.

```text
Client timeout
   ↓
Request cancelled (kernel-side, cooperative)
   ↓
Server cancellation notification
   ↓
Server cleanup
```

Açık mekanizma ayrıntıları (DEFERRED — STEP 7 implementasyonunda kapatılır):
- Kernel server thread'i öldürecek mi, yoksa cancellation flag mı?
- Server'a IPC cancellation notification kanalı nasıl kurulacak?
- Server cleanup garantisi nasıl sağlanacak?
- Cancel edilmiş client request'inin server digest'i ne olacak?

## 4. Priority Donation (DEFERRED — STEP 8)

**Karar (2026-08-14): DEFERRED.** Şu an tek CPU + round-robin + preempt kapalıyken
donation konsepti işlevsiz; v0.x sonrasına ertelendi. Aday model kayıt altında:

```text
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

Ertelenen mekanizma soruları (STEP 8 ile kapatılır):
- Donation süresi
- Nested IPC / chained donation
- Timeout etkileşimi
- Malicious priority abuse (sonsuz yüksek öncelikli donorler)
- Kimi'nin "no nested blocking IPC" deadlock kuralı — büyük IPC sistemlerinde
  kısıtlayıcı olabilir; gözden geçirilecek (STEP 8).

## 5. Hata Kodları (FROZEN — 2026-08-14)

**Karar: Az sayıda temel IPC kodu FROZEN, extensible.** Capability hataları tek
kaynak kalır (`CAP_INVALID` / `CAP_REVOKED` / `CAP_NO_RIGHTS`, CAPABILITY_MODEL.md §4).
IPC katmanı yalnız ekler:

| Kod | Anlam |
|---|---|
| IPC_OK | başarılı |
| IPC_TIMEOUT | süre doldu |
| IPC_CANCELLED | client iptal etti (cooperative) |
| IPC_NO_SERVER | hedef yok |
| IPC_QUEUE_FULL | kuyruk dolu |
| IPC_INVALID_MESSAGE | geçersiz mesaj formatı |

Not: `SERVER_DIED` / `PERMISSION_DENIED` capability katmanında kapsanır —
IPC'de tekrarlanmaz (tek kaynak kuralı). Yeni kod eklemek = hata envanterini
gözden geçirerek yapılır, kopya üretmez.
