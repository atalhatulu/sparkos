use alloc::vec::Vec;
use core::arch::asm;

use crate::memory;

#[no_mangle]
pub static mut KERNEL_RSP: u64 = 0;
#[no_mangle]
pub static mut KERNEL_RIP: u64 = 0;

/// Base virtual address of the dedicated user code region.
const USER_CODE_BASE: u64 = memory::USER_ADDR_BASE; // 0x4000_0000
/// Top of the dedicated user stack region.
const USER_STACK_TOP: u64 = memory::USER_STACK_TOP; // 0x7FFF_0000

/// Maps `bytes` into the dedicated user region starting at `virt`, returning
/// the mapped virtual address (== `virt`). The underlying physical frames are
/// freshly allocated from the dedicated user pool, so user PTEs never alias
/// kernel heap or kernel image pages.
///
/// On failure (e.g. the user frame allocator has not been seeded) returns
/// `Err(())` so callers can fall back to a degraded path.
fn map_user_image(virt: u64, bytes: &[u8], writable: bool) -> Result<u64, ()> {
    let bytes_needed = bytes.len().max(1);
    let pages = (bytes_needed + 4095) / 4096;
    memory::map_user_range(virt, pages as u64, writable).map_err(|_| ())?;
    unsafe {
        core::ptr::write_bytes(virt as *mut u8, 0, pages * 4096);
        if !bytes.is_empty() {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), virt as *mut u8, bytes.len());
        }
    }
    Ok(virt)
}

pub fn execute_ring3_app() {
    crate::serial_println!("[USER_MODE] Kullanici Modu uygulamasina (Ring 3) geciliyor...");

    // 1. Kullanici Kodu (Code) icin dedicated user region'a 4KB alan ayir.
    let user_code: Vec<u8> = {
        let mut c = Vec::<u8>::with_capacity(4096);
        c.resize(4096, 0);
        // x86_64 Makine Kodu (Uygulama):
        // mov eax, 1 (B8 01 00 00 00) -> sys_exit
        // int 0x80   (CD 80)
        c[0] = 0xB8;
        c[1] = 0x01;
        c[2] = 0x00;
        c[3] = 0x00;
        c[4] = 0x00;
        c[5] = 0xCD;
        c[6] = 0x80;
        c
    };

    let code_ptr = match map_user_image(USER_CODE_BASE, &user_code, false) {
        Ok(p) => p,
        Err(_) => {
            // Fallback: legacy kernel-heap mapping (still marked user-accessible).
            unsafe { crate::memory::set_user_accessible(user_code.as_ptr() as u64, 4096); }
            user_code.as_ptr() as u64
        }
    };

    // 2. Kullanici Modeli icin dedicated user region'a 4KB yigin ayir.
    let stack_base = USER_STACK_TOP - 4096;
    let stack_top = match map_user_image(stack_base, &[], true) {
        Ok(_) => USER_STACK_TOP,
        Err(_) => {
            // Fallback: kernel-heap stack, marked user-accessible.
            let mut s = Vec::<u8>::with_capacity(4096);
            s.resize(4096, 0);
            unsafe { crate::memory::set_user_accessible(s.as_ptr() as u64, 4096); }
            s.as_ptr() as u64 + 4096
        }
    };

    // 3. Segment Selectorleri al
    let user_data = crate::gdt::GDT.1.user_data_selector.0;
    let user_code_sel = crate::gdt::GDT.1.user_code_selector.0;

    // 4. Ring 0'dan Ring 3'e gecis (iretq ile) ve donus icin Kernel state kaydi
    unsafe {
        asm!(
            "mov {kernel_rsp}, rsp",
            "lea {temp_reg}, [rip + 2f]",
            "mov {kernel_rip}, {temp_reg}",
            "cli",                  // Kesmeleri kapat
            "push {data_sel}",      // SS (Stack Segment)
            "push {stack}",         // RSP (Yigin Isaretcisi)
            "pushf",                // RFLAGS (Bayraklar)
            "pop {temp_reg}",
            "or {temp_reg}, 0x200", // RFLAGS icinde IF bitini 1 yap
            "push {temp_reg}",
            "push {code_sel}",      // CS (Code Segment)
            "push {code}",          // RIP (Komut Isaretcisi - Uygulamanin baslangici)
            "iretq",                // Ring 3'e Atla!
            "2:",                   // Return label
            kernel_rsp = out(reg) KERNEL_RSP,
            kernel_rip = out(reg) KERNEL_RIP,
            temp_reg = out(reg) _,
            data_sel = in(reg) user_data as u64,
            stack = in(reg) stack_top as u64,
            code_sel = in(reg) user_code_sel as u64,
            code = in(reg) code_ptr as u64,
        );
    }
    crate::serial_println!("[USER_MODE] Kullanici modundan basariyla dönüldü.");
}

/// Async variant of [`exec_elf`] used by the app-lifecycle module (`app.rs`).
///
/// Yields the cooperative kernel executor before entering userspace so that
/// timer / IPC / input tasks keep making progress. With the current
/// synchronous iretq model the user app runs to completion once launched, so
/// the function returns `Ok(())`/`Err(..)` after the app exits.
pub async fn exec_elf_async(elf_bytes: &[u8]) -> Result<(), &'static str> {
    crate::task::yield_now().await;
    exec_elf(elf_bytes)
}

pub fn exec_elf(elf_bytes: &[u8]) -> Result<(), &'static str> {
    crate::serial_println!("[USER_MODE] ELF dosyasi yukleniyor...");
    let elf = crate::elf::parse_elf(elf_bytes)?;

    if elf.segments.is_empty() {
        return Err("No loadable segments found in ELF");
    }

    // Simplification: assume 1 loadable segment for a simple bare-metal app
    let segment = &elf.segments[0];

    // Dedicated user region'na ELF segment yukle: her segment page'i icin fresh
    // user frame tahsis et, boylece user PTE'leri kernel sayfalarina uzanmaz.
    let code_ptr = match map_user_image(USER_CODE_BASE, &segment.data, false) {
        Ok(p) => p,
        Err(_) => {
            // Fallback: kernel-heap yukleme, user-accessible isaretlenir.
            let mut user_code = Vec::<u8>::with_capacity(segment.memsz as usize);
            user_code.extend_from_slice(&segment.data);
            user_code.resize(segment.memsz as usize, 0);
            let cp = user_code.as_ptr() as u64;
            unsafe { crate::memory::set_user_accessible(cp, segment.memsz as usize); }
            cp
        }
    };

    // Kullanici stack'ini dedicated region'a haritala.
    let stack_base = USER_STACK_TOP - 4096;
    let stack_top = match map_user_image(stack_base, &[], true) {
        Ok(_) => USER_STACK_TOP,
        Err(_) => {
            let mut user_stack = Vec::<u8>::with_capacity(4096);
            user_stack.resize(4096, 0);
            let sp = user_stack.as_ptr() as u64;
            unsafe { crate::memory::set_user_accessible(sp, 4096); }
            sp + 4096
        }
    };

    let user_data = crate::gdt::GDT.1.user_data_selector.0;
    let user_code_sel = crate::gdt::GDT.1.user_code_selector.0;

    // Calculate actual entry point based on loaded address
    let actual_entry = code_ptr + (elf.entry_point - segment.vaddr);

    unsafe {
        asm!(
            "mov {kernel_rsp}, rsp",
            "lea {temp_reg}, [rip + 2f]",
            "mov {kernel_rip}, {temp_reg}",
            "cli",
            "push {data_sel}",
            "push {stack}",
            "pushf",
            "pop {temp_reg}",
            "or {temp_reg}, 0x200", // Enable interrupts
            "push {temp_reg}",
            "push {code_sel}",
            "push {code}",
            "iretq",
            "2:",
            kernel_rsp = out(reg) KERNEL_RSP,
            kernel_rip = out(reg) KERNEL_RIP,
            temp_reg = out(reg) _,
            data_sel = in(reg) user_data as u64,
            stack = in(reg) stack_top as u64,
            code_sel = in(reg) user_code_sel as u64,
            code = in(reg) actual_entry as u64,
        );
    }

    crate::serial_println!("[USER_MODE] ELF basariyla sonlandi.");
    Ok(())
}
