use alloc::vec::Vec;
use core::arch::asm;

#[no_mangle]
pub static mut KERNEL_RSP: u64 = 0;
#[no_mangle]
pub static mut KERNEL_RIP: u64 = 0;

pub fn execute_ring3_app() {
    crate::serial_println!("[USER_MODE] Kullanici Modu uygulamasina (Ring 3) geciliyor...");
    
    // 1. Kullanici Programi İcin 4KB Yigin (Stack) Ayarla
    let mut user_stack = Vec::<u8>::with_capacity(4096);
    user_stack.resize(4096, 0);
    let stack_ptr = user_stack.as_ptr() as u64;
    unsafe {
        crate::memory::set_user_accessible(stack_ptr, 4096);
    }
    let stack_top = stack_ptr + 4096;

    // 2. Kullanici Kodu (Code) İcin 4KB Alan Ayarla
    let mut user_code = Vec::<u8>::with_capacity(4096);
    user_code.resize(4096, 0);
    
    // x86_64 Makine Kodu (Uygulama):
    // mov eax, 1 (B8 01 00 00 00) -> sys_exit
    // int 0x80   (CD 80)
    user_code[0] = 0xB8;
    user_code[1] = 0x01;
    user_code[2] = 0x00;
    user_code[3] = 0x00;
    user_code[4] = 0x00;
    user_code[5] = 0xCD;
    user_code[6] = 0x80;

    let code_ptr = user_code.as_ptr() as u64;
    unsafe {
        crate::memory::set_user_accessible(code_ptr, 4096);
    }

    // 3. Segment Selectorleri al
    // RPL (Requested Privilege Level) = 3 (Kullanici modu)
    let user_data = crate::gdt::GDT.1.user_data_selector.0;
    let user_code = crate::gdt::GDT.1.user_code_selector.0;

    // 4. Ring 0'dan Ring 3'e gecis (iretq ile) ve donus icin Kernel state kaydi
    unsafe {
        asm!(
            "mov {kernel_rsp}, rsp",
            "lea {temp_reg}, [rip + 2f]",
            "mov {kernel_rip}, {temp_reg}",
            "cli",                  // Kesmeleri kapat
            "push {data_sel}",      // SS (Stack Segment)
            "push {stack}",         // RSP (Yigin İsaretcisi)
            "pushf",                // RFLAGS (Bayraklar)
            "pop {temp_reg}",
            "or {temp_reg}, 0x200", // RFLAGS icinde IF (Interrupt Enable) bitini 1 yap
            "push {temp_reg}",
            "push {code_sel}",      // CS (Code Segment)
            "push {code}",          // RIP (Komut İsaretcisi - Uygulamanin baslangici)
            "iretq",                // Ring 3'e Atla!
            "2:",                   // Return label
            kernel_rsp = out(reg) KERNEL_RSP,
            kernel_rip = out(reg) KERNEL_RIP,
            temp_reg = out(reg) _,
            data_sel = in(reg) user_data as u64,
            stack = in(reg) stack_top as u64,
            code_sel = in(reg) user_code as u64,
            code = in(reg) code_ptr as u64,
        );
    }
    crate::serial_println!("[USER_MODE] Kullanici modundan basariyla dönüldü.");
}

pub fn exec_elf(elf_bytes: &[u8]) -> Result<(), &'static str> {
    crate::serial_println!("[USER_MODE] ELF dosyasi yukleniyor...");
    let elf = crate::elf::parse_elf(elf_bytes)?;
    
    if elf.segments.is_empty() {
        return Err("No loadable segments found in ELF");
    }

    // Simplification: assume 1 loadable segment for a simple bare-metal app
    let segment = &elf.segments[0];
    
    // Allocate memory on kernel heap, but mark as user accessible
    let mut user_code = Vec::<u8>::with_capacity(segment.memsz as usize);
    user_code.extend_from_slice(&segment.data);
    user_code.resize(segment.memsz as usize, 0);

    let code_ptr = user_code.as_ptr() as u64;
    unsafe {
        crate::memory::set_user_accessible(code_ptr, segment.memsz as usize);
    }

    // Allocate stack
    let mut user_stack = Vec::<u8>::with_capacity(4096);
    user_stack.resize(4096, 0);
    let stack_ptr = user_stack.as_ptr() as u64;
    unsafe {
        crate::memory::set_user_accessible(stack_ptr, 4096);
    }
    let stack_top = stack_ptr + 4096;

    let user_data = crate::gdt::GDT.1.user_data_selector.0;
    let user_code_sel = crate::gdt::GDT.1.user_code_selector.0;

    // Calculate actual entry point based on loaded address
    // If it's a PIE, entry_point is relative to base.
    // If it's EXEC (absolute), we assume the user compiled it with base address 0.
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

