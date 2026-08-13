# SparkOS Kernel — /goal (L7 Drivers)

## Görev
`~/Documents/GitHub/sparkos` repo'sunda **L7 (Drivers)** seviyesini implement et. **SADECE aşağıdaki dosyalara dokun:**
- `src/pci.rs` — genişlet (PCI config, BAR, genel sürücü iskeleti)
- `src/usb.rs` — YENI dosya (USB host controller iskeleti)
- `src/display.rs` — YENI dosya (framebuffer/display driver iyileştirme sarmalayıcı)
- `src/keyboard.rs` / `src/mouse.rs` — sadece iyileştirme gerekiyorsa (opsiyonel)

**KESiN: `src/main.rs`'e DOKUNMA.** `pub mod` eklemelerini HERMES yapacak. Mevcut `src/syscall.rs`, `src/sync.rs`, `src/ipc.rs`, `src/fs.rs`, `src/ata.rs`, `src/rtl8139.rs`, `src/gui.rs`'e DOKUNMA (gui.rs mevcut ve büyük, onu bozma).

## Proje
- Rust freestanding x86_64 `no_std` kernel, `bootloader 0.9`, `x86_64 0.15`, `spin 0.9`
- Build: `cargo build` (0 error gerekir). **NOT: terminal komutlarını `env -u PYTHONPATH <cmd>` ile çalıştır.**
- L0-L5 tamam. `pci.rs` mevcut PCI tarama (`lspci`) yapıyor.

## Mevcut L7 Durumu
- `src/pci.rs` (91 satır) — PCI config space okuma, cihaz numaralandırma (`lspci`), vendor/device listesi.
- `src/gui.rs` (1063) — VBE/VESA framebuffer, backbuffer, pencereler, redraw. Çok sağlam, DOKUNMA.
- `src/keyboard.rs` (171), `src/mouse.rs` (155) — PS/2 sürücüleri çalışıyor.
- `src/rtl8139.rs` (191) — ağ NIC sürücüsü (L8'e ait ama var).
- **EKŞİK:** PCI'da IO BAR'ları okuma/yazma, DMA (bus mastering) yapılandırma, genel "sürücü" trait/iskeleti, USB host controller (UHCI/EHCI/xHCI) iskeleti, disk controller soyutlaması.

## Yapılacaklar
1. **`src/pci.rs` genişlet:**
   - PCI BAR (Base Address Register) okuma/yazma → mem/io BAR ayrımı ve boyut.
   - Bus mastering / DMA enable bit yapılandırma.
   - `PciDevice` yapısı + genel `Driver` trait iskeleti (örnek: `init()`, `probe(dev)`).
   - Keypressed mask / MSI (opsiyonel).

2. **`src/usb.rs` (yeni):**
   - USB host controller mimarisi özeti (UHCI/EHCI/xHCI) — gerçek donanım kod gerektirmez, iskelet + Register map yorumları.
   - `UsbHostController` trait, cihaz numaralandırma pencere (descriptor parsing iskeleti).
   - NOT: Gerçek USB donanım sürücüsü çok büyük; bu adımda SAĞLAM İSKELET + temel yapılar yeterli, çalışan tak-çalıştır beklenmez (QEMU USB opsiyonel).

3. **`src/display.rs` (yeni):**
   - Mevcut gui.rs'i sarmalayan, CRT/info, resolution değiştirme iskeleti, `DisplayInfo` yapısı.
   - `gui.rs`'e DOKUNMA — sadece onun üzerine inşa et.

4. **Opsiyonel:** `keyboard.rs`/`mouse.rs` iyileştirme (sadece hata varsa).

## Teknik
- `no_std` + alloc, memory-mapped I/O için `x86_64` crate'in `PhysAddr`/mmio helper'ları.
- Mevcut `pci.rs`'in API'sini oku, uyumlu genişlet.
- Yeni modüller bağımsız (main.rs'e bağlanmadan) derlenebilmeli — dokunulan mevcut dosyaların derlemesini bozma.

## Teslim
1. Kısa analiz: pci.rs mevcut yapısı, ne eklendi.
2. `src/pci.rs` (genişletilmiş) + `src/usb.rs` + `src/display.rs`.
3. `cargo build` çıktısı (0 error). Yeni modüller main'e bağlı değil → geçici test `pub mod` ile doğrula (main.rs'e ekleme, sadece geçici).
4. `git diff --stat`.

Claude: "Eğer görev mantıksız/yapılamazsa YAPMA — raporla. İddialarını önce dosyayı okuyarak doğrula."
