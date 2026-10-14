#!/usr/bin/env python3
"""mkfs.spfs — SPFS v1/v2 Disk Formatter Tool (Faz 5.1)

Formats a raw disk image with SPFS Superblock, Inode Bitmap, Block Bitmap,
Inode Table, and initial Root Directory.
"""
import sys
import struct
import argparse

SPFS_MAGIC = 0x53504653  # "SPFS"
BLOCK_SIZE = 512
TOTAL_BLOCKS = 2048
INODE_COUNT = 32

def create_spfs(image_path, total_blocks=TOTAL_BLOCKS):
    with open(image_path, 'r+b') as f:
        # 1. Superblock (Sector 0)
        free_blocks = total_blocks - 8  # 0: SB, 1: Inode BM, 2: Block BM, 3-6: Inode Table, 7: Root Dir
        free_inodes = INODE_COUNT - 1   # Inode 0 is root dir
        sb = struct.pack('<IIIIII', SPFS_MAGIC, BLOCK_SIZE, total_blocks, INODE_COUNT, free_blocks, free_inodes)
        sb += b'\x00' * (512 - len(sb))
        f.seek(0)
        f.write(sb)

        # 2. Inode Bitmap (Sector 1)
        # Inode 0 is allocated for root
        inode_bm = bytearray(512)
        inode_bm[0] = 0x01
        f.seek(1 * BLOCK_SIZE)
        f.write(inode_bm)

        # 3. Block Bitmap (Sector 2)
        # Blocks 0..7 are allocated
        block_bm = bytearray(512)
        block_bm[0] = 0xFF
        f.seek(2 * BLOCK_SIZE)
        f.write(block_bm)

        # 4. Inode Table (Sectors 3..6, 4 sectors * 8 inodes = 32 inodes, 64B each)
        f.seek(3 * BLOCK_SIZE)
        # Inode 0: Root Directory
        # inode_id: 0, file_type: 2 (Directory), flags: 3 (RW), size: 512, block_count: 1, direct[0]: 7
        root_inode = struct.pack('<IBB2sII', 0, 2, 3, b'\x00\x00', 512, 1)
        direct_blocks = [7] + [0] * 7
        root_inode += struct.pack('<8I', *direct_blocks)
        root_inode += b'\x00' * (64 - len(root_inode))
        f.write(root_inode)

        # Zero remaining 31 inodes
        f.write(b'\x00' * (64 * (INODE_COUNT - 1)))

        # 5. Root Directory Data Block (Sector 7)
        # DirectoryEntry: inode_id (4B), name (28B) -> 32B each
        root_dir_data = bytearray(512)
        # Entry 0: "." -> Inode 0
        struct.pack_into('<I28s', root_dir_data, 0, 0, b'.')
        # Entry 1: ".." -> Inode 0
        struct.pack_into('<I28s', root_dir_data, 32, 0, b'..')
        f.seek(7 * BLOCK_SIZE)
        f.write(root_dir_data)

    print(f"[mkfs.spfs] Successfully formatted {image_path} with SPFS (blocks={total_blocks}, inodes={INODE_COUNT})")

def main():
    parser = argparse.ArgumentParser(description='Format disk image with SPFS')
    parser.add_argument('image', help='Target disk image file')
    parser.add_argument('--blocks', type=int, default=TOTAL_BLOCKS, help='Total blocks')
    args = parser.parse_args()
    create_spfs(args.image, args.blocks)

if __name__ == '__main__':
    main()
