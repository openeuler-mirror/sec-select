ENTRY(_start)

PHDRS {
    text   PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data   PT_LOAD FLAGS(6);
    bss    PT_LOAD FLAGS(6);
}

SECTIONS {
    . = 0x40080000;

    __text_start = .;
    .text : {
        *(.text._start)
        *(.text .text.*)
    } :text
    __text_end = .;

    . = ALIGN(4096);
    __rodata_start = .;
    .rodata : {
        *(.rodata .rodata.*)
    } :rodata
    __rodata_end = .;

    . = ALIGN(4096);
    __data_start = .;
    .data : {
        *(.data .data.*)
    } :data

    . = ALIGN(4096);
    .bss : {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    } :bss

    . = ALIGN(4096);
    __data_end = .;

    . = ALIGN(4096);
    __page_table_start = .;
    . += 0x80000;
    __page_table_end = .;

    . = ALIGN(4096);
    __heap_start = .;
    . += 0x700000;
    __heap_end = .;

    . = ALIGN(4096);
    __stack_start = .;
    . += 0x200000;
    __stack_end = .;

    /DISCARD/ : {
        *(.comment)
        *(.note.*)
        *(.eh_frame*)
    }
}
