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
fn capture_shared_kernel_cr3() {
    SHARED_KERNEL_CR3.call_once(|| {
        let (frame, _) = x86_64::registers::control::Cr3::read();
        frame.start_address().as_u64()
    });
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
            cap_table: alloc::vec::Vec::new(),
            allowed_ports: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler data
// ---------------------------------------------------------------------------

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
            // Load the next process's address space (CR3) if set (nonzero).
            "mov rax, [rsi + 64]",
            "test rax, rax",
            "jz 1f",
            "mov cr3, rax",
            "1:",
            "mov rsp, [rsi + 48]",
            "jmp qword ptr [rsi + 56]", // resume next process
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
    SCHEDULER.lock().table.insert(pid, p);
    pid
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
        p.ctx = RegisterContext {
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rsp: p.kernel_stack.as_ptr() as u64 + p.kernel_stack.len() as u64,
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
        p.ctx = RegisterContext {
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rsp: p.kernel_stack.as_ptr() as u64 + p.kernel_stack.len() as u64,
            rip: user_process_stub as usize as u64,
            cr3: user_cr3, // enter_user_current + switch both load this CR3
        };
        s.ready.push_back(pid);
    }
    crate::task::PROCESS_LIST.lock().insert(pid, name.to_string());
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
    let (target_ctx, allowed_ports) = {
        let mut s = SCHEDULER.lock();
        s.current = Some(pid);
        if let Some(p) = s.table.get_mut(&pid) {
            p.state = ProcessState::Running;
        }
        let ports = s.table.get(&pid).and_then(|p| p.allowed_ports);
        (&s.table[&pid].ctx as *const RegisterContext, ports)
    };

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
    let (user_rip, user_rsp, user_cr3, user_cs, user_ss, allowed_ports, kstack_top) = {
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
            p.user_rip,
            p.user_rsp,
            p.user_cr3,
            p.user_cs,
            p.user_ss,
            p.allowed_ports,
            krsp,
        )
    };

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
            "mov rax, [rdi + 64]",
            "test rax, rax",
            "jz 1f",
            "mov cr3, rax",
            "1:",
            "mov rsp, [rdi + 48]",
            "jmp qword ptr [rdi + 56]",
        )
}

/// Terminate the running process and switch to the next ready one (or idle),
/// or — Aşama 5.2 — resume the cooperative executor that entered a service via
/// [`enter_service`]. Never returns.
pub fn exit_current() -> ! {
    // Cooperative-resume branch: if the running context was entered by
    // `enter_service`, `EXECUTOR_RESUME` holds the executor's saved kernel
    // context. Take it in its own statement (the temporary guard is dropped
    // immediately — holding it across the jump would deadlock the executor's
    // later `lock()` in enter_service's save pointer). Then mark the service
    // terminated and jump straight back to the executor, skipping the ready
    // queue (the executor is not a schedulable process; it resumes inline).
    let exec_ctx: Option<RegisterContext> = EXECUTOR_RESUME.lock().take();
    if let Some(mut ctx) = exec_ctx {
        {
            let mut s = SCHEDULER.lock();
            if let Some(pid) = s.current {
                if let Some(p) = s.table.get_mut(&pid) {
                    p.state = ProcessState::Terminated;
                    p.exited = true;
                    // Görev A (CAP_INV-13): CSpace otomatik temizliği
                    crate::cap::destroy_process_cspace(&mut p.cap_table);
                    // Görev A (CAP_INV-13): Kanal ve IRQ unbind / hangup
                    crate::ipc::hangup_channel_for_pid(pid as u32);
                    p.allowed_ports = None;
                    crate::task::KILLED_PROCESSES.lock().push(pid);
                }
            }
            // Executor is not a process; nothing is "current" after the resume.
            s.current = None;
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
                p.allowed_ports = None;
                crate::task::KILLED_PROCESSES.lock().push(pid);
            }
        }
        match s.ready.pop_front() {
            Some(pid) => {
                s.current = Some(pid);
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
    let elf = crate::elf::parse_elf(elf_bytes)?;
    if elf.segments.is_empty() {
        return Err("No loadable segments found in ELF");
    }
    let seg = &elf.segments[0];
    let cr3 = crate::memory::clone_active_cr3().ok_or("no free frames for exec")?;

    let code_base = crate::memory::USER_ADDR_BASE;
    let code_len = seg.memsz.max(1);
    crate::memory::map_user_region_in_cr3(cr3, code_base, code_len, false)?;
    crate::memory::write_user_region_in_cr3(cr3, code_base, &seg.data, code_len);

    let stack_base = crate::memory::USER_STACK_TOP - 4096;
    crate::memory::map_user_region_in_cr3(cr3, stack_base, 4096, true)?;

    let actual_entry = code_base + (elf.entry_point - seg.vaddr);
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

/// Current running pid (0 == none/idle).
pub fn current_pid() -> u64 {
    SCHEDULER.lock().current.unwrap_or(0)
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

