# SparkOS Kernel — /goal (L8 Networking)

## Görev
`~/Documents/GitHub/sparkos` repo'sunda **L8 (Networking)** seviyesini implement et. **SADECE aşağıdaki dosyalara dokun:**
- `src/net.rs` — güçlendir (mevcut ağ yığını, ICMP ping, UDP DNS)
- `src/net_socket.rs` — YENI dosya (socket sistemi)
- `src/rtl8139.rs` — opsiyonel (sadece hata varsa)

**KESiN: `src/main.rs`, `src/syscall.rs`'e DOKUNMA.** `pub mod` eklemelerini HERMES yapacak. Mevcut diğer modüllere dokunma.

## Proje
- Rust freestanding x86_64 `no_std` kernel, `bootloader 0.9`, `x86_64 0.15`
- Build: `cargo build` (0 error). **NOT: terminal komutlarını `env -u PYTHONPATH <cmd>` ile çalıştır.**
- L0-L7 tamam. `net.rs` (227) mevcut, `rtl8139.rs` (191) NIC sürücüsü.

## Mevcut L8 Durumu
- `src/net.rs` (227 satır) — mevcut ağ yığını: rtl8139 üzerinden paket gönderim/alım, ICMP ping (8.8.8.8), UDP üzerinden DNS çözümleme (`host <domain>`).
- `src/rtl8139.rs` (191) — RTL8139 PCI NIC sürücüsü: MAC, RX/TX descriptor, I/O port. ÇALIŞIYOR.
- **EKŞİK:** TCP/UDP gerçek socket desteği YOK (sadece ICMP + UDP DNS). Socket soyutlama (listen/connect/bind), TCP three-way handshake, port yönetimi, gerçek bir ağ yığını (IP/Ethernet framing doğrulaması) eksik.

## Yapılacaklar
1. **`src/net_socket.rs` (yeni):** Socket sistemi
   - `SocketType`: Udp, Tcp, Raw.
   - `Socket`: fd benzeri, yerel/uzak port, `SocketAddr` (ip + port), state (Closed/Listening/Connected).
   - `SocketTable`: açık socket'ler, `socket(type)`, `bind`, `listen`, `connect`, `send`, `recv`, `close`.
   - IPv4 adres temsili (`Ipv4Addr` tipi veya u32).
   - UDP paket gönderim/alım basit hali (mevcut net.rs'in UDP DNS kodunu temel al).
   - TCP: three-way handshake durum makinesi iskeleti (SYN/SYN-ACK/ACK), gerçek gönderim çok derin — iskelet + temel durumlar yeterli.

2. **`src/net.rs` güçlendir:** Mevcut UDP/DNS mantığını net_socket.rs'e soyutla, IP header doğrulama, checksum hesaplama, ARP desteği (IP→MAC çözümleme) ekle (mevcut yaklaşım debug).

3. **Port yönetimi:** 16-bit port alanı, `bind`'de kullanım kontrolü.

## Teknik
- `no_std` + alloc. Paket tamponları sabit boyutlu (max_frames).
- `sync::Mutex`/`BlockingChannel` kullanılabilir.
- Mevcut `net.rs`'in rtl8139 entegrasyonunu oku, uyumlu genişlet.
- Gerçek internet bağlantısı imkansız (QEMU NAT), ama kod yapısı gerçek yığına uygun olmalı.

## Teslim
1. Kısa analiz: net.rs mevcut yapısı, ne eklendi.
2. `src/net_socket.rs` + güçlendirilmiş `src/net.rs`.
3. `cargo build` çıktısı (0 error). Yeni modül main'e bağlı değil → geçici `pub mod` ile doğrula (main.rs'e ekleme, geçici).
4. `git diff --stat`.

AGY: "Eğer görev mantıksızsa YAPMA — raporla. İddialarını önce dosyayı okuyarak doğrula."
