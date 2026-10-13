# SYS_FORK ve COW Altyapısı Durum Raporu (Faz 0.9)

**Tarih:** 2026-10-13  
**Durum:** Analiz Tamamlandı — Kod Değiştirilmedi (Karar Faz 3'e ertelendi)

---

## 1. Yönetici Özeti
SparkOS mikroçekirdeğinde `SYS_FORK` (syscall 14) ve Copy-on-Write (COW) mekanizmaları kısmen mevcuttur ancak eksik ve birbirine bağlanmamış durumdadır:
- `SYS_FORK` syscall tablosunda tanımlıdır ve `fork_current()` fonksiyonunu çağırır.
- `fork_current()` yeni bir PCB ve CR3 kökü tahsis eder; fakat kullanıcı sayfalarını kopyalamaz ve COW olarak kaydetmez. Ayrıca çocuk sürece dönüş değeri olarak `0` (RAX=0) iletme mekanizması eksiktir.
- `handle_cow_fault` sayfa hatası (Page Fault #PF) işleyicisinde doğru şekilde dinlenmektedir; ancak `register_cow_page` çekirdeğin hiçbir yerinden çağrılmadığı için `is_cow_page` her zaman `false` dönmektedir.

---

## 2. SYS_FORK İncelemesi

### 2.1 Syscall Giriş Noktası (`src/syscall.rs`)
- Syscall numarası: `SYS_FORK = 14`.
- Ring-3 çağrısı geldiğinde:
  1. `syscall_cap::check_process_exec()` ile çağıran sürecin `EXECUTE` yetkisi denetlenir. Yetki yoksa `EACCES` döner.
  2. `crate::task::process::fork_current()` çağrılır.
  3. Ebeveyn sürece çocuğun `pid` değeri döner.

### 2.2 Çekirdek Gerçekleşimi (`src/task/process.rs::fork_current`)
- Çağıran sürecin `is_user` bayrağı kontrol edilir; kullanıcı süreci değilse `-1` döner.
- `crate::memory::clone_active_cr3()` ile ebeveynin sayfa tablosu kopyalanır (`child_cr3`).
- Yeni PID tahsis edilir (`alloc_pid()`).
- Ebeveynin `user_rip`, `user_rsp`, `user_cs`, `user_ss` değerleri çocuğa aktarılır.
- `seed_new_process` ile standart I/O ve exec hakları verilir.
- Çocuk süreç `ready` kuyruğuna eklenir.

### 2.3 Tespit Edilen Zayıflıklar ve Eksiklikler
1. **Dönüş Değeri Eksikliği:** Unix `fork()` semantiğinde çocuk sürecin `RAX` (veya mimari syscall dönüş) register'ı `0` olmalıdır. `fork_current()` içinde çocuğun `RegisterContext` yapısı oluşturulurken `rax` açıkça sıfırlanmamaktadır.
2. **Kullanıcı Sayfalarının Klonlanmaması:** `clone_active_cr3()` yalnızca düşük supervisor/kernel eşlemelerini ve ilk PDPT girişini kopyalamaktadır (`dest[0]`). Kullanıcı kod, veri ve stack alanları (`USER_ADDR_BASE` = 0x4000_0000 ve yukarısı) kopyalanmamakta veya COW olarak işaretlenmemektedir. Bu nedenle çocuk süreç kullanıcı kodunu veya yığınını ebeveynle paylaşamaz ya da sayfa hatasına düşer.

---

## 3. Copy-on-Write (COW) Mekanizması İncelemesi

### 3.1 COW Kayıt ve Takip Altyapısı (`src/memory.rs`)
- **Veri Yapısı:** `COW_PAGES: IrqMutex<BTreeMap<(u64, u64), PhysAddr>>` (CR3 + sanal sayfa adresi anahtarı, orijinal fiziksel adres değeri).
- **`register_cow_page(cr3, virt, original_phys)`:**
  - Verilen sanal sayfayı COW tablosuna ekler.
  - **Bulgu:** Çekirdek genelinde **HİÇBİR YERDEN ÇAĞRILMAMAKTADIR**. Çağrı sayısı 0'dır.
- **`is_cow_page(cr3, virt)`:**
  - Adresin COW tablosunda olup olmadığını sorgular. `COW_PAGES` boş olduğu için daima `false` döner.

### 3.2 Sayfa Hatası İşleyicisi (`src/interrupts.rs::page_fault_handler`)
- Satır 378-386:
  ```rust
  if error_code.contains(PageFaultErrorCode::CAUSED_BY_WRITE) {
      let (frame, _) = x86_64::registers::control::Cr3::read();
      let current_cr3 = frame.start_address().as_u64();
      if crate::memory::is_cow_page(current_cr3, addr) {
          if crate::memory::handle_cow_fault(current_cr3, addr).is_ok() {
              return; // CoW çözüldü, geri dön
          }
      }
  }
  ```
- Mantık doğru kurgulanmıştır: Kullanıcı yazma hatası aldığında ve sayfa COW listesindeyse, `handle_cow_fault` çalıştırılır ve başarılıysa hata giderilip kullanıcı koduna dönülür.

### 3.3 COW Çözümleme (`src/memory.rs::handle_cow_fault`)
- Yeni bir fiziksel sayfa (`user_alloc_frame()`) tahsis edilir.
- Eski sayfadaki 4096 bayt yeni sayfaya kopyalanır (`copy_nonoverlapping`).
- Hedef sayfa tablosunda ilgili sanal sayfa `WRITABLE` olarak yeniden eşlenir.
- `tlb_shootdown` ile diğer çekirdeklerin TLB'si temizlenir.
- Kod mantığı sağlamdır; ancak tetikleyici kayıt eksiktir.

---

## 4. Faz 3 İçin Öneri ve Karar Seçenekleri
Faz 3 (Süreç Modeli) kapsamında verilecek mimari karar:
1. **Seçenek A (Tam COW Fork):** `clone_active_cr3` sırasında kullanıcının tüm PT girişlerini salt-okunur (read-only) yapıp `register_cow_page` ile kaydetmek ve `fork_current`'te çocuğa RAX=0 döndürmek.
2. **Seçenek B (Mikroçekirdek `spawn` Modeli - Önerilen):** Mikroçekirdek felsefesine uygun olarak Unix-tarzı ağır `fork()` yerine temiz `spawn(path, argv)` modelini birincil süreç oluşturma yöntemi yapmak. `SYS_FORK`'u ya tamamen kaldırmak ya da açıkça `ENOSYS` döndürmek.
