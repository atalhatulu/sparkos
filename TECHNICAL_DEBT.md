# SparkOS — Technical Debt & Architectural Ledger
**Frozen Foundation:** Architecture Freeze #5  
**Status:** Experimental; host tests and opt-in QEMU regressions (not a formal proof)

---

## 1. Resolved Invariants & Defenses

### `TD-MED-3`: Physical Frame Allocator Reclamation (SEC-06)
- **Module:** `src/memory.rs` (`UserFrameAllocator`, `user_alloc_frame`, `user_free_frame`)
- **Status:** **RESOLVED in Faz 25**
- **Implementation:** LIFO Recycled Frame Cache (`free_list: Vec<PhysFrame>`) with strict allocated frame verification (`allocated_frames: BTreeSet<u64>`), guaranteeing immediate physical frame reuse and complete double-free defense.

### `TD-MED-1`: Multi-Surface VMA Offset Allocation
- **Module:** `src/surface.rs` (`SurfaceInfo.vma_addr`)
- **Status:** **RESOLVED in Freeze #4 / Hardened in Freeze #5**
- **Implementation:** 16-bit bitmap slot allocator (`used_mask`), `0x70000000 + slot * 16MB`, upper-bound user address verification (`< 0x80000000`).

### `TD-HIGH-1`: IPC Buffer Unbounded Allocation (SEC-12)
- **Module:** `src/ipc.rs` & `src/syscall.rs` (`sys_ipc_create_endpoint`)
- **Status:** **RESOLVED in Freeze #5**
- **Implementation:** `MAX_ENDPOINT_CAPACITY = 256` clamp, returns `-EINVAL` on out-of-bounds requests, eliminating kernel heap OOM DoS.

---

### `TD-MED-2`: `libspark::event::next()` Blocking Event Loop
- **Module:** `sdk/libspark/src/lib.rs` (`pub mod event`, `pub mod ipc`)
- **Status:** **RESOLVED in Faz 31**
- **Implementation:** Integrated `raw_syscall5` with `SYS_IPC_RECV` for true blocking IPC event reception, eliminating busy-loop CPU spin and enabling kernel idle HLT power saving.

---

## 2. Active Technical Debt (Monitored)

- **Userspace migration:** compositor and built-in app logic still run in Ring 0. Ring-3 process records do not establish app isolation by themselves.
- **Scheduling:** `SYS_YIELD` triggers cooperative rescheduling via `schedule()`; SleepMutex / WaitQueue based kernel lock migration (WM, VFS) planned for Faz 4.
- **Fork:** `SYS_FORK` returns `ENOSYS` (-38) by architectural decision; SparkOS explicitly follows spawn/capability-oriented process model (`kernel/src/task/process.rs`). Legacy fork semantics will not be added to kernel core.
- **SMP reclamation:** cross-core TLB shootdown IPI lifetime synchronization in progress (Faz 4.5); concurrent execution of shared user address spaces on APs remains gated by completion of remote ACK protocols.
- **ABI:** SDK exec/waitpid numbering reconciled with kernel (`kernel/src/syscall.rs`: SYS_FORK=14, SYS_EXEC=15, SYS_WAITPID=16).
- **Scope:** heap exhaustion, all driver/DMA ownership paths (`kernel/src/dma_region.rs`), and physical hardware have not been comprehensively verified. See [hardening notes](docs/HARDENING.md).
- **Architectural Freeze:** "Freeze #5" refers to the baseline capability and memory architecture frozen prior to Faz 4 SMP multi-core development.

---

## 3. Package & Storage Specifications

### `SPFS v2 Block Engine (Faz 24)`:
- **Capacity:** InodeV2 64-byte layout, 6 Direct Blocks (3 KiB) + 1 Single Indirect Block (64 KiB) + 1 Double Indirect Block (8 MiB+).
- **Safety Guarantee:** Transactional block allocation with automatic rollback on error / ENOSPC, guaranteeing zero orphan blocks.

### `SPKG v1 Specification`:
- **Integrity Mechanism:** FNV-1a 32-bit checksum (`calculate_checksum`).
- **Future SPKG v2:** Ed25519 asymmetric cryptographic signatures.

## 2026-10-13: Memory / GUI hardening

- Fixed SIGCHLD scheduler borrow conflict that prevented the kernel build.
- Private userspace page-table subtree; owner-only surface mappings; no one-frame fallback.
- Central surface unmap/free/accounting; window close and process exit use the same path.
- Contiguous recycled-frame allocation and transactional process construction.
- Bounded/fallible ELF allocation; low kernel memory, surface/stack ranges, and page-overlapping segments rejected.
- Actual-kernel QEMU regression feature and reproducible snapshot runner added.

Historical “resolved”, “zero leak” and “formally audited” labels above are not a claim that every kernel path has been proved correct.
