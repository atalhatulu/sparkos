# SPARKOS — Capability Microkernel Evrimi: Aşama Raporu ve Sonraki İş Listesi

> Bu rapor, SparkOS monolitik makro-kernel'ini capability-based microkernel'e dönüştürme
> yolculuğunun **mevcut durum** özeti ve **yapılacak iş** listesidir. Antigravity'ye
> (AGY) görev verirken bu dosyayı kullanın — içindeki bağlam self-contained'dır
> (commit ID'leri, dosya yolları, FROZEN semantik kuralları).

---

## 1. Proje Genel Bakış

- **Repo:** `github.com/atalhatulu/sparkos` — local `~/Documents/GitHub/sparkos`, branch `main`
- **Mimarı:** x86_64, `no_std` freestanding Rust monolitik makro-kernel
- **Hedef:** capability-based microkernel'e kademeli EVRİM (asla yeşil alandan yeniden yazım)
- **Kullanıcı modeli:** `deepseek-v4-flash` (Hermes), **AGY = "Gemini 3.7 Flash (High)"** (sabitlendi)
- **Doğrulama standardı (her aşama):** `cargo build` **0 error** + host test + QEMU boot regression
- **İş bölümü:** Kod üretimi → AGY. Eleştiri/plan revizyonu → fcc-claude (read-only).
  Doğrulama + glue + commit → Hermes (orkestratör). **Hermes kod yazmaz, AGY yazar.**

### FROZEN semantik kuralları (kodlarken dokunma)
Aşağıdaki semantik **FROZEN**'dır — ihlal edilirse durup rapor edilir:
- **STEP 1–3 (cap core):** grant ⊆ parent rights; revoke child → sibling etkilenmez;
  revoke parent → lineage ölür; transfer → eski lineage'dan kaçar; close → kendi handle;
  lend; generation mismatch → `CAP_INVALID`; epoch → `CAP_REVOKED`; rights → `CAP_NO_RIGHTS`;
  deref refcount + FREE.
- **Asama 4:** kalıcı devir = `transfer`, revoke-edilebilir geçici = `lend`.
  **Sessiz drop YASAK.**
- **Asama 5:** Asama 4 tamamlanmadan başlanmaz (IPC capability tabanlı olmalı).

---

## 2. MEVCUT DURUM — Tamamlanan 3 aşama

| Aşama | İçerik | Commit | Durum |
|---|---|---|---|
| **1** | İzole capability core (`src/cap.rs`, 9 invariant test) | `85c14a1` | ✅ |
| **2.0** | `cap::init()` boot'a bağlandı + SYS_EXEC exploit fix | `d3c0a2e` | ✅ |
| **2** | Syscall yetki kontrolü köprüsü (`src/syscall_cap.rs`) | `5c92526` | ✅ |

### Aşama 2'nin teslim ettiği (fcc eleştirisi işlendi)
- **`src/syscall_cap.rs`** (yeni): slice-pure fonksiyonlar —
  `check_fd_access_in_table`, `check_process_exec_in_table`, `grant_fd_in_table`,
  `close_fd_in_table`, seed'ler (`seed_stdio`, `seed_process_exec`) + 3 host test.
- **`src/cap.rs`**: `check_rights()` eklendi — **pasif** yetki kontrolü.
  Kritik bulgu: `deref`+Drop, refcount'u 0'a düşürüp objeyi `valid=false` yapıyordu
  (ikinci check `Invalid` veriyordu). `check_rights` refcount'a dokunmaz, fd-capability'yi
  **kalıcı** kılar. **Bu, fcc'nin "CapObject → gerçek kaynak eşlemesi" bulgusunun derin parçası.**
- **`src/syscall.rs`**: dispatcher'a gating — READ/WRITE/CLOSE/LSEEK/CONNECT/SEND/RECV/
  FORK/EXEC → `EACCES` dönüşü, mevcut davranış korundu.
- **`src/syscall_storage.rs` + `src/net_socket.rs`**: `sys_open` / `sys_socket`'te
  capability provision (fcc B2), `sys_close`'te `close_fd` (fcc G1).
- **`src/task/process.rs`**: `Process.cap_table: Vec<(u32, CapHandle)>` + `with_cap_table`
  closure accessor + `seed_new_process` (`create_user_process` + `fork_current` — B1 stdio seed).

### Doğrulama kanıtı (3/3 yeşil)
- Kernel build: **0 error**
- Host test: **12/12** (9 cap invariant + 3 syscall_cap PURE)
- QEMU boot: `[OK] Capability core initialized (root capability)` + syscall dispatcher OK —
  gating sistemi bozmadı (tek `[FAIL] Network Init` pre-existing, RTL8139 yok)

---

## 3. SONRAKİ AŞAMALAR (sıra: 2 ✅ → 3/4 paralel → 5)
### Aşama 3 — Memory/pointer güvenliği (per-process CR3 + user ptr zorlaması)
**Hedef:** `src/sec_mem.rs`'teki `validate_user_ptr` deliğini kapat; capability tabanlı
memory izolasyonu kur. Şu an `validate_user_ptr` bir yorumla *"doğrulanmadan kullanılıyor"*
diye itiraf ediyor.
**Kapsam:**
- `sec_mem.rs`: `validate_user_ptr`'i eksiksiz uygula (canonical, user half, her page user-mapped).
- `syscall_storage.rs` + `net_socket.rs`: TÜM `from_raw_parts` kullanımlarına
  `validate_user_ptr` öncesi check ekle.
- `process.rs`: per-process CR3 destegi (single address space varsayımı kırılır →
  her process ayrı page table). `cap.rs` MAP right'ı memory mapping izninin sembolik karşılığı.
**Test:** kernel adresine write → `-EFAULT`; user-half dışı pointer → `-EFAULT`;
process A'nın memory'sine B erişimi → fault; QEMU boot regression.
**Dosyalar:** `src/sec_mem.rs`, `src/syscall_storage.rs`, `src/net_socket.rs`, `src/task/process.rs`.

### Aşama 4 — IPC'yi capability modeline taşı
**Hedef:** `src/ipc.rs`'teki typed `BlockingChannel`'ı capability tabanlı yap;
capability mesaj içinde taşınabilir olsun. Mevcut `BlockingChannel` API **KORUNUR**
(uygulamalar bozulmasın).
**Kapsam:**
- Channel capability'si (bir process'in channel'a send/recv hakkı).
- Message payload yanında `CapHandle` taşınabilir.
- **Capability transfer kuralları (FROZEN):** kalıcı devir → `transfer`
  (geri alınamaz, sahiplik aktarımı); revoke-edilebilir geçici → `lend`
  (mesaj kuyruktayken sender lend'i iptal ederse cap slot `Revoked` işaretlenir,
  payload yine teslim edilir). Sessiz drop yasak.
**Test:** channel capability'si olmayan process send → `EACCES`; lend sonrası iptal →
payload teslim + slot `Revoked`; transfer devir → kalıcı; QEMU boot regression (IPC demo çalışır).
**Dosyalar:** `src/ipc.rs`, `src/cap.rs` (transfer API).

### Aşama 5 — Driver/service ayrıştırması → microkernel
**Hedef:** Monolitikten kademeli geçiş. Driver'lar user-space'e taşınır (fault isolation).
**En büyük ve en riskli aşama.** Asama 4 tamamlanmadan başlanmaz.
**Kapsam (fcc: donanımsız servis önce, ilk donanım adayı serial):**
- **Donanımsız servis** ile başla: örn. `keyboard` veya `fb_query` — user-space servis,
  capability tabanlı IPC ile kernel'e erişir. Bu, servis mimarisini driver karmaşıklığı
  olmadan doğrular.
- İlk donanım adayı **serial** (rtl8139 QEMU'da yok; serial en basit, IOMMU gerektirmez).
- Driver/servis crash → kernel etkilenmeden restart (fault recovery).

---

## 4. Kalan Teknik Borç (AGY'ye devredilecek — qa kapıları açık)
- **`src/cap.rs` 123–124 / 378–382**: `is_revoked` ve `revoke` içindeki
  **unsafe pointer cast** (u64 → AtomicU64). Asama 2 sonrasına ertelendi — temizlenecek.
- **`docs/goal_cap-2.md`**: Asama 2 REVIZE v2 spec — untracked, kullanıcı "dur" dediği
  için commit edilmedi. Karar kullanıcıda.
- **Global fd yapısı**: syscall'lar caller kimliği olmadan global fd tabloları kullanıyor
  (single-process varsayımı). Per-process fd'ye geçiş Asama 3/4'e ertelendi.
- **`bootstrap_root()` dönüşü** main.rs'te `let _root` ile çöpe atılıyor (fcc B3:
  root/self handle hiçbir process'te yok). `seed_process_exec` (B3) üretildi, entegrasyonda bağlandı.

---

## 5. AGY'ye Görev Verme Notları (Antigravity)
1. **Model:** `agy --model "Gemini 3.7 Flash (High)"` (sabitlendi).
2. **Komut formülü:** `cd ~/Documents/GitHub/sparkos && agy --dangerously-skip-permissions
   --print "$(cat docs/goal_<asama>.md)" --model "Gemini 3.7 Flash (High)" --print-timeout 15m`
3. **AGY Google Cloud'da, yerel fs'e erişemez** → promt'a **içerik göm** (yol değil).
   Kod bloğu döner; Hermes repo'ya yazar/entegre eder.
4. **AGY wrapper'ları stub bırakabilir** (SCHEDULER gibi kernel bağımlılıklarını
   net göremez) → "AGY sketch; orkestratör glue yapar" kuralı geçerlidir.
   AGY slice-pure core üretir, kernel glue'ı Hermes bağlar, **derleme doğrulaması şart**.
5. **fcc-claude (eleştiri):** `--allowedTools "Read,Glob,Grep,Bash"`, `env -u PYTHONPATH`,
   kısa prompt + `--max-turns 30`. Plan aşamasında zorunlu.
6. **FROZEN ihlali → dur, raporla.** Hermes mimari kararı tek başına değiştirmez.

---

*Rapor tarihi: 2026-08-14. Kaynak: `docs/EVOLUTION_PLAN_V2.md` (plan v3), commit geçmişi,
fcc Asama 2 eleştirisi (`/tmp/fcc_a2_out.txt`).*
