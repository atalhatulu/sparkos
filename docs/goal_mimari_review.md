# /goal — SparkOS Capability Microkernel: Formal Mimari Review + Kapanış Analizi

> Bu dosyayı AGY'ye (Antigravity CLI) görev olarak ver. İçerik tamamen kendine
> yetendir — local dosyaları okumana gerek YOK; tüm kontrat ve bağlam burada.
> Yerel repo'ya asla yazma — çıktıyı tam metin olarak döndür. Derin, kapsamlı,
> hızlı bitirme; uzun ve titiz bir analiz bekle (fcc-claude 1+ saat süren işlere
> paralel çalışıyor; sen de eşdeğer derinlikte ol).

---

## Senin görevin

SparkOS'un capability tabanlı mikrokernel mimarisinin **resmi, kapsamlı bir güvenlik
& mimari review** dokümanını üret. Bu bir kod review değil — bir **sözleşme review**:
aşağıda verilen FROZEN/PROVISIONAL/DEFERRED kararları ile capability + IPC sözleşmeleri
arasındaki tutarlılığı, boşlukları, ve kapanış eksiklerini tespit et.

**Teslim:** Aşağıdaki bölümleri içeren, kapsamlı bir Türkçe markdown review dokümanı.
Yerel dosyaya YAZMA — tüm içeriği cevabında döndür.

---

## 1. SparkOS mimari özetin (kabul ettiğimiz durum)

SparkOS, Rust ile yazılan, x86_64 üzerinde tek CPU'lu (SMP-ready), 64-bit bir
işletim sistemi çekirdeğidir. Monolitik kökenden **capability mikrokernel**e evriliyor.

Mevcut bileşenler:
- **Capability core (`cap.rs`)** — handle+rights+lineage modeli, grant/transfer/revoke
- **Capability gating (`syscall_cap.rs`)** — syscall'lara capability köprüsü
- **IPC (`ipc.rs`)** — Ring 3 message IPC (send/recv/try_recv) + capability-in-message
- **Process modeli (`task/process.rs`)** — preemptive single-CPU scheduler, round-robin
- **Fork/Exec + CR3 izolasyonu** — her user process ayrı address space
- **Filesystem, ELF loader, network (RTL8139), USB, disk (ATA)**
- **TSS IOPB + IO port izni** (Aşama 5.0) — dar port erişimi
- **Aşama 5.2**: user-space servis çerçevesi + ilk hardware-less servis (sürüyor)

### Mevcut syscall seti
```
SYS_READ=0  SYS_EXIT=1  SYS_OPEN=2  SYS_CLOSE=3  SYS_WRITE=4  SYS_LSEEK=8
SYS_YIELD=9  SYS_SOCKET=10  SYS_CONNECT=11  SYS_SEND=12  SYS_RECV=13
SYS_FORK=14  SYS_EXEC=15
SYS_IPC_SEND=20  SYS_IPC_RECV=21  SYS_IOPERM=22  SYS_IPC_TRY_RECV=23
```

---

## 2. FROZEN — sözleşme kararları (BUNLAR DEĞİŞMEZ)

Aşağıdakiler resmi karar. Review'ında bunları veri olarak kabul et, eleştirme.

| # | Konu | Karar |
|---|---|---|
| 1 | IPC Cancellation (temel model) | Cooperative cancellation |
| 2 | IPC Error Code seti | Az sayıda temel kod, extensible; capability hataları tek kaynak |
| 3 | `SYS_IPC_TRY_RECV` (non-blocking IPC) | Kalsın + FROZEN |
| 4 | Lend → Grant | YASAK (bilinçli v0.x kısıtı) |
| 5 | Minimal capability seeding | Servise başlangıçta asgari hak |
| 6 | Embedded ELF / initrd bootstrap | Servisler gömülü başlar |
| 7 | Device access | Capability-gated; TSS IOPB yalnız dar port |

### Capability model özü (FROZEN)
- Capability ≠ Resource. Capability = handle + rights + lineage ptr.
- `derived_rights ⊆ parent_rights` her zaman (INV-9). Grant/transfer yalnız daraltır, genişletemez.
- `revoke` O(1), non-blocking, monotonik, idempotent. Lazy epoch chain-walk.
- `revoke ≠ free`, `revoke ≠ cancel`, `close ≠ revoke`.
- Transfer → lineage'dan kopar, yeni root, recall edilemez. Grant → recall edilebilir.
- `close(h)` → slot free, generation++ → stale handle CAP_INVALID.
- Hata kodları ayrı: `CAP_INVALID` / `CAP_REVOKED` / `CAP_NO_RIGHTS` (asla karışmaz).
- Sessiz drop YASAK; receiver sonsuz blok olmasın (DoS karşıtı).
- Revoke in-flight claim'leri durdurmaz; onlar tamamlanır, sonra drain.
- Resource yalnız refcount==0 iken free (capability lifetime ≠ resource lifetime).

### Capability Rights bitmask
`READ WRITE MAP IO DMA TRANSFER GRANT DESTROY EXECUTE MANAGE`

### IPC model özü (FROZEN)
- Hibrit: Kucuk → Message IPC (Send/Recv/Call/Reply); Buyuk → Shared mem (Map/Lend);
  Zaman-esaslı → Ring buffer.
- Capability-in-message: **dequeue/delivery anında doğrulanır**, enqueue anında değil.
- Sessiz drop YASAK; revoke edilmiş cap taşıyan mesaj receiver'a error bildirilir.
- Teslim edilen cap aynı node'u paylaşır → revoke ikisini de öldürür.

### IPC Error Code seti (FROZEN — tek kaynak kuralı)
Capability hataları kapalı (tek kaynak): `CAP_INVALID / CAP_REVOKED / CAP_NO_RIGHTS`.
IPC katmanı yalnız ekler:
```
IPC_OK  IPC_TIMEOUT  IPC_CANCELLED  IPC_NO_SERVER  IPC_QUEUE_FULL  IPC_INVALID_MESSAGE
```
NOT: `SERVER_DIED` / `PERMISSION_DENIED` capability katmanında kapsanır — IPC'de TEKRARLANMAZ.
Yeni kod eklemek = hata envanterini gözden geçirerek, kopya üretmeden.

---

## 3. PROVISIONAL — yön net, detay gelişebilir

| Konu | Yön |
|---|---|
| User-space supervisor / restart politikası | User-space supervisor; auto-restart detayı gelişecek |
| Device-service parçalanması | Hangi cihaz hangi serviste — sonra netleşir, prensip sabit |
| SMP-ready mimari | Single-core şimdi, SMP-ready |

---

## 4. DEFERRED — ertelendi (YAZMA, sadece not et)

| Konu | Ne zaman |
|---|---|
| Priority Donation (STEP 8) | v0.x sonrası |
| Lend expiry (timer bağımlı) | v0.x; yalnız `return` |
| SMP aktivasyonu | İleri aşama |
| Nested / chained donation | STEP 8 ile |
| Detaylı device-service framework | İleri aşama |

---

## 5. Review'unun teslim edeceği bölümler (TAM olarak)

1. **FROZEN sözleşmeler arası tutarlılık analizi** — capability model + IPC +
   hata kodu seti arasında çelişki/boşluk var mı? (örn: revoke ve in-flight IPC
   cap arasındaki davranış, IPC error setinin capability hatalarıyla örtüşmesi).
2. **Güvenlik invariantlarının eksik listesi** — FROZEN sözleşmeyi garantilemek
   için kodda KANITLANMASI gereken invariantları resmi olarak listele (CAP_INV-n
   biçiminde). En az 15 invariant.
3. **Açık güvenlik riskleri** — capability microkernel için kontrat düzeyinde
   henüz kapanmamış riskler (race, capability sızıntısı, DoS, confused deputy,
   intra-IPC lineage, process exit'te capability temizliği vb.).
4. **Kapanış eksikleri** — "Bu mimaride hâlâ kararsız/eksik olan ne kaldı?"
   sorusunun cevabı olarak, FROZEN'ların uygulanması için şart ama henüz karar
   verilmemiş mekanizmalar (örn. capability'in process exit'te otomatik temizliği,
   IPC cancellation mekanizma ayrıntısı, servis izolasyon hakları).
5. **Formal doğrulama yaklaşımı önerisi** — bu invariantları nasıl doğrularız?
   (host-tested unit test, property-based, simulator).
6. **Zarar/öncelik sıralı aksiyon listesi** — hangi açık risk en önce kapanmalı.

Gereksinim: en az 250 satır, derin ve titiz. Her iddiayı yukarıdaki FROZEN
sözleşmeye dayandır. Uydurma filne/API ismi kullanma — bu mimari review, kod review değil.

---

## Önemli kurallar

- Eğer bir bölüm mantıksızsa veya yukarıda çelişiyorsa, söyleme, ÇÖZÜM ÖNER.
- FROZEN'ları değiştirmeye kalkma — yalnız tutarlılık/boşluk tespiti yap.
- DEFERRED listesine DOKUNMA, sadece "ileri aşamada kapatılır" notu olarak bırak.
- Sonuçta belirgin bir `<!-- GOAL_COMPLETE -->` işareti koy.
