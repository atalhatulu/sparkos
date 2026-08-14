# SparkOS Capability Microkernel: Formal Mimari Review ve Kapanış Analizi

> Bu doküman AGY (Antigravity CLI, Gemini 3.7 Flash High) tarafından üretildi.
> Kaynak görev: `docs/goal_mimari_review.md`. Tarih: 2026-08-14.
> FROZEN sözleşmelere dayalı sözleşme review'üdür; kod review değildir.

## Yönetici Özeti ve Sözleşme Çerçevesi

Bu doküman, SparkOS çekirdeğinin tek çekirdekli (SMP-ready) x86_64 mimarisi üzerinde monolitik yapıdan capability tabanlı mikrokernel modeline evrilme sürecindeki çekirdek ve IPC sözleşmelerinin resmi mimari incelemesidir (formal architectural review). İnceleme; dondurulmuş (FROZEN) kararların iç tutarlılığını, sözleşmeler arası açık/gri alanları, biçimsel güvenlik invariantlarını, sınır güvenlik risklerini ve kapanış eksiklerini ele almaktadır.

---

## 1. FROZEN Sözleşmeler Arası Tutarlılık ve Boşluk Analizi

```
+---------------------------------------------------------------------------------------+
|                           SPARKOS CAPABILITY & IPC TOPOLOJİSİ                         |
+---------------------------------------------------------------------------------------+
|  [Process A: CSpace]                                         [Process B: CSpace]      |
|  Slot 0: Handle(0, gen:1) -> CapNode A                       Slot 0: Free             |
|  Slot 1: Handle(1, gen:1) -> Channel Cap                     Slot 1: Handle(1, gen:1) |
|           |                                                             ^             |
|           v                                                             |             |
|  +------------------+         SYS_IPC_SEND(Channel, Msg)       +------------------+   |
|  |  CapNode A       | ---------------------------------------> |  CapNode B       |   |
|  |  Rights: R|W|G   |     [Payload + In-Flight Cap Handle]     |  Rights: R|W     |   |
|  +------------------+                                          +------------------+   |
|           |                                                             |             |
|           +---------------------- Lineage Tree -------------------------+             |
|                                         |                                             |
|                                         v                                             |
|                             +------------------------+                                |
|                             | Resource Object (Arc)  |                                |
|                             | RefCount = 2           |                                |
|                             +------------------------+                                |
+---------------------------------------------------------------------------------------+
```

### 1.1. Dequeue Anında Doğrulama (Delivery-Time Validation) ve Mesaj Veri Yükü Bütünlüğü

* **Sözleşme Kuralı:** Capability-in-message kontrolü mesaj kuyruğa girerken (enqueue) değil, alıcı tarafından çekilirken (dequeue/delivery) doğrulanır. Sessiz drop yasaktır; revoke edilmiş capability taşıyan mesaj alıcıya hata ile bildirilir.
* **Tespit Edilen Boşluk / Çelişki:**
  Gönderici süreç bir IPC mesajı içinde hem bir veri tamponu (payload bytes) hem de bir capability handle gönderdiğinde; capability mesaj kuyrukta beklerken verici veya üst ata tarafından revoke edilirse ne olur?
  - Eğer çekirdek tüm sistem çağrısını `CAP_REVOKED` ile sonlandırırsa, mesajın içindeki kontrol verisi (payload) alıcıya ulaşamaz. Bu durum, sunucu-istemci durum makinelerinde veri kaybına veya protokoler kilitlenmelere (deadlock) yol açar.
  - Eğer çekirdek mesajı `IPC_OK` ile teslim edip capability'yi sessizce alıcının CSpace'ine yazmazsa, "sessiz drop yasaktır" sözleşmesi doğrudan çiğnenir.
* **Mimari Çözüm:**
  IPC mesaj üstbilgisi (header) ve syscall dönüş modeli genişletilmeli; syscall genel sonucu `IPC_OK` dönerken, mesaj üstbilgisinde yer alan capability teslim statüsü alanı `CAP_REVOKED` olarak bayraklanmalıdır. Alıcının CSpace tablosunda ilgili handle slotu tahsis edilmemeli veya geçersiz handle (`0`) olarak bırakılmalıdır. Böylece veri kaybı yaşanmadan capability iptali alıcıya deterministik olarak raporlanır.

---

### 1.2. O(1) Lazy Epoch Lineage Yürüyüşü ile Çok Seviyeli Soy Ağaçlarının Uyumu

* **Sözleşme Kuralı:** `revoke` işlemi O(1), non-blocking, monotonik ve idempotenttir. Soy (lineage) takibi tembel dönem (lazy epoch) zincir yürüyüşü ile işletilir. Grant edilen capability aynı düğüm/lineage göstericisini paylaşır.
* **Tespit Edilen Boşluk / Çelişki:**
  Çok seviyeli bir grant zincirinde ($P_0 \xrightarrow{\text{grant}} P_1 \xrightarrow{\text{grant}} P_2 \xrightarrow{\dots} P_k$), $P_0$ düzeyinde `revoke` çağrıldığında $P_0$'ın yerel düğümündeki `revoked_epoch` değerinin güncellenmesi $O(1)$'dir. Ancak en uçtaki $P_k$ süreci bir işlem yapmak istediğinde, köke kadar olan tüm ata düğümlerin `revoked_epoch` değerlerini doğrulamak zorundadır. Bu zincir yürüyüşü derinlik ($k$) kadar zaman alır ($O(k)$). Eğer derinlik sınırlandırılmazsa, kötü niyetli bir süreç yapay olarak derin zincirler oluşturup çekirdeğin syscall yürütme süresinde determinizmi (Worst-Case Execution Time - WCET) bozabilir.
* **Mimari Çözüm:**
  1. Çekirdek seviyesinde azami soy derinliği katı bir sabitle (örn. `MAX_LINEAGE_DEPTH = 8`) sınırlandırılmalıdır. Bu sınırı aşan grant çağrıları `CAP_NO_RIGHTS` ile reddedilmelidir.
  2. Zincir yürüyüşü sırasında ara düğümlere erişildiğinde yol sıkıştırması (path compression / lazy cache) uygulanmalı; ata düğümün güncel epoch değeri alt düğümlere kopyalanarak sonraki erişimlerin amortize maliyeti $O(1)$'e indirilmelidir.

---

### 1.3. Transfer Ayrılması (Lineage Severance) ile Kuyruk İçi Revocation Ayrışması

* **Sözleşme Kuralı:** `TRANSFER` yetkiyi soydan tamamen koparır, yeni bir bağımsız kök oluşturur ve geri çağrılamaz. Capability-in-message kontrolü dequeue anında yapılır.
* **Tespit Edilen Boşluk / Çelişki:**
  Gönderici `TRANSFER` yetkisiyle bir capability'yi IPC kuyruğuna koyduğunda:
  - Eğer soydan koparma işlemi enqueue anında yapılırsa: Alıcı mesajı almadan önce çökerse veya mesajı hiç okumazsa, transfer edilen kaynak CSpace'ler arası boşlukta (limbo) kalır; kaynak serbest bırakılamaz.
  - Eğer soydan koparma işlemi dequeue anında yapılırsa: Mesaj kuyruktayken gönderici kendi CSpace'indeki handle'ı `close` veya `revoke` ederse, alıcının meşru transfer hakkı haksız yere iptal edilir.
* **Mimari Çözüm:**
  `TRANSFER` işlemi için **"In-Flight Transient Node"** modeli uygulanmalıdır:
  1. Enqueue anında göndericinin yerel CSpace slotu derhal kapatılır (`generation++`, `CAP_INVALID`).
  2. Düğüm çekirdek IPC mesaj tamponuna asılı bir transient kök olarak bağlanır; gönderici artık bu düğüm üzerinde hiçbir tasarrufa sahip olamaz.
  3. Dequeue anında düğüm doğrudan alıcının CSpace'ine yeni bağımsız kök olarak kaydedilir. Eğer alıcı mesajı almadan önce çökerse, kuyruk tasfiye yordamı (queue purge) bu transient düğümü deterministik olarak `DESTROY` eder.

---

### 1.4. Hata Kodu Envanteri ve Katmanlar Arası Tekil Kaynak Sözleşmesi

* **Sözleşme Kuralı:** Capability hataları kapalı ve tek kaynaktır: `CAP_INVALID`, `CAP_REVOKED`, `CAP_NO_RIGHTS`. IPC katmanı yalnızca operasyonel durumları ekler: `IPC_OK`, `IPC_TIMEOUT`, `IPC_CANCELLED`, `IPC_NO_SERVER`, `IPC_QUEUE_FULL`, `IPC_INVALID_MESSAGE`. `SERVER_DIED` ve `PERMISSION_DENIED` kodları tekrarlanmaz.
* **Tespit Edilen Boşluk:**
  IPC kanalının karşı ucundaki sunucunun çökmesi (`SERVER_DIED`) veya istemcinin kanala yazma izninin bulunmaması (`PERMISSION_DENIED`) durumlarının tekil kaynak kuralına göre tam olarak nasıl ifade edileceği netleştirilmelidir.
* **Mimari Çözüm:**
  - `SERVER_DIED` durumu, istemcinin elindeki kanal/uç nokta (endpoint) capability'sinin sunucu çöküşünde geçersizleşmesiyle doğrudan `CAP_INVALID` veya `CAP_REVOKED` olarak raporlanmalıdır.
  - `PERMISSION_DENIED` durumu, istemcinin ilgili kanal capability'sinde `WRITE` veya `TRANSFER` hakkı olmaması sebebiyle doğrudan `CAP_NO_RIGHTS` olarak raporlanmalıdır.
  - ABI seviyesinde hata kodları ayrık bit alanlarına (tagged union / bitmask) oturtulmalıdır:
    - Bit [31..16]: Hata Katmanı (`0x0001` = Capability, `0x0002` = IPC).
    - Bit [15..0]: Tanımlı hata sabiti.

---

## 2. Güvenlik Invariantlarının Resmi Listesi

SparkOS mikrokernel mimarisinin doğruluğunu ve güvenliğini matematiksel olarak garanti altına almak için kod tabanında kanıtlanması gereken 18 temel invariant aşağıda tanımlanmıştır:

```
[CAP_INV-1: Monotonic Attenuation]   ---> Child Rights ⊆ Parent Rights
[CAP_INV-4: Lazy Epoch Revocation]   ---> IsRevoked(c) ⇔ ∃ a ∈ Ancestors: c.epoch <= a.revoked_epoch
[CAP_INV-6: Resource vs Cap Lifetime] ---> Resource Free ⇔ RefCount == 0
[CAP_INV-8: Transfer Lineage Sever]  ---> Transfer(c) ⇒ Lineage(c') = {Root(c')}
[CAP_INV-10: Lend-Grant Prohibition] ---> c.is_lend == true ⇒ Grant(c) = FORBIDDEN
```

### `CAP_INV-1`: Monotonic Rights Attenuation (Yetki Azaltma Monotonikliği)
Bir capability'den yeni bir capability türetildiğinde (derive/grant/transfer), türetilen haklar kümesi daima ata haklar kümesinin kesin alt kümesi olmalıdır:
$$\forall c' \in \text{Derive}(c) \implies \text{Rights}(c') \subseteq \text{Rights}(c)$$
Hiçbir sistem çağrısı veya çekirdek işlemi mevcut hakları genişletemez.

### `CAP_INV-2`: Authority Confinement (Mutlak Yetki Sınırlandırması)
Bir kullanıcı süreci, kendi CSpace tablosunda geçerli ve gerekli hakları içeren bir handle bulunmayan hiçbir çekirdek nesnesine, fiziksel bellek sayfasına veya donanım portuna doğrudan veya dolaylı erişemez.

### `CAP_INV-3`: Generation & Stale Handle Invalidation (Nesil Sayacı ve Eski Handle Geçersizliği)
Bir handle kapatıldığında (`close(h)`), CSpace slotunun `generation` sayacı kesin olarak artırılır ($\text{gen}' = \text{gen} + 1$). Eski generation değerine sahip handle'lar ile yapılan tüm çağrılar çekirdek tarafından derhal `CAP_INVALID` ile reddedilir.

### `CAP_INV-4`: Lazy Epoch Revocation Correctness (Tembel Dönem Geri Çağırma Doğruluğu)
Bir düğüm revoke edildiğinde `revoked_epoch` değeri sistemin güncel dönemine eşitlenir. Bir capability düğümünün geçerliliği, köke kadar olan tüm ata düğümlerin dönem kontrolüyle belirlenir:
$$\text{IsRevoked}(c) \iff \exists a \in \text{Ancestors}(c) : c.\text{epoch} \le a.\text{revoked_epoch}$$

### `CAP_INV-5`: Revocation Non-Interference with In-Flight Claims (Yürütme Anı Dokunulmazlığı)
`revoke` işlemi çağrıldığı anda çekirdek içinde o nesne üzerinde devam eden (in-flight claim) atomik operasyonları aniden sonlandırmaz. Devam eden işlem tamamlanana kadar nesne referansı korunur; işlem bittiğinde nesne erişime kapatılır.

### `CAP_INV-6`: Resource Lifetime vs Capability Lifetime Independence (Kaynak ve Yetki Ömrü Bağımsızlığı)
Bir fiziksel çekirdek kaynağı ($R$), yalnızca o kaynağa işaret eden toplam aktif referans sayısı sıfıra ulaştığında serbest bırakılır:
$$\text{Free}(R) \iff \text{RefCount}(R) == 0$$
Tüm capability'ler revoke edilse veya kapatılsa dahi, devam eden claim referansları sıfırlanmadan kaynak belleği geri iade edilemez.

### `CAP_INV-7`: IPC Delivery-Time Validation (IPC Teslim Anı Doğrulaması)
Mesaj kuyruğunda bekleyen capability referansları, alıcı süreç tarafından `SYS_IPC_RECV` veya `SYS_IPC_TRY_RECV` ile kuyruktan çıkarıldığı anda doğrulanır. Kuyrukta bekleme süresinde revoke edilmiş olan capability'ler alıcının CSpace'ine aktarılamaz.

### `CAP_INV-8`: Transfer Lineage Severance & Root Promotion (Transfer Soy Koparma)
`TRANSFER` yetkisiyle aktarılan bir capability, hedef CSpace'e yerleştiği anda ata soy ağacından tamamen koparılır ve bağımsız bir kök düğüm (`Root Node`) haline gelir. Gönderenin yapacağı sonraki `revoke` çağrıları bu düğümü etkileyemez.

### `CAP_INV-9`: Grant Lineage Preservation & Recallability (Grant Soy Korunumu ve Geri Çağrılabilirlik)
`GRANT` yetkisiyle türetilen/aktarılan bir capability, verenin soy ağacına çocuk düğüm olarak eklenir. Veren süreç `revoke` çağırdığında, bu düğümden türetilmiş tüm alt düğümler özyinelemeli olarak geçersiz hale gelir.

### `CAP_INV-10`: Lend-Grant Symmetry Prohibition (Ödünçten Bağış Türetme Yasağı)
`LEND` edilmiş bir capability üzerinden hiçbir şekilde `GRANT` türetilemez:
$$c.\text{is\_lend} == \text{true} \implies \text{Derive}(c, \text{GRANT}) = \text{ERROR(CAP\_NO\_RIGHTS)}$$
Ödünç alınan hak yalnızca `RETURN` edilebilir veya alt-ödünç (`SUB-LEND`) verilebilir.

### `CAP_INV-11`: No Silent Drop in IPC (Sessiz Mesaj Kaybı Yasağı)
Kuyruğa alınmış hiçbir IPC mesajı sistem tarafından sessizce yok edilemez. Hata, çökme veya yetki iptali durumlarında gönderici veya alıcı süreç mutlaka deterministik bir hata kodu ile bilgilendirilir.

### `CAP_INV-12`: Error Code Exclusivity & Orthogonality (Hata Kodu Ayrışıklığı)
Capability hata kodları (`CAP_*`) ile IPC hata kodları (`IPC_*`) birbirini kapsamaz, maskelemez veya çakışmaz. Her hata kendi katmanının tanımlı sözleşmesine uygun olarak üretilir.

### `CAP_INV-13`: Process Exit Clean-Up Determinism (Süreç Kapanış Temizliği)
Bir süreç sonlandığında (`SYS_EXIT` veya istisnai sonlanma), sürecin CSpace tablosundaki tüm handle'lar taranır; sahip olduğu kök grant capability'leri revoke edilir, referans sayaçları atomik olarak düşürülür ve bekleyen IPC kuyrukları temizlenir.

### `CAP_INV-14`: Hardware Gating Alignment (Donanım Erişim Kilidi Uyumu)
G/Ç portları (IOPB), MMIO sayfaları veya DMA kanalları; ilgili sürecin CSpace'inde geçerli `IO`, `MAP` veya `DMA` yetkisi bulunmadan donanım seviyesinde (TSS veya Sayfa Tablosu) aktif hale getirilemez.

### `CAP_INV-15`: CSpace Isolation & Virtual Indexing (CSpace İzolasyonu)
Process $A$, Process $B$'nin CSpace tablosunu veya doğrudan handle indekslerini taklit edemez, okuyamaz ve değiştiremez. Tüm handle çözünürlüğü çağıran sürecin CR3 ve PCB bağlamına kilitlidir.

### `CAP_INV-16`: Idempotent & Monotonic Revoke (Geri Çağırma Tekdüzeliği)
Bir capability için `revoke(c)` işleminin birden fazla kez çağrılması sistem durumunu bozmaz (idempotent). Revoke edilmiş bir capability hiçbir koşulda tekrar geçerli (`valid`) durumuna döndürülemez (monotonik).

### `CAP_INV-17`: Queue Capacity & DoS Bounding (Kuyruk Kapasite Sınırı)
Tüm IPC kanalları sabit veya kesin sınırlı (bounded) mesaj kuyruklarına sahiptir. Mesaj kuyruğu dolduğunda gönderici `IPC_QUEUE_FULL` alır; çekirdek belleği kontrolsüz mesaj birikimiyle tüketilemez.

### `CAP_INV-18`: Atomic Handle Slot Allocation (Atomik Handle Tahsisi)
CSpace içinde yeni bir capability slotunun tahsis edilmesi, generation kontrolü ve handle indeksinin üretimi atomik olarak gerçekleştirilir; çift tahsis (double allocation) veya yarış durumu oluşamaz.

---

## 3. Açık Güvenlik Riskleri

Mikrokernel mimarilerinde monolitik çekirdeklere kıyasla güvenliğin en çok zedelendiği noktalar servis sınırları ve yetki çevrimleridir:

```
+-------------------------------------------------------------------------------+
|                       GÜVENLİK TEHDİT MATRİSİ                                 |
+-------------------------------------------------------------------------------+
| Tehdit Türü          | Etki Alanı       | Mekanizma      | Önleme Modeli     |
+----------------------+------------------+----------------+-------------------+
| Confused Deputy      | Servis CSpace    | Handle Alias   | Dual-Space Lookup |
| TOCTOU Revocation    | Kernel Syscall   | Async Revoke   | RAII CapClaim     |
| Zombie Lineage       | Process Exit     | Orphan Nodes   | Auto-Revoke Epoch |
| CSpace Exhaustion    | IPC Inflow       | Handle DoS     | CSpace Quota      |
| TSS IOPB Leak        | Context Switch   | Dirty Port Map | TSS Sync on Switch|
+----------------------+------------------+----------------+-------------------+
```

### 3.1. Şaşkın Vekil (Confused Deputy) ve Handle İndeks Karışıklığı
* **Tehdit Senaryosu:** Çok istemcili bir sistem servisi (örneğin ATA disk veya VFS servisi), istemciden gelen istekleri işlerken istemcinin mesaj gövdesinde gönderdiği ham tamsayı handle değerini (örn. `h = 3`) doğrudan kendi CSpace'inde çözmeye çalışabilir. Bu durumda istemci, sunucunun kendi 3 numaralı yetkili handle'ını (örneğin ham disk blok yazma yetkisi) kötüye kullanabilir.
* **Mimari Önlem:** Çekirdek seviyesinde kullanıcı mesaj yükü içindeki tamsayılar asla capability olarak kabul edilmez. Capability aktarımı yalnızca çekirdek kontrollü IPC üstbilgisi üzerinden yapılmalı; çekirdek, aktarılan capability'yi alıcının CSpace'inde yeni bir slota yerleştirip alıcıya yalnızca bu yeni yerel handle indeksini bildirmelidir.

---

### 3.2. Eşzamanlı Geri Çağırma ve TOCTOU (Time-of-Check to Time-of-Use)
* **Tehdit Senaryosu:** Bir iş parçacığı $T_1$, geçerli bir capability ile `SYS_WRITE` başlatır. Çekirdek girişinde hak doğrulanır. Tam bu anda başka bir süreç $T_2$, `revoke` çağırarak $T_1$'in yetkisini iptal eder. Eğer çekirdek nesneye doğrudan ham göstericiyle erişiyorsa, nesne serbest bırakılabilir (Use-After-Free) veya yetkisiz yazma gerçekleşebilir.
* **Mimari Önlem:** Çekirdek içinde RAII tabanlı bir `CapClaim` koruyucusu zorunlu kılınmalıdır. Hak doğrulandığında `CapClaim` nesnesi oluşturularak atomik referans sayacı artırılmalı; `revoke` yalnızca yeni claim'leri engellemeli, mevcut claim kapsam dışına çıkana kadar bellek güvenliği korunmalıdır.

---

### 3.3. Süreç Kapanışında Yetim Soy Zincirleri (Zombie/Orphan Lineages)
* **Tehdit Senaryosu:** Bir ana süreç ($P_{main}$), bir istemciye (`GRANT`) yetkisi verir ve ardından aniden çöker (`crash`) veya `SYS_EXIT` çağırır. $P_{main}$'in CSpace bellek alanı serbest bırakılırsa, istemcinin elindeki capability soy ağacında geçersiz kılınmamış veya bozuk bellek adreslerini gösteren ata göstericileri kalır.
* **Mimari Önlem:** Süreç sonlanma yordamında (`process_exit`), sürecin kök olduğu tüm grant ağaçları otomatik olarak taranarak `revoked_epoch = MAX` yapılmalı ve referanslar serbest bırakılmalıdır (`Auto-Revoke on Exit`).

---

### 3.4. CSpace Slot Tükenmesi ve Dağıtık Hizmet Engelleme (DoS)
* **Tehdit Senaryosu:** Kötü niyetli bir istemci, bir sistem servisine (örn. Ağ Yığını) ardı ardına küçük yetki parçacıkları (grant/transfer capability) içeren IPC mesajları gönderir. Sunucunun CSpace slot tablosu (örn. 256 veya 1024 slot) dolar. Sunucu meşru istemcilere yeni handle tahsis edemez hale gelir ve kilitlenir.
* **Mimari Önlem:** Her CSpace için süreç başına azami slot kotası konulmalı ve `SYS_IPC_RECV` çağrısında CSpace doluluğu durumunda mesaj çekme işlemi güvenli bir hata kodu ile (`CAP_INVALID_TARGET_SLOT` veya `IPC_RECEIVER_FULL`) reddedilmelidir.

---

### 3.5. TSS IOPB ve Görev Geçişi (Task Switch) İzolasyon Sızıntısı
* **Tehdit Senaryosu:** Süreç $A$, dar bir G/Ç port aralığı için (`SYS_IOPERM`) yetki alır ve TSS IOPB tablosunda ilgili port bitleri sıfırlanır (erişime açılır). Tek çekirdekli sistemde round-robin scheduler Süreç $B$'ye geçtiğinde, TSS IOPB tablosu temizlenmezse veya geçersiz kılınmazsa, Süreç $B$ yetkisi olmadığı halde donanım portlarına doğrudan erişebilir.
* **Mimari Önlem:** Scheduler context switch (`switch_to`) sırasında, yeni sürecin CSpace'inde `IO` yetkisi yoksa TSS IOPB ofseti derhal `0xFFFF` (tüm portlar yasak) yapılmalı; `IO` yetkisi varsa yalnızca o sürecin izinli port maskesi TSS'e yüklenmelidir.

---

## 4. Kapanış Eksikleri (Architectural Closing Gaps)

Mevcut dondurulmuş sözleşmelerin eksiksiz uygulanabilmesi için karara bağlanması gereken 5 temel mimari mekanizma aşağıdadır:

```
+-------------------------------------------------------------------------------+
|                       MİMARİ KAPANIŞ GEREKSİNİMLERİ                           |
+-------------------------------------------------------------------------------+
| 1. Process Exit Temizliği     --> Auto-Revoke + Channel Hangup Protokolü      |
| 2. Cooperative Cancellation   --> SYS_IPC_CANCEL + Thread Cancel Flag Sözleşmesi|
| 3. Dequeue Hata Raporlama     --> Per-Capability Slot Status ABI Düzenlemesi  |
| 4. CSpace Slot Yapısı         --> Bounded Static Slot Array vs Multi-Level    |
| 5. Bootstrap Seeding Modeli   --> Initrd ELF Loader Minimal Cap Dağıtım Şeması|
+-------------------------------------------------------------------------------+
```

### 4.1. Süreç Sonlanma ve Kanal Askı (Hangup) Protokolü
Bir süreç sonlandığında işletilecek temizlik sırası kesinleştirilmelidir:
1. Sürecin sahip olduğu tüm CSpace slotları taranır.
2. Sahip olduğu kök capability'ler (`Root Nodes`) için `revoke` tetiklenir (tüm çocuk grant'lar anında düşer).
3. Bağlı olduğu IPC kanallarına çekirdek tarafından `HANGUP` sinyali işlenir; kanalın diğer ucunda bekleyen veya yeni mesaj göndermek isteyen süreçlere anında `IPC_NO_SERVER` döner.

---

### 4.2. İşbirlikçi IPC İptali (Cooperative Cancellation) Yürütme Mekanizması
`IPC_CANCELLED` hatasının nasıl tetikleneceği sözleşmede netleştirilmelidir:
- İptal mekanizması asenkron bir sinyal yerine bir capability yetkisi üzerinden işletilmelidir: İptal hakkına sahip olan süreç, hedef iş parçacığının IPC beklemesini sonlandırmak için `SYS_IPC_CANCEL(target_thread_cap)` çağrısı yapar.
- Hedef iş parçacığı çekirdekte `BlockedOnIpc` durumundayken bu bayrağı görür, bekleme kuyruğundan çıkar ve kullanıcı uzayına `IPC_CANCELLED` hata koduyla döner.

---

### 4.3. CSpace Bellek Modeli ve Slot Tahsis Stratejisi
Her sürecin CSpace yapısının mimari tasarımı belirlenmelidir:
- **Öneri:** v0.x aşaması için sabit boyutlu (örn. 256 slotluk) düz dizi tablosu (`Fixed Array CSpace`) kullanılmalıdır. Her slot `generation: u32`, `rights: u16`, `node_ptr: *mut CapNode` içerir. Bu model deterministik $O(1)$ erişim sağlar ve dinamik bellek tahsisi parçalanmasını (fragmentation) engeller.

---

### 4.4. Çekirdek İlk Tohumlama (Bootstrap Seeding) Sözleşmesi
Sistem açılışında çekirdekten kullanıcı uzayına yetki aktarım sırası belirlenmelidir:
1. Çekirdek açılışta donanım kaynaklarını (bellek yöneticisi, kesme denetleyicisi, IOPB) kapsayan kök capability'leri üretir.
2. Gömülü ELF başlatıcısı (`initrd bootstrap`), `init` / `supervisor` sürecini oluştururken bu kök yetkileri asgari düzeyde sınırlandırarak `init` sürecinin CSpace'ine aktarır.
3. `init` süreci, sürücü servislerini (RTL8139 ağ, ATA disk) başlatırken yalnızca ihtiyaç duydukları port ve bellek yetkilerini `GRANT` ile dağıtır.

---

### 4.5. Ertelenen Konular (DEFERRED Envanteri)
Aşağıdaki konular resmi kararla ertelenmiştir; v0.x kapsamında uygulanmayacak, ileri aşamalarda ele alınacaktır:
- Priority Donation (STEP 8)
- Lend expiry (timer bağımlı geri alma; v0.x'te yalnızca açık `return`)
- SMP çekirdek aktivasyonu (tek çekirdek preemptive scheduler korunur)
- Nested / chained donation
- Detaylı device-service framework

---

## 5. Biçimsel Doğrulama Yaklaşımı Önerisi

Tanımlanan invariantların ve durum geçişlerinin kod seviyesinde doğrulanması için 3 katmanlı bir doğrulama matrisi uygulanmalıdır:

```
+-------------------------------------------------------------------------------+
|                       3 KATMANLI DOĞRULAMA PİRAMİDİ                           |
+-------------------------------------------------------------------------------+
|                                                                               |
|         [Katman 3: Kani Formal Verification]                                  |
|         - SAT/SMT tabanlı matematiksel model denetimi                         |
|         - Bitmask hak azaltma ve epoch monotonikliği kanıtı                  |
|                                                                               |
|         [Katman 2: Loom & Concurrency Model Checking]                         |
|         - In-flight claim, race conditions, atomic slot lifecycle             |
|         - Tüm olası thread interleaving permütasyonlarının testi              |
|                                                                               |
|         [Katman 1: Host-Tested Property-Based Testing (Proptest)]             |
|         - 1.000.000+ rastgele CSpace ve Lineage ağaç operasyonu               |
|         - CAP_INV-1'den CAP_INV-18'e kadar her adımda invariant denetimi     |
|                                                                               |
+-------------------------------------------------------------------------------+
```

### Katman 1: Host-Tested Özellik Tabanlı Testler (Property-Based Testing - `proptest`)
* `cap.rs` ve `lineage.rs` mantığı `#![no_std]` çekirdekten bağımsız olarak standart Rust host ortamında derlenir.
* `proptest` kütüphanesi kullanılarak rastgele oluşturulan $N$ adet süreç, CSpace, Grant, Transfer, Revoke, Close ve Lend adımı işletilir.
* Her adımdan sonra `CAP_INV-1` (hak azaltma), `CAP_INV-3` (generation artışı) ve `CAP_INV-4` (epoch doğruluğu) durum invariantları tam ağaç yürüyüşü ile doğrulanır.

### Katman 2: Eşzamanlılık ve Yarış Durumu Model Denetimi (`loom`)
* Çok iş parçacıklı erişim ve in-flight claim senaryoları `loom` simülatörü altında test edilir.
* Bir iş parçacığı capability kullanırken diğerinin eşzamanlı `revoke` veya `close` yaptığı tüm olası çizelgeleme (interleaving) durumları taranarak veri yarışı (data race) ve Use-After-Free oluşmadığı kanıtlanır.

### Katman 3: Çekirdek İçi Çalışma Zamanı Denetimleri (Debug Invariant Assertions)
* `debug_assertions` açıkken her syscall giriş ve çıkışında CSpace bütünlüğü denetlenir:
  ```rust
  debug_assert!(derived_rights.is_subset_of(parent_rights), "CAP_INV-1 VIOLATION");
  debug_assert!(slot.generation != 0, "CAP_INV-3 VIOLATION");
  ```

---

## 6. Zarar ve Öncelik Sıralı Aksiyon Listesi

Aşağıdaki aksiyon planı, mimarinin güvenliğini en hızlı ve en sağlam şekilde kapatmak üzere önceliklendirilmiştir:

```
+----------+-------------------------------------------------------+-------------+
| Öncelik  | Aksiyon Maddesi                                       | Hedef       |
+----------+-------------------------------------------------------+-------------+
| CRITICAL | 1. Process Exit CSpace & IPC Auto-Revoke Temizliği    | Aşama 5.2   |
| CRITICAL | 2. Dequeue Anı Revocation Payload Ayrışımı (ABI)     | Aşama 5.2   |
| HIGH     | 3. Context Switch Sırasında TSS IOPB Senkronizasyonu  | Aşama 5.0   |
| HIGH     | 4. RAII `CapClaim` Mekanizması ile In-Flight Güvenlik | Aşama 5.1   |
| MEDIUM   | 5. CSpace Sabit Dizi ve Slot Kotası Sınırlandırması   | Aşama 5.2   |
| MEDIUM   | 6. Host Ortamında `proptest` Invariant Test Süiti    | Aşama 5.3   |
+----------+-------------------------------------------------------+-------------+
```

### 1. [CRITICAL / P0] Process Exit Otomatik Temizlik ve Kanal Askı (Hangup) Entegrasyonu
* **Gerekçe:** Süreç çöktüğünde veya kapandığında yetim (orphan) soy ağaçlarının kalması sistemde bellek sızıntısına ve geçersiz capability kullanımına yol açar.
* **Uygulama:** `task/process.rs` içindeki çıkış yordamına `cspace_destroy_and_revoke_all()` ve `ipc_channel_hangup()` adımları eklenmelidir.

### 2. [CRITICAL / P0] Dequeue Anında Revocation ve Mesaj Üstbilgisi Durum Ayrışımı
* **Gerekçe:** Mesaj içindeki capability iptal edildiğinde veri yükünün kaybolması sunucu/istemci protokollerini kilitler.
* **Uygulama:** `SYS_IPC_RECV` dönüşünde capability durumunu belirten ayrık statü alanı ABI'ye eklenmeli; sessiz drop engellenmelidir.

### 3. [HIGH / P1] Context Switch Sırasında TSS IOPB İzolasyonunun Sağlanması
* **Gerekçe:** G/Ç port yetkilerinin süreçler arası sızması donanım güvenliğini tamamen bozar.
* **Uygulama:** `switch_to` makrosunda yeni sürecin `IO` yetkisi yoksa TSS IOPB ofseti `0xFFFF` yapılarak portlar donanımsal olarak kapatılmalıdır.

### 4. [HIGH / P1] RAII `CapClaim` Mekanizması ile Syscall İçi Güvenlik
* **Gerekçe:** Syscall yürütülürken asenkron gelen `revoke` çağrılarının Use-After-Free oluşturmaması gerekir.
* **Uygulama:** Çekirdek nesne erişimleri `Arc` benzeri atomik referans sayacı tutan `CapClaim` koruyucusu arkasına alınmalıdır.

### 5. [MEDIUM / P2] CSpace Slot Kotası ve DoS Koruma Bariyeri
* **Gerekçe:** Kötü niyetli süreçlerin IPC üzerinden hedef sunucunun CSpace slotlarını tüketmesi engellenmelidir.
* **Uygulama:** CSpace başına sabit sınır (örn. 256 slot) konulmalı ve kota aşımlarında deterministik hata dönülmelidir.

### 6. [MEDIUM / P2] Host Ortamında `proptest` ile Biçimsel Invariant Doğrulama Testleri
* **Gerekçe:** Mimari invariantların (CAP_INV-1 .. 18) kod değişikliklerinde gerileme (regression) oluşturmadığının sürekli kanıtlanması gerekir.
* **Uygulama:** `tests/cap_invariants.rs` test modülü yazılarak CI/CD süreçlerine entegre edilmelidir.

---

<!-- GOAL_COMPLETE -->
