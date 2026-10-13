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

// The kernel itself occupies low memory. Keep ELF images in the dedicated
// user-code region and below surface slots and the user stack.
pub const USER_ADDR_MIN: u64 = 0x4000_0000;
pub const USER_ADDR_MAX: u64 = 0x7000_0000;
pub const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PROGRAM_HEADERS: usize = 128;

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
    ImageTooLarge,
    OutOfMemory,
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

    if phnum > MAX_PROGRAM_HEADERS {
        return Err(ElfError::ImageTooLarge);
    }

    if phentsize < size_of::<Elf64_Phdr>() {
        return Err(ElfError::HeadersOutOfBounds);
    }

    let headers_end = phoff.checked_add(phnum.checked_mul(phentsize).ok_or(ElfError::HeadersOutOfBounds)?)
        .ok_or(ElfError::HeadersOutOfBounds)?;

    if headers_end > bytes.len() {
        return Err(ElfError::HeadersOutOfBounds);
    }

    let mut segments = Vec::new();
    let mut ranges = Vec::new();
    segments.try_reserve_exact(phnum).map_err(|_| ElfError::OutOfMemory)?;
    ranges.try_reserve_exact(phnum).map_err(|_| ElfError::OutOfMemory)?;
    let mut image_bytes = 0usize;

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

            if memsz == 0 { continue; }
            let mapped_bytes = memsz.checked_add((vaddr & 4095) as usize)
                .and_then(|n| n.checked_add(4095)).map(|n| n & !4095)
                .ok_or(ElfError::ImageTooLarge)?;
            image_bytes = image_bytes.checked_add(mapped_bytes).ok_or(ElfError::ImageTooLarge)?;
            if image_bytes > MAX_IMAGE_BYTES { return Err(ElfError::ImageTooLarge); }
            ranges.push((offset, filesz, memsz));

            segments.push(LoadedSegment {
                vaddr,
                filesz: filesz as u64,
                memsz: memsz as u64,
                flags,
                data: Vec::new(),
            });
        }
    }

    if segments.is_empty() {
        return Err(ElfError::NoLoadableSegments);
    }

    // Çakışan segment kontrolü
    for i in 0..segments.len() {
        for j in (i + 1)..segments.len() {
            let a_start = segments[i].vaddr & !4095;
            let a_end = (segments[i].vaddr + segments[i].memsz + 4095) & !4095;
            let b_start = segments[j].vaddr & !4095;
            let b_end = (segments[j].vaddr + segments[j].memsz + 4095) & !4095;

            if a_start < b_end && b_start < a_end {
                return Err(ElfError::OverlappingSegments);
            }
        }
    }

    // Entry point geçerlilik kontrolü: entry point en az bir PT_LOAD segmenti sınırlarında olmalı
    let entry = ehdr.e_entry;
    let entry_valid = segments.iter().any(|seg| {
        seg.flags & PF_X != 0 && entry >= seg.vaddr && entry < (seg.vaddr + seg.memsz)
    });

    if !entry_valid {
        return Err(ElfError::InvalidEntryPoint);
    }

    // Allocate payloads only after every header and the aggregate budget pass.
    // A tiny malformed file with a huge BSS must return an error, not panic
    // in the global allocator and take down the kernel.
    for (seg, (offset, filesz, memsz)) in segments.iter_mut().zip(ranges) {
        seg.data.try_reserve_exact(memsz).map_err(|_| ElfError::OutOfMemory)?;
        seg.data.extend_from_slice(&bytes[offset..offset + filesz]);
        seg.data.resize(memsz, 0);
    }

    Ok(ElfFile {
        entry_point: entry,
        segments,
    })
}

#[cfg(test)]
mod regression_tests {
    use super::*;

    fn image(segments: &[(u64, u64, u32)], entry: u64) -> Vec<u8> {
        let mut bytes = alloc::vec![0u8; 64 + 56 * segments.len()];
        bytes[..7].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1]);
        bytes[16..18].copy_from_slice(&2u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&62u16.to_le_bytes());
        bytes[24..32].copy_from_slice(&entry.to_le_bytes());
        bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
        bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
        bytes[56..58].copy_from_slice(&(segments.len() as u16).to_le_bytes());
        for (i, &(addr, size, flags)) in segments.iter().enumerate() {
            let ph = &mut bytes[64 + i * 56..64 + (i + 1) * 56];
            ph[..4].copy_from_slice(&PT_LOAD.to_le_bytes());
            ph[4..8].copy_from_slice(&flags.to_le_bytes());
            ph[16..24].copy_from_slice(&addr.to_le_bytes());
            ph[40..48].copy_from_slice(&size.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn oversized_bss_returns_error_without_allocating_payload() {
        let bytes = image(&[(USER_ADDR_MIN, MAX_IMAGE_BYTES as u64 + 1, PF_R | PF_X)], USER_ADDR_MIN);
        assert_eq!(parse_elf(&bytes), Err(ElfError::ImageTooLarge));
    }

    #[test]
    fn aggregate_segment_budget_is_enforced() {
        let bytes = image(&[(USER_ADDR_MIN, 9 * 1024 * 1024, PF_X),
            (USER_ADDR_MIN + 0x2000000, 9 * 1024 * 1024, PF_R)], USER_ADDR_MIN);
        assert_eq!(parse_elf(&bytes), Err(ElfError::ImageTooLarge));
    }

    #[test]
    fn excessive_program_headers_are_rejected() {
        let mut bytes = image(&[], USER_ADDR_MIN);
        bytes[56..58].copy_from_slice(&((MAX_PROGRAM_HEADERS + 1) as u16).to_le_bytes());
        assert_eq!(parse_elf(&bytes), Err(ElfError::ImageTooLarge));
    }

    #[test]
    fn entry_must_be_executable() {
        let bytes = image(&[(USER_ADDR_MIN, 64, PF_R | PF_W)], USER_ADDR_MIN);
        assert_eq!(parse_elf(&bytes), Err(ElfError::InvalidEntryPoint));
    }

    #[test]
    fn kernel_surface_and_stack_addresses_are_rejected() {
        for addr in [0x400000, 0x70000000, 0x7fff0000, u64::MAX - 4] {
            let bytes = image(&[(addr, 64, PF_X)], addr);
            assert_eq!(parse_elf(&bytes), Err(ElfError::KernelAddressViolation));
        }
    }

    #[test]
    fn valid_bss_is_zeroed() {
        let bytes = image(&[(USER_ADDR_MIN, 4096, PF_X)], USER_ADDR_MIN);
        let elf = parse_elf(&bytes).unwrap();
        assert_eq!(elf.segments[0].data, alloc::vec![0; 4096]);
    }

    #[test]
    fn separate_segments_must_not_alias_the_same_page() {
        let bytes = image(&[(USER_ADDR_MIN, 32, PF_X), (USER_ADDR_MIN + 128, 32, PF_W)], USER_ADDR_MIN);
        assert_eq!(parse_elf(&bytes), Err(ElfError::OverlappingSegments));
    }
}
