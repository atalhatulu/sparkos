#!/bin/bash
set -e
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cd "$DIR"

echo "==> Building SparkOS..."
cargo bootimage 2>&1 | grep -v "^   Compiling\|^    Finished\|^    Locking\|Downloading\|^  Downloaded\|^  Installing"

BIN=$(ls -1t target/*/debug/bootimage-sparkos.bin 2>/dev/null | head -1)
if [ -z "$BIN" ]; then
    echo "FAIL: bootimage not found!"
    exit 1
fi

if [ ! -f "disk.img" ]; then
    echo "==> Creating 10MB persistent virtual disk (disk.img)..."
    dd if=/dev/zero of=disk.img bs=1M count=10 2>/dev/null
fi

echo "==> QEMU'da baslatiliyor..."

# VNC portu dene, meşgulse serial kullan
VNC_PORT=""
if ! ss -tln | grep -q ":5900 "; then
    VNC_PORT="-vnc :0"
    echo "    VNC :0'da yayin"
fi

# VNC baglantisi (varsa)
if [ -n "$VNC_PORT" ] && which vncviewer &>/dev/null; then
    (sleep 0.5; vncviewer -fullscreen :0 2>/dev/null) &
fi

echo ""
echo "    Serial cikti:"
echo ""

timeout --foreground 60 qemu-system-x86_64 \
    -drive format=raw,file="$BIN",index=0,media=disk \
    -drive format=raw,file=disk.img,index=1,media=disk \
    -serial stdio \
    -vga std \
    -m 256M \
    $VNC_PORT
