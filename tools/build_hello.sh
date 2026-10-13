#!/usr/bin/env bash
# Rebuild the checked-in, minimal Ring-3 ELF fixture with the SDK address layout.
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$repo/target/fixtures"
for fixture in hello surface_probe gui_demo; do
    as --64 "$repo/fixtures/$fixture.S" -o "$repo/target/fixtures/$fixture.o"
    ld -T "$repo/sdk/linker/app.ld" -o "$repo/fixtures/$fixture.elf" "$repo/target/fixtures/$fixture.o"
done
