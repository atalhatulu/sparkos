use alloc::vec::Vec;
use core::arch::asm;

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
    // int 0x80 (CD 80) -> Kernel'e Syscall yapar (Ekrana "KULLANICI MODU" yazar)
    // jmp $    (EB FE) -> Sonsuz Dongu (Programin kapanmamasi icin)
    user_code[0] = 0xCD;
    user_code[1] = 0x80;
    user_code[2] = 0xEB;
    user_code[3] = 0xFE;

    let code_ptr = user_code.as_ptr() as u64;
    unsafe {
        crate::memory::set_user_accessible(code_ptr, 4096);
    }

    // 3. Segment Selectorleri al
    // RPL (Requested Privilege Level) = 3 (Kullanici modu)
    let user_data = crate::gdt::GDT.1.user_data_selector.0;
    let user_code = crate::gdt::GDT.1.user_code_selector.0;

    // 4. Ring 0'dan Ring 3'e gecis (iretq ile sahte bir interrupt dönüşü yapıyoruz)
    unsafe {
        asm!(
            "cli",                  // Kesmeleri kapat
            "push {data_sel}",      // SS (Stack Segment)
            "push {stack}",         // RSP (Yigin İsaretcisi)
            "pushf",                // RFLAGS (Bayraklar)
            "pop rax",
            "or rax, 0x200",        // RFLAGS icinde IF (Interrupt Enable) bitini 1 yap
            "push rax",
            "push {code_sel}",      // CS (Code Segment)
            "push {code}",          // RIP (Komut İsaretcisi - Uygulamanin baslangici)
            "iretq",                // Ring 3'e Atla!
            data_sel = in(reg) user_data as u64,
            stack = in(reg) stack_top as u64,
            code_sel = in(reg) user_code as u64,
            code = in(reg) code_ptr as u64,
            options(noreturn)
        );
    }
}
