# SparkOS — Technical Debt & Architectural Ledger
**Frozen Foundation:** Architecture Freeze #5  
**Status:** Hardened & Formally Audited  

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

## 2. Active Technical Debt (Monitored)

### `TD-MED-2`: `libspark::event::next()` Blocking Event Loop
- **Module:** `sdk/libspark/src/lib.rs` (`pub mod event`)
- **Current Behavior:** Falls back to `yield_cpu()` busy-loop when event queue is empty instead of true kernel-level blocking sleep.
- **Resolution Plan:** Transition to blocking IPC endpoint wait (`SYS_IPC_RECV`).
- **Target Phase:** Event Subsystem Hardening

---

## 3. Package & Storage Specifications

### `SPFS v2 Block Engine (Faz 24)`:
- **Capacity:** InodeV2 64-byte layout, 6 Direct Blocks (3 KiB) + 1 Single Indirect Block (64 KiB) + 1 Double Indirect Block (8 MiB+).
- **Safety Guarantee:** Transactional block allocation with automatic rollback on error / ENOSPC, guaranteeing zero orphan blocks.

### `SPKG v1 Specification`:
- **Integrity Mechanism:** FNV-1a 32-bit checksum (`calculate_checksum`).
- **Future SPKG v2:** Ed25519 asymmetric cryptographic signatures.
