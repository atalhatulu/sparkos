# SparkOS — Hedef Mimari vs Windows / Linux / macOS

> Sürüm: 1.0 · Tarih: 2026-08-14
> Bu dosya, SparkOS'un **hedeflenen** mikrokernel-capability mimarisini, bugünkü
> **mevcut** kod gerçeğiyle birlikte, üç büyük işletim sistemine karşı kıyaslar.
> "Hedef" sütunu dokümanlardaki FROZEN sözleşmelerden (CAPABILITY_MODEL.md /
> IPC_CONTRACT.md) doğar; "Mevcut" sütunu gerçek repo durumundandır (commit +
> Aşama 5.2 uncommitted 1055 satır).
> Notasyon: ✅ FROZEN · 🔶 KISMEN/eşleşen · ⚠️ ACIK/karar verilmedi.

---

## 1. Genel Felsefe

| | Windows | Linux | macOS | **SparkOS (HEDEF)** | **SparkOS (MEVCUT)** |
|---|---|---|---|---|---|
| Çekirdek türü | Hibrit | Monolitik | Hibrit (XNU) | **Mikrokernel** | 🔶 Monolitik'ten mikrokernel'e geçişte |
| Erişim modeli | Access Token | uid/gid + capabilities | Code signing + sandbox | **Capability (handle + rights)** | ✅ Capability core aktif (Aşama 1-5) |
| Yetki verme | Token grant | root / capset | Entitlement | **Grant / Transfer / Lend** | ✅ Grant+Transfer mevcut; 🔶 Lend gelecek |
| Tasarım felsefesi | Tescilli | Dev, hız öncelikli | Apple güvenlik | **seL4 tarzı güvenlik + Windows hız hedefi** | Capability yolunda, monolit hızlı kalıyor |

---

## 2. Process / Adres Alanı

| | Windows | Linux | macOS | **SparkOS (HEDEF)** | **SparkOS (MEVCUT)** |
|---|---|---|---|---|---|
| Proses izolasyonu | Ayrı VA | Ayrı VA | Ayrı VA | **Ayrı CR3 + capability gating** | ✅ Fork/Exec + CR3 izolasyonu (`50837d2`) |
| Adres uzayı ölçeği | 128TB kullanıcı VA | 128TB | 128TB | **Kendi minimal mm** | 🔶 Basit allocator + page mapping |
| Process modeli | Job/Process/Thread | Task/Thread | Task/Thread | **User-space servis ağacı** | 🔶 → Aşama 5.2'de servis çerçevesi |
| Scheduler | Çok çekirdek, öncelikli | CFS/EEVDF | QOS, throttling | **Hedef: öncelik takviyeli, SMP** | 🔶 Tek CPU, round-robin, **preempt KAPALI default** |

---

## 3. User-Space Servisleri (— Aşama 5.2'nin Kalbi)

| | Windows | Linux | macOS | **SparkOS (HEDEF)** | **SparkOS (MEVCUT)** |
|---|---|---|---|---|---|
| Servis modeli | Win32.csrss + işlemler | Çoğu kernelde | launchd + XPC | **IPC ile izole user-space servisleri** | 🔶 Tek ring3 app (`exec_elf`); servis yok |
| İlk servis | csrss.exe | getty/init | launchd | **İlk hardware-less servis** | ⚠️ Bu aşamada yazılıyor (fcc aktif) |
| Servis izolasyonu | Kernel korumalı | Birbirinden zayıf | Sandbox (entitlement) | **Her servis kendi capability setine sahip** | ⚠️ Tasarım dokümanında, kodda henüz |

### ACIK / KARAR VERİLMEDİ (5.2 ile ilgili)
- **Servis sağlığı/yeniden başlatma** (Windows SCM / launchd keep-alive karşılığı) — kapsam ve politika netleşmedi.
- **Servis-yükleyici**: İlk servisler gömülü ELF olarak mı, diske mi (fcc'nin FS işiyle) kurulacak ayrılmadı.
- **Servisler arası izolasyon varsayılan rights:** minimal-seed mi, inherit mi olacak — AŞAMA 5.2 tamamlanırken netleşecek.

---

## 4. IPC — En Net Fark Taşıyan Katman

| | Windows | Linux | macOS | **SparkOS (HEDEF)** | **SparkOS (MEVCUT)** |
|---|---|---|---|---|---|
| IPC türleri | ALPC/LPC | socket/sysv/binder | Mach ports | **Hibrit: Message / SharedMem / RingBuf** | ✅ Message IPC (Ring 3), shared-mem/ring 🔶 |
| Capability iletimi | Kernel token | fd passing | Mach send-right | **Capability-in-message (FROZEN)** | ✅ `cap.rs` + `ipc.rs` (+284) |
| Syscall alt kümesi | — | — | — | Send/Recv/Call/Reply | ✅ SEND/RECV/TRY_RECV mevcut; CALL/REPLY ⚠️ |
| Hata kodu seti | — | — | — | IPC_OK… | ⚠️ Taslak (STEP 7'de freez) |

### kıyas — SparkOS'un IPC avantajı
- **Capability mesajda taşınır ve dequeue anında doğrulanır** (FROZEN). Windows/Linux/macOS'ta ham yetki geçişi fd/index üzerinden, anında iptal etkisi yok.
- Non-blocking alım (`TRY_RECV`) eklenmiş; bu **plan dışı uzantı** — kullanıcıya soruldu, "bırak" yönünde değerlendirildi ama **resmi karar commit'e girmedi** (⚠️ doğrula).

### ACIK (IPC_CONTRACT.md §3-4)
- **STEP 7 — Timeout / Cancellation:** client timeout'ta server'a nasıl notification gidecek, server cleanup garantisi, cancel edilmiş request digest'i — **AÇIK**.
- **STEP 8 — Priority Donation:** priority inversion çözümü; donation süresi, nested/chained donation, timeout etkileşimi, malicious abuse — **AÇIK**.
- Deadlock: Kimi'nin "no nested blocking IPC" kuralı gözden geçirilecek (kısıtlayıcı olabilir) — **AÇIK**.

---

## 5. Yetki İptali (Sadece SparkOS'ta Tasarlanan Nokta)

| | Windows | Linux | macOS | **SparkOS (HEDEF)** | **SparkOS (MEVCUT)** |
|---|---|---|---|---|---|
| Anlık yetki iptali | Yok (token süre sonu) | root ile yeniden | Hak iptali zor | **O(1) revoke + lazy epoch (FROZEN)** | ✅ Model tam, `cap.rs`'te mekanizma var |
| Kardeş izolasyonu | — | — | — | **revoke(child) → sibling etkilenmez** | ✅ |
| Transfer sonrası | Kalıcı | Kalıcı | Kalıcı | **Transfer = recall edilemez (yeni root)** | ✅ Asama 1'de |
| Lend | — | — | — | **Geçici, auto-revoke** | ⚠️ **Expiry ERTELENDİ** (v0.x, timer bağımlı) |
| In-flight IPC | — | — | — | **revoke ≠ cancel; drain eder** | ✅ Semantik FROZEN |

### ACIK / KARAR VERİLMEDİ (capability)
- **Lend expiry mekanizması** — timer/clock çekirdek altyapısı gelene kadar pasif; yalnız `return` ile sona eriyor. (CAPABILITY_MODEL.md §5)
- **Üst seviye lend** (lend edilen cap'in tekrar grant'ı) — bilinçli v0.x kısıtı; ileride yeniden değerlendirilecek. **AÇIK, karar verilmedi.**
- **Global revoke counter + cached chain-check** (amortized O(1)) — v0.2'de şart değil; gelecekte. **AÇIK.**

---

## 6. Donanım / Düşük Seviye

| | Windows | Linux | macOS | **SparkOS (HEDEF)** | **SparkOS (MEVCUT)** |
|---|---|---|---|---|---|
| SMP | Çok çekirdek | Çok çekirdek | Çok çekirdek | **Çok çekirdek destek** | 🔶 `smp.rs` var (APIC/trampoline) ama **aktif multi-core değil** |
| IO izolasyonu | IOMMU | IOMMU | IOMMU | **IO right + TSS IOPB** | ✅ Aşama 5.0 — TSS IOPB + IO port (commit `bfa062a`) |
| Aygıt modeli | WDF/KMDF | Driver modeli | IOKit | **Capability-gated device** | 🔶 pci/rtl8139/usb module'ları, meta hedef ayrı |
| Virtualization | Hyper-V | KVM | Hypervisor.framework | — (kapsam dışı) | — |

### ACIK
- **SMP aktifleştirme:** `init_smp` çağrılıyor, AP başlatma trampoline hazır ama çekirdek ağırlıklı tek CPU'da koşuyor; scheduler single-CPU. **Çok çekirdek yörüngesi planlanmadı** — mikrokernel IPC önceliği buna göre.
- **Device servisleri:** capability-gated device katmanı (IOMMU eşlenikleri) — yalnızca kavramsal; hangi cihazların servis olarak soyutlanacağı **kararsız**.

---

## 7. Özet Karar Matrisi

| Boyut | Windows | Linux | macOS | **SparkOS (HEDEF)** | **SparkOS (MEVCUT)** |
|---|---|---|---|---|---|
| Güvenlik modeli | Token | uid/cap | Sign+sandbox | **Capability** | ✅ Cap aktif |
| Mikrokernel | Hayır | Hayır | Hibrit | **Evet** | 🔶 Geçişte |
| O(1) anlık yetki iptali | Yok | Yok | Yok | **Var** | ✅ Model; expire/lend ⚠️ |
| IPC + cap-in-message | ALPC | fd/socket | Mach | **Hibrit + capability** | ✅ Message IPC + cap |
| User-space servis | Evet | Evet | Evet | **Evet** | 🔶 İlk servis 5.2'de |
| SMP | Evet | Evet | Evet | **Evet** | 🔶 Tek CPU (hazırlık var) |
| IO izolasyonu | IOMMU | IOMMU | IOMMU | **IO right / TSS IOPB** | ✅ 5.0 |

---

## 8. Açık Konular — Durum Matrisi (2026-08-14 Karar)

Aşağıdaki sınıflandırma, ChatGPT ile yapılan mimari karar oturumu sonucu netleştirildi.
**Notasyon:** 🔒 FROZEN (değişmez sözleşme) · 🟡 PROVISIONAL (yön belli, gelişebilir) · ⏸️ DEFERRED (ertelendi).

### 🔒 FROZEN — sözleşmesi donmuş

| # | Konu | Karar |
|---|---|---|
| 1 | IPC Cancellation (temel model) | **Cooperative cancellation** — temel model sabit; mekanizma ayrıntısı STEP 7'de |
| 2 | IPC Error Code seti | **Az sayıda temel kod, extensible** (mevcut capability kodlarıyla tek kaynak — ayrıntı §8 notu) |
| 3 | `SYS_IPC_TRY_RECV` (non-blocking IPC) | **Kalsın + FROZEN** — plan-dışıydı, resmi kararla onaylandı |
| 4 | Lend → Grant | **YASAK** (bilinçli v0.x kısıtı) |
| 5 | Minimal capability seeding | **Minimal-seed** — servise başlangıçta asgari hak verilir |
| 6 | Embedded ELF / initrd bootstrap | **Embedded ELF + initrd** — servisler gömülü başlar |
| 7 | Device access | **Capability-gated prensibi** — her cihaz erişimi capability ile; TSS IOPB yalnız dar port izni |

### 🟡 PROVISIONAL — yön net, detay gelişebilir

| # | Konu | Yön |
|---|---|---|
| 1 | User-space supervisor / restart politikası | **User-space supervisor** — auto-restart politikası detayı 5.2'de şekillenecek |
| 2 | Device-service parçalanması | Hangi cihaz hangi serviste — **sonra netleşir**, prensip (B/§7) sabit |
| 3 | SMP-ready mimari | **Single-core şimdi, SMP-ready** — kod hazır, aktif değil |

### ⏸️ DEFERRED — ertelendi

| # | Konu | Ne zaman |
|---|---|---|
| 1 | Priority Donation (STEP 8) | v0.x sonrası — tek CPU + preempt kapalıyken işlevsiz |
| 2 | Lend expiry (timer bağımlı) | v0.x — timer/clock çekirdeği gelene kadar; yalnız `return` |
| 3 | SMP aktivasyonu | İleri aşama |
| 4 | Nested / chained donation | STEP 8 ile birlikte |
| 5 | Detaylı device-service framework | İleri aşama |

### Not — IPC Error Codes tek kaynak kuralı

Capability hataları kapalı kalır (tek kaynak: `CAP_INVALID` / `CAP_REVOKED` / `CAP_NO_RIGHTS`).
IPC katmanı yalnız ekler: `IPC_TIMEOUT`, `IPC_CANCELLED`, `IPC_NO_SERVER`, `IPC_QUEUE_FULL`, `IPC_INVALID_MESSAGE`.
`SERVER_DIED` / `PERMISSION_DENIED` capability katmanında kapsanır — tekrarlama yok.

---

*Yeni kararlar için tüm dokümanlar `docs/ARCH_DECISIONS.md`'deki karar kaydına işlenir. FROZEN değişikliği = mimari karar değişikliğidir (ADN-1 kuralı). Kod gerçeğiyle doğrulanmıştır; tahmin içermez.*
