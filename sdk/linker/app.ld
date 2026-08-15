/*
 * SparkOS Standard Userspace Linker Script
 * Target: x86_64 ELF Executable
 * Base Address: 0x400000 (4 MB)
 */

ENTRY(_start)

SECTIONS
{
    . = 0x400000;

    .text : ALIGN(4K) {
        *(.text._start)
        *(.text .text.*)
    }

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K) {
        *(.data .data.*)
    }

    .bss : ALIGN(4K) {
        *(.bss .bss.*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.comment)
        *(.eh_frame)
        *(.note.*)
    }
}
