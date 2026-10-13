# Kernel / GUI hardening — 2026-10-13

This change repairs the build and memory lifecycle of the current desktop. It does
not turn the built-in GUI into a userspace compositor or certify the entire OS.

## Changes

- Fixed the SIGCHLD parent notification borrow conflict in `send_signal`.
- Added a hardware-independent physical frame pool. Returned contiguous surface
  extents can be reused; failed large requests do not discard smaller regions.
  DMA allocation now requests a contiguous extent instead of assuming separate
  allocations are adjacent.
- A new process owns its low PDPT and initially empty 1–2 GiB subtree. Low kernel
  mappings remain supervisor-only. Surface buffers are mapped only in the recorded
  owner's CR3; the compositor accesses them through the physical-memory mapping.
- Surface creation reserves a slot and quota, allocates all required frames, and
  rolls back quotas/backing/PTEs on failure. A single physical frame is never used
  as a fallback for a multi-page buffer. Unmapping validates the expected physical
  backing before freeing it, reclaims empty user paging structures, and updates
  the same accounting for syscall destruction, window close, and process exit.
- Process exit performs GUI cleanup outside the scheduler lock. Window close does
  not destroy the active caller's kernel stack. Background processes are not
  terminated merely because their final window closed.
- Address-space and pending-process guards reclaim partially constructed apps.
  Private code/stack pages carry a software ownership bit so address-space teardown
  does not free external DMA backing as if it owned it.
- ELF parsing validates the entire image before payload allocation, limits load
  memory to 16 MiB and program headers to 128, uses fallible reservations, requires
  an executable entry segment, and rejects page-overlapping segments. Unaligned
  mappings include the initial page offset when computing page count.
- SDK binaries link at `0x40000000`, below the surface region at `0x70000000`.
  The previous 4 MiB base overlaps the kernel. Existing binaries need rebuilding.
  Fixtures are now under `fixtures/`, not an ignored scratch-directory dependency.
- User pointer validation checks access rights at every paging level and rejects
  the low kernel address region. Service data initialization uses the owner's
  physical mapping rather than relying on accidentally shared user page tables.
- Restored the executor interrupt-enable state after Ring-3 service return. The
  smoke test now waits for an actual compositor frame, not just a boot banner.
- `run.sh` stops when the build fails instead of potentially booting an old image.

## Reproduce

From the repository root, rebuild checked-in assembly fixtures if their source or
linker script changed (GNU `as` and `ld` are required):

```sh
bash tools/build_hello.sh
```

Host tests run production capability, IPC, ELF, filesystem, network, damage and
frame-pool code; some older tests still exercise duplicate mock GUI models.

```sh
cd scratch/cap_test
cargo test -Zbuild-std= --target x86_64-unknown-linux-gnu -- --test-threads=1
```

From the repository root, build the **opt-in** actual-kernel regression image:

```sh
cargo bootimage --features kernel-regression
python3 tools/qemu_smoke.py \
  --image target/x86_64-unknown-none/debug/bootimage-sparkos.bin \
  --disk disk.img --output target/qemu-regression --regression
```

The runner uses snapshots for both disks, checks serial failures and completion
markers, saves a PPM screenshot, and terminates its QEMU process. It clears old
results before each run. `--cpus 1 --memory 128M` selects the smaller configuration.
Default `cargo bootimage` excludes the regression and allocation-failure injection
code. `disk.img` is not modified by these tests.

The actual-kernel tests cover:

1. Two processes using the same surface virtual address without sharing backing;
   no surface alias in the kernel CR3; rejection of destruction by another owner.
2. Failure before backing allocation, during page-table allocation, and after
   partial mapping; recovery of quotas, physical frames and mappings.
3. An existing mapping collision without overwriting/freeing that mapping.
4. Surface quota rejection and 24 create/destroy cycles of a 1280×720 buffer.
5. Failed terminal construction at multiple allocation stages, including after
   PCB creation, with frame/process counts returning to baseline.
6. Eight real built-in terminal open/maximize/restore/composite/close cycles.
7. Real Ring-3 ELF write/exit and complete private-address-space reclamation.
8. A Ring-3 assembly probe that creates a surface, writes its first/last pixels,
   destroys it and reuses the slot 20 times through the actual syscall ABI.

## Verified results

- Host suite: **896 passed, 0 failed** (includes the production ELF and frame-pool regressions).
- QEMU TCG: all eight real-kernel regression groups passed with **2 vCPUs / 256 MiB**
  and **1 vCPU / 128 MiB**; both continued to the desktop after the tests.
- SDK `hello`: release build succeeded with the actual userspace linker script.
- Existing kernel build emits 16 warnings; they are not represented as errors or
  as a clean-warning build. Hardware validation and IOMMU isolation were not tested.

## Remaining architecture work

- Built-in terminal/files/editor logic and `wm::compositor_task` execute in Ring 0.
  Migrating them requires an ABI/event-loop/scheduler change, not merely registering
  an ELF process. `SYS_YIELD` is currently a no-op; SDK exec/waitpid constants and
  the kernel dispatch table also disagree.
- Fork does not yet implement copying the caller's pages and complete user register
  continuation. Fresh-process address-space creation must not be confused with fork.
- AP bootstrap and work-stealing examples are not an integrated parallel user
  scheduler. Two-vCPU boot tests do not prove concurrent userspace correctness.
  In particular, a shootdown timeout must become a reliable lifetime barrier
  before processes can share active address spaces across CPUs.
- General heap exhaustion, every driver/DMA ownership path, CoW sharing/refcounts,
  filesystem crash consistency, and bare-metal hardware are outside this test suite.
- Existing compiler warnings and mock-based tests remain. Passing host invariants
  is not formal verification or a substitute for the real-kernel tests above.

The next migration step is to reconcile the process/GUI ABI and implement a tested
userspace yield/resume/event path, then move the terminal to an SDK ELF. The
compositor/display service can follow with explicit framebuffer and surface grants.


## GUI interaction follow-up

User close requests now share an ownership-checked path for the titlebar button
and Alt+F4. Repeated requests keep a dirty editor's confirmation visible and mark
the window for repaint; Escape cancels and explicit discard closes it. Launcher
keyboard damage now covers the actual top dropdown and launcher button. Keyboard
producer/consumer use shared Ctrl/Shift/Alt bits (1/2/8). Window destruction also
cleans Settings and Task Manager instance state.

The real-kernel GUI regression passed under QEMU with 2 CPUs and 256 MiB:
repeated Alt+F4, Escape cancellation, titlebar close, explicit discard, resource
reclaim, Ctrl+Escape, arrow navigation, and dropdown repaint bounds. The previous
eight regression groups also passed, followed by the first desktop frame. This
checks input dispatch directly; it does not simulate physical keyboard IRQs.
Log: `target/hardening/qemu-gui/serial.log`.


## Settings interaction and theme follow-up

Existing GUI input/layout work was preserved. Settings now distinguishes a local
category redraw from a global theme refresh. Theme refresh snapshots windows and
rerenders their app surfaces after releasing WM and input instance locks. Sidebar
gaps no longer activate categories; out-of-client clicks are not clamped into
widgets. The System page no longer re-enters WM during maximize/resize rendering,
and reports frame-pool usage instead of a fixed RAM size or unverified preemption.
Theme button text was shortened after inspecting a real screenshot for overflow.

Validation: 906 host tests passed after correcting two existing harness compile
errors (Result alias and damage module name). All ten actual-kernel regression
groups passed on 2 CPUs/256 MiB and 1 CPU/128 MiB. The new group checks two Settings
instances, mouse and keyboard navigation, surface pixels after theme refresh,
theme restoration, System maximize/restore and frame reclamation. QEMU sendkey
also opened Settings through the launcher and switched the theme; screenshots
are in `target/hardening/settings-preview/`. The final label-only shortening was
made after those captures. Logs: `target/hardening/settings-host-tests.log` and
`target/hardening/qemu-settings-{1cpu,2cpu}/serial.log`.

Remaining: some applications still use fixed palettes (rerendering alone does not
make their contents theme-aware); full GUI lifetime/SMP and TaskMgr termination
need separate validation. Existing mock scheduler/app tests do not prove Ring-3
migration or real blocking scheduling.

## Desktop performance pass

- Compositor now consumes the bounded damage-region list rather than collapsing
  all damage into one bounding rectangle. Nearby rectangles merge when that is
  no more pixel work; capacity overflow chooses the least added bounding area.
- Windows outside each region are skipped before chrome rendering/surface lookup.
  Surface row copies iterate only the intersecting vertical span.
- Dirty framebuffer copies use correctly aligned u32 pointers even at odd x.
- The normal dev bootimage uses opt-level=2, preserving debug information, debug
  assertions and overflow checks. `run.sh` automatically gets this profile.
- Optimized execution exposed an undeclared RAX scratch/input collision in the
  Ring-3 entry assembly. The IRET frame now pushes inputs directly and uses an
  explicit initial RFLAGS value, rather than overwriting an input in RAX.

Validation: 909 host tests passed. All eleven kernel regression groups passed on
1 CPU/128 MiB and 2 CPUs/256 MiB using the optimized build, including actual Ring-3
write/exit and surface syscalls. A deterministic compositor test corrupts two
16x16 regions, reconstructs them, and compares the entire backbuffer with a full
reference frame. It verifies 512 processed pixels versus the previous 445176-pixel
bounding rectangle, one intersecting window, and no pixel work on an idle call.
This is a work-reduction measurement for that sparse-update case, not an FPS or
end-to-end input latency benchmark. Large drags can still damage broad regions;
VNC transport and cooperative task latency remain separate performance factors.

Logs: `target/hardening/perf-host-tests.log`,
`target/hardening/qemu-perf-fixed-{1cpu,2cpu}/serial.log`.
Earlier `qemu-perf-*cpu` logs without `fixed` captured the initial assembly fault;
they are retained as failure evidence, not successful validation.
