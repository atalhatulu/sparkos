#!/usr/bin/env bash
set -euo pipefail

# tools/check_deps.sh — Mimari Bağımlılık Denetleyicisi (Faz 1.6)
# Grup A (Core Kernel) dosyalarındaki B/C/D modül referanslarını denetler.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

python3 - << 'EOF'
import sys
import os
import re

GROUP_A_FILES = [
    "memory.rs", "frame_pool.rs", "allocator.rs", "interrupts.rs", "gdt.rs",
    "task/process.rs", "task/simple_executor.rs", "task/yield_now.rs", "task/keyboard.rs", "task/mod.rs",
    "cap.rs", "ipc.rs", "sync.rs", "syscall.rs", "syscall_cap.rs", "syscall_storage.rs",
    "smp.rs", "acpi.rs", "iommu.rs", "dma_region.rs", "klog.rs", "panic.rs",
    "ktrace.rs", "serial.rs", "perf.rs", "perf_histogram.rs", "irq_mutex.rs",
    "sec_mem.rs", "elf.rs", "user.rs", "fd.rs"
]

NON_A_MODULES = {
    # Grup B (Sürücüler)
    "ata", "rtl8139", "usb", "pci", "gpu", "gpu_virtio", "keyboard", "mouse", "vga_buffer", "display",
    # Grup C (Servisler)
    "fs", "net", "net_socket", "network_service", "network_manager", "pkg", "pkg_service",
    "store_service", "service", "auth", "security", "permission", "shell_service", "shell", "crash_reporter",
    # Grup D (Masaüstü/UI)
    "wm", "desktop", "dock", "system_bar", "cursor", "damage", "animation", "theme", "icons",
    "surface", "surface_raster", "ui", "gui", "input", "event", "clipboard", "notification",
    "launcher", "app", "app_manager", "app_registry", "browser_app", "editor_app", "editor",
    "files_app", "settings_app", "taskmgr_app", "terminal_app", "sysmon_app", "text", "font"
}

WHITELIST_DOC = "docs/ALLOWED_VIOLATIONS.md"
if not os.path.exists(WHITELIST_DOC):
    print(f"HATA: {WHITELIST_DOC} bulunamadi!", file=sys.stderr)
    sys.exit(1)

with open(WHITELIST_DOC) as f:
    whitelist_content = f.read()

# İzinli (file_rel, mod) çiftlerini çıkar
allowed_pairs = set()
for line in whitelist_content.splitlines():
    m = re.search(r"`kernel/src/([^`]+)`\s*\|\s*`crate::([^`]+)`", line)
    if m:
        allowed_pairs.add((m.group(1), m.group(2)))

print(f"[check_deps] Yuklenen izinli ters bagimlilik kurali: {len(allowed_pairs)}")

unauthorized = []
found_allowed = 0

for f in GROUP_A_FILES:
    path = os.path.join("kernel/src", f)
    if not os.path.exists(path):
        continue
    with open(path) as fp:
        lines = fp.readlines()
    for idx, line in enumerate(lines, 1):
        # Yorum satırlarını atla
        trimmed = line.strip()
        if trimmed.startswith("//") or trimmed.startswith("/*") or trimmed.startswith("*"):
            continue
        for mod in NON_A_MODULES:
            if re.search(r"\bcrate::" + mod + r"\b", line):
                pair = (f, mod)
                if pair in allowed_pairs:
                    found_allowed += 1
                else:
                    unauthorized.append((f, idx, mod, trimmed))

print(f"[check_deps] Denetlenen Grup A dosyalari: {len(GROUP_A_FILES)}")
print(f"[check_deps] Gecerli izinli referans sayisi: {found_allowed}")

if unauthorized:
    print(f"\n[check_deps] HATA: {len(unauthorized)} adet izinsiz ters bagimlilik bulundu!", file=sys.stderr)
    for f, line_no, mod, text in unauthorized:
        print(f"  {f}:{line_no} -> crate::{mod} ({text})", file=sys.stderr)
    print(f"\nLutfen bu bagimliligi temizleyin veya gerekcesiyle {WHITELIST_DOC} dosyasina ekleyin.", file=sys.stderr)
    sys.exit(1)

print("[check_deps] BASARILI: Hicbir izinsiz ters bagimlilik bulunmadi.")
EOF
