# SPARKOS — Capability Microkernel Evrimi ve Aşama Raporu

**Tarih:** 2026-08-14  
**Durum:** Aşama 1, 2.0, 2, 3 (Pointer Hardening) ve 4 (Capability IPC) Tamamlandı / Doğrulandı

---

## 1. Tespit Edilen Riskler ve Üretilen Çözümler

### A. Tanımsız Davranış (UB) ve Eşzamanlılık Riski (`src/cap.rs`)
- **Risk:** `cap.rs` içerisinde `is_revoked` ve `revoke` fonksiyonlarında `&u64 as *const u64 as *const AtomicU64` şeklinde güvensiz (unsafe) pointer tür dönüşümü bulunuyordu.
- **Çözüm:** `CoreState` zaten tek bir `spin::Mutex` altında korunmaktadır. Eşzamanlı yarış riski olmadan `state.nodes[node_idx].epoch.saturating_add(1)` ve `state.nodes[curr].epoch > 0` güvenli koduna dönüştürüldü, tüm güvensiz bloklar ve gereksiz atomik dönüşümler temizlendi.

### B. Bellek & Pointer Doğrulama Açıkları (`src/syscall.rs`)
- **Risk:** `SYS_EXEC` ve `sys_write` içerisinde doğrulamadan bağımsız `core::slice::from_raw_parts` kullanımları bulunuyordu.
- **Çözüm:** `sec_mem::validate_user_ptr` API'si doğrudan dilim döndürecek şekilde entegre edildi. Kullanıcı adres alanı sınırları (`0..0x8000_0000`) ve her sayfanın `USER_ACCESSIBLE` / `WRITABLE` bayrakları doğrulanarak çiğ işaretçi manipülasyonu ortadan kaldırıldı.

### C. Aşama 4: Capability Destekli IPC (`src/ipc.rs`)
- **Risk:** Klasik kanal yapısı (`BlockingChannel`) yetki kontrolü yapmıyor ve mesaj içinde kaynak/yetki transferi sağlayamıyordu.
- **Çözüm:** `CapMessage<M>` ve `CapChannel<M>` mimarisi uygulandı:
  - Kanal uç noktası (`ObjectKind::Endpoint`) üzerinden `Rights::WRITE` ve `Rights::READ` denetimi.
  - `TransferMode::Transfer`: Mülkiyet aktarımı (lineage koparılarak alıcı yeni root yapılır).
  - `TransferMode::Lend`: Geçici ödünç (gönderici mesaj kuyruktayken geri alabilir; geri alınırsa alıcıda `CapError::Revoked` döner, payload güvenle teslim edilir).
  - Geriye dönük uyumluluk: Eski `BlockingChannel` ve `SYSTEM_CHAN` API'si bozulmadan korundu.

---

## 2. Doğrulama ve Test Kanıtları

1. **Kernel Derlemesi (`cargo build`):**
   - **0 Hata**, 6 önemsiz uyarı.
2. **Bootimage Üretimi (`cargo bootimage`):**
   - `bootimage-sparkos.bin` başarıyla üretildi.
3. **QEMU Boot & Fonksiyonel Doğrulama:**
   - Serial Çıktısı:
     ```text
     [OK] Serial port ready
     [OK] Virtual Memory (Paging) Initialized
     [OK] Syscall dispatcher initialized
     [OK] Capability core initialized (root capability)
     [OK] Interrupts enabled
     [IPC Producer 1] Sent: 1
     [IPC Producer 2] Sent: 3
     [IPC Consumer] Received: 1
     [IPC Consumer] Received: 3
     [IPC Producer 1] Sent: 2
     [IPC Producer 2] Sent: 4
     [IPC Consumer] Received: 2
     [IPC Consumer] Received: 4
     [IPC Consumer] Test complete.
     ```

---

## 3. Sıradaki Adımlar (Aşama 5 — Microkernel Driver İzolasyonu)

- **IRQ Notification Endpoint Mimarisi:** Sürücülerin user-space'e taşınabilmesi için kernel kesmelerinin (`IRQ`) IPC bildirimine dönüştürülmesi.
- **Port I/O & MMIO İzinleri:** Serial sürücüsü için I/O port (`0x3F8`) veya bellek sayfalarının capability ile sürece bağlanması.
