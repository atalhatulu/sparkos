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
    
    // Multi-Core SMP TLB Shootdown IPI Handler (Faz 30)
    idt[TLB_SHOOTDOWN_VECTOR].set_handler_fn(tlb_shootdown_handler);

    // Multi-Core SMP Reschedule IPI Handler (Faz 30 Adım 2)
    idt[RESCHEDULE_IPI_VECTOR].set_handler_fn(reschedule_ipi_handler);

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

pub fn load_cpu_idt() {
    let guard = IDT.lock();
    if let Some(ref idt) = *guard {
        let idt_static: &'static InterruptDescriptorTable = unsafe {
            &*(idt as *const InterruptDescriptorTable)
        };
        idt_static.load();
    }
}

pub const TLB_SHOOTDOWN_VECTOR: u8 = 0xFD;
pub const RESCHEDULE_IPI_VECTOR: u8 = 0xFC;

extern "x86-interrupt" fn tlb_shootdown_handler(_stack_frame: InterruptStackFrame) {
    crate::smp::handle_tlb_shootdown_ipi();
    crate::smp::lapic_eoi();
}

extern "x86-interrupt" fn reschedule_ipi_handler(_stack_frame: InterruptStackFrame) {
    crate::smp::lapic_eoi();
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

extern "x86-interrupt" fn divide_error_handler(stack: InterruptStackFrame) {
    // User-mode divide errors recover the faulting process under the process
    // model; only kernel-mode faults halt the kernel.
    if fault_from_user(&stack) {
        recover_user_fault(&stack, 0, "#DE divide-by-zero");
    }
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

extern "x86-interrupt" fn invalid_opcode_handler(stack: InterruptStackFrame) {
    if fault_from_user(&stack) {
        recover_user_fault(&stack, 0, "#UD invalid opcode");
    }
    serial_println!("[PANIC] Invalid opcode");
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn device_not_available_handler(stack: InterruptStackFrame) {
    if fault_from_user(&stack) {
        recover_user_fault(&stack, 0, "#NM device not available");
    }
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
    stack: InterruptStackFrame,
    error_code: u64,
) {
    if fault_from_user(&stack) {
        recover_user_fault(&stack, 0, &alloc::format!("#SS err={:#x}", error_code));
    }
    serial_println!("[PANIC] Stack segment fault (error={:#x})", error_code);
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack: InterruptStackFrame,
    error_code: u64,
) {
    if fault_from_user(&stack) {
        recover_user_fault(&stack, 0, &alloc::format!("#GP err={:#x}", error_code));
    }
    serial_println!("[PANIC] GPF (error={:#x})", error_code);
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn alignment_check_handler(
    stack: InterruptStackFrame,
    error_code: u64,
) {
    if fault_from_user(&stack) {
        recover_user_fault(&stack, 0, &alloc::format!("#AC err={:#x}", error_code));
    }
    serial_println!("[PANIC] Alignment check (error={:#x})", error_code);
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
        "iretq"
    );
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

/// Aborts a faulting legacy user task without returning to it.
///
/// Mirrors `sys_exit`: it clobbers the interrupt stack frame's implicit return
/// by restoring the kernel's saved RSP/RIP (recorded by `user.rs` just before
/// `iretq` into Ring 3). Control therefore resumes in the kernel loop and the
/// kernel keeps running even though the user process died of a fault. This is
/// only valid for the legacy synchronous `user::execute_ring3_app`/`exec_elf`
/// path, which sets `user::KERNEL_RSP`/`user::KERNEL_RIP`. Process-model
/// processes must be recovered through [`recover_user_fault`] instead.
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

/// Recover a Ring-3 fault under the process model: terminate the faulting
/// process and resume the kernel (cooperative executor or next ready process)
/// instead of halting. `why` is a short human-readable fault description.
///
/// Falls back to the legacy [`kill_user_task`] only when no process-model
/// process is current — i.e. a legacy `user::execute_ring3_app`/`exec_elf` app
/// faulted and its `user::KERNEL_RSP`/`KERNEL_RIP` frame is valid.
///
/// Safety: the faulting process runs in Ring 3, so no kernel `Mutex` guard is
/// held by it (`SCHEDULER`/`EXECUTOR_RESUME` guards are dropped before every
/// `iretq`). The handler executes on TSS RSP0 — the faulting process's own
/// kernel stack — and `exit_current` abandons that stack via `jump_to_initial`
/// once the process is terminated, which is sound.
fn recover_user_fault(stack: &InterruptStackFrame, addr: u64, why: &str) -> ! {
    if crate::task::process::current_is_user_process() {
        if let Some((pid, name)) = crate::task::process::current_process_info() {
            crate::ktrace::log_trace(crate::klog::LogLevel::Warn, format_args!("PROC_FAULT pid={} name='{}' rip={:#x} addr={:#x} {}", pid, name, stack.instruction_pointer, addr, why));
            crate::serial_println!(
                "[USER-FAULT] process {} ('{}') faulted: rip={:#x}, addr={:#x}, {}",
                pid,
                name,
                stack.instruction_pointer,
                addr,
                why,
            );
        } else {
            crate::ktrace::log_trace(crate::klog::LogLevel::Warn, format_args!("PROC_FAULT rip={:#x} addr={:#x} {}", stack.instruction_pointer, addr, why));
            crate::serial_println!(
                "[USER-FAULT] user fault recovered: rip={:#x}, addr={:#x}, {}",
                stack.instruction_pointer,
                addr,
                why,
            );
        }
        // Diverges: marks the process Terminated, pushes it to
        // KILLED_PROCESSES, and resumes the cooperative executor (or switches
        // to the next ready process under the preemptive scheduler).
        crate::task::process::exit_current();
    }

    // Legacy path: no process-model process is current; restore the legacy
    // kernel frame so the kernel loop continues after the faulting app.
    crate::serial_println!(
        "[USER-FAULT] killed user task (legacy): rip={:#x}, addr={:#x}, {}",
        stack.instruction_pointer,
        addr,
        why,
    );
    kill_user_task()
}

extern "x86-interrupt" fn page_fault_handler(
    stack: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let addr = x86_64::registers::control::Cr2::read_raw();

    // Distinguish user vs kernel faults using the saved code-segment RPL.
    if fault_from_user(&stack) {
        // User fault: the process touched memory it is not allowed to. Recover
        // it under the process model. `recover_user_fault` diverges.
        recover_user_fault(&stack, addr, &alloc::format!("err={:?}", error_code));
    }

    // Kernel-space fault: this is a real kernel bug; halt and panic. Reached
    // only for kernel faults because the user branch above diverged.
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
    let current_tick = TICK.fetch_add(1, Ordering::Relaxed) + 1;

    // Aşama 7.2: Lend Expiry — süresi dolan ödünç capability'leri otomatik iptal (revoke) et.
    crate::cap::expire_lent_capabilities(current_tick);

    // Preemptive timer hook: drives the round-robin process scheduler when
    // it has been armed (default off, so existing behavior is unchanged).
    crate::task::process::timer_tick();

    // Aşama 5.1: IRQ notification (IRQ 0). Bağ yoksa irq_event tek atomic load
    // ile no-op'tur — timer her tick'te boş push yapmaz. Headless QEMU regresyonu
    // için deterministik olay kaynağı da burasıdır.
    crate::ipc::irq_event(0, 0);

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
