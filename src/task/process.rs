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

/// Reference to the dynamically-allocated idle kernel-stack array. Kept so the
/// idle process's `ctx.rsp` stays valid for the kernel's lifetime.
static IDLE_STACK: spin::Once<Box<[u8]>> = spin::Once::new();

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
#[unsafe(naked)]
extern "C" fn switch_context(current: *mut RegisterContext, next: *const RegisterContext) -> ! {
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
#[unsafe(naked)]
extern "C" fn switch_context_null_save(next: *const RegisterContext) -> ! {
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

/// Terminate the running process and switch to the next ready one (or idle).
/// Never returns. Used by kernel threads when they finish.
pub fn exit_current() -> ! {
    let next_ctx: *const RegisterContext = {
        let mut s = SCHEDULER.lock();
        if let Some(pid) = s.current {
            if let Some(p) = s.table.get_mut(&pid) {
                p.state = ProcessState::Terminated;
                p.exited = true;
                crate::task::KILLED_PROCESSES.lock().push(pid);
            }
        }
        match s.ready.pop_front() {
            Some(pid) => {
                s.current = Some(pid);
                if let Some(p) = s.table.get_mut(&pid) {
                    p.state = ProcessState::Running;
                }
                arm_quantum();
                &s.table[&pid].ctx as *const RegisterContext
            }
            None => {
                s.current = Some(0);
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
        let mut s = SCHEDULER.lock();
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

fn serial_spawn(kind: &str, pid: u64, name: &str) {
    crate::serial_println!(
        "[{}] process '{}' pid={} enqueued (CR3 isolated)",
        kind,
        name,
        pid
    );
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

