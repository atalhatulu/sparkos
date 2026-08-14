# SparkOS Developer SDK & Toolchain

SparkOS v0.1.0 için resmî uygulama geliştirme kiti (SDK) ve komut satırı aracı (`spark`).

---

## 🚀 Hızlı Başlangıç

### 1. Yeni Bir Uygulama Oluşturma
```bash
./sdk/spark new my_app
cd my_app
```

### 2. Uygulama Kodu (`src/main.rs`)
```rust
#![no_std]
#![no_main]

use libspark::println;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    println!("Hello from SparkOS Application!");
    libspark::exit(0);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    libspark::exit(1);
}
```

### 3. Derleme ve SPFS Paketleme
```bash
../sdk/spark build
../sdk/spark pack
```

### 4. QEMU Üzerinde Çalıştırma
```bash
../sdk/spark run
```

---

## 📦 `libspark` API Referansı

| Fonksiyon / Makro | Açıklama |
|---|---|
| `println!(...)` | Formatlı metni standart çıktıya (`stdout`, fd 1) yazar |
| `print!(...)` | Yeni satır eklemeden standart çıktıya yazar |
| `libspark::write(fd, buf)` | Dosya tanıtıcısına bayt dizisi yazar |
| `libspark::read(fd, buf)` | Dosya tanıtıcısından bayt okur |
| `libspark::open(path, flags)` | Belirtilen dosyayı açar |
| `libspark::close(fd)` | Açık dosya tanıtıcısını kapatır |
| `libspark::exit(code)` | Uygulamayı belirtilen kod ile sonlandırır |
| `libspark::yield_cpu()` | CPU zamanını kooperatif olarak devreder |
