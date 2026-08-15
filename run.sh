#!/bin/bash
set -e
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$DIR"

echo "================================================================"
echo "                   SparkOS x86_64 Boot Launcher                 "
echo "================================================================"

echo "[1/3] SparkOS Kernel Derleniyor..."
cargo bootimage 2>&1 | grep -v "^   Compiling\|^    Finished\|^    Locking\|Downloading\|^  Downloaded\|^  Installing"

BIN=$(ls -1t target/*/debug/bootimage-sparkos.bin 2>/dev/null | head -1)
if [ -z "$BIN" ]; then
    echo "HATA: bootimage-sparkos.bin bulunamadı!"
    exit 1
fi

if [ ! -f "disk.img" ]; then
    echo "[2/3] 10MB Sanal ATA Diski (disk.img) oluşturuluyor..."
    dd if=/dev/zero of=disk.img bs=1M count=10 2>/dev/null
fi

echo "[3/3] QEMU başlatılıyor..."

CLI_MODE=0
for arg in "$@"; do
    if [ "$arg" == "--cli" ] || [ "$arg" == "--headless" ] || [ "$arg" == "--no-gui" ]; then
        CLI_MODE=1
    fi
done

if [ "$CLI_MODE" -eq 1 ]; then
    echo "--> Saf Terminal Modu (Headless CLI) secildi. VNC penceresi acilmayacak."
    VNC_PORT_ARG="-display none"
else
    # VNC portunu belirle
    VNC_DISPLAY=":0"
    VNC_PORT_ARG="-vnc :0"

    # TigerVNC / VNC Viewer varsa pencereli (windowed) modda başlat (Tam ekran KAPALI)
    if which vncviewer &>/dev/null; then
        echo "--> TigerVNC Pencereli Modda (1024x768) açılıyor..."
        (sleep 0.8; vncviewer -geometry 1024x768 $VNC_DISPLAY 2>/dev/null || true) &
    fi
fi

# Arka planda açılan pencereleri script kapanırken temizle
cleanup() {
    kill $(jobs -p) 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo ""
echo "--> SparkOS Seri Port (COM1) Terminal Konsolu:"
echo "----------------------------------------------------------------"

# QEMU'yu başlat (SMP 2 Çekirdek, RTL8139 Ağ Kartı, ATA Disk, VGA Grafik)
qemu-system-x86_64 \
    -drive format=raw,file="$BIN",index=0,media=disk \
    -drive format=raw,file=disk.img,index=1,media=disk \
    -serial stdio \
    -vga std \
    -m 256M \
    -smp 2 \
    -netdev user,id=net0 \
    -device rtl8139,netdev=net0 \
    $VNC_PORT_ARG
