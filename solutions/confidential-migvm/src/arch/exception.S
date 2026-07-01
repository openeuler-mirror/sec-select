.macro SAVE_CONTEXT
    sub sp, sp, #272
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]

    mrs x0, elr_el1
    mrs x1, spsr_el1
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x0, x1, [sp, #248]
    stp x2, x3, [sp, #264]
.endm

.macro RESTORE_CONTEXT
    ldp x0, x1, [sp, #248]
    ldp x2, x3, [sp, #264]
    msr elr_el1, x0
    msr spsr_el1, x1

    ldp x0, x1, [sp, #0]
    ldp x2, x3, [sp, #16]
    ldp x4, x5, [sp, #32]
    ldp x6, x7, [sp, #48]
    ldp x8, x9, [sp, #64]
    ldp x10, x11, [sp, #80]
    ldp x12, x13, [sp, #96]
    ldp x14, x15, [sp, #112]
    ldp x16, x17, [sp, #128]
    ldp x18, x19, [sp, #144]
    ldp x20, x21, [sp, #160]
    ldp x22, x23, [sp, #176]
    ldp x24, x25, [sp, #192]
    ldp x26, x27, [sp, #208]
    ldp x28, x29, [sp, #224]
    ldr x30, [sp, #240]

    add sp, sp, #272
    eret
.endm

.section .text._start
.global _start
.balign 0x40
_start:
    // Linux arm64 kernel Image 64-byte header
    // Enables EDK2 LinuxLoader to chainload this binary via -bios + -kernel
    .word 0x14000010          // 0x00: b _start_body (offset 0x40, unconditional branch)
    .word 0                   // 0x04: padding
    .quad 0                   // 0x08: text_offset = 0 (entry at image start)
    .quad 0                   // 0x10: image_size = 0 (unknown)
    .quad 0                   // 0x18: flags = 0 (LE, 4KB pages, no PE)
    .quad 0                   // 0x20: res2 = 0
    .quad 0                   // 0x28: res3 = 0
    .quad 0                   // 0x30: res4 = 0
    .ascii "ARM\x64"          // 0x38: magic
    .int   0                  // 0x3C: res5 (PE header offset, 0 = none)

_start_body:
    mov x21, x0

    mrs x5, CurrentEL
    and x5, x5, #0xC
    cmp x5, #0x8
    b.eq drop_to_el1
    cmp x5, #0x4
    b.eq el1_init
    b el1_init

drop_to_el1:
    mrs x5, cnthctl_el2
    orr x5, x5, #3
    msr cnthctl_el2, x5
    msr cntvoff_el2, xzr

    mov x5, #0x3C5
    msr spsr_el2, x5
    adr x5, el1_init
    msr elr_el2, x5
    eret

el1_init:
    adrp x5, saved_fdt_pointer
    add x5, x5, #:lo12:saved_fdt_pointer
    str x21, [x5]

    mrs x5, cpacr_el1
    orr x5, x5, #0x300000
    msr cpacr_el1, x5
    isb

    adrp x2, __bss_start
    add x2, x2, #:lo12:__bss_start
    adrp x3, __bss_end
    add x3, x3, #:lo12:__bss_end
clear_bss:
    cmp x2, x3
    b.ge bss_done
    str xzr, [x2], #8
    b clear_bss
bss_done:

    adrp x2, __stack_end
    add x2, x2, #:lo12:__stack_end
    mov sp, x2

    adrp x3, exception_vector_table
    add x3, x3, #:lo12:exception_vector_table
    msr vbar_el1, x3

    bl setup_mmu

    adrp x0, saved_fdt_pointer
    add x0, x0, #:lo12:saved_fdt_pointer
    ldr x0, [x0]
    mov x1, xzr
    bl _start_rust_impl

hang:
    wfi
    b hang

.section .data
.global saved_fdt_pointer
saved_fdt_pointer:
    .quad 0

setup_mmu:
    adrp x0, __page_table_start
    add x0, x0, #:lo12:__page_table_start

    mov x1, #0
    mov x2, #0x200
clear_pt:
    str xzr, [x0, x1, lsl #3]
    add x1, x1, #1
    cmp x1, x2
    b.lt clear_pt

    ldr x3, =0x00000000
    orr x3, x3, #(2 << 2)
    orr x3, x3, #(2 << 8)
    orr x3, x3, #(1 << 10)
    orr x3, x3, #1
    str x3, [x0]

    add x1, x0, #0x1000
    orr x3, x1, #3
    str x3, [x0, #8]

    mov x5, #0
fill_l2_ram:
    lsl x6, x5, #21
    ldr x7, =0x40000000
    add x6, x6, x7
    ldr x7, =0x701
    orr x6, x6, x7
    str x6, [x1, x5, lsl #3]
    add x5, x5, #1
    cmp x5, #512
    b.lt fill_l2_ram

    msr ttbr0_el1, x0

    mov x3, #30
    orr x3, x3, #(1 << 8)
    orr x3, x3, #(1 << 10)
    orr x3, x3, #(3 << 12)
    mov x4, #2
    orr x3, x3, x4, lsl #32
    msr tcr_el1, x3

    ldr x3, =0x000000000004FF44
    msr mair_el1, x3

    mrs x3, sctlr_el1
    ldr x4, =0x1005
    orr x3, x3, x4
    msr sctlr_el1, x3
    isb

    ret

.section .text
exception_vector_table:
    .align 11

    b exc_sync_current_sp0
    .align 7
    b exc_irq_current_sp0
    .align 7
    b exc_fiq_current_sp0
    .align 7
    b exc_serror_current_sp0
    .align 7

    b exc_sync_current_spx
    .align 7
    b exc_irq_current_spx
    .align 7
    b exc_fiq_current_spx
    .align 7
    b exc_serror_current_spx
    .align 7

    b exc_sync_lower_aa64
    .align 7
    b exc_irq_lower_aa64
    .align 7
    b exc_fiq_lower_aa64
    .align 7
    b exc_serror_lower_aa64
    .align 7

    b exc_sync_lower_aa32
    .align 7
    b exc_irq_lower_aa32
    .align 7
    b exc_fiq_lower_aa32
    .align 7
    b exc_serror_lower_aa32
    .align 7

exc_sync_current_sp0:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_sync_exception
    RESTORE_CONTEXT

exc_irq_current_sp0:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_irq_exception
    RESTORE_CONTEXT

exc_fiq_current_sp0:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_fiq_exception
    RESTORE_CONTEXT

exc_serror_current_sp0:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_serror_exception
    RESTORE_CONTEXT

exc_sync_current_spx:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_sync_exception
    RESTORE_CONTEXT

exc_irq_current_spx:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_irq_exception
    RESTORE_CONTEXT

exc_fiq_current_spx:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_fiq_exception
    RESTORE_CONTEXT

exc_serror_current_spx:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_serror_exception
    RESTORE_CONTEXT

exc_sync_lower_aa64:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_sync_exception
    RESTORE_CONTEXT

exc_irq_lower_aa64:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_irq_exception
    RESTORE_CONTEXT

exc_fiq_lower_aa64:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_fiq_exception
    RESTORE_CONTEXT

exc_serror_lower_aa64:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_serror_exception
    RESTORE_CONTEXT

exc_sync_lower_aa32:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_sync_exception
    RESTORE_CONTEXT

exc_irq_lower_aa32:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_irq_exception
    RESTORE_CONTEXT

exc_fiq_lower_aa32:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_fiq_exception
    RESTORE_CONTEXT

exc_serror_lower_aa32:
    SAVE_CONTEXT
    mov x0, sp
    bl handle_serror_exception
    RESTORE_CONTEXT
