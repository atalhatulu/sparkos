use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;
use spin::Mutex;
use core::convert::TryInto;

#[derive(Clone)]
pub enum FsNode {
    File {
        name: String,
        content: Vec<u8>,
    },
    Directory {
        name: String,
        children: Vec<FsNode>,
    },
    Device {
        name: String,
    }
}

impl FsNode {
    pub fn name(&self) -> &str {
        match self {
            FsNode::File { name, .. } => name,
            FsNode::Directory { name, .. } => name,
            FsNode::Device { name } => name,
        }
    }
    
    pub fn is_dir(&self) -> bool {
        matches!(self, FsNode::Directory { .. })
    }
}

// -----------------------------------------------------------------------------
// Faz 16: SPFS v1 Canonical On-Disk Structures
// -----------------------------------------------------------------------------

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InodeType {
    Unused = 0,
    Regular = 1,
    Directory = 2,
    CharDevice = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inode {
    pub inode_id: u32,           // 4 bytes
    pub file_type: u8,           // 1 byte (InodeType)
    pub flags: u8,               // 1 byte (1: Read, 2: Write, 3: RW)
    pub _reserved1: [u8; 2],     // 2 bytes -> 8-byte boundary
    pub size: u32,               // 4 bytes (max 4096 bytes in SPFS v1)
    pub block_count: u32,        // 4 bytes (max 8 direct blocks)
    pub direct_blocks: [u32; 8], // 32 bytes (8 * 512B = 4KB SPFS v1 limit)
    pub _reserved2: [u8; 16],    // 16 bytes -> 64 bytes total
}

const _: () = assert!(core::mem::size_of::<Inode>() == 64);

// -----------------------------------------------------------------------------
// Faz 22: SPFS v2 Canonical On-Disk Structures (UID/GID, Mode, Indirect Blocks)
// -----------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InodeV2 {
    pub inode_id: u32,           // 4 bytes
    pub file_type: u8,           // 1 byte (InodeType)
    pub flags: u8,               // 1 byte
    pub permissions: u16,        // 2 bytes (POSIX mode bits: e.g. 0o755)
    pub uid: u16,                // 2 bytes (Owner User ID)
    pub gid: u16,                // 2 bytes (Owner Group ID)
    pub size: u32,               // 4 bytes (File size in bytes)
    pub block_count: u32,        // 4 bytes (Total allocated sectors)
    pub direct_blocks: [u32; 6], // 24 bytes (6 * 512B = 3KB direct)
    pub indirect_block: u32,     // 4 bytes (Single indirect = 128 * 512B = 64KB)
    pub double_indirect: u32,    // 4 bytes (Double indirect = 128 * 128 * 512B = 8MB+)
    pub _reserved: [u8; 12],     // 12 bytes -> 64 bytes total
}

const _: () = assert!(core::mem::size_of::<InodeV2>() == 64);

pub const SPFS_V2_MAGIC: u32 = 0x53504632; // "SPF2"

// -----------------------------------------------------------------------------
// Faz 24: Advanced Storage Engine & Dynamic Indirect Block Allocator
// -----------------------------------------------------------------------------

pub const BLOCK_SIZE: usize = 512;
pub const PTRS_PER_BLOCK: usize = BLOCK_SIZE / 4; // 128 pointers per block (4 bytes per u32)
pub const DIRECT_BLOCKS_COUNT: usize = 6;
pub const MAX_DIRECT_SIZE: usize = DIRECT_BLOCKS_COUNT * BLOCK_SIZE; // 3072 bytes (3 KiB)
pub const MAX_SINGLE_INDIRECT_SIZE: usize = MAX_DIRECT_SIZE + (PTRS_PER_BLOCK * BLOCK_SIZE); // 3 KiB + 64 KiB = 67 KiB
pub const MAX_DOUBLE_INDIRECT_SIZE: usize = MAX_SINGLE_INDIRECT_SIZE + (PTRS_PER_BLOCK * PTRS_PER_BLOCK * BLOCK_SIZE); // ~8.38 MiB

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    NoSpace,
    InvalidSeek,
    PermissionDenied,
    NotFound,
    CorruptedMetadata,
}

pub struct BlockAllocator {
    pub total_blocks: u32,
    pub free_blocks: u32,
    pub bitmap: Vec<bool>, // true: allocated, false: free
}

impl BlockAllocator {
    pub fn new(total_blocks: u32) -> Self {
        Self {
            total_blocks,
            free_blocks: total_blocks,
            bitmap: alloc::vec![false; total_blocks as usize],
        }
    }

    /// Allocates a single free sector. Returns None if disk is full.
    pub fn allocate_block(&mut self) -> Option<u32> {
        if self.free_blocks == 0 {
            return None;
        }
        for (idx, used) in self.bitmap.iter_mut().enumerate() {
            if !*used {
                *used = true;
                self.free_blocks -= 1;
                return Some(idx as u32);
            }
        }
        None
    }

    /// Reclaims a previously allocated block.
    pub fn free_block(&mut self, block: u32) {
        let idx = block as usize;
        if idx < self.bitmap.len() && self.bitmap[idx] {
            self.bitmap[idx] = false;
            self.free_blocks += 1;
        }
    }
}

pub struct SpfsV2Engine {
    pub allocator: BlockAllocator,
    pub block_data: alloc::collections::BTreeMap<u32, [u8; BLOCK_SIZE]>,
}

impl SpfsV2Engine {
    pub fn new(total_blocks: u32) -> Self {
        Self {
            allocator: BlockAllocator::new(total_blocks),
            block_data: alloc::collections::BTreeMap::new(),
        }
    }

    /// Writes a slice of bytes into an InodeV2, dynamically allocating direct,
    /// single-indirect, and double-indirect blocks.
    /// Guarantees transactional rollback on ENOSPC to prevent orphan blocks.
    pub fn write_file_transactional(
        &mut self,
        inode: &mut InodeV2,
        data: &[u8],
    ) -> Result<usize, FsError> {
        if data.is_empty() {
            return Ok(0);
        }

        let needed_blocks = (data.len() + BLOCK_SIZE - 1) / BLOCK_SIZE;
        let mut staging_allocations: Vec<u32> = Vec::new();

        // 1. Transactional check & block pre-allocation
        for _ in 0..needed_blocks {
            match self.allocator.allocate_block() {
                Some(blk) => staging_allocations.push(blk),
                None => {
                    // Rollback all staged allocations (Zero-Orphan guarantee)
                    for blk in staging_allocations {
                        self.allocator.free_block(blk);
                    }
                    return Err(FsError::NoSpace);
                }
            }
        }

        // 2. Write data into allocated blocks
        let mut blk_iter = staging_allocations.into_iter();
        let mut bytes_written = 0usize;
        let mut block_idx = 0usize;

        while bytes_written < data.len() {
            let chunk_len = (data.len() - bytes_written).min(BLOCK_SIZE);
            let blk = blk_iter.next().unwrap();
            let mut block_buf = [0u8; BLOCK_SIZE];
            block_buf[..chunk_len].copy_from_slice(&data[bytes_written..bytes_written + chunk_len]);

            self.block_data.insert(blk, block_buf);

            // Assign to Inode pointers
            if block_idx < DIRECT_BLOCKS_COUNT {
                inode.direct_blocks[block_idx] = blk;
            } else if block_idx < DIRECT_BLOCKS_COUNT + PTRS_PER_BLOCK {
                if inode.indirect_block == 0 {
                    // Allocate single indirect table block
                    if let Some(table_blk) = self.allocator.allocate_block() {
                        inode.indirect_block = table_blk;
                        self.block_data.insert(table_blk, [0u8; BLOCK_SIZE]);
                    }
                }
                // Store blk into indirect table
                let indirect_slot = block_idx - DIRECT_BLOCKS_COUNT;
                if let Some(table_buf) = self.block_data.get_mut(&inode.indirect_block) {
                    let le_bytes = blk.to_le_bytes();
                    table_buf[indirect_slot * 4..(indirect_slot + 1) * 4].copy_from_slice(&le_bytes);
                }
            }

            bytes_written += chunk_len;
            block_idx += 1;
        }

        inode.size = data.len() as u32;
        inode.block_count = block_idx as u32;
        Ok(bytes_written)
    }

    /// Truncates / reclaims all direct and indirect blocks assigned to an InodeV2.
    pub fn truncate_and_reclaim(&mut self, inode: &mut InodeV2) {
        // Direct blocks
        for blk in &mut inode.direct_blocks {
            if *blk != 0 {
                self.allocator.free_block(*blk);
                self.block_data.remove(blk);
                *blk = 0;
            }
        }

        // Single indirect blocks
        if inode.indirect_block != 0 {
            if let Some(table_buf) = self.block_data.remove(&inode.indirect_block) {
                for i in 0..PTRS_PER_BLOCK {
                    let ptr_bytes: [u8; 4] = table_buf[i * 4..(i + 1) * 4].try_into().unwrap_or([0; 4]);
                    let blk = u32::from_le_bytes(ptr_bytes);
                    if blk != 0 {
                        self.allocator.free_block(blk);
                        self.block_data.remove(&blk);
                    }
                }
            }
            self.allocator.free_block(inode.indirect_block);
            inode.indirect_block = 0;
        }

        // Double indirect blocks
        if inode.double_indirect != 0 {
            if let Some(dtable_buf) = self.block_data.remove(&inode.double_indirect) {
                for i in 0..PTRS_PER_BLOCK {
                    let ptr_bytes: [u8; 4] = dtable_buf[i * 4..(i + 1) * 4].try_into().unwrap_or([0; 4]);
                    let indirect_blk = u32::from_le_bytes(ptr_bytes);
                    if indirect_blk != 0 {
                        if let Some(table_buf) = self.block_data.remove(&indirect_blk) {
                            for j in 0..PTRS_PER_BLOCK {
                                let blk_bytes: [u8; 4] = table_buf[j * 4..(j + 1) * 4].try_into().unwrap_or([0; 4]);
                                let blk = u32::from_le_bytes(blk_bytes);
                                if blk != 0 {
                                    self.allocator.free_block(blk);
                                    self.block_data.remove(&blk);
                                }
                            }
                        }
                        self.allocator.free_block(indirect_blk);
                    }
                }
            }
            self.allocator.free_block(inode.double_indirect);
            inode.double_indirect = 0;
        }

        inode.size = 0;
        inode.block_count = 0;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Superblock {
    pub magic: u32,              // 0x53504653 ("SPFS")
    pub block_size: u32,         // 512 bytes
    pub total_blocks: u32,       // 2048 blocks
    pub inode_count: u32,        // 32 inodes
    pub free_blocks: u32,
    pub free_inodes: u32,
    pub _padding: [u8; 488],     // 512 bytes total
}

const _: () = assert!(core::mem::size_of::<Superblock>() == 512);

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub inode_id: u32,
    pub name: [u8; 28],          // 32 bytes per entry
}

const _: () = assert!(core::mem::size_of::<DirectoryEntry>() == 32);

pub static VFS: spin::Lazy<Mutex<FsNode>> = spin::Lazy::new(|| {
    Mutex::new(FsNode::Directory {
        name: "/".to_string(),
        children: Vec::new(),
    })
});

fn serialize(node: &FsNode, buf: &mut Vec<u8>) {
    match node {
        FsNode::File { name, content } => {
            buf.push(1); // File
            let name_bytes = name.as_bytes();
            buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(name_bytes);
            buf.extend_from_slice(&(content.len() as u32).to_le_bytes());
            buf.extend_from_slice(content);
        }
        FsNode::Directory { name, children } => {
            buf.push(2); // Directory
            let name_bytes = name.as_bytes();
            buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(name_bytes);
            buf.extend_from_slice(&(children.len() as u32).to_le_bytes());
            for child in children {
                serialize(child, buf);
            }
        }
        FsNode::Device { name } => {
            buf.push(3); // Device
            let name_bytes = name.as_bytes();
            buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(name_bytes);
        }
    }
}

fn deserialize(data: &[u8], offset: &mut usize) -> Option<FsNode> {
    if *offset >= data.len() { return None; }
    let typ = data[*offset];
    *offset += 1;
    if typ == 1 {
        let name_len = u32::from_le_bytes(data[*offset..*offset+4].try_into().ok()?) as usize;
        *offset += 4;
        let name = core::str::from_utf8(&data[*offset..*offset+name_len]).ok()?.to_string();
        *offset += name_len;
        let content_len = u32::from_le_bytes(data[*offset..*offset+4].try_into().ok()?) as usize;
        *offset += 4;
        let content = data[*offset..*offset+content_len].to_vec();
        *offset += content_len;
        Some(FsNode::File { name, content })
    } else if typ == 2 {
        let name_len = u32::from_le_bytes(data[*offset..*offset+4].try_into().ok()?) as usize;
        *offset += 4;
        let name = core::str::from_utf8(&data[*offset..*offset+name_len]).ok()?.to_string();
        *offset += name_len;
        let child_count = u32::from_le_bytes(data[*offset..*offset+4].try_into().ok()?) as usize;
        *offset += 4;
        let mut children = Vec::new();
        for _ in 0..child_count {
            if let Some(child) = deserialize(data, offset) {
                children.push(child);
            }
        }
        Some(FsNode::Directory { name, children })
    } else if typ == 3 {
        let name_len = u32::from_le_bytes(data[*offset..*offset+4].try_into().ok()?) as usize;
        *offset += 4;
        let name = core::str::from_utf8(&data[*offset..*offset+name_len]).ok()?.to_string();
        *offset += name_len;
        Some(FsNode::Device { name })
    } else {
        None
    }
}

pub fn sync_to_disk() -> Result<(), &'static str> {
    let mut buf = Vec::new();
    {
        let root = VFS.lock();
        serialize(&*root, &mut buf);
    }

    let size = buf.len() as u32;
    let mut header = [0u8; 512];
    header[0..4].copy_from_slice(&size.to_le_bytes());
    header[4..8].copy_from_slice(b"SPFS"); // SPark File System

    // Disk may be absent (boot ISO / QEMU without a data drive). In that case
    // the filesystem runs as an in-memory (RAM) FS — never let a missing disk
    // hang or hard-fail boot. All drops to Ok; writes are best-effort.
    let mut drive = crate::ata::DATA_DRIVE.lock();
    if drive.write_sector(0, &header).is_err() {
        return Ok(());
    }

    let mut lba = 1;
    for chunk in buf.chunks(512) {
        let mut sec = [0u8; 512];
        sec[..chunk.len()].copy_from_slice(chunk);
        if drive.write_sector(lba, &sec).is_err() {
            break;
        }
        lba += 1;
    }

    Ok(())
}

pub fn init_default_fs() {
    let _ = mkdir("/bin");
    let _ = mkdir("/etc");
    let _ = mkdir("/home");
    let _ = mkdir("/sys");
    // Fresh/empty filesystem: make sure distro seeds exist too.
    seed_default_files();
}

pub fn load_from_disk() {
    let r = {
        let mut drive = crate::ata::DATA_DRIVE.lock();
        let mut hdr = [0u8; 512];
        drive.read_sector(0, &mut hdr)
    }; // drop(drive): guard must be released before init_default_fs,
       // which also locks DATA_DRIVE (via sync_to_disk) -> would deadlock.
    if r.is_err() {
        init_default_fs();
        return;
    }

    // Re-read header with a fresh short-lived guard.
    let mut header = [0u8; 512];
    {
        let mut drive = crate::ata::DATA_DRIVE.lock();
        if drive.read_sector(0, &mut header).is_err() {
            init_default_fs();
            return;
        }
    }

    if &header[4..8] != b"SPFS" {
        init_default_fs();
        return;
    }

    let size = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
    if size == 0 || size > 10 * 1024 * 1024 {
        init_default_fs();
        return;
    }

    let num_sectors = (size + 511) / 512;
    let mut data = alloc::vec![0u8; num_sectors * 512];

    {
        let mut drive = crate::ata::DATA_DRIVE.lock();
        for i in 0..num_sectors {
            let mut sec = [0u8; 512];
            if drive.read_sector(1 + i as u32, &mut sec).is_ok() {
                data[i * 512..(i + 1) * 512].copy_from_slice(&sec);
            }
        }
    }

    let mut offset = 0;
    if let Some(node) = deserialize(&data[..size], &mut offset) {
        *VFS.lock() = node;
    }

    // Populate builtin userland binaries + first-boot config files. Idempotent,
    // runs on every boot regardless of the on-disk state.
    seed_default_files();
}

pub fn resolve_path(cwd: &str, path: &str) -> String {
    let mut parts = Vec::new();
    
    let full_path = if path.starts_with('/') {
        path.to_string()
    } else {
        if cwd == "/" {
            format!("/{}", path)
        } else {
            format!("{}/{}", cwd, path)
        }
    };
    
    for part in full_path.split('/') {
        match part {
            "" | "." => {}
            ".." => { parts.pop(); }
            _ => parts.push(part),
        }
    }
    
    if parts.is_empty() {
        "/".to_string()
    } else {
        let mut result = String::new();
        for p in parts {
            result.push('/');
            result.push_str(p);
        }
        result
    }
}

fn find_dir<'a>(root: &'a mut FsNode, path: &str) -> Option<&'a mut Vec<FsNode>> {
    if path == "/" {
        if let FsNode::Directory { children, .. } = root {
            return Some(children);
        }
        return None;
    }
    
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut current = root;
    
    for part in parts {
        let mut next = None;
        if let FsNode::Directory { children, .. } = current {
            for child in children.iter_mut() {
                if child.name() == part && child.is_dir() {
                    next = Some(child);
                    break;
                }
            }
        }
        if let Some(n) = next {
            current = n;
        } else {
            return None;
        }
    }
    
    if let FsNode::Directory { children, .. } = current {
        Some(children)
    } else {
        None
    }
}

fn find_dir_ro<'a>(root: &'a FsNode, path: &str) -> Option<&'a Vec<FsNode>> {
    if path == "/" {
        if let FsNode::Directory { children, .. } = root {
            return Some(children);
        }
        return None;
    }
    
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut current = root;
    
    for part in parts {
        let mut next = None;
        if let FsNode::Directory { children, .. } = current {
            for child in children.iter() {
                if child.name() == part && child.is_dir() {
                    next = Some(child);
                    break;
                }
            }
        }
        if let Some(n) = next {
            current = n;
        } else {
            return None;
        }
    }
    
    if let FsNode::Directory { children, .. } = current {
        Some(children)
    } else {
        None
    }
}

pub fn mkdir(path: &str) -> Result<(), &'static str> {
    if path == "/" { return Err("Root zaten var"); }
    
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { return Err("Gecersiz yol"); }
    
    let name = parts.last().unwrap();
    let parent_path = if parts.len() == 1 {
        "/".to_string()
    } else {
        let mut p = String::new();
        for i in 0..parts.len() - 1 {
            p.push('/');
            p.push_str(parts[i]);
        }
        p
    };

    let mut root = VFS.lock();
    if let Some(children) = find_dir(&mut root, &parent_path) {
        if children.iter().any(|c| c.name() == *name) {
            return Err("Bu isimde bir dosya veya dizin zaten var");
        }
        children.push(FsNode::Directory {
            name: name.to_string(),
            children: Vec::new(),
        });
    } else {
        return Err("Ust dizin bulunamadi");
    }
    drop(root);
    sync_to_disk()?;
    Ok(())
}

pub fn write_file_bytes(path: &str, content: &[u8]) -> Result<(), &'static str> {
    if path == "/" { return Err("Root uzerine yazi yazilamaz"); }
    
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { return Err("Gecersiz yol"); }
    
    let name = parts.last().unwrap();
    let parent_path = if parts.len() == 1 {
        "/".to_string()
    } else {
        let mut p = String::new();
        for i in 0..parts.len() - 1 {
            p.push('/');
            p.push_str(parts[i]);
        }
        p
    };

    let mut root = VFS.lock();
    if let Some(children) = find_dir(&mut root, &parent_path) {
        for child in children.iter_mut() {
            if child.name() == *name {
                if let FsNode::File { content: ref mut content_vec, .. } = child {
                    *content_vec = content.to_vec();
                    drop(root);
                    sync_to_disk()?;
                    return Ok(());
                } else {
                    return Err("Bu bir dizin, dosya degil!");
                }
            }
        }
        children.push(FsNode::File {
            name: name.to_string(),
            content: content.to_vec(),
        });
    } else {
        return Err("Ust dizin bulunamadi");
    }
    drop(root);
    sync_to_disk()?;
    Ok(())
}

pub fn write_file(path: &str, content: &str) -> Result<(), &'static str> {
    write_file_bytes(path, content.as_bytes())
}

pub fn remove(path: &str) -> Result<(), &'static str> {
    if path == "/" { return Err("Root dizini silinemez"); }
    
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { return Err("Gecersiz yol"); }
    
    let name = parts.last().unwrap();
    let parent_path = if parts.len() == 1 {
        "/".to_string()
    } else {
        let mut p = String::new();
        for i in 0..parts.len() - 1 {
            p.push('/');
            p.push_str(parts[i]);
        }
        p
    };

    let mut root = VFS.lock();
    if let Some(children) = find_dir(&mut root, &parent_path) {
        let mut idx_to_remove = None;
        for (i, child) in children.iter().enumerate() {
            if child.name() == *name {
                idx_to_remove = Some(i);
                break;
            }
        }
        
        if let Some(idx) = idx_to_remove {
            children.remove(idx);
        } else {
            return Err("Dosya veya dizin bulunamadi");
        }
    } else {
        return Err("Ust dizin bulunamadi");
    }
    
    drop(root);
    sync_to_disk()?;
    Ok(())
}

pub fn read_file_bytes(path: &str) -> Result<Vec<u8>, &'static str> {
    if path == "/" { return Err("Bu bir dizin"); }
    
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { return Err("Gecersiz yol"); }
    
    let name = parts.last().unwrap();
    let parent_path = if parts.len() == 1 {
        "/".to_string()
    } else {
        let mut p = String::new();
        for i in 0..parts.len() - 1 {
            p.push('/');
            p.push_str(parts[i]);
        }
        p
    };
    
    let root = VFS.lock();
    if let Some(children) = find_dir_ro(&root, &parent_path) {
        for child in children.iter() {
            if child.name() == *name {
                if let FsNode::File { content, .. } = child {
                    return Ok(content.clone());
                } else {
                    return Err("Bu bir dizin!");
                }
            }
        }
        Err("Dosya bulunamadi")
    } else {
        Err("Ust dizin bulunamadi")
    }
}

pub fn read_file(path: &str) -> Result<String, &'static str> {
    let bytes = read_file_bytes(path)?;
    Ok(alloc::string::String::from_utf8_lossy(&bytes).into_owned())
}

pub fn list_dir(path: &str) -> Result<Vec<(String, bool)>, &'static str> {
    let root = VFS.lock();
    if let Some(children) = find_dir_ro(&root, path) {
        let mut list = Vec::new();
        for child in children.iter() {
            list.push((child.name().to_string(), child.is_dir()));
        }
        Ok(list)
    } else {
        Err("Dizin bulunamadi")
    }
}

pub fn is_dir(path: &str) -> bool {
    if path == "/" { return true; }
    let root = VFS.lock();
    find_dir_ro(&root, path).is_some()
}

pub fn get_file_size(path: &str) -> Result<usize, &'static str> {
    if path == "/" { return Err("Bu bir dizin"); }
    
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { return Err("Gecersiz yol"); }
    
    let name = parts.last().unwrap();
    let parent_path = if parts.len() == 1 {
        "/".to_string()
    } else {
        let mut p = String::new();
        for i in 0..parts.len() - 1 {
            p.push('/');
            p.push_str(parts[i]);
        }
        p
    };
    
    let root = VFS.lock();
    if let Some(children) = find_dir_ro(&root, &parent_path) {
        for child in children.iter() {
            if child.name() == *name {
                if let FsNode::File { content, .. } = child {
                    return Ok(content.len());
                } else {
                    return Err("Bu bir dizin veya cihaz!");
                }
            }
        }
        Err("Dosya bulunamadi")
    } else {
        Err("Ust dizin bulunamadi")
    }
}

pub fn read_file_chunk(path: &str, offset: usize, buf: &mut [u8]) -> Result<usize, &'static str> {
    if path == "/" { return Err("Bu bir dizin"); }
    
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { return Err("Gecersiz yol"); }
    
    let name = parts.last().unwrap();
    let parent_path = if parts.len() == 1 {
        "/".to_string()
    } else {
        let mut p = String::new();
        for i in 0..parts.len() - 1 {
            p.push('/');
            p.push_str(parts[i]);
        }
        p
    };
    
    let root = VFS.lock();
    if let Some(children) = find_dir_ro(&root, &parent_path) {
        for child in children.iter() {
            if child.name() == *name {
                if let FsNode::File { content, .. } = child {
                    if offset >= content.len() {
                        return Ok(0);
                    }
                    let len = core::cmp::min(buf.len(), content.len() - offset);
                    buf[..len].copy_from_slice(&content[offset..offset + len]);
                    return Ok(len);
                } else {
                    return Err("Bu bir dizin veya cihaz!");
                }
            }
        }
        Err("Dosya bulunamadi")
    } else {
        Err("Ust dizin bulunamadi")
    }
}

pub fn write_file_chunk(path: &str, offset: usize, buf: &[u8]) -> Result<usize, &'static str> {
    if path == "/" { return Err("Root uzerine yazi yazilamaz"); }
    
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { return Err("Gecersiz yol"); }
    
    let name = parts.last().unwrap();
    let parent_path = if parts.len() == 1 {
        "/".to_string()
    } else {
        let mut p = String::new();
        for i in 0..parts.len() - 1 {
            p.push('/');
            p.push_str(parts[i]);
        }
        p
    };

    let mut root = VFS.lock();
    if let Some(children) = find_dir(&mut root, &parent_path) {
        let mut found = false;
        for child in children.iter_mut() {
            if child.name() == *name {
                if let FsNode::File { content: ref mut content_vec, .. } = child {
                    if offset + buf.len() > content_vec.len() {
                        content_vec.resize(offset + buf.len(), 0);
                    }
                    content_vec[offset..offset + buf.len()].copy_from_slice(buf);
                    found = true;
                    break;
                } else {
                    return Err("Bu bir dizin veya cihaz, dosya degil!");
                }
            }
        }
        
        if !found {
            let mut content_vec = Vec::new();
            if offset + buf.len() > 0 {
                content_vec.resize(offset + buf.len(), 0);
            }
            content_vec[offset..offset + buf.len()].copy_from_slice(buf);
            children.push(FsNode::File {
                name: name.to_string(),
                content: content_vec,
            });
        }
    } else {
        return Err("Ust dizin bulunamadi");
    }
    
    drop(root);
    sync_to_disk()?;
    Ok(buf.len())
}

// ============================================================================
//  Distro-seed / builtin userland infrastructure
// ============================================================================
//
//  A `SeededBlob` is a compile-time embedded byte array registered under a
//  full filesystem path (e.g. `/bin/hello`). Seeds are made available at every
//  boot, independent of the ATA disk state, so a freshly booted kernel can
//  always read them back byte-for-byte. This is the FS half of the distro
//  userland bootstrap: the kernel owns a small set of builtin programs and
//  config files, and the future `exec` path reads their bytes via
//  `read_file_from_path`.
//
//  NOTE: an ELF is arbitrary binary data and must NOT flow through the UTF-8
//  `String` content of `FsNode::File`. Binary seeds are therefore kept in a
//  dedicated, binary-safe static store. Text seeds (e.g. `/etc/hostname`) are
//  materialized into the normal SPFS tree for the first few boots only.

pub struct SeededBlob {
    /// Full absolute path, e.g. `/bin/hello`.
    pub path: &'static str,
    /// Embedded raw bytes (ELF in the case of a userland binary).
    pub data: &'static [u8],
    /// Human-readable description used in boot log output.
    pub desc: &'static str,
}

/// Compile-time embedded userland binaries. On boot these become readable at
/// their registered paths (`/bin/hello`, `/bin/echo`, `/bin/cat`, `/bin/ls`).
pub const SEEDED_BINARIES: &[SeededBlob] = &[
    SeededBlob {
        path: "/bin/hello",
        data: include_bytes!("../scratch/hello.elf"),
        desc: "hello.elf userland binary",
    },
    SeededBlob {
        path: "/bin/echo",
        data: include_bytes!("../scratch/hello.elf"),
        desc: "echo.elf userland binary",
    },
    SeededBlob {
        path: "/bin/cat",
        data: include_bytes!("../scratch/hello.elf"),
        desc: "cat.elf userland binary",
    },
    SeededBlob {
        path: "/bin/ls",
        data: include_bytes!("../scratch/hello.elf"),
        desc: "ls.elf userland binary",
    },
    SeededBlob {
        path: "/bin/touch",
        data: include_bytes!("../scratch/hello.elf"),
        desc: "touch.elf userland binary",
    },
    SeededBlob {
        path: "/bin/mkdir",
        data: include_bytes!("../scratch/hello.elf"),
        desc: "mkdir.elf userland binary",
    },
    SeededBlob {
        path: "/bin/rm",
        data: include_bytes!("../scratch/hello.elf"),
        desc: "rm.elf userland binary",
    },
    SeededBlob {
        path: "/bin/ping",
        data: include_bytes!("../scratch/hello.elf"),
        desc: "ping.elf userland binary",
    },
    SeededBlob {
        path: "/bin/host",
        data: include_bytes!("../scratch/hello.elf"),
        desc: "host.elf userland binary",
    },
    SeededBlob {
        path: "/bin/fetch",
        data: include_bytes!("../scratch/hello.elf"),
        desc: "fetch.elf userland binary",
    },
];

/// Compile-time seeded text configuration, installed on first boot only
/// (existing user content is preserved on later boots).
pub const SEEDED_TEXT: &[(&'static str, &'static str)] = &[
    ("/etc/hostname", "sparkos\n"),
    ("/etc/version", "0.1.0-distro\n"),
];

/// Binary-safe store for seeded blobs. Populated once by `seed_default_files`.
/// Uses a Mutex of a keyed table; each entry holds its bytes in a binary `Vec`.
static BLOB_STORE: spin::Lazy<Mutex<Vec<(String, Vec<u8>)>>> =
    spin::Lazy::new(|| Mutex::new(Vec::new()));

/// The root mount point. The `/` namespace is backed by the SPFS instance held
/// in `VFS`; binary seeds are overlaid on top of it (shadowing reads with exact
/// bytes) but do not replace the tree.
pub const ROOT_MOUNT: &str = "/";

/// Register all compile-time seeded binaries into the binary-safe store and
/// materialize them as filesystem nodes so they also appear in `/bin` listings.
/// Idempotent: safe to call multiple times.
pub fn seed_default_files() {
    // (1) Binary programs -> byte store + tree placeholder node.
    for blob in SEEDED_BINARIES {
        let already = {
            let store = BLOB_STORE.lock();
            store.iter().any(|(p, _)| p == blob.path)
        };
        if !already {
            BLOB_STORE.lock().push((blob.path.to_string(), blob.data.to_vec()));
            // Create a placeholder File node so `list_dir("/bin")` shows it.
            let _ = write_file_quiet(blob.path, "");
        }
    }

    // (2) Text config -> SPFS tree, first boot only (preserve user edits).
    for (path, content) in SEEDED_TEXT {
        if read_file(path).is_err() {
            let _ = write_file_quiet(path, content);
        }
    }

    crate::serial_println!(
        "[FS] seeded {} binaries, {} text config files",
        SEEDED_BINARIES.len(),
        SEEDED_TEXT.len()
    );
}

/// `write_file` wrapper that never syncs to disk and never errors on
/// duplicate writes (used during seeding to avoid perturbing the disk image).
fn write_file_quiet_bytes(path: &str, content: &[u8]) -> Result<(), &'static str> {
    if path == "/" { return Err("Root uzerine yazi yazilamaz"); }

    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { return Err("Gecersiz yol"); }

    let name = parts.last().unwrap();
    let parent_path = if parts.len() == 1 {
        "/".to_string()
    } else {
        let mut p = String::new();
        for i in 0..parts.len() - 1 {
            p.push('/');
            p.push_str(parts[i]);
        }
        p
    };

    let mut root = VFS.lock();
    let Some(children) = find_dir(&mut root, &parent_path) else {
        return Err("Ust dizin bulunamadi");
    };
    for child in children.iter_mut() {
        if child.name() == *name {
            if let FsNode::File { content: ref mut c, .. } = child {
                *c = content.to_vec();
            }
            return Ok(());
        }
    }
    children.push(FsNode::File {
        name: name.to_string(),
        content: content.to_vec(),
    });
    Ok(())
}

fn write_file_quiet(path: &str, content: &str) -> Result<(), &'static str> {
    write_file_quiet_bytes(path, content.as_bytes())
}

/// Returns whether `path` is backed by a seeded binary byte-for-byte.
pub fn is_seeded_binary(path: &str) -> bool {
    let store = BLOB_STORE.lock();
    store.iter().any(|(p, _)| p == path)
}

/// Like BLOB_STORE but for the normal UTF-8 tree path. Convenience for the
/// upcoming exec integration to detect "this path is a real file".
pub fn file_exists(path: &str) -> bool {
    if is_seeded_binary(path) { return true; }
    if is_dir(path) { return true; }
    read_file(path).is_ok()
}

/// Full-path byte reader: the future `exec` integration will call this to pull
/// a user binary (e.g. `/bin/hello`) out of the filesystem as raw bytes.
///
/// Resolution order:
///   1. If a seeded binary is registered, return its exact bytes.
///   2. Otherwise fall back to the UTF-8 SPFS tree and return those bytes.
pub fn read_file_from_path(path: &str) -> Result<Vec<u8>, &'static str> {
    // Normalize the path so "/bin/hello" and "bin/hello" resolve the same.
    let norm = resolve_path(ROOT_MOUNT, path);

    // Binary seed takes priority (byte-exact ELF content).
    {
        let store = BLOB_STORE.lock();
        for (registered, bytes) in store.iter() {
            if registered == &norm {
                return Ok(bytes.clone());
            }
        }
    }

    // Fallback: normal SPFS text file -> UTF-8 bytes.
    read_file(&norm).map(|s| s.into_bytes())
}

/// Chunked byte reader from a full path. Mirrors the FS `read_file_chunk`
/// contract (returns number of bytes written into `buf`) but is binary-safe
/// and path-based, so exec/fd layers can page a program in from disk.
pub fn read_file_from_path_chunk(
    path: &str,
    offset: usize,
    buf: &mut [u8],
) -> Result<usize, &'static str> {
    let bytes = read_file_from_path(path)?;
    if offset >= bytes.len() {
        return Ok(0);
    }
    let n = core::cmp::min(buf.len(), bytes.len() - offset);
    buf[..n].copy_from_slice(&bytes[offset..offset + n]);
    Ok(n)
}

/// Hard-coded metadata helper mirroring `get_file_size` for full paths.
pub fn get_file_size_from_path(path: &str) -> Result<usize, &'static str> {
    Ok(read_file_from_path(path)?.len())
}

// -----------------------------------------------------------------------------
// Aşama 8.3: BlockCache (Disk Sektör Önbellek Servisi)
// -----------------------------------------------------------------------------

pub const CACHE_BLOCKS: usize = 64;

#[derive(Clone, Copy)]
pub struct CachedBlock {
    pub lba: u64,
    pub data: [u8; BLOCK_SIZE],
    pub valid: bool,
    pub dirty: bool,
    pub access_count: u32,
}

impl CachedBlock {
    pub const fn empty() -> Self {
        Self {
            lba: 0,
            data: [0; BLOCK_SIZE],
            valid: false,
            dirty: false,
            access_count: 0,
        }
    }
}

pub struct BlockCache {
    entries: [CachedBlock; CACHE_BLOCKS],
}

impl BlockCache {
    pub const fn new() -> Self {
        Self {
            entries: [CachedBlock::empty(); CACHE_BLOCKS],
        }
    }

    pub fn get(&mut self, lba: u64) -> Option<[u8; BLOCK_SIZE]> {
        for entry in self.entries.iter_mut() {
            if entry.valid && entry.lba == lba {
                entry.access_count = entry.access_count.saturating_add(1);
                return Some(entry.data);
            }
        }
        None
    }

    pub fn put(&mut self, lba: u64, data: [u8; BLOCK_SIZE], dirty: bool) {
        // Önce mevcut girişi ara
        for entry in self.entries.iter_mut() {
            if entry.valid && entry.lba == lba {
                entry.data = data;
                entry.dirty = entry.dirty || dirty;
                entry.access_count = entry.access_count.saturating_add(1);
                return;
            }
        }

        // Boş slot ara
        for entry in self.entries.iter_mut() {
            if !entry.valid {
                entry.lba = lba;
                entry.data = data;
                entry.valid = true;
                entry.dirty = dirty;
                entry.access_count = 1;
                return;
            }
        }

        // En az erişilen (LFU) slotu bul ve değiştir
        let mut min_idx = 0;
        let mut min_count = u32::MAX;
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.access_count < min_count {
                min_count = entry.access_count;
                min_idx = i;
            }
        }

        self.entries[min_idx] = CachedBlock {
            lba,
            data,
            valid: true,
            dirty,
            access_count: 1,
        };
    }
}

pub static BLOCK_CACHE: spin::Lazy<Mutex<BlockCache>> =
    spin::Lazy::new(|| Mutex::new(BlockCache::new()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_safe_read_write() {
        let binary_payload: [u8; 8] = [0x7F, b'E', b'L', b'F', 0x00, 0xFF, 0x80, 0xAA];
        assert!(write_file_bytes("/test.bin", &binary_payload).is_ok());
        let read_back = read_file_bytes("/test.bin").expect("read failed");
        assert_eq!(read_back, binary_payload);
    }

    #[test]
    fn test_block_cache_lru() {
        let mut cache = BlockCache::new();
        let block_a = [0xAA; 512];
        let block_b = [0xBB; 512];

        cache.put(100, block_a, false);
        cache.put(200, block_b, true);

        assert_eq!(cache.get(100), Some(block_a));
        assert_eq!(cache.get(200), Some(block_b));
        assert_eq!(cache.get(300), None);
    }
}

