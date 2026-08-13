# SparkOS Kernel — /goal (L10 Multicore & Power)

## Görev
`~/Documents/GitHub/sparkos` repo'sunda **L10 (Multicore & Power)** seviyesini implement et. **SADECE aşağıdaki dosyalara dokun:**
- `src/smp.rs` — YENI dosya (SMP: çok çekirdek başlatma iskeleti)
- `src/acpi.rs` — YENI dosya (ACPI tabloları: RSDP/RDSP, MADT/APIC, FADT)

**KESiN: `src/main.rs`'e DOKUNMA.** `pub mod` eklemelerini HERMES yapacak. Mevcut diğer modüllere dokunma (sadece oku).

## Proje
- Rust freestanding x86_64 `no_std` kernel, `bootloader 0.9`, `x86_64 0.15`
- Build: `cargo build` (0 error). **NOT: terminal komutlarını `env -u PYTHONPATH <cmd>` ile çalıştır.**
- L0-L9 tamam. Tek çekirdek (BSP) çalışıyor; AP (Application Processor) başlatılmadı.

## Mevcut L10 Durumu
- **SMP YOK:** Kernel yalnızca BSP'de (boot işlemci) çalışıyor. Diğer çekirdekler başlatılmamış.
- **ACPI YOK:** Önyükleyiciden BIOS tablolarına erişim, MADT/FADT okuma yok.
- `memory.rs` paging kuruyor, `gdt.rs` per-core GDT. `interrupts.rs` timer/IRQ kuruyor. Şimdilik Tek CPU.

## Yapılacaklar
1. **`src/acpi.rs` (yeni):**
   - `Rsdp` yapısı: ACPI RSDP bulma (EBDA / 0x000E0000-0x000FFFFF arama, "RSD PTR " imzası), checksum doğrulama, XSDT/RSDT işaretçisi.
   - `ACPITable` genel başlık parser'ı (signature, length, checksum).
   - `Madt` (Multiple APIC Description Table): APIC adresi, **Local APIC** ve **I/O APIC** girişlerini parse et.
   - `Fadt` iskelet: PM_TMR, reset register (opsiyonel).
   - `Fadt`/`Madt` okuma fonksiyonları: RSDP→RSDT/XSDT→MADT zinciri.

2. **`src/smp.rs` (yeni):**
   - `CpuInfo`: apic_id, is_bsp, local_apic_ptr.
   - **AP başlatma iskeleti:** `start_ap(apic_id, trampoline_addr)` — her çekirdek için GDT/IDT/stack kurulumunu BSP'den sonra AP kernel girişi (trampoline) hazırlar. Gerçek çekirdek başlatma çok derin; iskelet + durum makinesi yeterli.
   - `LocalApic` iskeleti: `write_icr`, `read_apic_id`, `enable_lapic` (yapı + register offset'ler).
   - `CpuSet`/çekirdek başına veri: `PerCpu<T>` kavram iskeleti (mutex'li).
   - MADT'den APIC ID'leri okuma, `cpu_count()` raporlama.

## Teknik
- `no_std` + alloc. `unsafe` pointer erişimi ACPI fiziksel adresler için gerekli (fiziksel→sanal çevrim: mevcut `memory.rs`/`gui::PHYS_OFFSET` benzeri bak).
- ACPI tabloları hizalama/checksum doğrulama zorunlu.
- SMP iskeleti gerçek donanım başlatmayı ÇALIŞTIRMAYABİLİR ama yapısal olarak doğru olmalı. QEMU `-smp 2` ile ileride test edilebilir.

## Teslim
1. Kısa analiz: mevcut durum, ne eklendi.
2. `src/acpi.rs` + `src/smp.rs`.
3. `cargo build` çıktısı (0 error). Geçici `pub mod` ile doğrula (main.rs'e kalıcı ekleme yok).
4. `git diff --stat`.

AGY: "Eğer görev mantıksızsa YAPMA — raporla. İddialarını önce dosyayı okuyarak doğrula."
