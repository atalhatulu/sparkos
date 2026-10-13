.intel_syntax noprefix
.global _start
.text
_start:
    mov r13d, 20
again:
    mov eax, 31                 # SYS_CREATE_SURFACE(64, 64)
    mov edi, 64
    mov esi, 64
    int 0x80
    test rax, rax
    js fail
    mov r12, rax
    mov ebx, 0x70000000
    mov dword ptr [rbx], 0x00112233
    mov dword ptr [rbx + 16380], 0x00445566
    cmp dword ptr [rbx], 0x00112233
    jne fail
    cmp dword ptr [rbx + 16380], 0x00445566
    jne fail
    mov eax, 33                 # SYS_DESTROY_SURFACE(id)
    mov rdi, r12
    int 0x80
    test rax, rax
    jne fail
    dec r13d
    jnz again
    xor edi, edi
    jmp exit
fail:
    mov edi, 1
exit:
    mov eax, 1                  # SYS_EXIT(status)
    int 0x80
    ud2
