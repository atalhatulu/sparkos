use alloc::vec::Vec;
use core::mem::size_of;

#[repr(C, packed)]
pub struct Elf64_Ehdr {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

#[repr(C, packed)]
pub struct Elf64_Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

pub const PT_LOAD: u32 = 1;

pub const PF_X: u32 = 1; // Executable
pub const PF_W: u32 = 2; // Writable
pub const PF_R: u32 = 4; // Readable

pub const USER_ADDR_MIN: u64 = 0x0040_0000;      // 4 MB
pub const USER_ADDR_MAX: u64 = 0x0000_7FFF_FFFF_0000; // Kullanıcı alanı tavanı (Ring 3)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    FileTooSmall,
    InvalidMagic,
    Not64Bit,
    NotLittleEndian,
    NotExecutable,
    InvalidMachine,
    HeadersOutOfBounds,
    SegmentOutOfBounds,
    InvalidSegmentBounds,
    KernelAddressViolation,
    OverlappingSegments,
    InvalidEntryPoint,
    NoLoadableSegments,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSegment {
    pub vaddr: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub flags: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfFile {
    pub entry_point: u64,
    pub segments: Vec<LoadedSegment>,
}

pub fn parse_elf(bytes: &[u8]) -> Result<ElfFile, ElfError> {
    if bytes.len() < size_of::<Elf64_Ehdr>() {
        return Err(ElfError::FileTooSmall);
    }

    let ehdr_ptr = bytes.as_ptr() as *const Elf64_Ehdr;
    let ehdr = unsafe { &*ehdr_ptr };

    // 1. Magic: 0x7F 'E' 'L' 'F'
    if ehdr.e_ident[0] != 0x7F || ehdr.e_ident[1] != b'E' || ehdr.e_ident[2] != b'L' || ehdr.e_ident[3] != b'F' {
        return Err(ElfError::InvalidMagic);
    }

    // 2. 64-bit Class (2)
    if ehdr.e_ident[4] != 2 {
        return Err(ElfError::Not64Bit);
    }

    // 3. Little Endian (1)
    if ehdr.e_ident[5] != 1 {
        return Err(ElfError::NotLittleEndian);
    }

    // 4. Strictly Executable (e_type == 2 / ET_EXEC) only for Faz 17
    if ehdr.e_type != 2 {
        return Err(ElfError::NotExecutable);
    }

    // 5. Machine: x86-64 (0x3E == 62)
    if ehdr.e_machine != 62 {
        return Err(ElfError::InvalidMachine);
    }

    let phoff = ehdr.e_phoff as usize;
    let phnum = ehdr.e_phnum as usize;
    let phentsize = ehdr.e_phentsize as usize;

    if phentsize < size_of::<Elf64_Phdr>() {
        return Err(ElfError::HeadersOutOfBounds);
    }

    let headers_end = phoff.checked_add(phnum.checked_mul(phentsize).ok_or(ElfError::HeadersOutOfBounds)?)
        .ok_or(ElfError::HeadersOutOfBounds)?;

    if headers_end > bytes.len() {
        return Err(ElfError::HeadersOutOfBounds);
    }

    let mut segments = Vec::new();

    for i in 0..phnum {
        let phdr_ptr = unsafe { bytes.as_ptr().add(phoff + i * phentsize) } as *const Elf64_Phdr;
        let phdr = unsafe { &*phdr_ptr };

        if phdr.p_type == PT_LOAD {
            let offset = phdr.p_offset as usize;
            let filesz = phdr.p_filesz as usize;
            let memsz = phdr.p_memsz as usize;
            let vaddr = phdr.p_vaddr;
            let flags = phdr.p_flags;

            // memsz >= filesz invariant
            if memsz < filesz {
                return Err(ElfError::InvalidSegmentBounds);
            }

            // Dosya sınır kontrolü
            if offset.checked_add(filesz).ok_or(ElfError::SegmentOutOfBounds)? > bytes.len() {
                return Err(ElfError::SegmentOutOfBounds);
            }

            // Kernel alanı koruması (vaddr ve vaddr + memsz kullanıcı alanında olmalı)
            let vaddr_end = vaddr.checked_add(memsz as u64).ok_or(ElfError::KernelAddressViolation)?;
            if vaddr < USER_ADDR_MIN || vaddr_end > USER_ADDR_MAX {
                return Err(ElfError::KernelAddressViolation);
            }

            let mut data = Vec::with_capacity(memsz);
            data.extend_from_slice(&bytes[offset..offset + filesz]);
            // BSS Sıfırlama: memsz > filesz ise kalan baytları sıfırla doldur
            if memsz > filesz {
                data.resize(memsz, 0);
            }

            segments.push(LoadedSegment {
                vaddr,
                filesz: filesz as u64,
                memsz: memsz as u64,
                flags,
                data,
            });
        }
    }

    if segments.is_empty() {
        return Err(ElfError::NoLoadableSegments);
    }

    // Çakışan segment kontrolü
    for i in 0..segments.len() {
        for j in (i + 1)..segments.len() {
            let a_start = segments[i].vaddr;
            let a_end = a_start + segments[i].memsz;
            let b_start = segments[j].vaddr;
            let b_end = b_start + segments[j].memsz;

            if a_start < b_end && b_start < a_end {
                return Err(ElfError::OverlappingSegments);
            }
        }
    }

    // Entry point geçerlilik kontrolü: entry point en az bir PT_LOAD segmenti sınırlarında olmalı
    let entry = ehdr.e_entry;
    let entry_valid = segments.iter().any(|seg| {
        entry >= seg.vaddr && entry < (seg.vaddr + seg.memsz)
    });

    if !entry_valid {
        return Err(ElfError::InvalidEntryPoint);
    }

    Ok(ElfFile {
        entry_point: entry,
        segments,
    })
}
