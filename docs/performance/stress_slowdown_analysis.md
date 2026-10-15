# Stres Testi %21 Yavaşlama Analizi ve Koşum Tablosu

## 1. Koşum Karşılaştırma Tablosu (10 sn ve 60 sn Eşdeğerleri)

| Commit / Aşama | 10 sn Koşum (İterasyon) | 60 sn Projeksiyon / Gerçek | Saniye Başına İterasyon | Not |
|---|---|---|---|---|
| **Commit 8f32da2** (Faz 4 Öncesi) | ~528,000 | 3,167,000 | ~52,800 op/s | İlkel spinlock, sıfır per-CPU doğrulama, lost-wakeup koruması yok |
| **Commit b5d1cec** (Faz 4 Rev 1) | ~418,000 | 2,509,000 | ~41,800 op/s | finish_switch + global kilit contention + ilkel Mutex switch yükü |
| **HEAD (4b5d0be -> de59fb7)** | **567,623** | **3,405,000** | **~56,762 op/s** | Per-CPU runqueue, IrqMutex hiyerarşi optimizasyonu, sıfır sızıntı |

## 2. %21 Yavaşlamanın Gerçek Kök Nedenleri
1. **`finish_switch` Protokolü ve İki Aşamalı Kilitlenme:**
   Eski kodda bir iş parçacığından diğerine geçerken eski sürecin durumu doğrulanmadan doğrudan `switch_context` yapılıyordu. Faz 4 ile gelen `finish_switch` protokolünde, hedef CPU'ya geçen süreç kilit devralıp `prev_pid`'nin `on_cpu` bayrağını temizlemek için ek bir atomik kontrol/bariyer çalıştırmaktadır.
2. **WaitQueue lost-wakeup Koruması ve Çift Yönlü Düğüm Güncellemeleri:**
   `wait_next` ve `wait_prev` alanlarının her uyutma/uyandırma işleminde güncellenmesi BTreeMap erişimleri ve bellek bariyerleri üretmektedir.
3. **irq_locks_held() ve PerCpuBlock Doğrulamaları:**
   Her kilit alma ve bırakmada per-CPU bloktaki atomik sayaçların güncellenmesi mikro-mimari düzeyinde ek yük getirmektedir.
4. **Sonuç:**
   Bu ek yükler emniyeti (güvenli SMP, lost-wakeup önleme, deadlocksuzluk) sağlamak için ödenen gerekli deterministik maliyettir. Son optimizasyonlarla birlikte HEAD'de performans tekrar 56.7k op/s seviyesine yükseltilmiştir.
