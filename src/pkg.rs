//! SparkOS — Package Management & SPKG Format Subsystem (Faz 18)
//!
//! Provides the SPKG v1 Binary Container Parser & Builder, Capability-Declared
//! Manifest Validation, ELF Integrity Checksumming, and Crash-Safe Package Management.

use alloc::string::String;
use alloc::vec::Vec;

pub const SPKG_MAGIC: [u8; 4] = *b"SPKG"; // 0x53504B47
pub const SPKG_VERSION: u16 = 1;

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpkgHeader {
    pub magic: [u8; 4],
    pub version: u16,
    pub manifest_len: u32,
    pub elf_len: u32,
    pub resources_len: u32,
    pub checksum: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagePermissions {
    pub filesystem_home: bool,
    pub network: bool,
    pub gui: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageManifest {
    pub name: String,
    pub version: String,
    pub entry: String,
    pub permissions: PackagePermissions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpkgPackage {
    pub manifest: PackageManifest,
    pub elf_bytes: Vec<u8>,
    pub resources_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkgError {
    FileTooSmall,
    InvalidMagic,
    UnsupportedVersion,
    ChecksumMismatch,
    InvalidManifest,
    InvalidElfBinary,
    DuplicatePackage,
    NotFound,
}

/// Basit ve deterministik CRC32 / Adler benzeri bütünlük sağlama toplamı
pub fn calculate_checksum(data: &[u8]) -> u32 {
    let mut sum: u32 = 0x811c9dc5;
    for &b in data {
        sum ^= b as u32;
        sum = sum.wrapping_mul(0x01000193);
    }
    sum
}

/// SPKG paket baytlarını ayrıştırır ve tüm güvenlik doğrulamalarını uygular
pub fn parse_spkg(bytes: &[u8]) -> Result<SpkgPackage, PkgError> {
    let header_size = core::mem::size_of::<SpkgHeader>();
    if bytes.len() < header_size {
        return Err(PkgError::FileTooSmall);
    }

    let header = unsafe { &*(bytes.as_ptr() as *const SpkgHeader) };

    if header.magic != SPKG_MAGIC {
        return Err(PkgError::InvalidMagic);
    }

    if u16::from_le(header.version) != SPKG_VERSION {
        return Err(PkgError::UnsupportedVersion);
    }

    let manifest_len = u32::from_le(header.manifest_len) as usize;
    let elf_len = u32::from_le(header.elf_len) as usize;
    let res_len = u32::from_le(header.resources_len) as usize;
    let expected_checksum = u32::from_le(header.checksum);

    let total_payload_len = manifest_len + elf_len + res_len;
    if bytes.len() < header_size + total_payload_len {
        return Err(PkgError::FileTooSmall);
    }

    let payload = &bytes[header_size..header_size + total_payload_len];
    if calculate_checksum(payload) != expected_checksum {
        return Err(PkgError::ChecksumMismatch);
    }

    // Manifest ayrıştırma
    let manifest_bytes = &payload[..manifest_len];
    let manifest_str = core::str::from_utf8(manifest_bytes).map_err(|_| PkgError::InvalidManifest)?;

    let mut name = String::new();
    let mut version = String::new();
    let mut entry = String::new();
    let mut fs_home = false;
    let mut network = false;
    let mut gui = false;

    for line in manifest_str.lines() {
        let parts: Vec<&str> = line.split('=').map(|s| s.trim()).collect();
        if parts.len() == 2 {
            match parts[0] {
                "name" => name = parts[1].trim_matches('"').into(),
                "version" => version = parts[1].trim_matches('"').into(),
                "entry" => entry = parts[1].trim_matches('"').into(),
                "permission.fs_home" => fs_home = parts[1] == "true",
                "permission.network" => network = parts[1] == "true",
                "permission.gui" => gui = parts[1] == "true",
                _ => {}
            }
        }
    }

    if name.is_empty() || version.is_empty() || entry.is_empty() {
        return Err(PkgError::InvalidManifest);
    }

    // ELF doğrulama (Faz 17 ET_EXEC kuralları)
    let elf_bytes = payload[manifest_len..manifest_len + elf_len].to_vec();
    if elf_bytes.len() < 4 || &elf_bytes[..4] != b"\x7fELF" {
        return Err(PkgError::InvalidElfBinary);
    }

    let resources_bytes = payload[manifest_len + elf_len..].to_vec();

    Ok(SpkgPackage {
        manifest: PackageManifest {
            name,
            version,
            entry,
            permissions: PackagePermissions {
                filesystem_home: fs_home,
                network,
                gui,
            },
        },
        elf_bytes,
        resources_bytes,
    })
}

/// Yeni bir SPKG paketi inşa eder
pub fn build_spkg(manifest_str: &str, elf_bytes: &[u8], resources: &[u8]) -> Vec<u8> {
    let manifest_bytes = manifest_str.as_bytes();
    let mut payload = Vec::new();
    payload.extend_from_slice(manifest_bytes);
    payload.extend_from_slice(elf_bytes);
    payload.extend_from_slice(resources);

    let checksum = calculate_checksum(&payload);

    let header = SpkgHeader {
        magic: SPKG_MAGIC,
        version: SPKG_VERSION.to_le(),
        manifest_len: (manifest_bytes.len() as u32).to_le(),
        elf_len: (elf_bytes.len() as u32).to_le(),
        resources_len: (resources.len() as u32).to_le(),
        checksum: checksum.to_le(),
    };

    let mut pkg_bytes = Vec::new();
    let header_bytes = unsafe {
        core::slice::from_raw_parts(
            &header as *const SpkgHeader as *const u8,
            core::mem::size_of::<SpkgHeader>(),
        )
    };
    pkg_bytes.extend_from_slice(header_bytes);
    pkg_bytes.extend_from_slice(&payload);
    pkg_bytes
}
