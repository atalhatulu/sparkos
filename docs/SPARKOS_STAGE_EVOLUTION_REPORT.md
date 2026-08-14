# SPARKOS — Capability Microkernel Evrimi: Aşama 5.0 ve IOPB Raporu

**Tarih:** 2026-08-14  
**Tamamlanan Aşamalar:** Aşama 1, 2.0, 2, 3 (Pointer Hardening), 4 (Capability IPC), 5.0 (TSS IOPB & Ring 3 IPC Syscalls)

---

## 1. Mimari Dönüşüm: IOPL Yerine TSS IOPB (I/O Permission Bitmap)

### Neden IOPL=3 Reddedildi?
- `IOPL=3` kullanımı, sürece tüm 65.536 I/O portunu (PIC, PIT, ATA, ACPI, güç yönetimi vb.) sınırsız açarak mikroçekirdeğin "en az yetki" (least privilege) kuralını tamamen ihlal eder.
- Bu nedenle x86_64 donanım seviyesinde çalışan **TSS IOPB** mimarisine geçilmiştir.

### Uygulanan TSS IOPB Mimarisi (`src/gdt.rs`)
- **Veri Yapısı:** `TssWithIopb` (104 bayt `TaskStateSegment` + 8192 bayt `io_bitmap` + 1 bayt `0xFF` trailing bayt).
- **GDT 64-bit TSS Descriptor:** Descriptor limiti `size_of::<TssWithIopb>() - 1` (8296 bayt) olarak 16-baytlık System Descriptor şeklinde GDT'ye yüklendi.
- **`iomap_base`:** TSS başlangıcından bitmap'e olan 104 baytlık ofset (`0x68`) donanıma bildirildi.
- **Port Güvenliği:** Varsayılan olarak tüm 8192 bayt `0xFF` (tüm portlar Ring 3 için `#GP` ile yasaklı).
- **Context Switch Entegrasyonu (`src/task/process.rs`):** Süreç PCB'sinde `allowed_ports: Option<(u16, u16)>` alanı eklendi. Yalnızca `Rights::IO` capability'sine sahip sürücü süreci çalıştığında (örn. Serial: `0x3F8..=0x3FF`) ilgili bitler `0` (izinli) yapılır; süreç değiştiğinde bitmap sıfırlanır (`reset_io_bitmap`).

---

## 2. Ring 3 Mikroçekirdek IPC Syscall Köprüsü (`src/syscall.rs` & `src/ipc.rs`)

1. **`SYS_IPC_SEND` (20):**
   - Kullanıcı tamponu `validate_user_ptr` ile doğrulanır.
   - Endpoint üzerinde `Rights::WRITE` capability kontrolü yapılır.
   - `TransferMode::Transfer` (mülkiyet devri) veya `TransferMode::Lend` (ödünç) yetki aktarımı desteklenir.
2. **`SYS_IPC_RECV` (21):**
   - Kullanıcı tamponu `validate_user_ptr_mut` ile doğrulanır.
   - Endpoint üzerinde `Rights::READ` capability kontrolü yapılır.
   - Mesajla gelen capability handle varsa güvenle kullanıcı alanına yazılır.
3. **`SYS_IOPERM` (22):**
   - Sürecin `Rights::IO` capability'si doğrulanır.
   - Belirtilen dar port aralığı için TSS IOPB güncellenir.

---

## 3. Doğrulama Kanıtları

- **Derleme:** `cargo build` **0 hata**.
- **Bootimage:** `cargo bootimage` başarıyla üretildi.
- **QEMU Boot & Regression:** Paging, TSS/GDT, Syscall Dispatcher, PIT Timer, Interrupts ve IPC Producer/Consumer testleri sıfır regresyonla çalıştı.
