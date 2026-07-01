.section .text._start
.global _start
_start:
    ldr x0, =0x09000000
    mov w1, #'H'
    str w1, [x0]
    mov w1, #'e'
    str w1, [x0]
    mov w1, #'l'
    str w1, [x0]
    mov w1, #'l'
    str w1, [x0]
    mov w1, #'o'
    str w1, [x0]
1:
    wfi
    b 1b
