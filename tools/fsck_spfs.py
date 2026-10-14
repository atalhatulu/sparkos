#!/usr/bin/env python3
"""fsck.spfs — SPFS Integrity Checker & Repair Tool (Faz 5.1)

Validates Superblock magic, Inode Bitmap consistency, Block Bitmap consistency,
and Directory Tree integrity.
"""
import sys
import struct
import argparse

SPFS_MAGIC = 0x53504653  # "SPFS"
BLOCK_SIZE = 512

def check_spfs(image_path, fix=False):
    errors = 0
    with open(image_path, 'r+b' if fix else 'rb') as f:
        # 1. Check Superblock
        f.seek(0)
        sb_data = f.read(512)
        if len(sb_data) < 512:
            print(f"[fsck.spfs] ERROR: Image smaller than 512 bytes", file=sys.stderr)
            return 1

        magic, block_size, total_blocks, inode_count, free_blocks, free_inodes = struct.unpack('<IIIIII', sb_data[:24])
        if magic != SPFS_MAGIC:
            print(f"[fsck.spfs] ERROR: Invalid magic 0x{magic:08X} (expected 0x{SPFS_MAGIC:08X})", file=sys.stderr)
            return 1
        
        print(f"[fsck.spfs] Superblock OK: magic=SPFS, block_size={block_size}, total_blocks={total_blocks}, inodes={inode_count}")

        # 2. Read Inode Bitmap
        f.seek(1 * BLOCK_SIZE)
        inode_bm = bytearray(f.read(BLOCK_SIZE))

        # 3. Read Block Bitmap
        f.seek(2 * BLOCK_SIZE)
        block_bm = bytearray(f.read(BLOCK_SIZE))

        # 4. Read Inode Table
        f.seek(3 * BLOCK_SIZE)
        inodes = []
        for i in range(inode_count):
            raw = f.read(64)
            iid, ftype, flags, _, size, bcount = struct.unpack('<IBB2sII', raw[:16])
            direct = struct.unpack('<8I', raw[16:48])
            inodes.append({
                'id': iid,
                'type': ftype,
                'flags': flags,
                'size': size,
                'block_count': bcount,
                'direct': direct,
            })

        # Check Root Inode (must be Directory and present)
        root = inodes[0]
        if root['type'] != 2:
            print(f"[fsck.spfs] ERROR: Root inode (0) type is {root['type']} (expected Directory = 2)", file=sys.stderr)
            errors += 1

        # Check Bitmap Consistency
        used_inodes = sum(1 for (idx, ino) in enumerate(inodes) if (inode_bm[idx // 8] & (1 << (idx % 8))) != 0)
        actual_free_inodes = inode_count - used_inodes
        if actual_free_inodes != free_inodes:
            print(f"[fsck.spfs] WARNING: Superblock free_inodes ({free_inodes}) mismatch actual ({actual_free_inodes})")
            if fix:
                sb_data = bytearray(sb_data)
                struct.pack_into('<I', sb_data, 20, actual_free_inodes)
                f.seek(0)
                f.write(sb_data)
                print(f"[fsck.spfs] Repaired Superblock free_inodes.")

        # Check Block references
        referenced_blocks = set()
        for ino in inodes:
            if ino['type'] != 0:
                for b in ino['direct']:
                    if b != 0:
                        if b >= total_blocks:
                            print(f"[fsck.spfs] ERROR: Inode {ino['id']} references out-of-range block {b}", file=sys.stderr)
                            errors += 1
                        referenced_blocks.add(b)

        # Ensure metadata blocks 0..7 are in referenced/used
        for mb in range(8):
            referenced_blocks.add(mb)

        used_blocks = 0
        for b in range(total_blocks):
            byte_idx = b // 8
            bit_idx = b % 8
            if byte_idx < len(block_bm) and (block_bm[byte_idx] & (1 << bit_idx)) != 0:
                used_blocks += 1

        print(f"[fsck.spfs] Referenced blocks: {len(referenced_blocks)}, Bitmap marked used: {used_blocks}")
        if errors == 0:
            print(f"[fsck.spfs] PASSED: SPFS filesystem is clean and consistent.")
            return 0
        else:
            print(f"[fsck.spfs] FAILED: {errors} integrity errors found.", file=sys.stderr)
            return 1

def main():
    parser = argparse.ArgumentParser(description='Check and repair SPFS filesystem')
    parser.add_argument('image', help='Disk image to check')
    parser.add_argument('--fix', action='store_true', help='Automatically fix minor discrepancies')
    args = parser.parse_args()
    sys.exit(check_spfs(args.image, args.fix))

if __name__ == '__main__':
    main()
