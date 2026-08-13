use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use x86_64::instructions::port::Port;
use x86_64::PrivilegeLevel;
use crate::serial_println;
use spin::Mutex;
use core::sync::atomic::{AtomicU64, Ordering};

pub static IDT: Mutex<Option<InterruptDescriptorTable>> = Mutex::new(None);

pub const PIC_IRQ_BASE: u8 = 32;

pub fn init_idt() {
    let mut idt = InterruptDescriptorTable::new();
    
    // Exceptions WITHOUT error code (HandlerFunc)
    idt.divide_error.set_handler_fn(divide_error_handler);
    idt.debug.set_handler_fn(debug_handler);
    idt.non_maskable_interrupt.set_handler_fn(nmi_handler);
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    idt.overflow.set_handler_fn(overflow_handler);
    idt.bound_range_exceeded.set_handler_fn(bound_range_handler);
    idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
    idt.device_not_available.set_handler_fn(device_not_available_handler);
    
    // Exceptions WITH error code (HandlerFuncWithErrCode)
    idt.invalid_tss.set_handler_fn(invalid_tss_handler);
    idt.segment_not_present.set_handler_fn(segment_not_present_handler);
    idt.stack_segment_fault.set_handler_fn(stack_segment_handler);
    idt.general_protection_fault.set_handler_fn(general_protection_fault_handler);
    idt.alignment_check.set_handler_fn(alignment_check_handler);
    
    // Diverging (returns !)
    unsafe {
        idt.double_fault.set_handler_fn(double_fault_handler)
            .set_stack_index(crate::gdt::DOUBLE_FAULT_IST_INDEX);
    }
    
    // Syscall (Ring 3 accessible)
    unsafe {
        idt[0x80].set_handler_addr(x86_64::VirtAddr::new(syscall_entry as *const () as u64))
            .set_privilege_level(x86_64::PrivilegeLevel::Ring3);
    }
    
    // Page fault (special error code type)
    idt.page_fault.set_handler_fn(page_fault_handler);
    
    // Hardware IRQs
    idt[PIC_IRQ_BASE + 0].set_handler_fn(timer_handler);
    idt[PIC_IRQ_BASE + 1].set_handler_fn(keyboard_handler);
    idt[PIC_IRQ_BASE + 12].set_handler_fn(mouse_handler);
    idt[PIC_IRQ_BASE + 14].set_handler_fn(ata1_handler);
    idt[PIC_IRQ_BASE + 15].set_handler_fn(ata2_handler);
    
    // Store in static, then load into IDTR
    *IDT.lock() = Some(idt);
    let guard = IDT.lock();
    let idt_ref = guard.as_ref().unwrap();
    let idt_static: &'static InterruptDescriptorTable = unsafe {
        &*(idt_ref as *const InterruptDescriptorTable)
    };
    idt_static.load();
    
    serial_println!("[interrupts] IDT loaded");
}

pub fn init_pic() {
    let mut cmd_master = Port::new(0x20u16);
    let mut data_master = Port::new(0x21u16);
    let mut cmd_slave = Port::new(0xA0u16);
    let mut data_slave = Port::new(0xA1u16);
    
    unsafe {
        cmd_master.write(0x11u8);
        cmd_slave.write(0x11u8);
        data_master.write(PIC_IRQ_BASE);
        data_slave.write(PIC_IRQ_BASE + 8);
        data_master.write(4u8);
        data_slave.write(2u8);
        data_master.write(0x01u8);
        data_slave.write(0x01u8);
        data_master.write(0u8);
        data_slave.write(0u8);
    }
    
    serial_println!("[interrupts] PIC remapped to IRQ base {}", PIC_IRQ_BASE);
}

pub fn init_timer() {
    let divisor: u16 = (1193182u32 / 1000) as u16;
    let mut cmd: Port<u8> = Port::new(0x43u16);
    let mut data: Port<u8> = Port::new(0x40u16);
    
    unsafe {
        cmd.write(0x36u8);
        data.write((divisor & 0xFF) as u8);
        data.write((divisor >> 8) as u8);
    }
    
    serial_println!("[interrupts] PIT timer: 1000 Hz (divisor={})", divisor);
}

// ========== Exception Handlers (no error code) ==========

extern "x86-interrupt" fn divide_error_handler(_stack: InterruptStackFrame) {
    serial_println!("[PANIC] Divide-by-zero");
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn debug_handler(_stack: InterruptStackFrame) {
    serial_println!("[DEBUG] Trap");
}

extern "x86-interrupt" fn nmi_handler(_stack: InterruptStackFrame) {
    serial_println!("[DEBUG] NMI");
}

extern "x86-interrupt" fn breakpoint_handler(_stack: InterruptStackFrame) {
    serial_println!("[DEBUG] Breakpoint");
}

extern "x86-interrupt" fn overflow_handler(_stack: InterruptStackFrame) {
    serial_println!("[PANIC] Overflow");
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn bound_range_handler(_stack: InterruptStackFrame) {
    serial_println!("[PANIC] Bound range");
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn invalid_opcode_handler(_stack: InterruptStackFrame) {
    serial_println!("[PANIC] Invalid opcode");
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn device_not_available_handler(_stack: InterruptStackFrame) {
    serial_println!("[PANIC] Device not available (no FPU/SSE)");
    loop { x86_64::instructions::hlt(); }
}

// ========== Exception Handlers WITH error code ==========

extern "x86-interrupt" fn invalid_tss_handler(
    _stack: InterruptStackFrame,
    _error_code: u64,
) {
    serial_println!("[PANIC] Invalid TSS (error={:#x})", _error_code);
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn segment_not_present_handler(
    _stack: InterruptStackFrame,
    _error_code: u64,
) {
    serial_println!("[PANIC] Segment not present (error={:#x})", _error_code);
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn stack_segment_handler(
    _stack: InterruptStackFrame,
    _error_code: u64,
) {
    serial_println!("[PANIC] Stack segment fault (error={:#x})", _error_code);
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn general_protection_fault_handler(
    _stack: InterruptStackFrame,
    _error_code: u64,
) {
    serial_println!("[PANIC] GPF (error={:#x})", _error_code);
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn alignment_check_handler(
    _stack: InterruptStackFrame,
    _error_code: u64,
) {
    serial_println!("[PANIC] Alignment check (error={:#x})", _error_code);
    loop { x86_64::instructions::hlt(); }
}

// ========== Diverging handlers ==========

extern "x86-interrupt" fn double_fault_handler(
    _stack: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    serial_println!("[FATAL] Double fault (error={:#x})", _error_code);
    loop { x86_64::instructions::hlt(); }
}

#[unsafe(naked)]
extern "C" fn syscall_entry() {
    unsafe {
        core::arch::naked_asm!(
            "push rbp",
            "mov rbp, rsp",
            "push rbx",
            "push r12",
            "push r13",
            "push r14",
            "push r15",
            
            // SysV ABI: arg1=rdi, arg2=rsi, arg3=rdx, arg4=rcx, arg5=r8, arg6=r9
            // Linux Syscall: num=rax, arg1=rdi, arg2=rsi, arg3=rdx, arg4=r10, arg5=r8
            "mov r9, r8",   // arg5 -> r9 (C arg6)
            "mov r8, r10",  // arg4 -> r8 (C arg5)
            "mov rcx, rdx", // arg3 -> rcx (C arg4)
            "mov rdx, rsi", // arg2 -> rdx (C arg3)
            "mov rsi, rdi", // arg1 -> rsi (C arg2)
            "mov rdi, rax", // num  -> rdi (C arg1)
            
            "call syscall_handler_inner",
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop rbx",
            "pop rbp",
            "iretq",
        )
    }
}

#[no_mangle]
extern "C" fn syscall_handler_inner(num: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> u64 {
    crate::syscall::syscall_dispatcher(num, arg1, arg2, arg3, arg4, arg5)
}

// ========== Page fault ==========

/// Returns `true` when the fault originated from a Ring-3 (user) context.
/// The CPU saves the user code selector's RPL bits in the interrupt stack
/// frame; RPL == 3 means the faulting instruction ran in user mode.
fn fault_from_user(stack: &InterruptStackFrame) -> bool {
    stack.code_segment.rpl() == PrivilegeLevel::Ring3
}

/// Aborts the faulting user task without returning to it.
///
/// Mirrors `sys_exit`: it clobbers the interrupt stack frame's implicit return
/// by restoring the kernel's saved RSP/RIP (recorded by `user.rs` just before
/// `iretq` into Ring 3). Control therefore resumes in the kernel loop and the
/// kernel keeps running even though the user process died of a page fault.
fn kill_user_task() -> ! {
    unsafe {
        core::arch::asm!(
            "cli",
            "mov rsp, {kernel_rsp}",
            "jmp {kernel_rip}",
            kernel_rsp = in(reg) crate::user::KERNEL_RSP,
            kernel_rip = in(reg) crate::user::KERNEL_RIP,
            options(noreturn)
        );
    }
}

extern "x86-interrupt" fn page_fault_handler(
    stack: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let addr = x86_64::registers::control::Cr2::read_raw();

    // Distinguish user vs kernel faults using the saved code-segment RPL.
    if fault_from_user(&stack) {
        // User fault: the process touched memory it is not allowed to. Kill the
        // task and let the kernel continue running. `kill_user_task` diverges.
        serial_println!(
            "[USER-FAULT] killed user task: rip={:#x}, addr={:#x}, err={:?}",
            stack.instruction_pointer,
            addr,
            error_code,
        );
        kill_user_task();
    }

    // Kernel-space fault (and any survivable non-user path): this is a real
    // kernel bug; halt and panic. Reached only for kernel faults because the
    // user branch above diverged.
    serial_println!(
        "[PANIC] Kernel Page Fault at {:#x}, access={:#x}",
        stack.instruction_pointer,
        addr,
    );
    serial_println!("  Error code: {:?}", error_code);
    loop { x86_64::instructions::hlt(); }
}

// ========== IRQ Handlers ==========

static TICK: AtomicU64 = AtomicU64::new(0);

extern "x86-interrupt" fn timer_handler(_stack: InterruptStackFrame) {
    TICK.fetch_add(1, Ordering::Relaxed);
    
    // Tick çıktısı kaldırıldı — debug için serial_println vardı
    
    unsafe {
        let mut eoi = Port::new(0x20u16);
        eoi.write(0x20u8);
    }
}

pub fn get_tick() -> u64 {
    TICK.load(Ordering::Relaxed)
}

extern "x86-interrupt" fn keyboard_handler(_stack: InterruptStackFrame) {
    use x86_64::instructions::port::PortReadOnly;
    
    let mut data: PortReadOnly<u8> = PortReadOnly::new(0x60u16);
    
    unsafe {
        let scancode = data.read();
        crate::task::keyboard::add_scancode(scancode);
    }
    
    // EOI
    unsafe {
        let mut eoi = Port::new(0x20u16);
        eoi.write(0x20u8);
    }
}

extern "x86-interrupt" fn ata1_handler(_stack: InterruptStackFrame) {
    unsafe {
        let mut eoi_slave = Port::new(0xA0u16);
        eoi_slave.write(0x20u8);
        let mut eoi_master = Port::new(0x20u16);
        eoi_master.write(0x20u8);
    }
}

extern "x86-interrupt" fn ata2_handler(_stack: InterruptStackFrame) {
    unsafe {
        let mut eoi_slave = Port::new(0xA0u16);
        eoi_slave.write(0x20u8);
        let mut eoi_master = Port::new(0x20u16);
        eoi_master.write(0x20u8);
    }
}

extern "x86-interrupt" fn mouse_handler(_stack: InterruptStackFrame) {
    crate::mouse::handle_interrupt();
    unsafe {
        let mut eoi_slave = Port::new(0xA0u16);
        eoi_slave.write(0x20u8);
        let mut eoi_master = Port::new(0x20u16);
        eoi_master.write(0x20u8);
    }
}
