//! src/panic.rs — Crash dump ve panic yardimcilari.
//!
//! main.rs'teki `#[panic_handler]` buradaki `crash_dump`'u cagirabilir.
//! Kullanilana kadar main.rs tarafindan gosterilmeyen infra modulu —
//! bu yuzden dead_code uyarilari bastirildi.

#![allow(dead_code)]

use core::arch::asm;
use core::panic::PanicInfo;

/// Anlik 8 genel amacli register'i okur.
/// Sira: RAX, RBX, RCX, RDX, RSI, RDI, RBP, RSP.
#[inline(never)]
fn read_cpu_regs() -> [u64; 8] {
    let (mut rax, mut rbx, mut rcx, mut rdx) = (0u64, 0u64, 0u64, 0u64);
    let (mut rsi, mut rdi, mut rbp, mut rsp) = (0u64, 0u64, 0u64, 0u64);
    unsafe {
        asm!(
            "mov {}, rax",
            "mov {}, rbx",
            "mov {}, rcx",
            "mov {}, rdx",
            "mov {}, rsi",
            "mov {}, rdi",
            "mov {}, rbp",
            "mov {}, rsp",
            out(reg) rax,
            out(reg) rbx,
            out(reg) rcx,
            out(reg) rdx,
            out(reg) rsi,
            out(reg) rdi,
            out(reg) rbp,
            out(reg) rsp,
        );
    }
    [rax, rbx, rcx, rdx, rsi, rdi, rbp, rsp]
}

/// Panic mesaji + CR2 + register dokumunu yazar. Serial ciktisi kullanir.
pub fn crash_dump(info: &PanicInfo) {
    let cr2 = x86_64::registers::control::Cr2::read_raw();
    let [rax, rbx, rcx, rdx, rsi, rdi, rbp, rsp] = read_cpu_regs();

    crate::serial_println!("========================================");
    crate::serial_println!("        KERNEL CRASH DUMP");
    crate::serial_println!("========================================");
    crate::serial_println!("Panic: {}", info);
    crate::serial_println!("CR2  (fault addr): {:#018x}", cr2);
    crate::serial_println!("RSP  (stack ptr) : {:#018x}", rsp);
    crate::serial_println!("RBP  (base ptr)  : {:#018x}", rbp);
    crate::serial_println!("----------------------------------------");
    crate::serial_println!("RAX = {:#018x}    RBX = {:#018x}", rax, rbx);
    crate::serial_println!("RCX = {:#018x}    RDX = {:#018x}", rcx, rdx);
    crate::serial_println!("RSI = {:#018x}    RDI = {:#018x}", rsi, rdi);
    crate::serial_println!("========================================");
}

/// Interrupt'lari bekleyerek CPU'yu durdurur.
pub fn halt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

/// Kerneli abort eder (panic handler icin cikis noktasi).
pub fn abort() -> ! {
    crate::serial_println!("[PANIC] Kernel abort()");
    halt_loop()
}
