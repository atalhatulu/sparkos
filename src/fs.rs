use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;
use spin::Mutex;
use core::convert::TryInto;

#[derive(Clone)]
pub enum FsNode {
    File {
        name: String,
        content: String,
    },
    Directory {
        name: String,
        children: Vec<FsNode>,
    }
}

impl FsNode {
    pub fn name(&self) -> &str {
        match self {
            FsNode::File { name, .. } => name,
            FsNode::Directory { name, .. } => name,
        }
    }
    
    pub fn is_dir(&self) -> bool {
        matches!(self, FsNode::Directory { .. })
    }
}

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
            let content_bytes = content.as_bytes();
            buf.extend_from_slice(&(content_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(content_bytes);
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
        let content = core::str::from_utf8(&data[*offset..*offset+content_len]).ok()?.to_string();
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
    } else {
        None
    }
}

pub fn sync_to_disk() {
    let mut buf = Vec::new();
    {
        let root = VFS.lock();
        serialize(&*root, &mut buf);
    }
    
    let size = buf.len() as u32;
    let mut header = [0u8; 512];
    header[0..4].copy_from_slice(&size.to_le_bytes());
    header[4..8].copy_from_slice(b"SPFS"); // SPark File System
    
    let mut drive = crate::ata::DATA_DRIVE.lock();
    let _ = drive.write_sector(0, &header);
    
    let mut lba = 1;
    for chunk in buf.chunks(512) {
        let mut sec = [0u8; 512];
        sec[..chunk.len()].copy_from_slice(chunk);
        let _ = drive.write_sector(lba, &sec);
        lba += 1;
    }
}

pub fn load_from_disk() {
    let mut drive = crate::ata::DATA_DRIVE.lock();
    let mut header = [0u8; 512];
    if drive.read_sector(0, &mut header).is_err() { return; }
    
    if &header[4..8] != b"SPFS" { return; } // Bos veya formati farkli
    
    let size = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
    if size == 0 || size > 10 * 1024 * 1024 { return; } // Güvenlik siniri
    
    let num_sectors = (size + 511) / 512;
    let mut data = alloc::vec![0u8; num_sectors * 512];
    
    for i in 0..num_sectors {
        let mut sec = [0u8; 512];
        if drive.read_sector(1 + i as u32, &mut sec).is_ok() {
            data[i * 512 .. (i+1) * 512].copy_from_slice(&sec);
        }
    }
    
    let mut offset = 0;
    if let Some(node) = deserialize(&data[..size], &mut offset) {
        *VFS.lock() = node;
    }
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
    sync_to_disk();
    Ok(())
}

pub fn write_file(path: &str, content: &str) -> Result<(), &'static str> {
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
                if let FsNode::File { content: ref mut content_str, .. } = child {
                    *content_str = content.to_string();
                    drop(root);
                    sync_to_disk();
                    return Ok(());
                } else {
                    return Err("Bu bir dizin, dosya degil!");
                }
            }
        }
        children.push(FsNode::File {
            name: name.to_string(),
            content: content.to_string(),
        });
    } else {
        return Err("Ust dizin bulunamadi");
    }
    sync_to_disk();
    Ok(())
}

pub fn read_file(path: &str) -> Result<String, &'static str> {
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
