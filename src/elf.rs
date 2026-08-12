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

pub struct LoadedSegment {
    pub vaddr: u64,
    pub memsz: u64,
    pub data: Vec<u8>,
}

pub struct ElfFile {
    pub entry_point: u64,
    pub segments: Vec<LoadedSegment>,
}

pub fn parse_elf(bytes: &[u8]) -> Result<ElfFile, &'static str> {
    if bytes.len() < size_of::<Elf64_Ehdr>() {
        return Err("File too small to be ELF");
    }

    let ehdr_ptr = bytes.as_ptr() as *const Elf64_Ehdr;
    let ehdr = unsafe { &*ehdr_ptr };

    // Check Magic Number: 0x7F 'E' 'L' 'F'
    if ehdr.e_ident[0] != 0x7F || ehdr.e_ident[1] != b'E' || ehdr.e_ident[2] != b'L' || ehdr.e_ident[3] != b'F' {
        return Err("Invalid ELF magic");
    }
    
    // Check if 64-bit (Class = 2)
    if ehdr.e_ident[4] != 2 {
        return Err("Not a 64-bit ELF");
    }

    let mut segments = Vec::new();

    let phoff = ehdr.e_phoff as usize;
    let phnum = ehdr.e_phnum as usize;
    let phentsize = ehdr.e_phentsize as usize;

    if phoff + (phnum * phentsize) > bytes.len() {
        return Err("Program headers out of bounds");
    }

    for i in 0..phnum {
        let phdr_ptr = unsafe { bytes.as_ptr().add(phoff + i * phentsize) } as *const Elf64_Phdr;
        let phdr = unsafe { &*phdr_ptr };

        if phdr.p_type == PT_LOAD {
            let offset = phdr.p_offset as usize;
            let filesz = phdr.p_filesz as usize;
            let memsz = phdr.p_memsz as usize;
            let vaddr = phdr.p_vaddr;

            if offset + filesz > bytes.len() {
                return Err("Segment data out of bounds");
            }

            let mut data = Vec::with_capacity(memsz);
            data.extend_from_slice(&bytes[offset..offset + filesz]);
            // Zero pad the rest if memsz > filesz (e.g. .bss)
            if memsz > filesz {
                data.resize(memsz, 0);
            }

            segments.push(LoadedSegment {
                vaddr,
                memsz: memsz as u64,
                data,
            });
        }
    }

    Ok(ElfFile {
        entry_point: ehdr.e_entry,
        segments,
    })
}
