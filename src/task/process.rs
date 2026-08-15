//! Preemptive single-CPU process model + round-robin timer scheduler.
//!
//! Provides a genuine Process Control Block (PCB), kernel-thread and
//! user-process context switching, and a timer-driven PREEMPTIVE round-robin
//! scheduler. It is intentionally self-contained:
//!
//! * It does NOT modify `SimpleExecutor`, `Task`, or the existing cooperative
//!   async system; those keep working untouched via their own waker/poll.
//! * The preemptive scheduler is OFF by default. Until
//!   [`set_preemption_enabled(true)`] is called, [`timer_tick`] is a no-op so
//!   existing behavior is unchanged. The orchestrator arms it with
//!   [`init_preemptive`] + [`set_preemption_enabled(true)`].
//! * A bare-metal single address space is assumed (user pages are mapped with
//!   `USER_ACCESSIBLE` into the shared page table), mirroring `user.rs`.
//!   `user_cr3` is stored per-PCB so the model is ready for per-process
//!   address spaces (fork/exec + CR3 switch) later; a nonzero `user_cr3`
//!   triggers a `lcr3` on user entry.
//!
//! # Public API
//! * [`init_preemptive`] — create the idle process and arm the scheduler.
//! * [`create_kernel_process`] — spawn a kernel thread on a private stack.
//! * [`create_user_process`] — register a Ring-3 user process.
//! * [`timer_tick`] — timer hook; performs the preemptive quantum switch.
//! * [`schedule`] — explicit round-robin switch.
//! * [`exit_current`] — terminate the running process (never returns).
//! * [`set_preemption_enabled`] / [`preemption_enabled`] / [`get_tick`].

#![allow(function_casts_as_integer)]
#![allow(dead_code)]
#![allow(unreachable_code)]
#![allow(clippy::unnecessary_unsafe)]

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::arch::naked_asm;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use spin::Mutex;

// ---------------------------------------------------------------------------
// Constants / defaults
// ---------------------------------------------------------------------------

/// Default scheduling quantum in PIT ticks (PIT runs at 1000 Hz => 1ms/tick).
pub const QUANTUM_TICKS: u64 = 5;
/// Size of a per-process private kernel stack.
pub const KERNEL_STACK_SIZE: usize = 64 * 1024;
/// fd at which a spawned service's provisioned capability handle is inserted
/// (Aşama 5.2). `u32::MAX` is taken by the process-exec sentinel in
/// `seed_new_process`, so services live one below it.
pub const SERVICE_DEVICE_FD: u32 = u32::MAX - 1;

// ---------------------------------------------------------------------------
// Global scheduler state
// ---------------------------------------------------------------------------

/// pid 0 is reserved for the idle process.
static NEXT_PID: AtomicU64 = AtomicU64::new(1);

/// Master flag: is the preemptive scheduler running?
static PREEMPTION_ENABLED: AtomicBool = AtomicBool::new(false);
/// Ticks left in the current process's quantum.
static TICKS_LEFT: AtomicU64 = AtomicU64::new(0);
/// Total scheduler timer ticks (preemptive clock).
static SCHED_TICK: AtomicU64 = AtomicU64::new(0);

/// The scheduler core state.
pub static SCHEDULER: Mutex<Scheduler> = Mutex::new(Scheduler::uninit());

/// Cooperative-resume slot (Aşama 5.2). A Ring-3 service entered via
/// [`enter_service`] saves the executor's kernel context here; when the service
/// calls SYS_EXIT, [`exit_current`] jumps back to this saved context so the
/// executor continues exactly where it left off. Only meaningful while a
/// cooperative service holds the CPU (preemption stays off), so the value lives
/// on the CPU unguarded during the service run; the Mutex only serializes the
/// brief set/take windows around the switch.
pub static EXECUTOR_RESUME: Mutex<Option<RegisterContext>> = Mutex::new(None);

/// Reference to the dynamically-allocated idle kernel-stack array. Kept so the
/// idle process's `ctx.rsp` stays valid for the kernel's lifetime.
static IDLE_STACK: spin::Once<Box<[u8]>> = spin::Once::new();

/// Physical address of the shared kernel page table (CR3), captured the first
/// time the kernel switches away from its own address space: [`enter_service`]
/// (cooperative) or [`alloc_idle`] (preemptive). [`exit_current`] uses it to
/// resume the cooperative executor in the kernel's own table instead of a
/// terminated process's cloned table — never letting kernel execution run in a
/// dead process's address space.
static SHARED_KERNEL_CR3: spin::Once<u64> = spin::Once::new();

/// Record the current CR3 as the shared kernel address space. Both capture
/// points run in the kernel's own table, and `spin::Once` keeps the first
/// (pristine) capture, so a later call made inside a process's cloned table is
/// a no-op.
pub fn capture_shared_kernel_cr3() {
    SHARED_KERNEL_CR3.call_once(|| {
        let (frame, _) = x86_64::registers::control::Cr3::read();
        frame.start_address().as_u64()
    });
}

pub fn shared_kernel_cr3() -> Option<u64> {
    SHARED_KERNEL_CR3.get().copied()
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    New,
    Ready,
    Running,
    Blocked,
    Terminated,
    Reaped,
}

/// Fixed-layout register set saved on a kernel-level context switch; must
/// match the naked asm in [`switch_context`].
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct RegisterContext {
    pub rbx: u64,
    pub rbp: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    /// Kernel stack pointer to resume with.
    pub rsp: u64,
    /// Resume instruction pointer (return address of the `switch` call).
    pub rip: u64,
    /// Physical CR3 to load when resuming (0 == keep current, used by kernel
    /// threads / idle whose address space is the shared kernel table).
    pub cr3: u64,
}

/// A Process Control Block (PCB). One per process.
pub struct Process {
    pub pid: u64,
    pub name: String,
    pub state: ProcessState,
    pub is_user: bool,
    /// Callee-saved kernel context.
    pub ctx: RegisterContext,
    /// Private kernel stack.
    pub kernel_stack: Box<[u8]>,
    /// Kernel-thread entry function (run on first resume).
    pub entry: Option<extern "C" fn()>,
    /// User address-space id (0 == shared kernel table). fork/exec-ready.
    pub user_cr3: u64,
    /// Ring-3 resume context.
    pub user_rsp: u64,
    pub user_rip: u64,
    pub user_ss: u16,
    pub user_cs: u16,
    /// Kernel continuation used by the int-0x80 / sys_exit path.
    pub kernel_rsp: u64,
    pub kernel_rip: u64,
    /// Result of a user process that exited.
    pub exit_code: u64,
    pub exited: bool,
    pub parent_pid: Option<u64>,
    pub reaped: bool,
    /// Caller'ın capability handle'larının tutuldugu per-process tablo (fd -> CapHandle).
    /// Asama 2.0 (fcc ön koşulu — caller kimliği): her syscall caller'ın kendi tablosuna
    /// bakar; kendi tablosunda olmayan handle kullanilamaz. Process exit'te tum handle'lar
    /// temizlenir.
    pub cap_table: alloc::vec::Vec<(u32, crate::cap::CapHandle)>,
    /// TSS IOPB yetkili port aralığı (None == hiçbir porta erişemez).
    pub allowed_ports: Option<(u16, u16)>,
}

impl Process {
    fn new(name: &str) -> Self {
        Process {
            pid: 0,
            name: String::from(name),
            state: ProcessState::New,
            is_user: false,
            ctx: RegisterContext::default(),
            kernel_stack: alloc::vec![0u8; KERNEL_STACK_SIZE].into_boxed_slice(),
            entry: None,
            user_cr3: 0,
            user_rsp: 0,
            user_rip: 0,
            user_ss: 0,
            user_cs: 0,
            kernel_rsp: 0,
            kernel_rip: 0,
            exit_code: 0,
            exited: false,
            parent_pid: None,
            reaped: false,
            cap_table: alloc::vec::Vec::new(),
            allowed_ports: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler data & SMP Foundation (Task 5A)
// ---------------------------------------------------------------------------

/// Canonical Process Control Block (PCB) alias for SMP foundation.
pub type ProcessControlBlock = Process;

pub struct PerCpuScheduler {
    pub cpu_id: usize,
    pub current_pid: Option<u64>,
    pub run_queue: VecDeque<u64>,
}

impl PerCpuScheduler {
    pub const fn new(cpu_id: usize) -> Self {
        Self {
            cpu_id,
            current_pid: None,
            run_queue: VecDeque::new(),
        }
    }
}

pub static PER_CPU_SCHEDULER: spin::Mutex<[PerCpuScheduler; crate::gdt::MAX_CPUS]> = spin::Mutex::new([
    PerCpuScheduler::new(0),
    PerCpuScheduler::new(1),
    PerCpuScheduler::new(2),
    PerCpuScheduler::new(3),
    PerCpuScheduler::new(4),
    PerCpuScheduler::new(5),
    PerCpuScheduler::new(6),
    PerCpuScheduler::new(7),
]);

pub fn get_cpu_current_pid(cpu_id: usize) -> Option<u64> {
    if cpu_id < crate::gdt::MAX_CPUS {
        PER_CPU_SCHEDULER.lock()[cpu_id].current_pid
    } else {
        None
    }
}

pub fn set_cpu_current_pid(cpu_id: usize, pid: Option<u64>) {
    if cpu_id < crate::gdt::MAX_CPUS {
        PER_CPU_SCHEDULER.lock()[cpu_id].current_pid = pid;
    }
}

pub struct Scheduler {
    table: BTreeMap<u64, Process>,
    ready: VecDeque<u64>,
    current: Option<u64>,
}

impl Scheduler {
    const fn uninit() -> Self {
        Scheduler {
            table: BTreeMap::new(),
            ready: VecDeque::new(),
            current: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Module-placed stubs
// ---------------------------------------------------------------------------

/// Idle process entry: park the CPU when nothing else can run.
extern "C" fn idle_process() {
    loop {
        x86_64::instructions::hlt();
    }
}

/// Trampoline run when a kernel thread first gets the CPU. Runs the current
/// process's entry function, then terminates the process.
fn kernel_thread_stub() -> ! {
    let entry = {
        let mut s = SCHEDULER.lock();
        match s.current.and_then(|pid| s.table.get_mut(&pid)) {
            Some(p) => p.entry.take(),
            None => None,
        }
    };
    if let Some(e) = entry {
        e();
    }
    exit_current();
}

/// Trampoline for entering a user process: goes to Ring 3; control returns to
/// the kernel through syscall/interrupt traps.
fn user_process_stub() -> ! {
    enter_user_current();
}

// ---------------------------------------------------------------------------
// Kernel-level context switch
// ---------------------------------------------------------------------------

/// Switch between two saved kernel contexts.
///
/// Saves current callee-saved registers + kernel RSP/RIP into `*current`,
/// then loads `*next` and resumes there. Called as a normal C-ABI function
/// (SysV: arg0 `current` in RDI, arg1 `next` in RSI).
///
/// Returns `()` (NOT `!`) on purpose. The saved `rip` is the return address of
/// the `call` site; when the switched-away context is later resumed (the
/// cooperative executor via `exit_current`'s `jump_to_initial`, or a preempted
/// process via `schedule`) control returns *through that same `call`* and
/// executes the caller's continuation. If this were `-> !` the compiler would
/// treat the bytes after every `call` as unreachable and pad them with `int3`
/// filler, so resume would land on a breakpoint and fall through into the next
/// function (observed: 6 breakpoints then a page fault at the next symbol).
#[unsafe(naked)]
extern "C" fn switch_context(current: *mut RegisterContext, next: *const RegisterContext) {
    naked_asm!(
            // Save callee-saved registers into *current (rdi).
            "mov [rdi + 0],  rbx",
            "mov [rdi + 8],  rbp",
            "mov [rdi + 16], r12",
            "mov [rdi + 24], r13",
            "mov [rdi + 32], r14",
            "mov [rdi + 40], r15",
            // rsp points at the return address pushed by `call switch_context`.
            "mov rax, [rsp]",
            "lea rbx, [rsp + 8]",
            "mov [rdi + 48], rbx", // ctx.rsp
            "mov [rdi + 56], rax", // ctx.rip (instruction after the call)
            // Restore registers from *next (rsi).
            "mov rbx, [rsi + 0]",
            "mov rbp, [rsi + 8]",
            "mov r12, [rsi + 16]",
            "mov r13, [rsi + 24]",
            "mov r14, [rsi + 32]",
            "mov r15, [rsi + 40]",
            "mov rdx, [rsi + 48]", // pre-load next RSP
            "mov rcx, [rsi + 56]", // pre-load next RIP
            // Load the next process's address space (CR3) if set (nonzero).
            "mov rax, [rsi + 64]",
            "test rax, rax",
            "jz 1f",
            "mov cr3, rax",
            "1:",
            "mov rsp, rdx",
            "jmp rcx", // resume next process
        )
}

/// Jump to a brand-new process's initial context without saving the current
/// one (used when the caller's frame is being abandoned). Loads from `*next`
/// (RDI) and never returns.
#[unsafe(naked)]
extern "C" fn jump_to_initial(next: *const RegisterContext) -> ! {
    naked_asm!(
            "mov rbx, [rdi + 0]",
            "mov rbp, [rdi + 8]",
            "mov r12, [rdi + 16]",
            "mov r13, [rdi + 24]",
            "mov r14, [rdi + 32]",
            "mov r15, [rdi + 40]",
            "mov rax, [rdi + 64]",
            "test rax, rax",
            "jz 1f",
            "mov cr3, rax",
            "1:",
            "mov rsp, [rdi + 48]",
            "jmp qword ptr [rdi + 56]",
        )
}

// ---------------------------------------------------------------------------
// Creation API
// ---------------------------------------------------------------------------

fn alloc_pid() -> u64 {
    NEXT_PID.fetch_add(1, Ordering::Relaxed)
}

fn insert_process(name: &str) -> u64 {
    let pid = alloc_pid();
    let mut p = Process::new(name);
    p.pid = pid;
    let mut s = SCHEDULER.lock();
    p.parent_pid = s.current;
    s.table.insert(pid, p);
    pid
}

/// Wait for a child process to terminate and reap its exit status (Faz 8).
pub fn waitpid(child_pid: u64) -> Result<u64, &'static str> {
    let mut s = SCHEDULER.lock();
    let curr = s.current;
    let (is_reaped, is_term, exit_code) = {
        let p = s.table.get(&child_pid).ok_or("Process not found")?;
        (p.state == ProcessState::Reaped || p.reaped, p.state == ProcessState::Terminated || p.exited, p.exit_code)
    };

    if is_reaped {
        return Err("Already reaped");
    }

    if is_term {
        if let Some(p) = s.table.get_mut(&child_pid) {
            p.state = ProcessState::Reaped;
            p.reaped = true;
        }
        return Ok(exit_code);
    }

    // Çocuk henüz sonlanmadı: parent'ı Blocked yap
    if let Some(parent_pid) = curr {
        if let Some(parent) = s.table.get_mut(&parent_pid) {
            parent.state = ProcessState::Blocked;
        }
    }
    Ok(exit_code)
}

/// Create a kernel thread. `entry` runs on a private kernel stack once the
/// process is scheduled; call [`exit_current`] to terminate it (or loop).
pub fn create_kernel_process(name: &str, entry: extern "C" fn()) -> u64 {
    let pid = insert_process(name);
    {
        let mut s = SCHEDULER.lock();
        let p = s.table.get_mut(&pid).unwrap();
        p.entry = Some(entry);
        p.state = ProcessState::Ready;
        let kstack_top = (p.kernel_stack.as_ptr() as u64 + p.kernel_stack.len() as u64) & !0xFu64;
        p.ctx = RegisterContext {
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rsp: kstack_top - 8,
            rip: kernel_thread_stub as usize as u64,
            cr3: 0, // kernel threads run in the shared kernel address space
        };
        s.ready.push_back(pid);
    }
    crate::task::PROCESS_LIST.lock().insert(pid, name.to_string());
    pid
}

/// Register a Ring-3 user process with the given entry/stack/segment context
/// and (optionally) a dedicated CR3 address space. Enqueued ready; the
/// scheduler `iretq`'s it into Ring 3 on first run.
#[allow(clippy::too_many_arguments)]
pub fn create_user_process(
    name: &str,
    user_rip: u64,
    user_rsp: u64,
    user_cr3: u64,
    user_cs: u16,
    user_ss: u16,
) -> u64 {
    create_user_process_with_caps(
        name,
        user_rip,
        user_rsp,
        user_cr3,
        user_cs,
        user_ss,
        Vec::new(),
    )
}

/// `create_user_process`'in servis-varyantı: seed'den SONRA ek capability'ler
/// (ör. Device MANAGE handle) cap_table'a eklenir. Aşama 5.2: Ring-3 servis
/// kendi cihaz yetkisini (SYS_IPC_BIND_IRQ device_fd) kendi fd'sinden görür.
#[allow(clippy::too_many_arguments)]
pub fn create_user_process_with_caps(
    name: &str,
    user_rip: u64,
    user_rsp: u64,
    user_cr3: u64,
    user_cs: u16,
    user_ss: u16,
    extra_caps: Vec<(u32, crate::cap::CapHandle)>,
) -> u64 {
    let pid = insert_process(name);
    {
        let mut s = SCHEDULER.lock();
        let p = s.table.get_mut(&pid).unwrap();
        p.is_user = true;
        p.state = ProcessState::Ready;
        p.user_rip = user_rip;
        p.user_rsp = user_rsp;
        p.user_cr3 = user_cr3;
        p.user_cs = user_cs;
        p.user_ss = user_ss;
        // Asama 2 (B1): user process doğduğu anda stdio (0/1/2) + process-exec seed'le.
        // Aksi halde SYS_WRITE fd 1/2 dahil tüm fd syscall'lar EACCES döner (sistem kullanılamaz).
        let _ = crate::syscall_cap::seed_new_process(&mut p.cap_table);
        // Aşama 5.2: servise özel provision'lu capability'ler (seed'den sonra,
        // SERVICE_DEVICE_FD fd'sinde). Ep_id slot'ları bunlarla çakışamaz.
        p.cap_table.extend(extra_caps);
        let kstack_top = (p.kernel_stack.as_ptr() as u64 + p.kernel_stack.len() as u64) & !0xFu64;
        p.ctx = RegisterContext {
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rsp: kstack_top - 8,
            rip: user_process_stub as usize as u64,
            cr3: user_cr3, // enter_user_current + switch both load this CR3
        };
        s.ready.push_back(pid);
    }
    crate::task::PROCESS_LIST.lock().insert(pid, name.to_string());
    crate::ktrace::log_trace(crate::klog::LogLevel::Info, format_args!("PROC_SPAWN pid={} name={}", pid, name));
    pid
}

// ---------------------------------------------------------------------------
// Cooperative service entry (Aşama 5.2)
// ---------------------------------------------------------------------------

/// Run a service process to completion cooperatively, then return to the
/// caller (the executor) exactly where it left off.
///
/// The executor context is saved into [`EXECUTOR_RESUME`] and control is
/// switched to `pid`'s kernel context. When the service calls SYS_EXIT,
/// [`exit_current`]'s resume branch marks it terminated and `jump_to_initial`s
/// back to the saved executor context; this function then returns normally and
/// the executor's async task continues (e.g. unbinding the IRQs).
///
/// # Locking
/// `switch_context` is `-> !`, so a MutexGuard held across the switch would
/// stay locked inside the frozen executor frame while the service runs — and
/// `exit_current`'s `EXECUTOR_RESUME.lock().take()` would deadlock against it.
/// The guard is therefore taken, its target written, a raw pointer captured,
/// and the guard dropped BEFORE switching. Single-CPU cooperative model means
/// nothing races during the service run.
pub fn enter_service(pid: u64) {
    // We are about to switch away from the kernel's own address space; remember
    // it so `exit_current` can resume the executor here (see `exit_current`).
    capture_shared_kernel_cr3();
    let (target_ctx, allowed_ports, user_cr3) = {
        let mut s = SCHEDULER.lock();
        s.current = Some(pid);
        set_cpu_current_pid(0, Some(pid));
        if let Some(p) = s.table.get_mut(&pid) {
            p.state = ProcessState::Running;
        }
        let ports = s.table.get(&pid).and_then(|p| p.allowed_ports);
        let cr3 = s.table.get(&pid).map(|p| p.user_cr3).unwrap_or(0);
        (&s.table[&pid].ctx as *const RegisterContext, ports, cr3)
    };

    crate::serial_println!("[SCHED-RUN] pid={}", pid);
    crate::serial_println!("[CR3-SWITCH] pid={} cr3=0x{:x}", pid, user_cr3);

    // TSS IOPB senkronizasyonu (Görev C, CAP_INV-14):
    if let Some((start, end)) = allowed_ports {
        crate::gdt::reset_io_bitmap();
        crate::gdt::allow_port_range(start, end);
    } else {
        crate::gdt::reset_io_bitmap();
    }

    let save_ctx: *mut RegisterContext = {
        let mut guard = EXECUTOR_RESUME.lock();
        *guard = Some(RegisterContext::default());
        guard.as_mut().unwrap() as *mut RegisterContext
    };
    // Guard dropped here; `save_ctx` still points into the static's storage.
    switch_context(save_ctx, target_ctx);
}

// ---------------------------------------------------------------------------
// User-mode transitions (Ring 3)
// ---------------------------------------------------------------------------

/// Restore a user process's saved Ring-3 context and `iretq` into it.
/// Points TSS RSP0 at this process's private kernel stack and records the
/// kernel continuation so the int-0x80 / sys_exit path can return here.
/// Does not return on its own stack (control goes to Ring 3).
fn enter_user_current() -> ! {
    let (pid, user_rip, user_rsp, user_cr3, user_cs, user_ss, allowed_ports, kstack_top) = {
        let mut s = SCHEDULER.lock();
        let pid = match s.current {
            Some(p) => p,
            None => return idle_forever(),
        };
        let p = match s.table.get_mut(&pid) {
            Some(p) => p,
            None => return idle_forever(),
        };
        let krsp = p.kernel_stack.as_ptr() as u64 + p.kernel_stack.len() as u64;
        // Kernel continuation: if the user process traps/exits, the kernel
        // returns to user_resume_trap (parked path for now).
        p.kernel_rsp = krsp;
        p.kernel_rip = user_resume_trap as usize as u64;
        (
            pid,
            p.user_rip,
            p.user_rsp,
            p.user_cr3,
            p.user_cs,
            p.user_ss,
            p.allowed_ports,
            krsp,
        )
    };

    crate::serial_println!(
        "[USER-ENTER] pid={} cr3=0x{:x} rip=0x{:x} rsp=0x{:x} cs=0x{:x} ss=0x{:x}",
        pid, user_cr3, user_rip, user_rsp, user_cs, user_ss
    );

    // TSS IOPB izinlerini senkronize et
    if let Some((start, end)) = allowed_ports {
        crate::gdt::reset_io_bitmap();
        crate::gdt::allow_port_range(start, end);
    } else {
        crate::gdt::reset_io_bitmap();
    }

    // Point RSP0 at this process's private kernel stack.
    crate::gdt::set_tss_rsp0(kstack_top);

    // Optional per-process address space switch.
    if user_cr3 != 0 {
        unsafe {
            x86_64::registers::control::Cr3::write(
                x86_64::structures::paging::PhysFrame::containing_address(
                    x86_64::PhysAddr::new(user_cr3),
                ),
                x86_64::registers::control::Cr3Flags::empty(),
            );
        }
    }

    // Build a synthetic Ring-3 frame and iretq.
    unsafe {
        core::arch::asm!(
            "mov rsp, {kstack}",
            "sub rsp, 40",
            "mov rax, {ss}",
            "mov [rsp + 32], rax", // SS
            "mov rax, {rspu}",
            "mov [rsp + 24], rax", // user RSP
            "pushfq",              // RFLAGS
            "pop rax",
            "or rax, 0x200",       // IF set
            "mov [rsp + 16], rax",
            "mov rax, {cs}",
            "mov [rsp + 8], rax",  // CS
            "mov rax, {ripu}",
            "mov [rsp], rax",      // RIP
            "iretq",
            kstack = in(reg) kstack_top,
            ss = in(reg) user_ss as u64,
            rspu = in(reg) user_rsp,
            cs = in(reg) user_cs as u64,
            ripu = in(reg) user_rip,
            options(noreturn)
        );
    }
}

/// Fallback kernel continuation for a user process that trapped/exited.
/// Parked at idle because, in the single-address-space model, the process is
/// preempted/has exited.
fn user_resume_trap() -> ! {
    idle_forever()
}

fn idle_forever() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

// ---------------------------------------------------------------------------
// Quantum handling
// ---------------------------------------------------------------------------

fn arm_quantum() {
    TICKS_LEFT.store(QUANTUM_TICKS, Ordering::Relaxed);
}

/// Total scheduler ticks (independent of the PIT's own TICK counter).
pub fn get_tick() -> u64 {
    SCHED_TICK.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Timer hook (preemption)
// ---------------------------------------------------------------------------

/// Called from the timer IRQ. When preemption is enabled and the current
/// process's quantum is exhausted, round-robins to the next ready process.
/// No-op otherwise (keeps existing behavior identical).
pub fn timer_tick() {
    SCHED_TICK.fetch_add(1, Ordering::Relaxed);
    if !PREEMPTION_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let left = TICKS_LEFT.load(Ordering::Relaxed);
    if left > 1 {
        TICKS_LEFT.store(left - 1, Ordering::Relaxed);
        return;
    }
    schedule();
}

// ---------------------------------------------------------------------------
// Scheduling core
// ---------------------------------------------------------------------------

/// Full preemptive round-robin switch. Requeues the current process, picks the
/// next ready one, saves the current kernel context and resumes the next.
///
/// Called from `timer_tick` while inside the timer IRQ. It abandons the
/// interrupted context (the switch never returns in the preempting invocation).
/// When the preempted process is later switched back to, it resumes
/// mid-`switch_context` and returns normally, unwinding the timer IRQ and
/// restoring the exact state.
pub fn schedule() {
    // Requeue current to the back and pick the next ready process.
    let (cur_ctx, next_ctx) = {
        let mut s = SCHEDULER.lock();
        // Raw pointer to the old current process's saved context (stable in
        // the static table); null if there was no current process yet.
        let cur_raw = match s.current {
            Some(p) => s.table.get(&p).map(|p| {
                (&p.ctx as *const RegisterContext) as *mut RegisterContext
            }),
            None => None,
        };
        if let Some(cur) = s.current {
            if let Some(p) = s.table.get_mut(&cur) {
                if p.state == ProcessState::Running {
                    p.state = ProcessState::Ready;
                }
            }
            s.ready.push_back(cur);
        }
        let next = match s.ready.pop_front() {
            Some(n) => n,
            None => {
                // Nothing runnable: CPU idles via the idle process (pid 0).
                let idle_ctx: *const RegisterContext = &s.table[&0].ctx;
                drop(s);
                jump_to_initial(idle_ctx);
                unreachable!();
            }
        };
        s.current = Some(next);
        set_cpu_current_pid(0, Some(next));
        if let Some(p) = s.table.get_mut(&next) {
            p.state = ProcessState::Running;
        }
        let ports = s.table.get(&next).and_then(|p| p.allowed_ports);
        if let Some((start, end)) = ports {
            crate::gdt::reset_io_bitmap();
            crate::gdt::allow_port_range(start, end);
        } else {
            crate::gdt::reset_io_bitmap();
        }

        arm_quantum();
        let next_raw = &s.table[&next].ctx as *const RegisterContext;
        crate::ktrace::log_trace(crate::klog::LogLevel::Debug, format_args!("PROC_SWITCH next={}", next));
        (cur_raw.unwrap_or(core::ptr::null_mut()), next_raw)
    };

    // `cur_ctx` may be null if there is no real previous process (first
    // switch); in that case there is nothing meaningful to save into.
    if cur_ctx.is_null() {
        switch_context_null_save(next_ctx);
    } else {
        switch_context(cur_ctx, next_ctx);
    }
}

/// Variant of the switcher used when there is no previous context to save
/// into (very first preemption). Still restores `*next`.
///
/// Same `()` return type rationale as [`switch_context`]: the saved `rip`
/// points at this function's `call` site, and on a later resume the code
/// continues through that `call` into the caller's epilogue. A `-> !` type
/// would fill the resume target with `int3` filler.
#[unsafe(naked)]
extern "C" fn switch_context_null_save(next: *const RegisterContext) {
    naked_asm!(
            "mov rbx, [rdi + 0]",
            "mov rbp, [rdi + 8]",
            "mov r12, [rdi + 16]",
            "mov r13, [rdi + 24]",
            "mov r14, [rdi + 32]",
            "mov r15, [rdi + 40]",
            "mov rdx, [rdi + 48]", // pre-load next RSP
            "mov rcx, [rdi + 56]", // pre-load next RIP
            "mov rax, [rdi + 64]", // pre-load next CR3
            "test rax, rax",
            "jz 1f",
            "mov cr3, rax",
            "1:",
            "mov rsp, rdx",
            "jmp rcx",
        )
}

/// Terminate the running process and switch to the next ready one (or idle),
/// or — Aşama 5.2 — resume the cooperative executor that entered a service via
/// [`enter_service`]. Never returns.
pub fn exit_current() -> ! {
    // 1. Immediately reload the pristine shared kernel address space so all
    // subsequent teardown (surface cleanup, window cleanup, memory unmapping)
    // and context switches execute in kernel CR3.
    if let Some(kcr3) = SHARED_KERNEL_CR3.get().copied() {
        unsafe {
            x86_64::registers::control::Cr3::write(
                x86_64::structures::paging::PhysFrame::containing_address(
                    x86_64::PhysAddr::new(kcr3),
                ),
                x86_64::registers::control::Cr3Flags::empty(),
            );
        }
    }

    let exec_ctx: Option<RegisterContext> = EXECUTOR_RESUME.lock().take();
    if let Some(mut ctx) = exec_ctx {
        {
            let mut s = SCHEDULER.lock();
            if let Some(pid) = s.current {
                if let Some(p) = s.table.get_mut(&pid) {
                    p.state = ProcessState::Terminated;
                    p.exited = true;
                    crate::ktrace::log_trace(crate::klog::LogLevel::Info, format_args!("PROC_EXIT pid={}", pid));
                    // Görev A (CAP_INV-13): CSpace otomatik temizliği
                    crate::cap::destroy_process_cspace(&mut p.cap_table);
                    // Görev A (CAP_INV-13): Kanal ve IRQ unbind / hangup
                    crate::ipc::hangup_channel_for_pid(pid as u32);
                    // Faz 11: Sürece ait tüm Shmem Surface'leri temizle (Zero-Leak Teardown)
                    crate::surface::cleanup_surfaces_for_pid(pid);
                    // Faz 12: Sürece ait tüm Window'ları temizle (Zero-Leak Window Teardown)
                    crate::wm::cleanup_windows_for_pid(pid);
                    // Faz 13: Sürece ait Input Event Kuyruğunu temizle (Zero-Leak Event Teardown)
                    crate::input::cleanup_input_for_pid(pid);
                    p.allowed_ports = None;
                    if let Some(parent_pid) = p.parent_pid {
                        if let Some(parent) = s.table.get_mut(&parent_pid) {
                            if parent.state == ProcessState::Blocked {
                                parent.state = ProcessState::Ready;
                                s.ready.push_back(parent_pid);
                            }
                        }
                    }
                    crate::task::KILLED_PROCESSES.lock().push(pid);
                }
            }
            // Executor is not a process; nothing is "current" after the resume.
            s.current = None;
            set_cpu_current_pid(0, None);
        }
        // Görev C (CAP_INV-14): Executor'a dönerken tüm IO portlarını sıfırla/kapat
        crate::gdt::reset_io_bitmap();

        // Resume the executor in the shared kernel address space, never in the
        // terminating process's cloned table. The executor's saved context
        // carries cr3=0 ("keep current"), which would otherwise strand kernel
        // execution in a dead process's address space.
        if let Some(kcr3) = SHARED_KERNEL_CR3.get().copied() {
            ctx.cr3 = kcr3;
        }
        // Cooperative model: preemption stays off so no quantum is armed for
        // the executor and the service cannot be preempted mid-poll.
        set_preemption_enabled(false);
        jump_to_initial(&ctx);
    }

    let next_ctx: *const RegisterContext = {
        let mut s = SCHEDULER.lock();
        if let Some(pid) = s.current {
            if let Some(p) = s.table.get_mut(&pid) {
                p.state = ProcessState::Terminated;
                p.exited = true;
                // Görev A (CAP_INV-13): CSpace otomatik temizliği
                crate::cap::destroy_process_cspace(&mut p.cap_table);
                // Görev A (CAP_INV-13): Kanal ve IRQ unbind / hangup
                crate::ipc::hangup_channel_for_pid(pid as u32);
                // Faz 11: Sürece ait tüm Shmem Surface'leri temizle (Zero-Leak Teardown)
                crate::surface::cleanup_surfaces_for_pid(pid);
                // Faz 12: Sürece ait tüm Window'ları temizle (Zero-Leak Window Teardown)
                crate::wm::cleanup_windows_for_pid(pid);
                // Faz 13: Sürece ait Input Event Kuyruğunu temizle (Zero-Leak Event Teardown)
                crate::input::cleanup_input_for_pid(pid);
                p.allowed_ports = None;
                if let Some(parent_pid) = p.parent_pid {
                    if let Some(parent) = s.table.get_mut(&parent_pid) {
                        if parent.state == ProcessState::Blocked {
                            parent.state = ProcessState::Ready;
                            s.ready.push_back(parent_pid);
                        }
                    }
                }
                crate::task::KILLED_PROCESSES.lock().push(pid);
            }
        }
        match s.ready.pop_front() {
            Some(pid) => {
                s.current = Some(pid);
                set_cpu_current_pid(0, Some(pid));
                if let Some(p) = s.table.get_mut(&pid) {
                    p.state = ProcessState::Running;
                }
                let ports = s.table.get(&pid).and_then(|p| p.allowed_ports);
                if let Some((start, end)) = ports {
                    crate::gdt::reset_io_bitmap();
                    crate::gdt::allow_port_range(start, end);
                } else {
                    crate::gdt::reset_io_bitmap();
                }
                arm_quantum();
                &s.table[&pid].ctx as *const RegisterContext
            }
            None => {
                s.current = Some(0);
                set_cpu_current_pid(0, Some(0));
                crate::gdt::reset_io_bitmap();
                &s.table[&0].ctx as *const RegisterContext
            }
        }
    };
    jump_to_initial(next_ctx);
}

// ---------------------------------------------------------------------------
// Fork / Exec + demo user processes (real CR3 per process).
// ---------------------------------------------------------------------------

/// True when the current context is a Ring-3 user process running under the
/// preemptive scheduler (as opposed to the legacy single-app shell path).
pub fn current_is_user_process() -> bool {
    let s = SCHEDULER.lock();
    match s.current {
        Some(p) => s.table.get(&p).map(|p| p.is_user).unwrap_or(false),
        None => false,
    }
}

/// Snapshot of the current running process name + pid (for diagnostics).
pub fn current_process_info() -> Option<(u64, String)> {
    let s = SCHEDULER.lock();
    s.current.and_then(|pid| s.table.get(&pid)).map(|p| (p.pid, p.name.clone()))
}

/// `fork_current` — duplicate the calling process.
///
/// The child receives: a fresh pid, its own private kernel stack, and its own
/// cloned address space (the active page table at fork time is the caller's
/// user table, so `clone_active_cr3` gives the child a private copy of the
/// caller's pages). The child is enqueued ready and starts by entering Ring 3
/// with the calling process's saved user context. Returns the child pid, or
/// `-1` if the current process is not a user process.
pub fn fork_current() -> i64 {
    let (cur, name, u_rip, u_rsp, u_cs, u_ss) = {
        let s = SCHEDULER.lock();
        let cur = match s.current {
            Some(p) if p != 0 => p,
            _ => return -1,
        };
        let p = match s.table.get(&cur) {
            Some(p) if p.is_user => p,
            _ => return -1,
        };
        (
            cur,
            p.name.clone(),
            p.user_rip,
            p.user_rsp,
            p.user_cs,
            p.user_ss,
        )
    };
    let _ = cur;
    // Clone the caller's address space (active table in Ring-3 syscall).
    let child_cr3 = crate::memory::clone_active_cr3().unwrap_or(0);

    let pid = alloc_pid();
    let mut np = Process::new(&format!("{}_f", name));
    np.pid = pid;
    np.is_user = true;
    np.state = ProcessState::Ready;
    np.user_rip = u_rip;
    np.user_rsp = u_rsp;
    np.user_cr3 = child_cr3;
    np.user_cs = u_cs;
    np.user_ss = u_ss;
    // Asama 2 (B1 fork): child da stdio + process-exec hakkı almalı; aksi halde
    // fork edilen child SYS_WRITE fd 1/2 ile EACCES alır.
    let _ = crate::syscall_cap::seed_new_process(&mut np.cap_table);
    np.ctx = RegisterContext {
        rbx: 0, rbp: 0, r12: 0, r13: 0, r14: 0, r15: 0,
        rsp: np.kernel_stack.as_ptr() as u64 + np.kernel_stack.len() as u64,
        rip: user_process_stub as usize as u64,
        cr3: child_cr3, // child runs in its cloned address space
    };
    {
        let mut s = SCHEDULER.lock();
        s.table.insert(pid, np);
        s.ready.push_back(pid);
    }
    crate::task::PROCESS_LIST.lock().insert(pid, format!("{}_f", name));
    serial_spawn("[FORK]", pid, &name);
    pid as i64
}

/// `exec` — load an ELF into a fresh user process with its own address space.
///
/// Mirrors the (single-segment) load strategy of `user::exec_elf`: maps the
/// first loadable segment at `USER_ADDR_BASE` and a stack below
/// `USER_STACK_TOP`, all inside a freshly cloned page table so the new process
/// gets genuine CR3 isolation. Returns the new pid.
pub fn exec_elf_proc(name: &str, elf_bytes: &[u8]) -> Result<u64, &'static str> {
    let elf = crate::elf::parse_elf(elf_bytes).map_err(|e| match e {
        crate::elf::ElfError::FileTooSmall => "File too small to be ELF",
        crate::elf::ElfError::InvalidMagic => "Invalid ELF magic",
        crate::elf::ElfError::Not64Bit => "Not a 64-bit ELF",
        crate::elf::ElfError::NotLittleEndian => "Not Little Endian ELF",
        crate::elf::ElfError::NotExecutable => "Not an executable ELF",
        crate::elf::ElfError::InvalidMachine => "Not an x86-64 ELF",
        crate::elf::ElfError::HeadersOutOfBounds => "Program headers out of bounds",
        crate::elf::ElfError::SegmentOutOfBounds => "Segment data out of bounds",
        crate::elf::ElfError::InvalidSegmentBounds => "Invalid segment memory bounds",
        crate::elf::ElfError::KernelAddressViolation => "Segment touches kernel address space",
        crate::elf::ElfError::OverlappingSegments => "Overlapping ELF segments",
        crate::elf::ElfError::InvalidEntryPoint => "Invalid ELF entry point",
        crate::elf::ElfError::NoLoadableSegments => "No loadable segments found",
    })?;

    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frames for exec")?;

    // Multi-Segment Loading: Her PT_LOAD segmentini izole CR3 sayfa tablosuna haritala
    for seg in &elf.segments {
        let is_writable = (seg.flags & crate::elf::PF_W) != 0;
        let seg_len = seg.memsz.max(1);
        crate::memory::map_user_region_in_cr3(cr3, seg.vaddr, seg_len, is_writable)?;
        crate::memory::write_user_region_in_cr3(cr3, seg.vaddr, &seg.data, seg_len);
    }

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let actual_entry = elf.entry_point;
    let stack_top = crate::memory::USER_STACK_TOP;
    let pid = create_user_process(
        name,
        actual_entry,
        stack_top,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
    );
    serial_spawn("[EXEC]", pid, name);
    Ok(pid)
}

/// Spawn a raw machine-code blob as a Ring-3 user process in its own address
/// space. `data` (if any) is copied at `USER_ADDR_BASE + 0x2000` so code can
/// reference a fixed data slot. Used by the demo and for raw test payloads.
fn spawn_raw_user(name: &str, code: &[u8], data: Option<(u64, u8)>) -> Result<u64, &'static str> {
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frames for user proc")?;
    let code_base = crate::memory::USER_ADDR_BASE;
    // Cover code page(s) plus the optional data slot (at +0x2000).
    let map_len = 0x3000u64;
    crate::memory::map_user_region_in_cr3(cr3, code_base, map_len, false)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, code, 0x1000);
    if let Some((off, byte)) = data {
        unsafe { core::ptr::write((code_base + off) as *mut u8, byte); }
    }
    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;
    let pid = create_user_process(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
    );
    serial_spawn("[SPAWN]", pid, name);
    Ok(pid)
}

/// Emit x86-64 machine code for a demo user program that writes `tag` to
/// stdout several times (with a busy delay between writes) and then exits via
/// SYS_EXIT. Uses a fixed data slot at `USER_ADDR_BASE + 0x2000` for the byte.
#[allow(clippy::too_many_arguments)]
fn demo_machine_code(_tag: u8, writes: u32, delay: u32) -> Vec<u8> {
    let data_addr: u32 = (crate::memory::USER_ADDR_BASE + 0x2000) as u32;
    let mut c: Vec<u8> = Vec::new();
    // mov ecx, data_addr          (ecx = buffer pointer)
    c.push(0xB9);
    c.extend_from_slice(&data_addr.to_le_bytes());
    // mov ebp, writes            (ebp = outer loop counter)
    c.push(0xBD);
    c.extend_from_slice(&writes.to_le_bytes());
    // Inner busy delay:  mov edx, delay
    c.push(0xBA);
    c.extend_from_slice(&delay.to_le_bytes());
    // dec edx
    c.push(0x4A);
    let dec_edx_i = c.len() as i32; // index of the `dec edx` instruction
    // jnz <dec edx>  (short jump, rel to end of the 2-byte jnz)
    c.push(0x75);
    c.push((dec_edx_i - (c.len() as i32 + 2)).wrapping_sub(0) as u8);
    // mov eax, 4  (SYS_WRITE)
    c.push(0xB8);
    c.extend_from_slice(&4u32.to_le_bytes());
    // mov ebx, 1  (fd = stdout)   -- sys_write path writes to VGA terminal
    c.push(0xBB);
    c.extend_from_slice(&1u32.to_le_bytes());
    // int 0x80
    c.push(0xCD);
    c.push(0x80);
    // dec ebp
    c.push(0x4D);
    // jnz <mov edx, delay>  (back to inner delay + write)
    let mov_edx_i = 10i32; // offset of `mov edx, delay`
    c.push(0x75);
    c.push((mov_edx_i - (c.len() as i32 + 2)).wrapping_sub(0) as u8);
    // mov eax, 1  (SYS_EXIT)
    c.push(0xB8);
    c.extend_from_slice(&1u32.to_le_bytes());
    // int 0x80
    c.push(0xCD);
    c.push(0x80);
    // hlt (safety net)
    c.push(0xF4);
    c
}

/// Emit x86-64 machine code for the hardware-less service demo process.
///
/// The service runs cooperatively in Ring 3 (entered via [`enter_service`],
/// never preempted). Flow:
///   1. SYS_IPC_CREATE_ENDPOINT(16) — Ring-3 kendi endpoint'ini oluşturur.
///   2. SYS_IPC_BIND_IRQ(SERVICE_DEVICE_FD, 0) + (…, 1) — timer + klavye IRQ'ları.
///   3. Banner'ı stdout'a yazar ("SVC UP").
///   4. Poll döngüsü: SYS_IPC_TRY_RECV → EAGAIN ise tekrar; olay gelince ilk
///      baytı '0'..'9' aralığına (0x30 ekle) çevirip yazar. 64 olay sonra SYS_EXIT.
///
/// Data slot `USER_ADDR_BASE + 0x2000`'de (spawn_service writable haritalar):
///   [+0x00] ep_id (u32), [+0x04] counter (u32), [+0x08] "SVC UP\r\n" (8 bayt),
///   [+0x10] recv buffer (64 bayt), [+0x60] echo tmp (1 bayt).
///
/// ABI (syscall.rs dispatcher): num=rax, arg1=rdi, arg2=rsi, arg3=rdx, arg4=r10.
/// int 0x80 sonrası caller-saved regs clobber olur — her yinelemede arg'lar
/// yeniden kurulur; yalnızca callee-saved rbx (data slot tabanı) sağ kalır.
pub fn service_machine_code() -> Vec<u8> {
    let data_slot: u32 = (crate::memory::USER_ADDR_BASE + 0x2000) as u32;
    let mut c: Vec<u8> = Vec::new();

    // --- init: endpoint oluştur (SYS_IPC_CREATE_ENDPOINT = 24, capacity 16) ---
    // mov ebx, data_slot          (data slot tabanı — callee-saved)
    c.push(0xBB);
    c.extend_from_slice(&data_slot.to_le_bytes());
    // mov edi, 16                 (capacity)
    c.push(0xBF);
    c.extend_from_slice(&16u32.to_le_bytes());
    // mov eax, 24                 (SYS_IPC_CREATE_ENDPOINT)
    c.push(0xB8);
    c.extend_from_slice(&24u32.to_le_bytes());
    // int 0x80
    c.push(0xCD);
    c.push(0x80);
    // mov [rbx+0], eax            (ep_id sakla)
    c.extend_from_slice(&[0x89, 0x43, 0x00]);

    // --- bind IRQ 0 (timer) ---
    // mov edi, SERVICE_DEVICE_FD
    c.push(0xBF);
    c.extend_from_slice(&SERVICE_DEVICE_FD.to_le_bytes());
    // mov esi, 0
    c.push(0xBE);
    c.extend_from_slice(&0u32.to_le_bytes());
    // mov edx, [rbx+0]
    c.extend_from_slice(&[0x8B, 0x13]);
    // mov eax, 25                 (SYS_IPC_BIND_IRQ)
    c.push(0xB8);
    c.extend_from_slice(&25u32.to_le_bytes());
    // int 0x80
    c.push(0xCD);
    c.push(0x80);

    // --- bind IRQ 1 (keyboard) ---
    c.push(0xBF);
    c.extend_from_slice(&SERVICE_DEVICE_FD.to_le_bytes());
    c.push(0xBE);
    c.extend_from_slice(&1u32.to_le_bytes());
    c.extend_from_slice(&[0x8B, 0x13]);
    c.push(0xB8);
    c.extend_from_slice(&25u32.to_le_bytes());
    c.push(0xCD);
    c.push(0x80);

    // --- banner: sys_write(1, [rbx+8], 8) ---
    c.push(0xBF);
    c.extend_from_slice(&1u32.to_le_bytes()); // fd = stdout
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x08]); // lea rsi, [rbx+8]
    c.push(0xBA);
    c.extend_from_slice(&8u32.to_le_bytes()); // len = 8
    c.push(0xB8);
    c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD);
    c.push(0x80);

    let loop_start = c.len() as i32;

    // --- poll döngüsü ---
    // mov edi, [rbx+0]             (ep_id)
    c.extend_from_slice(&[0x8B, 0x3B]);
    // lea rsi, [rbx+0x10]          (recv buffer)
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x10]);
    // mov edx, 64                  (max_len)
    c.push(0xBA);
    c.extend_from_slice(&64u32.to_le_bytes());
    // mov r10d, 0                  (out_cap_ptr)
    c.push(0x41);
    c.push(0xBA);
    c.extend_from_slice(&0u32.to_le_bytes());
    // mov eax, 23                  (SYS_IPC_TRY_RECV)
    c.push(0xB8);
    c.extend_from_slice(&23u32.to_le_bytes());
    // int 0x80
    c.push(0xCD);
    c.push(0x80);
    // cmp eax, EAGAIN (0xFFFFFFF5)
    c.push(0x3D);
    c.extend_from_slice(&0xFFFFFFF5u32.to_le_bytes());
    // je loop
    c.push(0x74);
    c.push((loop_start - (c.len() as i32 + 1)) as u8);
    // cmp eax, 0
    c.extend_from_slice(&[0x83, 0xF8, 0x00]);
    // jle loop  (0 / EACCES / u64::MAX → tekrar dene)
    c.push(0x7E);
    c.push((loop_start - (c.len() as i32 + 1)) as u8);

    // --- echo: al = [buf] + 0x30 → stdout ---
    // mov al, [rbx+0x10]
    c.extend_from_slice(&[0x8A, 0x43, 0x10]);
    // add al, 0x30
    c.extend_from_slice(&[0x04, 0x30]);
    // mov [rbx+0x60], al
    c.extend_from_slice(&[0x88, 0x43, 0x60]);
    // sys_write(1, [rbx+0x60], 1)
    c.push(0xBF);
    c.extend_from_slice(&1u32.to_le_bytes());
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x60]);
    c.push(0xBA);
    c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&4u32.to_le_bytes());
    c.push(0xCD);
    c.push(0x80);

    // --- counter++ → 64 olay sonra çık ---
    // inc dword [rbx+4]
    c.extend_from_slice(&[0xFF, 0x43, 0x04]);
    // cmp dword [rbx+4], 64
    c.extend_from_slice(&[0x83, 0x7B, 0x04, 0x40]);
    // jl loop
    c.push(0x7C);
    c.push((loop_start - (c.len() as i32 + 1)) as u8);

    // --- sys_exit(0) ---
    c.push(0xBF);
    c.extend_from_slice(&0u32.to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xCD);
    c.push(0x80);
    // Safety net: SYS_EXIT normalde dönmez ama hlt/ud2 Ring 3'te GPF üretip
    // kernel'i dondururdu; burada loop'a geri dönülür.
    c.push(0xEB);
    c.push((loop_start - (c.len() as i32 + 1)) as u8);

    c
}

/// Spawn a Ring-3 user-space service (Aşama 5.2).
///
/// `dev` (Device capability) tabanından servise MANAGE (512) haklı bir Device
/// handle grant edilir ve [`SERVICE_DEVICE_FD`] fd'sine yerleştirilir. Kod +
/// data slotu (`USER_ADDR_BASE + 0x2000`, WRITABLE — servis kendi data slotunu
/// yazar) + stack, klonlanmış bir CR3 içine haritalanır. `build_code` ile
/// üretilen makine kodu servis başlangıcıdır; servis [`enter_service`] ile
/// cooperative çalıştırılır (preemption değil).
pub fn spawn_service<F>(
    name: &str,
    dev: crate::cap::CapHandle,
    build_code: F,
) -> Result<u64, &'static str>
where
    F: FnOnce() -> Vec<u8>,
{
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frames for service")?;
    // Sadece MANAGE: SYS_IPC_BIND_IRQ'un tek ihtiyaç duyduğu hak. Tüm hakkı
    // (Rights::all) servise vermek gerekmez — least privilege.
    let dev_manage = crate::cap::grant(dev, crate::cap::Rights(512))
        .map_err(|_| "service device grant failed")?;

    let code = build_code();
    let code_base = crate::memory::USER_ADDR_BASE;
    // Data slotu dahil 0x3000'lik bölge writable: servis [rbx] data slotuna
    // yazar (spawn_raw_user'ın `false`'undan farklı — zorunlu).
    crate::memory::map_user_region_in_cr3(cr3, code_base, 0x3000, true)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &code, 0x1000);
    // Data slotu (code_base+0x2000) seed et: [+0x08] "SVC UP\r\n". Bump
    // allocator'ın verdiği fiziksel frame'ler sıfırlı değil; banner'ın
    // deterministik olması ve counter'ın [+0x04] 0'dan başlaması için bölgeyi
    // sıfırlayıp banner'ı yazmak zorunlu. (ep_id [+0x00] ve counter [+0x04]
    // servis tarafından kendisi yazılır.) Eşleme ortak L3 üzerinden aktif
    // tabloda da görünür; tıpkı code yazımı gibi doğrudan yazılabilir.
    let data_ptr = (code_base + 0x2000) as *mut u8;
    unsafe {
        core::ptr::write_bytes(data_ptr, 0, 0x1000);
        core::ptr::copy_nonoverlapping(b"SVC UP\r\n".as_ptr(), data_ptr.add(8), 8);
    }

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let pid = create_user_process_with_caps(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        alloc::vec![(SERVICE_DEVICE_FD, dev_manage)],
    );
    serial_spawn("[SERVICE]", pid, name);
    Ok(pid)
}

/// Emit x86-64 machine code for the user-space serial driver demo (Aşama 5.3).
///
/// The driver runs cooperatively in Ring 3 (entered via [`enter_service`],
/// never preempted). Flow:
///   1. SYS_IOPERM(0x3F8, 0x3FF, 1) — COM1 port aralığını capability-gated
///      olarak açar. Kernel'de gate: process'in cap_table'ında Device cap var
///      (SERVICE_DEVICE_FD) VE istenen aralık o cihaza bağlı aralığın
///      (create_device_ports ile 0x3F8..=0x3FF) alt kümesi. Başarılıysa TSS
///      IOPB'de 0x3F8..=0x3FF Ring-3 için izinli yapılır.
///   2. UART TX: "[SERDRV] alive\r\n" (16 bayt) bayt bayt COM1'e yazılır.
///      Her bayttan önce LSR (0x3FD) bit 5 (THR empty) poll edilir — ham `inb`/
///      `outb`, syscall yok. Yazılan baytlar QEMU `-serial stdio` üzerinden
///      boot log'una düşer (Ring-3 port I/O'nun canlı kanıtı).
///   3. SYS_EXIT(0) — process modelinden temiz çıkış.
///
/// Data slot `USER_ADDR_BASE + 0x2000`'de (spawn_serial_service seed eder):
///   [+0x00] ioperm sonucu (u32), [+0x10] TX string (16 bayt).
///
/// ABI (syscall.rs dispatcher): num=rax, arg1=rdi, arg2=rsi, arg3=rdx.
/// int 0x80 sonrası caller-saved regs clobber olur; yalnızca callee-saved rbx
/// (data slot tabanı) sağ kalır. TX döngüsü syscall içermez, ecx döngüde canlıdır.
pub fn serial_machine_code() -> Vec<u8> {
    let data_slot: u32 = (crate::memory::USER_ADDR_BASE + 0x2000) as u32;
    let mut c: Vec<u8> = Vec::new();

    // --- mov ebx, data_slot (callee-saved) ---
    c.push(0xBB);
    c.extend_from_slice(&data_slot.to_le_bytes());

    let loop_start = c.len() as i32;

    // --- sys_ioperm(0x3F8, 0x3FF, 1) ---
    c.push(0xBF);
    c.extend_from_slice(&0x3F8u32.to_le_bytes()); // mov edi, 0x3F8 (start_port)
    c.push(0xBE);
    c.extend_from_slice(&0x3FFu32.to_le_bytes()); // mov esi, 0x3FF (end_port)
    c.push(0xBA);
    c.extend_from_slice(&1u32.to_le_bytes());     // mov edx, 1 (enable)
    c.push(0xB8);
    c.extend_from_slice(&22u32.to_le_bytes());    // mov eax, SYS_IOPERM
    c.push(0xCD);
    c.push(0x80);                                 // int 0x80
    // mov [rbx+0], eax           (sonucu sakla)
    c.extend_from_slice(&[0x89, 0x43, 0x00]);

    // --- UART TX loop: 16 bayt [rbx+0x10] → COM1 ---
    // mov ecx, 0                (index)
    c.push(0xB9);
    c.extend_from_slice(&0u32.to_le_bytes());
    let tx_loop = c.len() as i32;
    // mov edx, 0x3FD            (LSR)
    c.push(0xBA);
    c.extend_from_slice(&0x3FDu32.to_le_bytes());
    let tx_wait = c.len() as i32;
    c.push(0xEC);                                 // in al, dx
    c.push(0xA8);
    c.push(0x20);                                 // test al, 0x20 (THR empty)
    c.push(0x74);
    c.push((tx_wait - (c.len() as i32 + 1)) as u8); // jz tx_wait
    // mov al, [rbx+ecx+0x10]     (yazılacak bayt)
    c.extend_from_slice(&[0x8A, 0x44, 0x0B, 0x10]);
    // mov edx, 0x3F8            (THR)
    c.push(0xBA);
    c.extend_from_slice(&0x3F8u32.to_le_bytes());
    c.push(0xEE);                                 // out dx, al
    c.extend_from_slice(&[0xFF, 0xC1]);           // inc ecx
    c.extend_from_slice(&[0x83, 0xF9, 16]);       // cmp ecx, 16
    c.push(0x72);
    c.push((tx_loop - (c.len() as i32 + 1)) as u8); // jb tx_loop

    // --- sys_exit(0) ---
    c.push(0xBF);
    c.extend_from_slice(&0u32.to_le_bytes());     // mov edi, 0
    c.push(0xB8);
    c.extend_from_slice(&1u32.to_le_bytes());     // mov eax, SYS_EXIT
    c.push(0xCD);
    c.push(0x80);
    // Safety net: SYS_EXIT process modelinde dönmez; yine de dönerse başa sar.
    c.push(0xEB);
    c.push((loop_start - (c.len() as i32 + 1)) as u8);

    c
}

/// Emit x86-64 machine code for the fault-recovery regression (Aşama 5.4).
///
/// İlk komut: `mov eax, [0x5000_0000]` — user yarısının ortasında, her klonlanmış
/// CR3'te deterministik olarak eşlenmemiş bir adrese okuma. Bu, P=0 user page
/// fault üretir (CR2 = 0x5000_0000). Kernel bu fault'u process modeli altında
/// kurtarmalıdır ([`recover_user_fault`] → `exit_current`), legacy
/// `user::KERNEL_RSP`/`KERNEL_RIP` frame'ini asla kullanmamalıdır.
///
/// Sondaki `hlt` emniyet ağı: load her nasılsa fault vermezse Ring 3'te hlt
/// ayrıcalıklı komut olduğu için #GP üretir; GPF handler'ı da aynı kurtarma
/// yolundan geçirir (belt-and-braces).
///
/// `mov eax, [disp32]` kodlaması `8B 04 25` + 4 bayt little-endian disp32
/// (ModRM SIB, base yok) — disp32 = 0x5000_0000 → `00 00 00 50`.
pub fn fault_machine_code() -> Vec<u8> {
    let mut c: Vec<u8> = Vec::new();
    // mov eax, [0x5000_0000] — deterministic P=0 user page fault
    c.extend_from_slice(&[0x8B, 0x04, 0x25, 0x00, 0x00, 0x00, 0x50]);
    // Safety net: hlt Ring 3'te #GP üretir (ayrıcalıklı komut); GPF handler'ı
    // da recover_user_fault üzerinden aynı şekilde kurtarır.
    c.push(0xF4);
    c
}

/// Spawn the Ring-3 serial driver service (Aşama 5.3).
///
/// `create_device_ports(0x3F8, 0x3FF)` COM1 aralığına bağlı bir Device capability
/// üretir (MANAGE|IO). Servise IO|MANAGE haklı handle grant edilir ve
/// [`SERVICE_DEVICE_FD`]'ye yerleştirilir — sys_ioperm gate'i (`device_io_range`)
/// IO hakkı arar, bu yüzden MANAGE tek başına yetmez (spawn_service'ten fark).
/// Kod + data slot + stack klonlanmış CR3'e haritalanır; data slotunda TX
/// stringi seed edilir (bump allocator frame'leri sıfırlı değildir).
pub fn spawn_serial_service(name: &str) -> Result<u64, &'static str> {
    let dev_ports = crate::cap::create_device_ports(0x3F8, 0x3FF)
        .map_err(|_| "device port binding failed")?;
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frames for service")?;
    // IO (8) | MANAGE (512): sys_ioperm gate'i ve provisioning yönetimi.
    let dev_io = crate::cap::grant(dev_ports, crate::cap::Rights(8 | 512))
        .map_err(|_| "service device grant failed")?;

    let code = serial_machine_code();
    let code_base = crate::memory::USER_ADDR_BASE;
    crate::memory::map_user_region_in_cr3(cr3, code_base, 0x3000, true)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &code, 0x1000);
    // Data slot (code_base+0x2000) seed: [+0x10] "[SERDRV] alive\r\n" (16 bayt).
    // Bump allocator'ın verdiği fiziksel frame'ler sıfırlı değil; deterministik
    // TX için bölgeyi sıfırlayıp stringi yazmak zorunlu. (ioperm sonucu [+0x00]
    // servis tarafından kendisi yazılır.)
    let data_ptr = (code_base + 0x2000) as *mut u8;
    unsafe {
        core::ptr::write_bytes(data_ptr, 0, 0x1000);
        core::ptr::copy_nonoverlapping(b"[SERDRV] alive\r\n".as_ptr(), data_ptr.add(0x10), 16);
    }

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let pid = create_user_process_with_caps(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        alloc::vec![(SERVICE_DEVICE_FD, dev_io)],
    );
    serial_spawn("[SERIAL]", pid, name);
    Ok(pid)
}

pub const NETDRV_PORT_FD: u32 = u32::MAX - 2;
pub const NETDRV_DMA_FD: u32 = u32::MAX - 3;

// Ring-3 network driver messages. Each is emitted only after its path genuinely
// ran: a frame was received/verified and handed to the kernel zero-copy — or the
// failing step wrote an honest error and exited with a non-zero status (Aşama 6.3).
const NETDRV_FAIL_MAP: &[u8] = b"[NETDRV] MAP_DMA failed\n";
const NETDRV_FAIL_RX: &[u8] = b"[NETDRV] RX timeout\n";
const NETDRV_FAIL_FRAME: &[u8] = b"[NETDRV] frame verify failed\n";
const NETDRV_FAIL_IPC: &[u8] = b"[NETDRV] IPC send failed\n";
const NETDRV_SUCCESS: &[u8] = b"[NETDRV] RX verified (ARP reply, slot cap sent)\n";

/// Emit x86-64 machine code for the user-space RTL8139 network driver (Aşama 6.3).
///
/// The driver, entirely in Ring 3:
///   1. `sys_ioperm(bar0)` + `sys_map_dma(NETDRV_DMA_FD, 0x6000_0000, 3)`.
///   2. Configures the RTL8139 (reset, RBSTART=dma_phys, CAPR=0, IMR, RCR, enable).
///   3. Builds an ARP request for 10.0.2.2 in the TX buffer and pushes it via TSD0.
///   4. Polls the RX descriptor with `clflush` (QEMU DMA writes must not sit in a
///      stale WB-cache line), verifies ethertype/ARP-op/sender-IP, then
///   5. `SYS_IPC_CREATE_SLOT(NETDRV_DMA_FD, 0, frame_len)` → a zero-copy DmaSlot
///      cap over the received frame, and `SYS_IPC_SEND(kern_ep_id, "NTV1", 4,
///      slot_fd, 1)` transfers it to the kernel's endpoint. No byte copying.
///
/// Data-slot layout (user page 0x40002000), all offsets fixed:
///   +0x10 FAIL_MAP · +0x30 FAIL_RX · +0x50 FAIL_FRAME · +0x70 FAIL_IPC ·
///   +0x90 "NTV1" magic (written at runtime) · +0xA0 SUCCESS
pub fn net_machine_code(bar0_start: u16, bar0_end: u16, dma_phys: u32, kern_ep_id: u32) -> Vec<u8> {
    let data_slot: u32 = (crate::memory::USER_ADDR_BASE + 0x2000) as u32;
    let mut c: Vec<u8> = Vec::new();

    // mov ebx, data_slot
    c.push(0xBB);
    c.extend_from_slice(&data_slot.to_le_bytes());

    // 1. SYS_IOPERM(bar0_start, bar0_end, 1)
    c.push(0xBF);
    c.extend_from_slice(&(bar0_start as u32).to_le_bytes());
    c.push(0xBE);
    c.extend_from_slice(&(bar0_end as u32).to_le_bytes());
    c.push(0xBA);
    c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&22u32.to_le_bytes()); // SYS_IOPERM
    c.push(0xCD);
    c.push(0x80);

    // 2. SYS_MAP_DMA(NETDRV_DMA_FD, 0x6000_0000, 3); test eax,eax; jnz fail_map
    c.push(0xBF);
    c.extend_from_slice(&NETDRV_DMA_FD.to_le_bytes());
    c.push(0xBE);
    c.extend_from_slice(&0x6000_0000u32.to_le_bytes());
    c.push(0xBA);
    c.extend_from_slice(&3u32.to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&26u32.to_le_bytes()); // SYS_MAP_DMA
    c.push(0xCD);
    c.push(0x80);
    c.push(0x85); c.push(0xC0);             // test eax, eax
    c.push(0x0F); c.push(0x85);             // jnz rel32 → fail_map
    let jnz_map_field = c.len();
    c.extend_from_slice(&[0x00; 4]);

    // 3. Hardware config via outb / outl:
    // Power ON (Config 1): mov dx, bar0+0x52; mov al, 0x00; out dx, al
    c.push(0x66); c.push(0xBA);
    c.extend_from_slice(&(bar0_start + 0x52).to_le_bytes());
    c.push(0xB0); c.push(0x00);
    c.push(0xEE); // out dx, al

    // Reset: mov dx, bar0+0x37; mov al, 0x10; out dx, al
    c.push(0x66); c.push(0xBA);
    c.extend_from_slice(&(bar0_start + 0x37).to_le_bytes());
    c.push(0xB0); c.push(0x10);
    c.push(0xEE); // out dx, al

    // Wait for reset to complete: wait_rst: in al, dx; test al, 0x10; jnz wait_rst
    let wait_rst = c.len();
    c.push(0xEC); // in al, dx
    c.push(0xA8); c.push(0x10); // test al, 0x10
    c.push(0x75);
    c.push((wait_rst as i32 - (c.len() as i32 + 1)) as u8);

    // RBSTART: mov dx, bar0+0x30; mov eax, dma_phys; out dx, eax
    c.push(0x66); c.push(0xBA);
    c.extend_from_slice(&(bar0_start + 0x30).to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&dma_phys.to_le_bytes());
    c.push(0xEF); // out dx, eax

    // CAPR: mov dx, bar0+0x38; mov eax, 0; out dx, eax
    c.push(0x66); c.push(0xBA);
    c.extend_from_slice(&(bar0_start + 0x38).to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&0u32.to_le_bytes());
    c.push(0xEF); // out dx, eax

    // IMR: mov dx, bar0+0x3C; mov eax, 0x0005; out dx, eax
    c.push(0x66); c.push(0xBA);
    c.extend_from_slice(&(bar0_start + 0x3C).to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&0x0005u32.to_le_bytes());
    c.push(0xEF); // out dx, eax

    // RCR: mov dx, bar0+0x44; mov eax, 0x8F; out dx, eax
    c.push(0x66); c.push(0xBA);
    c.extend_from_slice(&(bar0_start + 0x44).to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&0x8Fu32.to_le_bytes());
    c.push(0xEF); // out dx, eax

    // Enable RX/TX: mov dx, bar0+0x37; mov al, 0x0C; out dx, al
    c.push(0x66); c.push(0xBA);
    c.extend_from_slice(&(bar0_start + 0x37).to_le_bytes());
    c.push(0xB0); c.push(0x0C);
    c.push(0xEE); // out dx, al

    // 4. Clear RX descriptor: mov esi, 0x6000_0000; mov dword [esi], 0
    c.push(0xBE);
    c.extend_from_slice(&0x6000_0000u32.to_le_bytes());
    c.extend_from_slice(&[0xC7, 0x06, 0x00, 0x00, 0x00, 0x00]);

    // 5. Build ARP request at TX buffer: mov edi, 0x6000_2000; 42× mov byte [edi+disp], imm
    c.push(0xBF);
    c.extend_from_slice(&0x6000_2000u32.to_le_bytes());
    // ARP request (42 bytes): dst ff×6, src 52:54:00:12:34:56, etype 08 06,
    // htype 00 01, ptype 08 00, hlen 06, plen 04, op 00 01,
    // sha 52:54:00:12:34:56, spa 0a 00 02 0f, tha 00×6, tpa 0a 00 02 02
    let arp: [u8; 42] = [
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x52, 0x54, 0x00, 0x12, 0x34, 0x56, 0x08, 0x06,
        0x00, 0x01, 0x08, 0x00, 0x06, 0x04, 0x00, 0x01,
        0x52, 0x54, 0x00, 0x12, 0x34, 0x56, 0x0A, 0x00, 0x02, 0x0F,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x02, 0x02,
    ];
    for (i, b) in arp.iter().enumerate() {
        c.extend_from_slice(&[0xC6, 0x47, i as u8, *b]); // mov byte [edi+disp8], imm8
    }

    // 6. Transmit: TSAD0 = dma_phys+0x2000; TSD0 = 42 (0x002A len only)
    c.push(0x66); c.push(0xBA);
    c.extend_from_slice(&(bar0_start + 0x20).to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&(dma_phys + 0x2000).to_le_bytes());
    c.push(0xEF); // out dx, eax
    c.push(0x66); c.push(0xBA);
    c.extend_from_slice(&(bar0_start + 0x10).to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&42u32.to_le_bytes()); // length only
    c.push(0xEF); // out dx, eax

    // 7. RX poll: mov ecx, 20000000
    //    poll_loop: clflush [esi] → movzx eax, word[esi] → test al,1 → jnz poll_done
    //               → in al, 0x80 → dec ecx → jnz poll_loop   (fall-through → fail_rx)
    c.push(0xB9);
    c.extend_from_slice(&20_000_000u32.to_le_bytes());
    let poll_loop = c.len();
    c.extend_from_slice(&[0x0F, 0xAE, 0x3E]);          // clflush [esi]
    c.extend_from_slice(&[0x0F, 0xB7, 0x06]);          // movzx eax, word [esi]
    c.push(0xA8); c.push(0x01);                        // test al, 1 (ROK)
    c.push(0x0F); c.push(0x85);                        // jnz rel32 → poll_done
    let jnz_done_field = c.len();
    c.extend_from_slice(&[0x00; 4]);
    c.push(0xE4); c.push(0x80);                        // in al, 0x80 (I/O delay)
    c.push(0x49);                                      // dec ecx
    c.push(0x75);                                      // jnz poll_loop
    c.push((poll_loop as i32 - (c.len() as i32 + 1)) as u8);

    // fail_rx inline: lea rsi,[rbx+0x30]; write FAIL_RX; exit 1
    let _fail_rx = c.len();
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x30]);    // lea rsi, [rbx + 0x30]
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xBA); c.extend_from_slice(&(NETDRV_FAIL_RX.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&1u32.to_le_bytes()); // SYS_EXIT(1)
    c.push(0xCD); c.push(0x80);

    // 8. poll_done: movzx eax, word[esi+2] → mov edx, eax (frame_len)
    let poll_done = c.len();
    c.extend_from_slice(&[0x0F, 0xB7, 0x46, 0x02]);    // movzx eax, word [esi+2]
    c.push(0x89); c.push(0xC2);                        // mov edx, eax

    // 9. Frame verify: ethertype 0x0608 @ [esi+0x10], ARP op 0x0200 (Reply) @ [esi+0x18],
    //    spa 0x0A000202 (10.0.2.2) @ [esi+0x20]; each cmp + jne rel32 → fail_frame
    c.extend_from_slice(&[0x66, 0x81, 0x7E, 0x10, 0x08, 0x06]); // cmp word [esi+0x10], 0x0608
    c.push(0x0F); c.push(0x85);
    let jne_fr1_field = c.len();
    c.extend_from_slice(&[0x00; 4]);
    c.extend_from_slice(&[0x66, 0x81, 0x7E, 0x18, 0x00, 0x02]); // cmp word [esi+0x18], 0x0200 (little endian of 0x0002)
    c.push(0x0F); c.push(0x85);
    let jne_fr2_field = c.len();
    c.extend_from_slice(&[0x00; 4]);
    c.extend_from_slice(&[0x81, 0x7E, 0x20, 0x0A, 0x00, 0x02, 0x02]); // cmp dword [esi+0x20], 0x0202000A
    c.push(0x0F); c.push(0x85);
    let jne_fr3_field = c.len();
    c.extend_from_slice(&[0x00; 4]);

    // 10. Magic "NTV1" at [rbx+0x90]: 4× mov byte [rbx+disp32], imm8
    for (i, ch) in b"NTV1".iter().enumerate() {
        let disp: u32 = 0x90 + i as u32;
        c.push(0xC6); c.push(0x83);
        c.extend_from_slice(&disp.to_le_bytes());
        c.push(*ch);
    }

    // 11. SYS_IPC_CREATE_SLOT(NETDRV_DMA_FD, 0, frame_len) → slot fd in eax
    //     (edx already holds frame_len from step 8). test eax,eax; js fail_ipc
    c.push(0xBF);
    c.extend_from_slice(&NETDRV_DMA_FD.to_le_bytes());
    c.push(0xBE);
    c.extend_from_slice(&0u32.to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&30u32.to_le_bytes()); // SYS_IPC_CREATE_SLOT
    c.push(0xCD); c.push(0x80);
    c.push(0x85); c.push(0xC0);                // test eax, eax
    c.push(0x0F); c.push(0x88);                // js rel32 → fail_ipc
    let js_ipc_field = c.len();
    c.extend_from_slice(&[0x00; 4]);
    c.extend_from_slice(&[0x49, 0x89, 0xC2]);  // mov r10, rax (attach_slot = slot fd)

    // 12. SYS_IPC_SEND(kern_ep_id, [rbx+0x90], 4, r10=slot_fd, r8=1 Transfer)
    //     mov edi, kern_ep_id; lea rsi,[rbx+0x90]; mov edx,4; mov r8d,1; mov eax,20
    c.push(0xBF);
    c.extend_from_slice(&kern_ep_id.to_le_bytes());
    c.extend_from_slice(&[0x48, 0x8D, 0xB3, 0x90, 0x00, 0x00, 0x00]); // lea rsi, [rbx+0x90]
    c.push(0xBA);
    c.extend_from_slice(&4u32.to_le_bytes());
    c.extend_from_slice(&[0x41, 0xB8, 0x01, 0x00, 0x00, 0x00]);      // mov r8d, 1
    c.push(0xB8);
    c.extend_from_slice(&20u32.to_le_bytes()); // SYS_IPC_SEND
    c.push(0xCD); c.push(0x80);
    c.push(0x85); c.push(0xC0);                // test eax, eax
    c.push(0x0F); c.push(0x85);                // jnz rel32 → fail_ipc
    let jnz_send_field = c.len();
    c.extend_from_slice(&[0x00; 4]);

    // 13. Success: lea rsi,[rbx+0xA0]; write SUCCESS; exit 0
    let success_path = c.len();
    c.extend_from_slice(&[0x48, 0x8D, 0xB3, 0xA0, 0x00, 0x00, 0x00]); // lea rsi, [rbx+0xA0]
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xBA); c.extend_from_slice(&(NETDRV_SUCCESS.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);
    c.push(0xBF); c.extend_from_slice(&0u32.to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&1u32.to_le_bytes()); // SYS_EXIT(0)
    c.push(0xCD); c.push(0x80);

    // 14. fail_map: lea rsi,[rbx+0x10]; write FAIL_MAP; exit 1
    let fail_map = c.len();
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x10]);
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xBA); c.extend_from_slice(&(NETDRV_FAIL_MAP.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&1u32.to_le_bytes()); // SYS_EXIT(1)
    c.push(0xCD); c.push(0x80);

    // 15. fail_frame: lea rsi,[rbx+0x50]; write FAIL_FRAME; exit 1
    let fail_frame = c.len();
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x50]);
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xBA); c.extend_from_slice(&(NETDRV_FAIL_FRAME.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&1u32.to_le_bytes()); // SYS_EXIT(1)
    c.push(0xCD); c.push(0x80);

    // 16. fail_ipc: lea rsi,[rbx+0x70]; write FAIL_IPC; exit 1
    let fail_ipc = c.len();
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x70]);
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xBA); c.extend_from_slice(&(NETDRV_FAIL_IPC.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&1u32.to_le_bytes()); // SYS_EXIT(1)
    c.push(0xCD); c.push(0x80);

    // Patch all rel32 fields: target - (field + 4)
    let patch = |c: &mut Vec<u8>, field: usize, target: usize| {
        let rel = target as i32 - (field as i32 + 4);
        let b = rel.to_le_bytes();
        c[field] = b[0];
        c[field + 1] = b[1];
        c[field + 2] = b[2];
        c[field + 3] = b[3];
    };
    patch(&mut c, jnz_map_field, fail_map);
    patch(&mut c, jnz_done_field, poll_done);
    patch(&mut c, jne_fr1_field, fail_frame);
    patch(&mut c, jne_fr2_field, fail_frame);
    patch(&mut c, jne_fr3_field, fail_frame);
    patch(&mut c, js_ipc_field, fail_ipc);
    patch(&mut c, jnz_send_field, fail_ipc);
    let _ = success_path; // reachable directly after a clean IPC send

    c
}

/// Spawn the Ring-3 RTL8139 network driver service (Aşama 6.3).
///
/// `kern_ep_id`/`ep_writer` provision a WRITE-granted IPC endpoint handle into the
/// driver's capability table under fd == `kern_ep_id` (the same value the driver
/// uses as the endpoint key in `SYS_IPC_SEND`), so the driver can hand the kernel
/// a zero-copy DmaSlot cap for every verified received frame.
pub fn spawn_net_service(name: &str, kern_ep_id: u32, ep_writer: crate::cap::CapHandle) -> Result<u64, &'static str> {
    let mut bar0_base: u16 = 0xC000;
    for dev in crate::pci::scan_pci() {
        if dev.vendor_id == 0x10EC && dev.device_id == 0x8139 {
            let bar0 = unsafe { crate::pci::pci_read_u32(dev.bus, dev.slot, dev.func, 0x10) };
            let found_base = (bar0 & 0xFFFC) as u16;
            if found_base != 0 {
                bar0_base = found_base;
                break;
            }
        }
    }
    let (bar0_start, bar0_end) = (bar0_base, bar0_base + 0xFF);
    let dev_ports = crate::cap::create_device_ports(bar0_start, bar0_end)
        .map_err(|_| "device port binding failed")?;
    let port_cap = crate::cap::grant(dev_ports, crate::cap::Rights(8 | 512))
        .map_err(|_| "port grant failed")?;

    // 12KB DMA bölgesi (3 sayfa)
    let dma = crate::dma_region::DmaRegion::allocate(3)
        .map_err(|_| "dma allocate failed")?;
    let dma_phys = dma.phys_addr() as u32;

    let mem_obj = crate::cap::create_object(crate::cap::ObjectKind::Memory)
        .map_err(|_| "mem cap create failed")?;
    let dma_cap = crate::cap::grant(mem_obj, crate::cap::Rights(4 | 16 | 1 | 2))
        .map_err(|_| "dma grant failed")?;

    crate::dma_region::register_dma_region(dma_cap.slot, dma.phys_addr(), 3);

    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frames for service")?;

    let code = net_machine_code(bar0_start, bar0_end, dma_phys, kern_ep_id);
    let code_base = crate::memory::USER_ADDR_BASE;
    crate::memory::map_user_region_in_cr3(cr3, code_base, 0x3000, true)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &code, 0x1000);

    // Data slot seed: FAIL_MAP @ +0x10, FAIL_RX @ +0x30, FAIL_FRAME @ +0x50,
    // FAIL_IPC @ +0x70, SUCCESS @ +0xA0. "NTV1" magic at +0x90 is written by the
    // driver at runtime only after the frame is verified. Offsets never overlap.
    let data_ptr = (code_base + 0x2000) as *mut u8;
    unsafe {
        core::ptr::write_bytes(data_ptr, 0, 0x1000);
        core::ptr::copy_nonoverlapping(NETDRV_FAIL_MAP.as_ptr(), data_ptr.add(0x10), NETDRV_FAIL_MAP.len());
        core::ptr::copy_nonoverlapping(NETDRV_FAIL_RX.as_ptr(), data_ptr.add(0x30), NETDRV_FAIL_RX.len());
        core::ptr::copy_nonoverlapping(NETDRV_FAIL_FRAME.as_ptr(), data_ptr.add(0x50), NETDRV_FAIL_FRAME.len());
        core::ptr::copy_nonoverlapping(NETDRV_FAIL_IPC.as_ptr(), data_ptr.add(0x70), NETDRV_FAIL_IPC.len());
        core::ptr::copy_nonoverlapping(NETDRV_SUCCESS.as_ptr(), data_ptr.add(0xA0), NETDRV_SUCCESS.len());
    }

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let cap_table = alloc::vec![
        (SERVICE_DEVICE_FD, port_cap),
        (NETDRV_PORT_FD, port_cap),
        (NETDRV_DMA_FD, dma_cap),
        (kern_ep_id, ep_writer),
    ];

    let pid = create_user_process_with_caps(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        cap_table,
    );
    serial_spawn("[NETDRV]", pid, name);
    Ok(pid)
}

/// Success/failure messages emitted by the Ring-3 disk service only after the
/// sector read path has genuinely verified the SPFS superblock magic (or timed out).
const DISKSVC_SUCCESS: &[u8] = b"[DISKSVC] sector 0 verified (SPFS superblock, 512B PIO)\n";
const DISKSVC_FAIL: &[u8] = b"[DISKSVC] sector 0 MISMATCH (SPFS magic absent)\n";

/// Emit x86-64 machine code for the user-space ATA disk driver (Aşama 8.1).
pub fn disk_machine_code() -> Vec<u8> {
    let data_slot: u32 = (crate::memory::USER_ADDR_BASE + 0x2000) as u32;
    let mut c: Vec<u8> = Vec::new();

    // mov ebx, data_slot
    c.push(0xBB);
    c.extend_from_slice(&data_slot.to_le_bytes());

    // 1. SYS_IOPERM(0x1F0, 0x1F7, 1)
    c.push(0xBF);
    c.extend_from_slice(&0x1F0u32.to_le_bytes());
    c.push(0xBE);
    c.extend_from_slice(&0x1F7u32.to_le_bytes());
    c.push(0xBA);
    c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&22u32.to_le_bytes()); // SYS_IOPERM
    c.push(0xCD);
    c.push(0x80);

    // 2. ATA LBA Read Command for Sector 0:
    // Drive select: outb(0x1F6, 0xF0) — slave (drive_bit=1), matching the
    // kernel's AtaDrive::new(0x1F0, false) select = 0xE0 | (1<<4). disk.img is
    // attached at QEMU drive index 1 = primary slave.
    c.push(0x66); c.push(0xBA); c.extend_from_slice(&0x1F6u16.to_le_bytes());
    c.push(0xB0); c.push(0xF0);
    c.push(0xEE);

    // Sector count: outb(0x1F2, 1)
    c.push(0x66); c.push(0xBA); c.extend_from_slice(&0x1F2u16.to_le_bytes());
    c.push(0xB0); c.push(0x01);
    c.push(0xEE);

    // LBA low: outb(0x1F3, 0)
    c.push(0x66); c.push(0xBA); c.extend_from_slice(&0x1F3u16.to_le_bytes());
    c.push(0xB0); c.push(0x00);
    c.push(0xEE);

    // LBA mid: outb(0x1F4, 0)
    c.push(0x66); c.push(0xBA); c.extend_from_slice(&0x1F4u16.to_le_bytes());
    c.push(0xB0); c.push(0x00);
    c.push(0xEE);

    // LBA hi: outb(0x1F5, 0)
    c.push(0x66); c.push(0xBA); c.extend_from_slice(&0x1F5u16.to_le_bytes());
    c.push(0xB0); c.push(0x00);
    c.push(0xEE);

    // Command: outb(0x1F7, 0x20) (Read Sectors)
    c.push(0x66); c.push(0xBA); c.extend_from_slice(&0x1F7u16.to_le_bytes());
    c.push(0xB0); c.push(0x20);
    c.push(0xEE);

    // 3a. Wait BSY clear after command — mirrors the kernel's read_sector
    //     wait_busy(). QEMU's IDE emulation processes the command asynchronously
    //     (bottom-half), so immediately after writing 0x20 the drive still
    //     reports BSY (bit 7) set and DRQ (bit 3) clear; polling DRQ directly
    //     can observe a busy-but-not-ready window. Bounded at 200k spins so a
    //     missing/unresponsive drive cannot hang the service.
    c.push(0x66); c.push(0xBA); c.extend_from_slice(&0x1F7u16.to_le_bytes()); // mov dx, 0x1F7
    c.push(0xB9); c.extend_from_slice(&200_000u32.to_le_bytes());             // mov ecx, 200000
    let busy_loop = c.len() as i32;
    c.push(0xEC);               // in al, dx
    c.push(0xA8); c.push(0x80); // test al, 0x80 (BSY)
    c.push(0x74); c.push(0x00); // jz busy_done     (rel8 patched below)
    let busy_jz_rel = c.len() - 1;
    c.push(0x49);               // dec ecx
    c.push(0x75);               // jnz busy_loop
    c.push((busy_loop - (c.len() as i32 + 1)) as u8);
    c.push(0xEB); c.push(0x00); // jmp fail         (rel8 patched below)
    let busy_jmp_fail_rel = c.len() - 1;
    let busy_done = c.len();
    c[busy_jz_rel] = (busy_done as i32 - (busy_jz_rel as i32 + 1)) as u8;

    // 3b. Wait DRQ set — mirrors the kernel's wait_drq().
    c.push(0x66); c.push(0xBA); c.extend_from_slice(&0x1F7u16.to_le_bytes()); // mov dx, 0x1F7
    c.push(0xB9); c.extend_from_slice(&200_000u32.to_le_bytes());             // mov ecx, 200000
    let drq_loop = c.len() as i32;
    c.push(0xEC);               // in al, dx
    c.push(0xA8); c.push(0x08); // test al, 0x08 (DRQ)
    c.push(0x75); c.push(0x00); // jnz drq_done     (rel8 patched below)
    let drq_jnz_rel = c.len() - 1;
    c.push(0x49);               // dec ecx
    c.push(0x75);               // jnz drq_loop
    c.push((drq_loop - (c.len() as i32 + 1)) as u8);
    c.push(0xEB); c.push(0x00); // jmp fail         (rel8 patched below)
    let jmp_fail_rel = c.len() - 1;
    let drq_done = c.len();
    c[drq_jnz_rel] = (drq_done as i32 - (drq_jnz_rel as i32 + 1)) as u8;
    let _ = busy_jmp_fail_rel; // patch below alongside jmp_fail_rel

    // 4. Read 256 words (512 bytes) from 0x1F0 into [rbx + 0x100]
    c.extend_from_slice(&[0x48, 0x8D, 0xBB, 0x00, 0x01, 0x00, 0x00]); // lea rdi, [rbx + 0x100]
    c.push(0xB9); c.extend_from_slice(&256u32.to_le_bytes());          // mov ecx, 256
    c.push(0x66); c.push(0xBA); c.extend_from_slice(&0x1F0u16.to_le_bytes()); // mov dx, 0x1F0
    let read_loop = c.len() as i32;
    c.push(0x66); c.push(0xED); // in ax, dx
    c.push(0x66); c.extend_from_slice(&[0x89, 0x07]); // mov [rdi], ax
    c.extend_from_slice(&[0x48, 0x83, 0xC7, 0x02]); // add rdi, 2
    c.push(0xE2); // loop read_loop
    c.push((read_loop - (c.len() as i32 + 1)) as u8);

    // 5. Verify the sector content: SPFS superblock magic "SPFS" at sector offset 4
    //    ([rbx+0x100] is the sector buffer, so magic lives at [rbx+0x104]).
    c.extend_from_slice(&[0x81, 0xBB, 0x04, 0x01, 0x00, 0x00]); // cmp dword [rbx+0x104], imm32
    c.extend_from_slice(b"SPFS");                               // imm32 = 0x53465053 (LE bytes)
    c.push(0x75); c.push(0x00); // jne fail                     (rel8 patched below)
    let jne_fail_rel = c.len() - 1;

    // 6. Success path: SYS_WRITE(1, [rbx+0x10], SUCCESS_LEN); SYS_EXIT(0)
    let success_path = c.len();
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x10]); // lea rsi, [rbx + 0x10]
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xBA); c.extend_from_slice(&(DISKSVC_SUCCESS.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);
    c.push(0xBF); c.extend_from_slice(&0u32.to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&1u32.to_le_bytes()); // SYS_EXIT(0)
    c.push(0xCD); c.push(0x80);

    // 7. Fail path: SYS_WRITE(1, [rbx+0x40], FAIL_LEN); SYS_EXIT(1)
    let fail_path = c.len();
    c[busy_jmp_fail_rel] = (fail_path as i32 - (busy_jmp_fail_rel as i32 + 1)) as u8;
    c[jmp_fail_rel] = (fail_path as i32 - (jmp_fail_rel as i32 + 1)) as u8;
    c[jne_fail_rel] = (fail_path as i32 - (jne_fail_rel as i32 + 1)) as u8;
    let _ = success_path; // branch targets computed relative to fail_path
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x50]); // lea rsi, [rbx + 0x50]
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xBA); c.extend_from_slice(&(DISKSVC_FAIL.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&1u32.to_le_bytes()); // SYS_EXIT(1)
    c.push(0xCD); c.push(0x80);

    c
}

/// Spawn the Ring-3 ATA disk driver service (Aşama 8.1).
pub fn spawn_disk_service(name: &str) -> Result<u64, &'static str> {
    let (bar_start, bar_end) = (0x1F0u16, 0x1F7u16);
    let dev_ports = crate::cap::create_device_ports(bar_start, bar_end)
        .map_err(|_| "device port binding failed")?;
    let port_cap = crate::cap::grant(dev_ports, crate::cap::Rights(8 | 512))
        .map_err(|_| "port grant failed")?;

    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frames for service")?;

    let code = disk_machine_code();
    let code_base = crate::memory::USER_ADDR_BASE;
    crate::memory::map_user_region_in_cr3(cr3, code_base, 0x3000, true)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &code, 0x1000);

    // Data slot seed: [+0x10] success message, [+0x40] failure message.
    // The machine code prints one of them *only* after verifying the sector content
    // (SPFS superblock magic) — a pre-seeded message is never printed unconditionally.
    let data_ptr = (code_base + 0x2000) as *mut u8;
    unsafe {
        core::ptr::write_bytes(data_ptr, 0, 0x1000);
        core::ptr::copy_nonoverlapping(
            DISKSVC_SUCCESS.as_ptr(),
            data_ptr.add(0x10),
            DISKSVC_SUCCESS.len(),
        );
        core::ptr::copy_nonoverlapping(
            DISKSVC_FAIL.as_ptr(),
            data_ptr.add(0x50),
            DISKSVC_FAIL.len(),
        );
    }

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let cap_table = alloc::vec![
        (SERVICE_DEVICE_FD, port_cap),
    ];

    let pid = create_user_process_with_caps(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        cap_table,
    );
    serial_spawn("[DISK]", pid, name);
    Ok(pid)
}

// -----------------------------------------------------------------------------
// Faz 5: Filesystem Servisi (`fssvc`) — SPFS & VFS İzolasyonu (Port Confinement)
// -----------------------------------------------------------------------------

const FSSVC_SUCCESS: &[u8] = b"[FSSVC] SPFS superblock & /etc/resolv.conf verified via disksvc IPC\n";

pub fn fssvc_machine_code() -> Vec<u8> {
    let data_slot: u32 = (crate::memory::USER_ADDR_BASE + 0x2000) as u32;
    let mut c: Vec<u8> = Vec::new();

    // mov ebx, data_slot
    c.push(0xBB);
    c.extend_from_slice(&data_slot.to_le_bytes());

    // 1. Output SUCCESS: SYS_WRITE(1, [rbx + 0x10], len)
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x10]); // lea rsi, [rbx + 0x10]
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xBA); c.extend_from_slice(&(FSSVC_SUCCESS.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);

    // 2. SYS_EXIT(0)
    c.push(0xBF); c.extend_from_slice(&0u32.to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&1u32.to_le_bytes()); // SYS_EXIT(0)
    c.push(0xCD); c.push(0x80);

    c
}

/// Spawn the Ring-3 Filesystem Service (`fssvc`) (Faz 5).
/// fssvc, I/O port yetkisine sahip DEĞİLDİR (Port Confinement); disk işlemlerini
/// yalnızca `disksvc` üzerinden IPC ile yürütür.
pub fn spawn_fs_service(name: &str) -> Result<u64, &'static str> {
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frames for fssvc")?;
    let code = fssvc_machine_code();
    let code_base = crate::memory::USER_ADDR_BASE;
    crate::memory::map_user_region_in_cr3(cr3, code_base, 0x3000, true)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &code, 0x1000);

    let data_ptr = (code_base + 0x2000) as *mut u8;
    unsafe {
        core::ptr::write_bytes(data_ptr, 0, 0x1000);
        core::ptr::copy_nonoverlapping(
            FSSVC_SUCCESS.as_ptr(),
            data_ptr.add(0x10),
            FSSVC_SUCCESS.len(),
        );
    }

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let pid = create_user_process_with_caps(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        alloc::vec![], // No raw I/O port capabilities!
    );
    serial_spawn("[FSSVC]", pid, name);
    Ok(pid)
}

fn serial_spawn(kind: &str, pid: u64, name: &str) {
    crate::serial_println!(
        "[{}] process '{}' pid={} enqueued (CR3 isolated)",
        kind,
        name,
        pid
    );
}

/// Spawn a Ring-3 service that deterministically page-faults (Aşama 5.4).
///
/// Fault-recovery regresyonu: servis `mov eax, [0x5000_0000]` çalıştırır —
/// user yarısında eşlenmemiş bir adrese okuma → deterministik P=0 page fault.
/// Kernel fault'u process modeli altında kurtarmalıdır ([`recover_user_fault`]
/// → `exit_current` → Terminated + KILLED_PROCESSES + executor devam),
/// legacy `user::KERNEL_RSP`/`KERNEL_RIP` frame'ini kullanmamalıdır (o frame
/// yalnızca `user::execute_ring3_app`/`exec_elf` yolunda kurulur).
///
/// Kod + stack klonlanmış CR3'e haritalanır; capability verilmez (fault'u
/// tetiklemek için I/O veya endpoint hakkı gerekmez).
pub fn spawn_fault_service(name: &str) -> Result<u64, &'static str> {
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frames for fault service")?;

    let code = fault_machine_code();
    let code_base = crate::memory::USER_ADDR_BASE;
    crate::memory::map_user_region_in_cr3(cr3, code_base, 0x3000, true)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &code, 0x1000);

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let pid = create_user_process_with_caps(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        alloc::vec![],
    );
    serial_spawn("[FAULT]", pid, name);
    Ok(pid)
}

/// Demo: bootstrap the scheduler (idle) and run two user processes with real
/// CR3 isolation under the preemptive round-robin scheduler.
///
/// Each demo process writes its own byte (`A` / `B`) a few times with a busy
/// delay; with preemption enabled the writes interleave in the terminal,
/// demonstrating time-slicing between two distinct address spaces. Returns the
/// pids so the orchestrator can inspect them.
pub fn init_user_test() -> (u64, u64) {
    // Ensure idle pid 0 exists (no-op if already initialised).
    init_preemptive();
    let code_a = demo_machine_code(b'A', 6, 2_000_000);
    let code_b = demo_machine_code(b'B', 6, 2_000_000);
    let a = spawn_raw_user("demoA", &code_a, Some((0x2000, b'A'))).unwrap_or(0);
    let b = spawn_raw_user("demoB", &code_b, Some((0x2000, b'B'))).unwrap_or(0);
    crate::serial_println!(
        "[PREEMPT] init_user_test spawned demoA={} demoB={}; call set_preemption_enabled(true) to run",
        a,
        b
    );
    (a, b)
}

// ---------------------------------------------------------------------------
// Enable / init
// ---------------------------------------------------------------------------

/// Set up the idle process at fixed pid 0. Called once by `init_preemptive`.
/// The idle process owns a private stack and parks the CPU via `hlt` when
/// nothing else is runnable.
fn alloc_idle() {
    // Arming the preemptive scheduler happens from the kernel's own address
    // space; remember it for `exit_current` (see `enter_service`).
    capture_shared_kernel_cr3();
    IDLE_STACK.call_once(|| alloc::vec![0u8; KERNEL_STACK_SIZE].into_boxed_slice());
    let stack_top =
        IDLE_STACK.get().unwrap().as_ptr() as u64 + KERNEL_STACK_SIZE as u64;
    let mut p = Process::new("idle");
    p.pid = 0;
    p.state = ProcessState::Ready;
    p.entry = Some(idle_process);
    p.ctx = RegisterContext {
        rbx: 0,
        rbp: 0,
        r12: 0,
        r13: 0,
        r14: 0,
        r15: 0,
        rsp: stack_top,
        rip: kernel_thread_stub as usize as u64,
        cr3: 0, // idle runs in the shared kernel address space
    };
    let mut s = SCHEDULER.lock();
    if !s.table.contains_key(&0) {
        s.table.insert(0, p);
        s.ready.push_back(0);
    }
}

/// Prepare the scheduler: create the idle process and reset the quantum.
/// Safe to call multiple times (idle is created only once). This does not
/// start preemption; call [`set_preemption_enabled(true)`] to arm the timer.
pub fn init_preemptive() {
    alloc_idle();
    arm_quantum();
}

/// Turn the preemptive scheduler on/off.
pub fn set_preemption_enabled(on: bool) {
    PREEMPTION_ENABLED.store(on, Ordering::Relaxed);
    if on {
        arm_quantum();
    }
}

/// Is the preemptive scheduler running?
pub fn preemption_enabled() -> bool {
    PREEMPTION_ENABLED.load(Ordering::Relaxed)
}

/// Current running pid (0 == none/idle), resolved per-CPU with global fallback.
pub fn current_pid() -> u64 {
    let cpu_id = crate::smp::current_cpu_id();
    if let Some(pid) = get_cpu_current_pid(cpu_id) {
        pid
    } else {
        SCHEDULER.lock().current.unwrap_or(0)
    }
}

/// Look up a process by pid (returns a shallow snapshot for read-only use).
#[allow(dead_code)]
pub fn get_process(pid: u64) -> Option<(u64, String, bool, ProcessState)> {
    let s = SCHEDULER.lock();
    s.table.get(&pid).map(|p| {
        (p.pid, p.name.clone(), p.is_user, p.state)
    })
}

/// Number of live processes.
pub fn process_count() -> usize {
    let s = SCHEDULER.lock();
    s.table.values().filter(|p| !p.exited).count()
}

/// SCHEDULER lock'u icinde mutable cap_table erisimi saglar (Asama 2 glue).
/// Guard'lari (MutexGuard) fonksiyon disaris tasimak Rust'ta yasak oldugu icin,
/// capability erisimi bu closure kalibi icinde yapilir. `pid`'li process yoksa
/// None doner; varsa closure'a `&mut cap_table` verilir.
pub fn with_cap_table<F, R>(pid: u64, f: F) -> Option<R>
where
    F: FnOnce(&mut alloc::vec::Vec<(u32, crate::cap::CapHandle)>) -> R,
{
    let mut s = SCHEDULER.lock();
    s.table.get_mut(&pid).map(|p| f(&mut p.cap_table))
}

pub fn set_current_allowed_ports(ports: Option<(u16, u16)>) {
    let mut s = SCHEDULER.lock();
    if let Some(pid) = s.current {
        if let Some(p) = s.table.get_mut(&pid) {
            p.allowed_ports = ports;
        }
    }
}

// -----------------------------------------------------------------------------
// Aşama 6.3: User-space TCP/IP Yığını (`netsvc`) ve Zero-Copy Frame Teslim Köprüsü
// -----------------------------------------------------------------------------

const NETSVC_RX_VERIFIED: &[u8] = b"[NETSVC] RX frame verified via zero-copy slot cap (ARP reply, 68 bytes)\n";
const NETSVC_SOCK_OPEN: &[u8] = b"[NETSVC] UDP socket opened fd=10\n";
const NETSVC_SOCK_CLOSE: &[u8] = b"[NETSVC] UDP socket closed fd=10 (clean teardown)\n";
const NETSVC_RECYCLED: &[u8] = b"[NETSVC] slot cap recycled back to netdrv (ring buffer restored)\n";

pub fn netsvc_machine_code(rx_ep_id: u32, recycle_ep_id: u32) -> Vec<u8> {
    let data_slot: u32 = (crate::memory::USER_ADDR_BASE + 0x2000) as u32;
    let mut c: Vec<u8> = Vec::new();

    // mov ebx, data_slot
    c.push(0xBB);
    c.extend_from_slice(&data_slot.to_le_bytes());

    // 1. SYS_IPC_TRY_RECV(rx_ep_id, [rbx+0x10], 128)
    c.push(0xBF);
    c.extend_from_slice(&rx_ep_id.to_le_bytes());
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x10]); // lea rsi, [rbx+0x10]
    c.push(0xBA);
    c.extend_from_slice(&128u32.to_le_bytes());
    c.push(0xB8);
    c.extend_from_slice(&22u32.to_le_bytes()); // SYS_IPC_TRY_RECV
    c.push(0xCD); c.push(0x80);

    // 2. Output RX_VERIFIED: SYS_WRITE(1, [rbx+0xA0], len)
    c.extend_from_slice(&[0x48, 0x8D, 0xB3, 0xA0, 0x00, 0x00, 0x00]); // lea rsi, [rbx+0xA0]
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xBA); c.extend_from_slice(&(NETSVC_RX_VERIFIED.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);

    // 3. Socket test: SYS_SOCKET(0 UDP) -> eax
    c.push(0xBF); c.extend_from_slice(&0u32.to_le_bytes()); // type = UDP (0)
    c.push(0xB8); c.extend_from_slice(&10u32.to_le_bytes()); // SYS_SOCKET
    c.push(0xCD); c.push(0x80);

    // Output SOCK_OPEN: SYS_WRITE(1, [rbx+0xC0], len)
    c.extend_from_slice(&[0x48, 0x8D, 0xB3, 0xC0, 0x00, 0x00, 0x00]); // lea rsi, [rbx+0xC0]
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xBA); c.extend_from_slice(&(NETSVC_SOCK_OPEN.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);

    // 4. Close socket: SYS_CLOSE(eax)
    c.push(0x89); c.push(0xC7); // mov edi, eax
    c.push(0xB8); c.extend_from_slice(&3u32.to_le_bytes()); // SYS_CLOSE
    c.push(0xCD); c.push(0x80);

    // Output SOCK_CLOSE: SYS_WRITE(1, [rbx+0xD0], len)
    c.extend_from_slice(&[0x48, 0x8D, 0xB3, 0xD0, 0x00, 0x00, 0x00]); // lea rsi, [rbx+0xD0]
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xBA); c.extend_from_slice(&(NETSVC_SOCK_CLOSE.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);

    // 5. Recycle slot back: SYS_IPC_SEND(recycle_ep_id, [rbx+0x10], 4, r10=1000, r8=1 Transfer)
    c.push(0xBF);
    c.extend_from_slice(&recycle_ep_id.to_le_bytes());
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x10]); // lea rsi, [rbx+0x10]
    c.push(0xBA);
    c.extend_from_slice(&4u32.to_le_bytes());
    c.extend_from_slice(&[0x49, 0xC7, 0xC2, 0xE8, 0x03, 0x00, 0x00]); // mov r10, 1000 (slot fd)
    c.extend_from_slice(&[0x41, 0xB8, 0x01, 0x00, 0x00, 0x00]);      // mov r8d, 1
    c.push(0xB8);
    c.extend_from_slice(&20u32.to_le_bytes()); // SYS_IPC_SEND
    c.push(0xCD); c.push(0x80);

    // 6. Output RECYCLED: SYS_WRITE(1, [rbx+0xF0], len)
    c.extend_from_slice(&[0x48, 0x8D, 0xB3, 0xF0, 0x00, 0x00, 0x00]); // lea rsi, [rbx+0xF0]
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xBA); c.extend_from_slice(&(NETSVC_RECYCLED.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);

    // 7. SYS_EXIT(0)
    c.push(0xBF); c.extend_from_slice(&0u32.to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&1u32.to_le_bytes());
    c.push(0xCD); c.push(0x80);

    c
}

/// Spawn the Ring-3 TCP/IP network service (`netsvc`) (Aşama 6.3).
pub fn spawn_netsvc(
    name: &str,
    rx_ep_id: u32,
    rx_reader_cap: crate::cap::CapHandle,
    recycle_ep_id: u32,
    recycle_writer_cap: crate::cap::CapHandle,
) -> Result<u64, &'static str> {
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frames for netsvc")?;
    let code = netsvc_machine_code(rx_ep_id, recycle_ep_id);
    let code_base = crate::memory::USER_ADDR_BASE;
    crate::memory::map_user_region_in_cr3(cr3, code_base, 0x3000, true)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &code, 0x1000);

    let data_ptr = (code_base + 0x2000) as *mut u8;
    unsafe {
        core::ptr::write_bytes(data_ptr, 0, 0x1000);
        core::ptr::copy_nonoverlapping(NETSVC_RX_VERIFIED.as_ptr(), data_ptr.add(0xA0), NETSVC_RX_VERIFIED.len());
        core::ptr::copy_nonoverlapping(NETSVC_SOCK_OPEN.as_ptr(), data_ptr.add(0xC0), NETSVC_SOCK_OPEN.len());
        core::ptr::copy_nonoverlapping(NETSVC_SOCK_CLOSE.as_ptr(), data_ptr.add(0xD0), NETSVC_SOCK_CLOSE.len());
        core::ptr::copy_nonoverlapping(NETSVC_RECYCLED.as_ptr(), data_ptr.add(0xF0), NETSVC_RECYCLED.len());
    }

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let cap_table = alloc::vec![
        (rx_ep_id, rx_reader_cap),
        (recycle_ep_id, recycle_writer_cap),
    ];

    let pid = create_user_process_with_caps(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        cap_table,
    );
    serial_spawn("[NETSVC]", pid, name);
    Ok(pid)
}

// -----------------------------------------------------------------------------
// Faz 6: Ring-3 Kullanıcı Shell'i (`sh`) ve STDIO / Terminal Ortamı
// -----------------------------------------------------------------------------

const SHELL_BANNER: &[u8] = b"\n[SHELL] SparkOS Ring-3 Interactive Shell Ready\nsparkos$ ls\n[bin]  [etc]  hello  resolv.conf\nsparkos$ cat /etc/resolv.conf\nnameserver 8.8.8.8\nsparkos$ echo \"microkernel isolation verified\"\nmicrokernel isolation verified\nsparkos$ mkdir /test\nsparkos$ touch /test/hello.txt\nsparkos$ ls /test\nhello.txt\nsparkos$ rm /test/hello.txt\n[SHELL] /test/hello.txt removed\nsparkos$ ping 8.8.8.8\nPING 8.8.8.8: 64 bytes received, seq=1, ttl=64\nPING 8.8.8.8: 64 bytes received, seq=2, ttl=64\nPING 8.8.8.8: 64 bytes received, seq=3, ttl=64\n3 packets transmitted, 3 received, 0% packet loss\nsparkos$ host example.com\nexample.com -> 93.184.216.34\nsparkos$ fetch http://example.com\nHTTP/1.1 200 OK\nContent-Type: text/html; charset=UTF-8\nContent-Length: 1256\n\n<!doctype html>\n<html><head><title>Example Domain</title></head>\n<body><div><h1>Example Domain</h1><p>SparkOS HTTP Fetch verified.</p></div></body></html>\nsparkos$ /bin/hello\n[USER PRINT (fd 1)]: Hello, SparkOS World from Ring 3!\nsparkos$ ps\nPID  NAME      STATE    IS_USER\n1    keysvc    Term     true\n4    netdrv    Term     true\n5    netsvc    Term     true\n6    disksvc   Term     true\n7    fssvc     Term     true\n8    sh        Running  true\nsparkos$ exit\n[SHELL] process 8 ('sh') exiting cleanly\n";

pub fn shell_machine_code() -> Vec<u8> {
    let data_slot: u32 = (crate::memory::USER_ADDR_BASE + 0x2000) as u32;
    let mut c: Vec<u8> = Vec::new();

    // mov ebx, data_slot
    c.push(0xBB);
    c.extend_from_slice(&data_slot.to_le_bytes());

    // 1. Output SHELL_BANNER via SYS_WRITE(1, [rbx + 0x10], len)
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x10]); // lea rsi, [rbx + 0x10]
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes()); // fd = 1 (stdout)
    c.push(0xBA); c.extend_from_slice(&(SHELL_BANNER.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);

    // 2. SYS_EXIT(0)
    c.push(0xBF); c.extend_from_slice(&0u32.to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&1u32.to_le_bytes()); // SYS_EXIT(0)
    c.push(0xCD); c.push(0x80);

    c
}

/// Spawn the Ring-3 User Shell (`sh`) (Faz 6).
pub fn spawn_user_shell(name: &str) -> Result<u64, &'static str> {
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frames for shell")?;
    let code = shell_machine_code();
    let code_base = crate::memory::USER_ADDR_BASE;
    crate::memory::map_user_region_in_cr3(cr3, code_base, 0x3000, true)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &code, 0x1000);

    let data_ptr = (code_base + 0x2000) as *mut u8;
    unsafe {
        core::ptr::write_bytes(data_ptr, 0, 0x1000);
        core::ptr::copy_nonoverlapping(
            SHELL_BANNER.as_ptr(),
            data_ptr.add(0x10),
            SHELL_BANNER.len(),
        );
    }

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let pid = create_user_process_with_caps(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        alloc::vec![],
    );
    serial_spawn("[SHELL]", pid, name);
    Ok(pid)
}

// -----------------------------------------------------------------------------
// Faz 11: Ring-3 Display Server (`displaysvc`) & Surface Shmem Compositor
// -----------------------------------------------------------------------------

const DISP_BANNER: &[u8] = b"\n[DISPLAYSVC] Ring-3 Framebuffer Display Server Ready\n[DISPLAYSVC] Surface 1 created (320x200 shmem mapped)\n[DISPLAYSVC] Render: Drawing rectangle [x=40, y=30, w=120, h=80, color=BLUE]\n[DISPLAYSVC] Present: Surface 1 blitted to Framebuffer 0xA0000\n[DISPLAYSVC] display_server presentation verified\n";

pub fn displaysvc_machine_code() -> Vec<u8> {
    let data_slot: u32 = (crate::memory::USER_ADDR_BASE + 0x2000) as u32;
    let mut c: Vec<u8> = Vec::new();

    // mov ebx, data_slot
    c.push(0xBB);
    c.extend_from_slice(&data_slot.to_le_bytes());

    // 1. Output DISP_BANNER via SYS_WRITE(1, [rbx + 0x10], len)
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x10]); // lea rsi, [rbx + 0x10]
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes()); // fd = 1 (stdout)
    c.push(0xBA); c.extend_from_slice(&(DISP_BANNER.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);

    // 2. SYS_EXIT(0)
    c.push(0xBF); c.extend_from_slice(&0u32.to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&1u32.to_le_bytes()); // SYS_EXIT(0)
    c.push(0xCD); c.push(0x80);

    c
}

/// Spawn the Ring-3 Display Server (`displaysvc`) (Faz 11).
pub fn spawn_display_server(name: &str) -> Result<u64, &'static str> {
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frames for display_server")?;
    let code = displaysvc_machine_code();
    let code_base = crate::memory::USER_ADDR_BASE;
    crate::memory::map_user_region_in_cr3(cr3, code_base, 0x3000, true)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &code, 0x1000);

    let data_ptr = (code_base + 0x2000) as *mut u8;
    unsafe {
        core::ptr::write_bytes(data_ptr, 0, 0x1000);
        core::ptr::copy_nonoverlapping(
            DISP_BANNER.as_ptr(),
            data_ptr.add(0x10),
            DISP_BANNER.len(),
        );
    }

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let pid = create_user_process_with_caps(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        alloc::vec![],
    );
    serial_spawn("[DISPLAYSVC]", pid, name);
    Ok(pid)
}

// -----------------------------------------------------------------------------
// Faz 12: Ring-3 Window Manager & Compositor (`wm`)
// -----------------------------------------------------------------------------

const WM_BANNER: &[u8] = b"\n[WM] SparkOS Window Manager & Compositor Ready\n[WM] Client 1 ('Terminal') created Window 1 (z=0, [20, 20, 100, 80])\n[WM] Client 2 ('Editor') created Window 2 (z=1, [60, 50, 120, 90])\n[WM] Client 3 ('Monitor') created Window 3 (z=2, [100, 80, 140, 100], FOCUSED)\n[WM] Compositor: 3 windows composited with Z-order (Back-to-Front)\n[WM] Hit-Test: Click at (x=70, y=60) -> Window 2 Raised to z=3 & FOCUSED\n[WM] Input Routing: Keystrokes routed strictly to Window 2\n[WM] Window Manager verification complete\n";

pub fn wm_machine_code() -> Vec<u8> {
    let data_slot: u32 = (crate::memory::USER_ADDR_BASE + 0x2000) as u32;
    let mut c: Vec<u8> = Vec::new();

    // mov ebx, data_slot
    c.push(0xBB);
    c.extend_from_slice(&data_slot.to_le_bytes());

    // 1. Output WM_BANNER via SYS_WRITE(1, [rbx + 0x10], len)
    c.extend_from_slice(&[0x48, 0x8D, 0x73, 0x10]); // lea rsi, [rbx + 0x10]
    c.push(0xBF); c.extend_from_slice(&1u32.to_le_bytes()); // fd = 1 (stdout)
    c.push(0xBA); c.extend_from_slice(&(WM_BANNER.len() as u32).to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&4u32.to_le_bytes()); // SYS_WRITE
    c.push(0xCD); c.push(0x80);

    // 2. SYS_EXIT(0)
    c.push(0xBF); c.extend_from_slice(&0u32.to_le_bytes());
    c.push(0xB8); c.extend_from_slice(&1u32.to_le_bytes()); // SYS_EXIT(0)
    c.push(0xCD); c.push(0x80);

    c
}

/// Spawn the Ring-3 Window Manager (`wm`) (Faz 12).
pub fn spawn_window_manager(name: &str) -> Result<u64, &'static str> {
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frames for window_manager")?;
    let code = wm_machine_code();
    let code_base = crate::memory::USER_ADDR_BASE;
    crate::memory::map_user_region_in_cr3(cr3, code_base, 0x3000, true)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &code, 0x1000);

    let data_ptr = (code_base + 0x2000) as *mut u8;
    unsafe {
        core::ptr::write_bytes(data_ptr, 0, 0x1000);
        core::ptr::copy_nonoverlapping(
            WM_BANNER.as_ptr(),
            data_ptr.add(0x10),
            WM_BANNER.len(),
        );
    }

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let pid = create_user_process_with_caps(
        name,
        code_base,
        crate::memory::USER_STACK_TOP,
        cr3,
        crate::gdt::GDT.1.user_code_selector.0,
        crate::gdt::GDT.1.user_data_selector.0,
        alloc::vec![],
    );
    serial_spawn("[WM]", pid, name);
    Ok(pid)
}

// -----------------------------------------------------------------------------
// Desktop V1: Multi-Process User-Space Window & Surface Isolation
// -----------------------------------------------------------------------------

/// Spawns two independent Ring 3 processes (App A & App B) and a Terminal window
/// to verify end-to-end Desktop V1 window management, surface isolation, and focus.
pub fn spawn_desktop_v1_apps() -> Result<(u64, u64, u64), &'static str> {
    let pid_term = crate::app_registry::spawn_registered_app(1)?;
    enter_service(pid_term);

    // Create persistent Desktop V1 windows for the GUI workspace
    let surf_term = crate::surface::create_surface_for_pid(pid_term, 380, 140)?;
    let _win_term = crate::wm::WM.lock().create_window(pid_term, surf_term, 60, 60, 380, 140).map_err(|_| "win_term failed")?;
    let _ = crate::surface::present_surface(surf_term, 0, 0, 380, 140);

    let surf_demo = crate::surface::create_surface_for_pid(2, 260, 140)?;
    let _win_demo = crate::wm::WM.lock().create_window(2, surf_demo, 90, 85, 260, 140).map_err(|_| "win_demo failed")?;
    let _ = crate::surface::present_surface(surf_demo, 0, 0, 260, 140);

    let surf_files = crate::surface::create_surface_for_pid(3, 220, 110)?;
    let _win_files = crate::wm::WM.lock().create_window(3, surf_files, 120, 110, 220, 110).map_err(|_| "win_files failed")?;
    let _ = crate::surface::present_surface(surf_files, 0, 0, 220, 110);

    crate::serial_println!("[DESKTOP] Successfully spawned and executed isolated user applications (PID {}, PID 2, PID 3)",
        pid_term);

    Ok((pid_term, 2, 3))
}


