SparkOS Asama 1 — Capability Core implementasyonu (bağımsız modül üretimi)

Senin görevin, yerel dosya sistemine BAĞIMLI olmadan, tamanen prompt içinde verilen
frozen sözleşmeye uygun, SAF RUST bir capability core modülü yazmaktır. Dosya yazmana
gerek YOK — implementasyonu tam ve çalışabilir halde yanıtın içinde döndür. Sonra o
modülü SparkOS repo'suna yerleştireceğim.

## Önemli: bu kod YEREL kernel'e bağlanmayacak

- x86_64, registers, interrupts, syscall API'lerine import ETME. 
- SADECE `alloc` (vec, boxed, collections) + `spin::Mutex` + `core` kullan.
- Böylece modül bağımsız ve host-tarafı `cargo test` ile test edilebilir olur.
- `#[cfg(test)]` unit testlerini DE modül içine göm.

## Frozen sözleşme (DEĞİŞMEZ — bu, dokümandan çıkarılan kesin semantik)

### Veri yapıları
```
pub struct CapHandle { pub slot: u32, pub generation: u32 }
pub struct Rights (u32 bitmask): READ, WRITE, MAP, IO, DMA, TRANSFER, GRANT, DESTROY, EXECUTE, MANAGE
pub struct CapNode { parent: Option<u32>, epoch: u64 }   // lineage node
pub struct CapObject { kind: ObjectKind, refcount: u64, generation: u64, valid: bool }
pub enum ObjectKind { Memory, Device, Endpoint, Generic }  // örnek türler
```

### Capability modeli (frozen)
- **Capability ≠ resource.** Handle, resource'a erişim hakkı; resource pointer değil.
- **3 katmanlı lifetime:**
  - Handle → generation (slot reuse koruması)
  - Authority → lineage + epoch (revoke)
  - Resource → claim / refcount
- **Handle reuse:** `handle.generation != slot.generation` → `Err(Invalid)` (CAP_INVALID).

### Revocation (lineage + epoch)
- `revoke(h)` → handle + TUM child lineage DEAD. Sibling'ler ETKİLENMEZ.
- Mekanizma: node.epoch++ (O(1)). Child'lar lazy walk ile tespit.
- `deref` path doğrulaması: çıkmadan önce lineage zinciri boyunca her node'un epoch'unu
  snapshot ile doğrula. Kırık → `Err(Revoked)` (CAP_REVOKED).
- **No resurrection:** REVOKED node bir daha LIVE olamaz.
- `revoke ≠ free`, `revoke ≠ cancel`, `close ≠ revoke`.

### Rights
- `derived_rights ⊆ parent_rights` (INV-9). grant/transfer yalnızca `req ∩ parent` verir.

### Operasyonlar (frozen semantik)
- **grant(parent_h, req)** → yeni child capability; parent aynen kalır; child lineage'a bağlanır
  (recall edilebilir); req ∩ parent haklar. Aynı parent'tan 2. grant → ayrı sibling.
- **transfer(src_h, req)** → capability'yi lineage'dan KOPARIR, YENİ ROOT yapar; src handle
  kapatılır; transfer edilen cap eski lineage revoke'undan ETKİLENMEZ.
- **lend(parent_h, req)** → geçici child; return(cap) → otomatik revoke. Lend'den sonra
  grant/transfer YAPILAMAZ (tek seviye). (Expiry/timer YOK — ertelendi, ekleme.)
- **reclaim(cap)** → lend return / revoke.
- **close(h)** → yalnızca o handle; slot free + gen++; türetilmişe DOKUNMAZ.
- **revoke(cap)** → handle + child lineage DEAD.
- **deref(cap, flags)** → CapAccess RAII guard döndürür; drop edilince refcount azaltır.
  Aynı resource'a iki deref → refcount 2; 2 guard drop → 0 → FREE.

### Object lifecycle
- Object header KALICI arena; FREE asla silmez, generation++ + valid=false.
- refcount==0 → FREE. Fiziksel resource serbest, header kalır.

### Concurrency (frozen)
- Tüm mutasyonlar (grant/transfer/revoke/close/lend/reclaim) TEK spinlock altında — atomik dilim.
- `deref` claim = generation check + epoch check + refcount++ AYNI KRİTİK DİLİMDE (FIX-1).
  Ayrı atomik işlem YOK.
- Epoch değişimi Ordering::AcqRel.

### Hata tipleri (asla karışmaz)
```
enum CapError { Invalid, Revoked, NoRights, NotFound, Exhausted, AlreadyExists }
```

### Public API (tam bu imzaları ver)
```
pub fn init()
pub fn create_object(kind: ObjectKind) -> Result<CapHandle>
pub fn grant(parent: CapHandle, req: Rights) -> Result<CapHandle>
pub fn transfer(src: CapHandle, req: Rights) -> Result<CapHandle>
pub fn lend(parent: CapHandle, req: Rights) -> Result<CapHandle>
pub fn reclaim(cap: CapHandle) -> Result<()>
pub fn close(cap: CapHandle) -> Result<()>
pub fn revoke(cap: CapHandle) -> Result<()>
pub fn deref(cap: CapHandle, flags: Rights) -> Result<CapAccess>
pub struct CapAccess { ... }  // RAII guard, Drop ile refcount düşür
```

## Test edilecek invariantlar (#[cfg(test)] içinde)
1. grant: derived ⊆ parent (fazla right isterse NoRights)
2. revoke child → sibling ETKİLENMEZ
3. revoke parent → child lineage ölür (lazy walk)
4. transfer → eski lineage revoke'undan ETKİLENMEZ
5. close → sadece kendi handle'ı, türetilmişi etkilemez
6. lend → return sonrası revoked; lend sonrası grant/transfer reddedilir
7. generation mismatch → Invalid
8. no resurrection: revoke'lu node tekrar deref edilemez
9. deref claim: iki deref → refcount 2; iki guard drop → 0 → FREE
10. object FREE: son deref drop'unda refcount 0 → FREE (header kalır, gen++)

## Teslim formatı
Yanıtında şunları ver:
1. **Kod bloğu:** tam `src/cap.rs` içeriği (module + impl + tests), tek kod bloğunda, derlenir.
2. **Kısa yorum:** hangi parçalar frozen uygulandı, hangileri bilinçli olarak DISARIDA (DEFERRED).
3. Eğer görev mantıksızsa veya sözleşmeyle çelişiyorsa YAPMA — çelişkiyi raporla, kendi başına
   mimari karar değiştirme.
