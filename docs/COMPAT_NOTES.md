# SparkOS POSIX Uyumluluk ve Geçiş Notları (Faz 3.6)

## 1. `SYS_FORK`'un Çekirdekten Çıkarılması

- **Gerekçe:** Mikroçekirdek mimarisinde tüm adres alanının ve açık tanıtıcıların kopyalanması (`fork`) karmaşık, kilit açığı yaratan ve SMP ölçeklenmesini zorlaştıran bir POSIX kalıntısıdır. Modern mikroçekirdeklerde süreç oluşturma `spawn` modeliyle (temiz adres alanı + capability devri) gerçekleştirilir.
- **Durum:** `SYS_FORK` sistem çağrısı çekirdekte `ENOSYS` dönecek şekilde kullanımdan kaldırılmıştır. Sabit `spark-abi` içinde geriye dönük binary uyumluluğu için ayrılmış numara olarak kalmaktadır.
- **Gelecek Planı (Faz 10):** Linux/POSIX uyumluluk servisi (`compat-svc`), kullanıcı alanında `fork()` davranışını `MemObject`'lerin COW (Copy-On-Write) klonlama yetenekleri üzerinden taklit edecektir. Çekirdek seviyesinde fork kodu barındırılmayacaktır.

## 2. Sinyal (Signal) Modeli

- Çekirdek yalnızca zorunlu sinyalleri (`SIGKILL` sonlandırma ve donanım istisnaları/fault bildirimleri) yönetir.
- Karmaşık POSIX sinyal mekanizmaları (`sigaction`, `sigprocmask`, kullanıcı alanı sinyal yığınları) çekirdeğe yük edilmeyecek, Faz 10'da kullanıcı alanı uyumluluk kütüphanesine devredilecektir.
