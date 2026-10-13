# Performans çalışması — Aşama 1: doğrulanmış başlangıç

Tarih: 2026-10-13T18:17:01.064669+00:00  
Repo: `/home/teha/Documents/GitHub/sparkos`  
Branch: `main`  
HEAD: `c26a32610523bc5d1a8fb76d682e570eca54a6af`

## Sonuç

Başlangıç doğrulaması tamamlandı. Normal ve regresyon imajları ayrı dizinlerde
üretildi. Son resize/focus/MRU düzeltmelerini içeren kaynakta **909 host testi** ve
her iki QEMU konfigürasyonunda **11 kernel regresyon grubu** geçti. Normal imajın
KVM masaüstü turu fault marker olmadan tamamlandı.

Bu aşamada kernel/GUI kaynak kodu değiştirilmedi. Önceden commit edilmemiş kullanıcı
ve geliştirme değişiklikleri korundu. HEAD tek başına bu başlangıcı temsil etmez.
Çalışma ağacındaki değişiklikler `starting-changes.patch`, 148 kaynak/yapılandırma/
belge dosyasının SHA-256 değerleri `source-sha256.json` ile kaydedildi. Derleme ve
testler sırasında bu dosyaların değişmediği doğrulandı. Manifest untracked kaynak
kimliğini de kaydeder; patch untracked içeriklerin yedeği değildir. Repo dışında
bir tam yedek alındığı iddia edilmiyor.

## Ortam

- Rust: `rustc 1.99.0-nightly (771916f90 2026-08-08)`
- Cargo: `cargo 1.99.0-nightly (b07e5a086 2026-08-07)`
- QEMU: `QEMU emulator version 11.1.2`
- Kernel hedefi: `x86_64-unknown-none`
- Geliştirme profili: opt-level=2, debug bilgisi/assertion/overflow kontrolü açık.
- Grafik: std VGA, 1280×720.
- Diskler: snapshot; asıl disk.img değiştirilmedi.
- Ağ: QEMU user networking, restrict=on.

## Test matrisi

| Test | Sonuç | Kanıt (target/performance-baseline altında) |
|---|---|---|
| Normal bootimage | Başarılı; 16 kernel uyarısı | normal-build.log |
| kernel-regression bootimage | Başarılı; 16 kernel uyarısı | regression-build.log |
| Host suite | 909 geçti, 0 hata | host-tests.log |
| QEMU TCG, 1 CPU / 128 MiB | 11 grup + ilk masaüstü frame'i | qemu-1cpu/serial.log |
| QEMU TCG, 2 CPU / 256 MiB | 11 grup + ilk masaüstü frame'i | qemu-2cpu/serial.log |
| Normal imaj, KVM, 2 CPU / 256 MiB | Gerçek giriş turu tamamlandı | desktop-kvm/trial.json |

Host harness uyarıları ayrıca mevcut: lib için 6, lib test için 152 uyarı.
Bu nedenle sonuç “uyarısız build” değildir. Host testlerinin bazıları mock model
kullanır; 909 testin tümü gerçek kernel davranışını sınayan testler değildir.

## Normal masaüstü görsel kontrolü

Gerçek QEMU klavye/fare olaylarıyla sürükleme, resize, minimize/restore, maximize,
Alt+Tab, terminal help, editör giriş/kapatma/iptal/discard, TaskMgr refresh,
Settings ve on terminal aç/kapat döngüsü uygulandı. Tur sonunda launcher yanıt verdi.

Özellikle şu üç ekran görüntüsü açılarak kontrol edildi:

- `desktop-kvm/03b-resized.png`: içerik yeni pencere boyutuna uyuyor.
- `desktop-kvm/03d-first-restore-attempt.png`: ilk Alt+Tab minimize pencereyi getiriyor.
- `desktop-kvm/05-switch-terminal.png`: arkadaki Files titlebar pasif renkte tutarlı.

Testte 29 ekran görüntüsü/adım kaydı var. Diğer adımların
logları ve görüntüleri aynı dizinde saklandı. Tüm uygulama özellikleri veya bütün
piksel çıktıları eksiksiz doğrulanmış sayılmamalı.

## İmaj kimlikleri

| İmaj | Boyut | SHA-256 |
|---|---|---|
| Normal | 1384448 bayt | `bfa196c8ba9b9fb77e8109c977c9131723b1f715c44ec4f006e605e3e4a49b78` |
| Regresyon | 1428992 bayt | `7bf5e6f30eb20b02efdadc20294fcda003709dc5131a5fa492e91c01d98f5d08` |

Normal yol: `target/performance-baseline/normal/x86_64-unknown-none/debug/bootimage-sparkos.bin`

Regresyon yolu: `target/performance-baseline/regression/x86_64-unknown-none/debug/bootimage-sparkos.bin`

İki imaj birbirinin üzerine yazılmıyor. Target dizini temizlenirse kanıtlar ve
imajlar kaybolabilir; kalıcı rapor bu dosyadır. `environment.json`, `images.json`
ve kaynak manifesti makine tarafından okunabilir kayıtları içerir.

## Tekrarlama

Repo kökünde:

```bash
mkdir -p target/performance-baseline
CARGO_NET_OFFLINE=true CARGO_TARGET_DIR="$PWD/target/performance-baseline/normal" cargo bootimage
CARGO_NET_OFFLINE=true CARGO_TARGET_DIR="$PWD/target/performance-baseline/regression" cargo bootimage --features kernel-regression
```

Host testleri, `scratch/cap_test` içinde:

```bash
cargo test --offline -Zbuild-std= --target x86_64-unknown-linux-gnu --target-dir /home/teha/Documents/GitHub/sparkos/target/performance-baseline/host -- --test-threads=1
```

Repo kökünde:

```bash
python3 tools/qemu_smoke.py --image target/performance-baseline/regression/x86_64-unknown-none/debug/bootimage-sparkos.bin --disk disk.img --output target/performance-baseline/qemu-1cpu --regression --cpus 1 --memory 128M
python3 tools/qemu_smoke.py --image target/performance-baseline/regression/x86_64-unknown-none/debug/bootimage-sparkos.bin --disk disk.img --output target/performance-baseline/qemu-2cpu --regression --cpus 2 --memory 256M
python3 tools/qemu_desktop_trial.py --image target/performance-baseline/normal/x86_64-unknown-none/debug/bootimage-sparkos.bin --disk disk.img --output target/performance-baseline/desktop-kvm --accel kvm
```

## Ölçüm sınırı ve sonraki aşama

Bu koşular doğruluk kontrolüdür. Derlemeler/test VM'leri eşzamanlı çalıştırıldı;
süreleri KVM/TCG performans farkı veya masaüstü gecikmesi olarak yorumlanamaz.
VNC istemci gecikmesi, p50/p95 frame süresi ve gerçek CPU kullanımı henüz ölçülmedi.
Mevcut piksel iş sayaçları zaman ölçümü değildir.

Sonraki iş: **Aşama 2, performans ölçüm altyapısı**. Önce zaman kaynağı ve
çözünürlüğü doğrulanmalı; ardından compositor, uygulama render, framebuffer
aktarımı ve executor tur süreleri düşük maliyetle ayrı kaydedilmeli. Ölçüm açık/
kapalı farkı kontrol edilmeli; per-frame seri log veya heap allocation eklenmemeli.
Performans koşuları tek VM ile ve sabit iş yükünde seri olarak yapılmalı.
