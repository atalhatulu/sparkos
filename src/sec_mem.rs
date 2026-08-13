//! L9 Security Memory: Güvenli syscall bellek erişimi.
//!
//! Mevcut syscall.rs / syscall_storage.rs, kullanıcıdan gelen pointer'ları
//! `core::slice::from_raw_parts` ile HİÇ doğrulamadan kullanır — bir kullanıcı
//! process'i kernel adresini ya da null'u syscall buffer'ı olarak geçerek
//! kernel belleğine erişebilir. Bu modül, HERMES'in syscall.rs'i bu güvenli
//! API'ye geçirmesi için temel oluşturur:
//!   - `validate_user_ptr`  → `Result<&[u8], &'static str>` (okuma)
//!   - `validate_user_ptr_mut` → `Result<&mut [u8], &'static str>` (yazma)
//!   - Her ikisi de: null'u reddeder, kernel adreslerini reddeder, ptr+len
//!     overflow'unu reddeder ve her sayfanın USER_ACCESSIBLE olduğunu paging
//!     üzerinden doğrular.
//!
//! NOT: `gui::PHYS_OFFSET`'e ve `x86_64`'ün `VirtAddr`/`Page` API'sine dayanır
//! (bkz. `memory.rs`'teki `active_level_4_table` — burada salt-okunur bir sayfa
//! tablosu yürüyücüsü ile yeniden uygulanır).

use x86_64::registers::control::Cr3;
use x86_64::structures::paging::{Page, PageTable, PageTableFlags};
use x86_64::VirtAddr;

/// Kullanıcı adres alanı üst sınırı: 0x0000_0000_8000_0000
/// (x86_64'ün 48-bit sanal adres alanının yarısı).
const USER_ADDR_LIMIT: u64 = 0x0000_0000_8000_0000;

// ---------------------------------------------------------------------------
// Sayfa tablosu yürüyücüsü (salt-okunur)
// ---------------------------------------------------------------------------

/// Aktif L4 tablosunun sanal adresini döndürür.
fn active_l4() -> *const PageTable {
    let (l4_frame, _) = Cr3::read();
    let phys = l4_frame.start_address().as_u64();
    let phys_offset = unsafe { crate::gui::PHYS_OFFSET };
    VirtAddr::new(phys_offset.wrapping_add(phys)).as_ptr()
}

/// Sayfa tablosunu P4'ten P1'e yürüyerek son girdinin bayraklarını döndürür.
/// Zincirde herhangi bir seviye PRESENT değilse `None`.
fn walk_page(page: Page) -> Option<PageTableFlags> {
    let l4 = unsafe { &*active_l4() };
    let p4 = &l4[page.p4_index()];
    if !p4.flags().contains(PageTableFlags::PRESENT) {
        return None;
    }
    let p3 = unsafe { &*VirtAddr::new(crate::gui::PHYS_OFFSET.wrapping_add(p4.addr().as_u64())).as_ptr::<PageTable>() };
    let p3e = &p3[page.p3_index()];
    if !p3e.flags().contains(PageTableFlags::PRESENT) {
        return None;
    }
    let p2 = unsafe { &*VirtAddr::new(crate::gui::PHYS_OFFSET.wrapping_add(p3e.addr().as_u64())).as_ptr::<PageTable>() };
    let p2e = &p2[page.p2_index()];
    if !p2e.flags().contains(PageTableFlags::PRESENT) {
        return None;
    }
    let p1 = unsafe { &*VirtAddr::new(crate::gui::PHYS_OFFSET.wrapping_add(p2e.addr().as_u64())).as_ptr::<PageTable>() };
    let p1e = &p1[page.p1_index()];
    if !p1e.flags().contains(PageTableFlags::PRESENT) {
        return None;
    }
    Some(p1e.flags())
}

/// Sayfa, kullanıcı modundan okunabilir mi?
fn page_is_user_accessible(page: Page) -> bool {
    walk_page(page)
        .map(|f| f.contains(PageTableFlags::USER_ACCESSIBLE))
        .unwrap_or(false)
}

/// Sayfa, kullanıcı modundan yazılabilir mi?
fn page_is_user_writable(page: Page) -> bool {
    walk_page(page)
        .map(|f| {
            f.contains(PageTableFlags::WRITABLE) && f.contains(PageTableFlags::USER_ACCESSIBLE)
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Doğrulama çekirdeği
// ---------------------------------------------------------------------------

fn check_user_range(ptr: u64, len: usize) -> Result<(), &'static str> {
    // 1. Pointer null olmamalı ve kullanıcı adres alanında olmalı.
    if ptr == 0 {
        return Err("null pointer");
    }
    if ptr >= USER_ADDR_LIMIT {
        return Err("address outside user space");
    }

    // 2. Aralık kullanıcı alanında bitmeli (overflow taşması kontrolü).
    let end = ptr
        .checked_add(len as u64)
        .ok_or("address range overflow")?;
    if end > USER_ADDR_LIMIT {
        return Err("address range outside user space");
    }

    // 3. Erişilen her sayfa user-accessible olmalı.
    let start_page = Page::containing_address(VirtAddr::new(ptr));
    let end_page = Page::containing_address(VirtAddr::new(end - 1));
    for page in Page::range_inclusive(start_page, end_page) {
        if !page_is_user_accessible(page) {
            return Err("page not user-accessible");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Genel API
// ---------------------------------------------------------------------------

/// Kullanıcı pointer'ını doğrular ve okuma için `&[u8]` dilimi döndürür.
///
/// Kernel adreslerini, null'u, taşan aralıkları ve user-accessible olmayan
/// sayfaları reddeder. `len == 0` ise boş dilim döndürür (sayfa kontrolü
/// atlanır).
pub fn validate_user_ptr<'a>(buf_ptr: u64, len: usize) -> Result<&'a [u8], &'static str> {
    if len == 0 {
        return Ok(&[]);
    }
    check_user_range(buf_ptr, len)?;
    Ok(unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len) })
}

/// Kullanıcı pointer'ını doğrular ve yazma için `&mut [u8]` dilimi döndürür.
///
/// `validate_user_ptr` ile aynı kontrolleri yapar ve ayrıca her sayfanın
/// WRITABLE bayrağına sahip olduğunu doğrular.
pub fn validate_user_ptr_mut<'a>(buf_ptr: u64, len: usize) -> Result<&'a mut [u8], &'static str> {
    if len == 0 {
        return Ok(&mut []);
    }
    check_user_range(buf_ptr, len)?;
    let start_page = Page::containing_address(VirtAddr::new(buf_ptr));
    let end_page = Page::containing_address(VirtAddr::new(buf_ptr + (len as u64) - 1));
    for page in Page::range_inclusive(start_page, end_page) {
        if !page_is_user_writable(page) {
            return Err("page not user-writable");
        }
    }
    Ok(unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len) })
}

/// L9 demo: çeşitli pointer senaryolarının doğrulama sonuçlarını yazdırır.
/// Pozitif durum, user.rs'in çalışma zamanında yarattığı USER_ACCESSIBLE
/// sayfalarla syscall.rs'e entegre edilince gerçekleşir; burada güvenlik
/// açısından kritik red senaryoları gösterilir.
pub fn demo() {
    crate::serial_println!("[SECMEM] -- validate_user_ptr demo --");
    crate::serial_println!("[SECMEM] NULL                 -> {:?}", validate_user_ptr(0, 8));
    crate::serial_println!("[SECMEM] kernel addr          -> {:?}", validate_user_ptr(0xffff_ffff_8000_0000, 8));
    crate::serial_println!("[SECMEM] overflow (len=MAX)   -> {:?}", validate_user_ptr(0x4000, usize::MAX));
    crate::serial_println!("[SECMEM] unmapped user addr   -> {:?}", validate_user_ptr(0x5000_0000, 8));
    crate::serial_println!("[SECMEM] kernel-mapped 0x1000 -> {:?}", validate_user_ptr(0x1000, 8));
    crate::serial_println!("[SECMEM] zero len (0x4000)    -> {:?}", validate_user_ptr(0x4000, 0).map(|s| s.len()));
    crate::serial_println!("[SECMEM] NULL write           -> {:?}", validate_user_ptr_mut(0, 8));
    crate::serial_println!("[SECMEM] unmapped write       -> {:?}", validate_user_ptr_mut(0x5000_0000, 8));
}
