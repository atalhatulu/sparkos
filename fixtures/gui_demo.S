.intel_syntax noprefix
.global _start

.text
_start:
    # 1. SYS_CREATE_SURFACE(width=128, height=64) -> id in RAX
    mov eax, 31             # SYS_CREATE_SURFACE
    mov edi, 128            # width
    mov esi, 64             # height
    int 0x80
    test rax, rax
    js fail
    mov r12, rax            # r12 = surface_id

    # 2. Render into the mapped surface backing at 0x70000000
    # Fill pixels with test pattern 0x002563EB (Royal Blue)
    mov rbx, 0x70000000     # VMA base for primary user surface
    mov ecx, 2048           # 128 * 64 / 4 dwords (assuming 128x64 surface)
    mov edx, 0x002563EB

fill_loop:
    mov [rbx], edx
    add rbx, 4
    dec ecx
    jnz fill_loop

    # 3. SYS_PRESENT_SURFACE(surface_id, packed_rect)
    # packed_rect: (x << 48) | (y << 32) | (w << 16) | h -> (0 << 48) | (0 << 32) | (128 << 16) | 64
    mov eax, 32             # SYS_PRESENT_SURFACE
    mov rdi, r12            # surface_id
    mov rsi, 0x00800040     # w=128 (0x0080), h=64 (0x0040)
    int 0x80

    # 4. Cooperative yield to scheduler: SYS_YIELD()
    mov eax, 9              # SYS_YIELD
    int 0x80

    # 5. SYS_DESTROY_SURFACE(surface_id)
    mov eax, 33             # SYS_DESTROY_SURFACE
    mov rdi, r12            # surface_id
    int 0x80
    test rax, rax
    jnz fail

    # 6. Clean exit: SYS_EXIT(0)
    xor edi, edi
    jmp exit

fail:
    mov edi, 1

exit:
    mov eax, 1              # SYS_EXIT
    int 0x80
    ud2
