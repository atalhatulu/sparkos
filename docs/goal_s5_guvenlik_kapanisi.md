# /goal — SparkOS Asama 5.5: Üç Güvenlik Kapanışı (FROZEN sözleşme gereklilikleri)

> Ver KURALLARI önce oku. Bu dosya SparkOS capability mikrokernelinde, Aşama 5
> kodda verilmiş ama tam kapatılmamış 3 güvenlik açığını kapatacak. Her üçü de
> FROZEN sözleşmenin (CAPABILITY_MODEL / IPC_CONTRACT) GEREĞİDİR — tasarım kararı
> değil, mevcut sözleşmenin uygulanması.
>
> **CRİTİK:** Yerel dosyaları OKUYAMAZSIN (Cloud). Aşağıdaki kaynak parçaları
> güncel koddan birebir alındı. Bunlara dayanarak tutarlı kod üreteceksin.
> Yerel dosyaya YAZMA — tüm değişiklikleri cevabında diff/code block olarak döndür.
> `<!-- GOAL_COMPLETE -->` ile bitir.
>
> Çelişki çıkarsa FROZEN sözleşmeyi değiştirme — DUR, çelişkiyi raporla.

---

## 0. Kural Özeti (FROZEN sözleşme)

1. **Sessiz drop YASAKTIR.** Revoke edilmiş capability taşıyan mesaj alıcıya deterministik hata ile bildirilir (CAP_INV-11, CAP_INV-12).
2. **Revoke ≠ Free, revoke ≠ cancel, close ≠ revoke.** Resource yalnız refcount==0 iken free.
3. Hata kodları tek kaynak: cap katmanı `CAP_INVALID / CAP_REVOKED / CAP_NO_RIGHTS`; IPC katmanı yalnız `IPC_TIMEOUT / IPC_CANCELLED / IPC_NO_SERVER / IPC_QUEUE_FULL / IPC_INVALID_MESSAGE`. `SERVER_DIED`, `PERMISSION_DENIED` TEKRARLANMAZ.
4. `derived_rights ⊆ parent_rights` her zaman. Genişletme yasak (aşağı 5'in yeni API'si dışında).
5. Donanım erişimi (IOPB/MMIO/DMA) yalnız capability ile; CSpace'de IO hakkı yoksa port kapalı (CAP_INV-14).

---

## 1. GÖREV A — Process-Exit Otomatik Temizlik + Kanal Hangup (P0, CAP_INV-13)

**Sorun:** `exit_current()` süreci Terminated yapıyor ama **CSpace'ini temizlemiyor**.
Süreç çökünce/çıkınca root olduğu grant tree'leri ölmüyor (zombie lineage), IPC kanallarının diğer ucu habersiz kalıyor. → bellek sızıntısı + geçersiz cap kullanımı.

### Mevcut kaynak (gerçek, güncel):

```rust
// src/task/process.rs (stil referansı — exit_current'in koop-resume dalı)
let exec_ctx: Option<RegisterContext> = EXECUTOR_RESUME.lock().take();
if let Some(mut ctx) = exec_ctx {
    {
        let mut s = SCHEDULER.lock();
        if let Some(pid) = s.current {
            if let Some(p) = s.table.get_mut(&pid) {
                p.state = ProcessState::Terminated;
                p.exited = true;
                crate::task::KILLED_PROCESSES.lock().push(pid);
            }
        }
        s.current = None;
    }
    ...
}
```

### Capability katmanında mevcut API'ler (gerçek):

```rust
// src/cap.rs
pub static ROOT_CAP: Once<CapHandle> = Once::new();
pub fn root_cap() -> Option<CapHandle>            // ROOT_CAP.get().copied()
pub fn create_object(kind: ObjectKind) -> Result<CapHandle>
pub fn grant(parent: CapHandle, req: Rights) -> Result<CapHandle>
pub fn transfer(src: CapHandle, req: Rights) -> Result<CapHandle>
pub fn revoke(cap: CapHandle) -> Result<()>
pub fn close(cap: CapHandle) -> Result<()>
pub fn check_rights(cap: CapHandle, needed: Rights) -> Result<()>
pub fn bootstrap_root() -> Result<CapHandle>       // ROOT_CAP.call_once ile doldurur
```

### Yapılacak (yalnız bunlar):

1. **`src/cap.rs`'e `destroy_owned(cap: CapHandle)` ekle** — capability'nin sahip olduğu tüm türetilmiş subtree'yi deterministik temizler:
   - `revoke(cap)` çağır (linage'i çocukla birlikte öldürür) — mevcut `revoke` yeterliyse YENİ API YAZMA, `revoke`'u kullan.
   - Sadece o process'in kendi CSpace slotunu `close` et.
   - FROZEN gereği: referans sayımına dokunma, resource'u `refcount==0` değilse free etme.
2. **`src/ipc.rs`'e `hangup_channel_for_pid(pid: u32)` ekle** — verilen process'in sahip olduğu endpoint'lerin kanal görünürlüğünü kapatır. Basit model: ENDPOINTS registry'den o process'e ait endpoint'leri `remove` et; bu channel'a `send` yapan başka bir process `CAP_INVALID` (endpoint handle stale) dönsün. Gerçekte hangup sinyali taşımak yerine v0.x'te "kanal kaldırıldı" determinizmi kabul edilebilir — AMA sessiz drop YASAK: kaldırılan kanala bekleyen mesajlar hiçbir receiver'a sessizce geçmemeli.
3. **`exit_current()`'in iki dalına (koop-resume + normal) entegre et:** KILLED_PROCESSES'e eklerken → `destroy_owned(process'in root cap'i)` + `hangup_channel_for_pid(pid)`. Process başına root cap nasıl izleniyorsa onu kullan (ROOT_CAP değil — her process ayrı root'a sahip olabilir; mevcut modelde tek global ROOT_CAP varsa, process'in CSpace'ini temizlemek için TÜM slotlarını revoke+close et — eksiksiz döngü).
4. `cap.rs` ve `ipc.rs`'e ünite testleri yaz: (a) çıkış sonrası subtree revoke, (b) hangup sonrası `send` CAP_INVALID, (c) bekleyen mesaj sessiz drop OLMADIĞI (deterministik hata).

---

## 2. GÖREV B — Dequeue Anında Revocation Payload Ayrışımı (P0, CAP_INV-7/11)

**Sorun:** IPC kuyruğunda bekleyen mesajın capability'si revoke edilirse, `SYS_IPC_RECV`
ile tam mesaj reddediliyor (`Err` dönerse) → **payload (veri) da kayboluyor**.
Şöyle: capability ölü diye veri de mi ölüyor? Handler deadlock'a giriyor.
FROZEN diyor: payload teslim edilir, capability durumu AYRI raporlanır.

### Mevcut kaynak (gerçek):

```rust
// src/ipc.rs — mevcut zarf & modlar
pub enum TransferMode { None, Transfer, Lend }
pub struct CapMessage<M> {
    pub payload: M,
    pub capability: Option<CapHandle>,
    pub transfer_mode: TransferMode,
}
// CapChannel::send / recv / try_recv / try_send — cap::check_rights_for_object(...)
// raw_ipc_recv(ep_id, receiver_cap) -> cap::Result<CapMessage<Vec<u8>>>
//   → channel.try_recv(receiver_cap)?

// src/syscall.rs — mevcut köprü
fn sys_ipc_recv(ep_id, buf_ptr, max_len, out_cap_ptr) -> u64 {
    ...
    match crate::ipc::raw_ipc_recv(ep_id as u32, receiver_cap) {
        Ok(msg) => copy_ipc_msg_to_user(msg, out_buf, out_cap_ptr, max_len),
        Err(cap::CapError::NoRights) => EACCES,
        Err(_) => u64::MAX,
    }
}
// copy_ipc_msg_to_user: payload'u kopyalar + capability varsa out_cap_ptr'ye slot+gen yazar
```

### Yapılacak (yalnız bunlar):

1. **`sys_ipc_recv` / `sys_ipc_try_recv` dönüş sözleşmesini değiştir** (ABI): 
   - Capability **meşru** (valid, doğrulanabilir) ise → mesaj teslim + `out_cap_ptr`'ye geçerli handle.
   - Capability **mesaj kuyruktayken revoke edilmiş** ise → **payload YİNE de teslim edilir**; capability yazılmaz; alıcıya durum ayrı bir çıkış parametresiyle `CAP_REVOKED` olarak bildirilir. **Sessiz drop YOK.**
   - Capability handle hiç yoksa → `CAP_INVALID` yerine "cap yok" durumu (mevcut davranışı koru).
2. Yeni return channel: mevcut int dönüşü korunur, ek durum bayrağını nasıl taşıyacağını belirle:
   - Öneri: `copy_ipc_msg_to_user` bir `out_cap_status_ptr` alanına u32 durum yazar (`0=ok/cap yok`, `1=CAP_REVOKED`), dönüş değeri yine teslim edilen bayt sayısı (payload). Sistemin versiyon/unused syscall arg ile geriye dönük uyumluluğu koru.
   - IPC hata kodu envanterine GİRİŞ yok — bu bir "hata" değil, "partial delivery" durumudur. FROZEN kural 3'e aykırı davranma.
3. Dequeue momentinde hangi kontrol çalışır? — `syscall.rs` köprüsünde (kernel/user sınırı) değil, `raw_ipc_recv`/`CapChannel::recv` seviyesinde yap: `CapMessage.capability`'yi `cap::check_rights(cap_when_picked, Rights::empty())` ile doğrula; `Err(Revoked)` → durumu işaretle ama payload'ü bırak. Kernel nesne ömrü için `refcount` güvenliğini koru (RAII claim paterni; yeni CapClaim API'si gerekiyorsa minimal tanımla).
4. `ipc.rs` + `syscall.rs`'e testler: revoked-cap mesajı → payload alınır, durum CAP_REVOKED, handle geçersiz.

---

## 3. GÖREV C — Context Switch TSS IOPB İzolasyonu (P1, CAP_INV-14)

**Sorun:** `serdrv` (pid 2) `sys_ioperm` ile COM1 portlarına (0x3F8..0x3FF) TSS IOPB'de
izin açtı. Round-robin scheduler başka bir process'e geçince TSS IOPB **yenilenmiyor** →
o process de IO portlarına erişebilir (yalnızca IO capability'si olmadan). Donanım sızıntısı.

### Mevcut kaynak (gerçek):

```rust
// src/gdt.rs — tam
pub fn allow_port_range(start: u16, end_inclusive: u16) { /* bit temizle (aç) */ }
pub fn deny_port_range(start: u16, end_inclusive: u16)  { /* bit EŞLE (kapat) */ }
pub fn reset_io_bitmap() { unsafe { TSS_DATA.io_bitmap = [0xFF; 8192]; } }
// TSS_DATA.io_bitmap: [u8; 8192] — varsayılan 0xFF (TÜM portlar kapalı)
// TssWithIopb { tss, io_bitmap: [u8;8192], trailing_byte: 0xFF }
```

### Switch-to / scheduler bağlamı (hangi dosyada olduğunu doğrula — process.rs task/switch bölümü):

- `arm_quantum()`, `jump_to_initial(&ctx)`, scheduler `s.current` değişimleri process.rs'de.
- `switch_to` eğer ayrı makro/register-context sıçraması ise onu bul; context değişiminin yapıldığı tek yer orası.

### Yapılacak (yalnız bunlar):

1. **Bağlam değişiminde IO bitmap'ini process'e göre senkronla.** Her process'e port izni bilgisi nasıl tutuluyorsa (syscall_cap / IO capability / simge) ona göre:
   - Yeni process'in CSpace'inde `IO` yetkili `create_device_ports` capability'si **yoksa** → `gdt::reset_io_bitmap()` (TÜM portlar kapanır — donanım #GP verir). Varsayılan + güvenli.
   - Yeni process'in `IO` yetki kap'ı **varsa** → o process'in izinli `(start, end)` aralığını TSS IOPB'ye `allow_port_range` ile tekrar yükle (önce reset).
   - Context switch'in **her** noktasında (coop-resume dahil) çalıştığından emin ol.
2. Capability-model basitliği: per-process `port_range: Option<(u16,u16)>` gibi bir alan process struct'ına eklenirse, `SYS_IOPERM` onu da güncelesin (tek kaynak doğruluğu için). TSS'e yazmak yan etki, gerçek kaynak process'in izin rengidir.
3. Test: bağlam değişimi sonrası `reset_io_bitmap` çağrıldığını; IO kap'lı process koştuğunda aralığın yeniden açıldığını doğrula (host unit + QEMU'da `[SERDRV]` yine çalışmalı, ama başka process IO portuna #GP almalı).

---

## 4. Test / Doğrulama Gereksinimi

- `src/cap.rs`, `src/ipc.rs`, `src/syscall.rs`, `src/task/process.rs` (ve gereken) içine `#[cfg(test)]` ünite testleri — mevcut test paterniyle uyumlu.
- **`scratch/run_cap_tests.sh`** ile çalışır olmalı: `env -u PYTHONPATH cargo test -- --test-threads=1` (repo /tmp izolasyonu kullanır; scratch/cap_test/src/lib.rs `#[path]` ile güncel src'i çeker). Toplam test sayısı artmalı (şu an 24).
- **QEMU regression korunmalı:** keysvc (SVC UP), `[SERDRV] alive`, `[USER-FAULT] recovered`, **PANIC YOK**, temiz timeout (EXIT=124). 5.5 kapanışları bu demoları kırmamalı, aksine tamamlamalı.
- Kök neden: `cargo build` 0 hata + 24+ test yeşil + QEMU demoları + PANIC yok.

---

## 5. DOKUNMA / YAZMA (Hard Quarantine)

Aşağıdakilere EL SÜRME, değiştirme, refactor ETME — sadece yukarıdaki 3 görev:

- `src/gdt.rs` mevcut `allow/deny/reset_io_bitmap`, `TSS_DATA` tanımı — koru (Görev C bunları ÇAĞIRIR, yeniden yazmaz).
- `src/infrastructure/lifecycle`, `src/gui/*`, `src/fs/*`, `src/net_*`, `src/usb*`, `src/ata*` — ilgisiz; DOKUNMA.
- DEFERRED kalemler (YAZMA, sadece DOKUNMA): Priority Donation, Lend expiry, SMP aktivasyonu, Nested/chained donation, DMA/IOMMU, devam eden device-service framework.
- FROZEN sözleşmenin hata kodu setini genişletme (B görevi durum bayrağı bir "hata kodu" DEĞİL, partial-delivery durumudur).
- `TransferMode`, `CapMessage`, `CapHandle`, `Rights` yapısal tanımlarını BOZMA — alan EKLEME/silme (B görevi bunları kullanır, yeniden tanımlamaz).

---

## 6. Teslim Biçimi

Cevabında, HER GÖREV İÇİN ayrı markdown kod bloğu:

```
## Görev A — src/cap.rs + src/ipc.rs + src/task/process.rs
<rust diff veya tam yeni kod, mevcut kaynakla tutarlı>
```

Implementasyon taslağı yerine **uygulanabilir tam kod** isteniyor. Sen Cloud'dasın,
yerel dosyayı göremezsin — verilen kaynağa dayanan mevcut API'leri CAĞIR, uydurma
API üretme (control et: `grep` edemezsin ama listedeki imzaları kullan). Bir şey
yoksa ve şart ise, minimal yeni API'yi açık işaretle ve gerekçesini yaz.

`<!-- GOAL_COMPLETE -->` ile bitir.
