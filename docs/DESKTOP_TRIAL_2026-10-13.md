# Gerçek giriş olaylarıyla masaüstü denemesi — 13 Ekim 2026

## Sonuç

Masaüstü temel kullanım turunu crash veya kalıcı donma olmadan tamamlıyor. Ancak
sorunsuz/akıcı ürün seviyesinde olduğu söylenemez: eski odak renginin ekranda
kalması, resize sonrası içeriğin eski ölçüde görünmesi ve minimize sonrası ilk
Alt+Tab'ın beklenen pencereye geçmemesi gözlendi. Bunlar birim testlerinin
kaçırdığı gerçek etkileşim sorunlarıdır. Bu çalışma test/teşhis turudur; aşağıdaki
üç sorun bu turda düzeltilmiş sayılmamalıdır.

## Yöntem

- Normal, optimize geliştirme bootimage'ı yeniden derlendi; kernel-regression
  feature kapalı. Uygulama handler'ları doğrudan çağrılmadı.
- QEMU 2 vCPU, 256 MiB, std VGA, 1280×720. KVM ve TCG ayrı çalıştırıldı.
- QMP üzerinden QEMU `sendkey`, `mouse_move`, `mouse_button` kullanıldı. Olaylar
  emüle klavye/fare, IRQ ve normal input dispatch yolundan geçti.
- Diskler snapshot modunda; asıl disk.img değiştirilmedi. Ağ restrict=on.
- Gerçek framebuffer ekran görüntüleri, seri loglar ve adım zamanları saklandı.
- Her tur yaklaşık 18–21 saniyelik scripted kullanım içerir. Bu süreler kasıtlı
  beklemeler içerdiğinden KVM/TCG hız karşılaştırması değildir. Uzun süreli soak,
  60 FPS ölçümü veya uçtan uca insan giriş gecikmesi testi yapılmış değildir.

## Denenen akışlar

| Akış | Gözlem |
|---|---|
| Açılış ve ilk desktop frame | KVM ve TCG'de geldi |
| Dosyalar penceresini 40 hareket olayıyla sürükleme | 8 ara görüntüde beklenen konuma ulaştı |
| Titlebar çift tıklama ile büyütme | İlk KVM/TCG turlarında çalıştı |
| Alt+Tab ile Terminal'e geçiş | Çalıştı; eski pencerenin başlık rengi kısmen kaldı |
| Terminal'e `help` yazıp Enter | Komut çıktısı göründü |
| Launcher'dan Editor açıp `test 123` yazma | Metin ve dirty işareti göründü |
| Alt+F4'ü iki kez gönderme | Unsaved onayı kaldı; belge kapanmadı |
| Escape ile iptal | Belge/metin korundu |
| Tekrar kapatma, D ile discard | Editor kapandı |
| Task Manager açma, Down/F5, kapatma | Liste göründü ve UI yanıt verdi; kill denenmedi |
| Settings tema değiştirme, System kategorisi | Yanıt verdi ve kapandı |
| Terminal'i 10 kez açıp kapatma | Her tur sonunda masaüstü yanıt verdi |
| Son bekleme sonrası launcher açma | Yanıt verdi |
| Köşeden resize | Çerçeve büyüdü; içerik eski boyutta kaldı |
| Minimize ve ardından Alt+Tab | İlk Alt+Tab, zaten odaklı Terminal'de kaldı |

İlk üç turda seri log başına 15 uygulama açılışı ve 13 pencere kapanışı kaydedildi;
başlangıçtaki iki pencere kaldı. Bu sayım ve HUD'un eski bellek değerine dönmesi,
tüm heap/IPC kaynakları için sızıntı kanıtı değildir.

## Bulgular ve öncelik

### 1. Resize bırakıldığında içerik ekrana aktarılmıyor — yüksek öncelik

Tekrarlama: Dosyalar penceresini taşı, sağ alt köşesinden büyüt, fareyi bırak.
Çerçeve yaklaşık 540×340 client boyutuna genişlerken tablo/status alanı eski
440×280 ölçülerinde görünüyor. Tam yenileme yapılan büyütme/tema değişimi görüntüyü
toparlıyor.

Kod bağlantısı: `src/wm.rs::handle_mouse_up` içindeki `resizing_window.take()`
yolunda `rerender_app_surface` çağrısı var, ardından yeni içeriği kapsayan damage
yok. Son resize hareketinin damage'i mouse-up öncesinde tüketilmişse yeni yüzey
framebuffer'a taşınmıyor. Bu, kaynak incelemesinin desteklediği neden açıklamasıdır.

Kanıt: `../target/hardening/realistic-kvm-resize/03b-resized.png`.

Önerilen düzeltme: resize tamamlanınca pencerenin güncel geometrisini damage
olarak ekle. Canlı resize isteniyorsa input olayı başına sınırsız render yerine
frame başına en son boyutu işleyen kontrollü rerender uygula. Mouse-up sonrasında
başka giriş yapmadan içeriğin güncellendiğini dış giriş testiyle doğrula.

### 2. Focus değişiminde eski titlebar rengi kalıyor — yüksek öncelik

Tekrarlama: Dosyalar'ı büyüt, Alt+Tab ile küçük Terminal'i öne al, sonra Editor aç.
Arkada kalan başlığın bazı parçaları aktif mavi, bazı parçaları pasif renkte kalır.

Kod bağlantısı: `create_window_with_meta` ve `raise_to_top_internal` önceki
pencerelerin focused bayraklarını indiriyor, ancak esas olarak yeni pencere ve
üst paneli damage işaretliyor. Eski odaklı pencerenin dekorasyonu ayrıca
geçersizleştirilmeli; tüm ekranı çizdirmek gerekmiyor.

Kanıt: `../target/hardening/realistic-kvm/05-switch-terminal.png` ve
`../target/hardening/realistic-kvm/07-editor-typed.png`.

Önerilen test: eski/yeni titlebar ve border piksellerini focus değişiminden sonra
tam redraw referansıyla karşılaştır; pencereler birbirinden uzakta da denensin.

### 3. Minimize sonrası MRU/Alt+Tab başlangıcı tutarsız — orta öncelik

Dosyalar küçültülünce Terminal odaklanıyor. İlk Alt+Tab buna rağmen Terminal'i
seçiyor ve küçültülmüş Dosyalar geri gelmiyor. `minimize_window` focus seçiyor;
`alt_tab_press` ise MRU listesinin ilk girdisinin o anki pencere olduğunu varsayıp
index=1 ile başlıyor. Bu iki state'in birlikte güncellenmesi incelenmeli.

Kanıt: `../target/hardening/realistic-tcg-complete/03c-minimized.png` ve
`../target/hardening/realistic-tcg-complete/03d-restored.png`. İkinci dosyanın adı
eski runner'da beklenen eylemi ifade eder; resim aslında restore başarısızlığını
gösterir. Yeni runner bunu `first-restore-attempt` olarak isimlendirir.

## Akıcılık konusunda ölçülen ve ölçülmeyen

KVM ve TCG sürükleme görüntülerinde mavi başlığın gözlenen başlangıçları:
(131,96), (181,116), (231,136), (281,156), (331,176), (381,196), (431,216),
(481,236). Bir piksellik fark pencere border'ıdır; hareket komutlarıyla uyumlu.
Bu 8 örnek, 40 hareketin sonunda birikmiş belirgin sürükleme gecikmesi olmadığını
gösterir; örnekler arasındaki her frame'i veya micro-stutter'ı ölçmez.

`run.sh` varsayılanında VNC görüntü yolu var; bu deney QEMU framebuffer'ını doğrudan
okudu. VNC encoding, istemci refresh ve ağ gecikmesi ölçülmedi. Kullanıcının
hissettiği takılmanın bu katmandan gelen kısmı hâlâ ayrı incelenmeli. HUD CPU
etiketi de gerçek CPU ölçeri değildir; `desktop.rs` süreç sayısından değer
üretiyor. Ekrandaki yüzdeyi performans kanıtı olarak kullanmadım.

## Tekrarlama

```bash
cd /home/teha/Documents/GitHub/sparkos
python3 tools/qemu_desktop_trial.py \
  --image target/hardening/kernel/x86_64-unknown-none/debug/bootimage-sparkos.bin \
  --disk disk.img --output target/hardening/manual-trial --accel kvm
```

KVM yoksa `--accel tcg` kullan. Pillow gerekir. Script normal başlangıç masaüstünü
ve başlangıç fare konumunu esas alır; farklı boot düzeni veya regresyon feature'ı
ile aynı sabit koordinatlar geçerli olmayabilir. `trial.json` içindeki fault listesi
ve seri log okunmalı; “komutlar gönderildi” sonucu tüm UI davranışlarının geçtiği
anlamına gelmez. Görüntüler ayrıca incelenmelidir.

Ek doğrulama: `realistic-tcg-restore` turunda ilk Alt+Tab pencereyi getirmedi,
ikincisi Dosyalar'ı geri getirdi. `03e-second-restore-attempt.png` bunu gösteriyor.
Restore aynı zamanda resize sonrası içeriği yeni boyutta görünür hale getirdi;
bu da eksik ekran yenilemesi teşhisini destekliyor. Bu turda da fault marker yok.

## Düzeltme ve yeniden deneme

Üç bulgu giderildi: resize mouse-up sonrası pencere damage'i ekleniyor; focus
kaybeden pencerenin dekorasyonu yenileniyor; minimize/normal kapatma ile focus
aktarılınca MRU yeni odağa göre güncelleniyor. Optimize normal imaj yeniden
derlendi. `realistic-fixed` TCG turunda gerçek girişlerle resize içeriği hemen
güncellendi, ilk Alt+Tab minimize pencereyi geri getirdi, arkadaki titlebar rengi
tutarlı kaldı. Görüntüler incelendi; tur panic/fault olmadan tamamlandı. Bu dar
etkileşim düzeltmesinden sonra tüm kernel regresyon paketi tekrar çalıştırılmadı.
