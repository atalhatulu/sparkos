.intel_syntax noprefix
.global _start

.text
_start:
    # sys_write(stdout=1, buf=msg, len=13)
    mov eax, 4      # syscall: sys_write
    mov edi, 1      # fd: stdout
    lea rsi, [rip + msg]
    mov edx, 13     # length
    int 0x80

    # sys_exit()
    mov eax, 1      # syscall: sys_exit
    xor edi, edi
    int 0x80

.data
msg:
    .ascii "Hello, World!"
