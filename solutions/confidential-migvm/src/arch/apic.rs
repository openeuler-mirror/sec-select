/*
 * Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
 * Global Trust Authority is licensed under the Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL v2.
 * You may obtain a copy of Mulan PSL v2 at:
 *     http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY KIND, EITHER EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR
 * PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use core::arch::asm;
use core::sync::atomic::{AtomicU32, Ordering};

static mut GICD_BASE: usize = 0x0800_0000;
static mut GICR_BASE: usize = 0x080A_0000;

const GICD_CTLR: usize = 0x0000;
const GICD_TYPER: usize = 0x0004;
const GICD_IGROUPR: usize = 0x0080;
const GICD_ISENABLER: usize = 0x0100;
const GICD_ICENABLER: usize = 0x0180;
const GICD_ICACTIVER: usize = 0x0380;
const GICD_IPRIORITYR: usize = 0x0400;
const GICD_ICFGR: usize = 0x0C00;
const GICD_IROUTER: usize = 0x6000;

const GICR_SGI_OFFSET: usize = 0x10000;

const GICR_CTLR_OFF: usize = 0x0000;
const GICR_TYPER_OFF: usize = 0x0008;
const GICR_WAKER_OFF: usize = 0x0014;
const GICR_IGROUPR0_OFF: usize = GICR_SGI_OFFSET + 0x0080;
const GICR_ISENABLER0_OFF: usize = GICR_SGI_OFFSET + 0x0100;
const GICR_ICENABLER0_OFF: usize = GICR_SGI_OFFSET + 0x0180;
const GICR_ICACTIVER0_OFF: usize = GICR_SGI_OFFSET + 0x0380;
const GICR_IPRIORITYR0_OFF: usize = GICR_SGI_OFFSET + 0x0400;

const GICD_CTLR_RWP: u32 = 1u32 << 31;
const GICD_CTLR_ARE_NS: u32 = 1u32 << 5;
const GICD_CTLR_ARE_S: u32 = 1u32 << 4;
const GICD_CTLR_ENABLE_G1A: u32 = 1u32 << 1;
const GICD_CTLR_ENABLE_G1: u32 = 1u32 << 0;

const GICR_CTLR_RWP: u32 = 1u32 << 3;

const GICR_WAKER_PROCESSOR_SLEEP: u32 = 1u32 << 1;
const GICR_WAKER_CHILDREN_ASLEEP: u32 = 1u32 << 2;

const GICR_TYPER_LAST: u64 = 1u64 << 4;
const GICR_TYPER_AFFINITY_MASK: u64 = 0xFFFF_FFFF_0000_0000;

const GICD_TYPER_LPIS: u32 = 1u32 << 17;

const GICD_INT_DEF_PRI_X4: u32 = 0xA0A0_A0A0;
const ICC_PMR_DEF_PRIO: u64 = 0xF0;
const ICC_SRE_EL1_SRE: u64 = 1u64 << 0;
const ICC_IGRPEN1_EL1_ENABLE: u64 = 1u64 << 0;
const ICC_CTLR_EL1_EOIMODE_DROP: u64 = 1u64 << 1;

const GICV3_REDIST_SIZE: usize = 0x20000;

const INTID_SPURIOUS: u32 = 1023;

static mut GIC_VERSION: u32 = 0;
static mut GICR_STRIDE: usize = GICV3_REDIST_SIZE;
static mut IS_VGIC: bool = false;

static PPI_NEXT: AtomicU32 = AtomicU32::new(16);

#[repr(C)]
pub struct InterruptStack {
    pub elr: u64,
    pub spsr: u64,
    pub esr: u64,
    pub far: u64,
    pub regs: [u64; 31],
}

unsafe fn mmio_read32(base: usize, offset: usize) -> u32 {
    core::ptr::read_volatile((base + offset) as *const u32)
}

unsafe fn mmio_write32(base: usize, offset: usize, val: u32) {
    core::ptr::write_volatile((base + offset) as *mut u32, val);
}

unsafe fn gicd_read(offset: usize) -> u32 {
    mmio_read32(GICD_BASE, offset)
}

unsafe fn gicd_write(offset: usize, val: u32) {
    mmio_write32(GICD_BASE, offset, val);
}

unsafe fn gicr_read(offset: usize) -> u32 {
    mmio_read32(GICR_BASE, offset)
}

unsafe fn gicr_write(offset: usize, val: u32) {
    mmio_write32(GICR_BASE, offset, val);
}

fn detect_gic_version() -> u32 {
    unsafe {
        let typer = gicd_read(GICD_TYPER);
        crate::uart_puts("[GIC] GICD_TYPER=0x");
        crate::uart_put_hex(typer as u64);
        crate::uart_puts("\n");

        if typer & GICD_TYPER_LPIS != 0 {
            3
        } else {
            2
        }
    }
}

unsafe fn read_midr_el1() -> u64 {
    let val: u64;
    asm!("mrs {}, MIDR_EL1", out(reg) val);
    val
}

fn detect_vgic() -> bool {
    unsafe {
        let midr = read_midr_el1();
        crate::uart_puts("[GIC] MIDR_EL1=0x");
        crate::uart_put_hex(midr);
        crate::uart_puts("\n");
        let implementer = (midr >> 24) & 0xFF;
        let partnum = (midr >> 4) & 0xFFF;
        if implementer == 0x41 && partnum == 0xD08 {
            crate::uart_puts("[GIC] Cortex-A53 physical CPU detected, no VGIC\n");
            false
        } else if implementer == 0x41 && (partnum == 0xD07 || partnum == 0xD09 || partnum == 0xD0A || partnum == 0xD0B || partnum == 0xD0C || partnum == 0xD0D || partnum == 0xD0E || partnum == 0xD4A || partnum == 0xD4B || partnum == 0xD4C || partnum == 0xD4D) {
            crate::uart_puts("[GIC] Physical ARM CPU detected, no VGIC\n");
            false
        } else {
            crate::uart_puts("[GIC] Unknown/virtual CPU, assuming VGIC present\n");
            true
        }
    }
}

pub fn is_vgic() -> bool {
    unsafe { IS_VGIC }
}

unsafe fn find_this_cpu_redistributor() -> bool {
    let mpidr = read_mpidr_el1();
    let aff0 = (mpidr & 0xFF) as u64;
    let aff1 = ((mpidr >> 8) & 0xFF) as u64;
    let aff2 = ((mpidr >> 16) & 0xFF) as u64;
    let aff3 = ((mpidr >> 32) & 0xFF) as u64;
    let affinity = (aff3 << 56) | (aff2 << 48) | (aff1 << 40) | (aff0 << 32);

    crate::uart_puts("[GICv3] MPIDR=0x");
    crate::uart_put_hex(mpidr);
    crate::uart_puts(" affinity=0x");
    crate::uart_put_hex(affinity);
    crate::uart_puts("\n");

    let redist_base = GICR_BASE;
    let mut ptr = redist_base;
    let mut cpu_idx = 0usize;

    loop {
        let typer = mmio_read64(ptr, GICR_TYPER_OFF);

        crate::uart_puts("[GICv3] GICR[");
        crate::uart_put_hex(cpu_idx as u64);
        crate::uart_puts("] TYPER=0x");
        crate::uart_put_hex(typer);
        crate::uart_puts("\n");

        let typer_affinity = typer & GICR_TYPER_AFFINITY_MASK;
        if typer_affinity == affinity {
            GICR_BASE = ptr;
            crate::uart_puts("[GICv3] Found our Redistributor at 0x");
            crate::uart_put_hex(ptr as u64);
            crate::uart_puts("\n");
            return true;
        }

        if typer & GICR_TYPER_LAST != 0 {
            break;
        }

        ptr += GICR_STRIDE;
        cpu_idx += 1;

        if cpu_idx > 512 {
            break;
        }
    }

    crate::uart_puts("[GICv3] WARNING: Redistributor not found for this CPU, using default\n");
    false
}

unsafe fn mmio_read64(base: usize, offset: usize) -> u64 {
    core::ptr::read_volatile((base + offset) as *const u64)
}

unsafe fn icc_sre_el1_read() -> u64 {
    let val: u64;
    asm!("mrs {}, ICC_SRE_EL1", out(reg) val);
    val
}

unsafe fn icc_sre_el1_write(val: u64) {
    asm!("msr ICC_SRE_EL1, {}", in(reg) val);
    asm!("isb");
}

unsafe fn icc_pmr_el1_write(val: u64) {
    asm!("msr ICC_PMR_EL1, {}", in(reg) val);
}

unsafe fn icc_igrpen1_el1_write(val: u64) {
    asm!("msr ICC_IGRPEN1_EL1, {}", in(reg) val);
}

unsafe fn icc_igrpen0_el1_write(val: u64) {
    asm!("msr ICC_IGRPEN0_EL1, {}", in(reg) val);
}

unsafe fn icc_igrpen1_el1_read() -> u64 {
    let val: u64;
    asm!("mrs {}, ICC_IGRPEN1_EL1", out(reg) val);
    val
}

unsafe fn icc_ctlr_el1_read() -> u64 {
    let val: u64;
    asm!("mrs {}, ICC_CTLR_EL1", out(reg) val);
    val
}

unsafe fn icc_ctlr_el1_write(val: u64) {
    asm!("msr ICC_CTLR_EL1, {}", in(reg) val);
    asm!("isb");
}

unsafe fn icc_bpr1_el1_write(val: u64) {
    asm!("msr ICC_BPR1_EL1, {}", in(reg) val);
    asm!("isb");
}

unsafe fn icc_iar1_el1_read() -> u64 {
    let val: u64;
    asm!("mrs {}, ICC_IAR1_EL1", out(reg) val);
    val
}

unsafe fn icc_eoir1_el1_write(val: u64) {
    asm!("msr ICC_EOIR1_EL1, {}", in(reg) val);
}

unsafe fn icc_dir_el1_write(val: u64) {
    asm!("msr ICC_DIR_EL1, {}", in(reg) val);
    asm!("isb");
}

unsafe fn read_mpidr_el1() -> u64 {
    let val: u64;
    asm!("mrs {}, MPIDR_EL1", out(reg) val);
    val
}

unsafe fn gicd_wait_for_rwp() {
    let mut count = 100000u32;
    while gicd_read(GICD_CTLR) & GICD_CTLR_RWP != 0 {
        count -= 1;
        if count == 0 {
            crate::uart_puts("[GICv3] WARNING: GICD RWP timeout\n");
            break;
        }
    }
}

unsafe fn gicr_wait_for_rwp() {
    let mut count = 100000u32;
    while gicr_read(GICR_CTLR_OFF) & GICR_CTLR_RWP != 0 {
        count -= 1;
        if count == 0 {
            crate::uart_puts("[GICv3] WARNING: GICR RWP timeout\n");
            break;
        }
    }
}

fn init_gicv3_dist() {
    crate::uart_puts("[GICv3] Distributor init...\n");

    let vgic = detect_vgic();
    unsafe {
        IS_VGIC = vgic;
    }
    if vgic {
        crate::uart_puts("[GICv3] VGIC detected, skipping Distributor reconfiguration\n");
    }

    unsafe {
        let typer = gicd_read(GICD_TYPER);
        let nr_spis = (((typer & 0x1F) + 1) * 32) as usize;
        crate::uart_puts("[GICv3] SPI count: ");
        crate::uart_put_hex(nr_spis as u64);
        crate::uart_puts("\n");

        let spi_limit = if nr_spis > 1020 { 1020 } else { nr_spis };

        if !vgic {
            gicd_write(GICD_CTLR, 0);
            gicd_wait_for_rwp();

            for i in (32..spi_limit).step_by(32) {
                gicd_write(GICD_IGROUPR + (i / 8), 0xFFFF_FFFF);
                gicd_write(GICD_ICACTIVER + (i / 8), 0xFFFF_FFFF);
                gicd_write(GICD_ICENABLER + (i / 8), 0xFFFF_FFFF);
            }

            for i in (32..spi_limit).step_by(4) {
                gicd_write(GICD_IPRIORITYR + i, GICD_INT_DEF_PRI_X4);
            }

            for i in (32..spi_limit).step_by(16) {
                gicd_write(GICD_ICFGR + (i / 4), 0);
            }

            gicd_wait_for_rwp();

            let mpidr = read_mpidr_el1();
            let aff0 = (mpidr & 0xFF) as u64;
            let aff1 = ((mpidr >> 8) & 0xFF) as u64;
            let aff2 = ((mpidr >> 16) & 0xFF) as u64;
            let aff3 = ((mpidr >> 32) & 0xFF) as u64;
            let irouter_val = (aff3 << 32) | (aff2 << 16) | (aff1 << 8) | aff0;
            for i in 32..spi_limit {
                gicr_write64_at(GICD_BASE, GICD_IROUTER + i * 8, irouter_val);
            }

            gicd_write(
                GICD_CTLR,
                GICD_CTLR_ARE_NS | GICD_CTLR_ENABLE_G1A | GICD_CTLR_ENABLE_G1,
            );
            gicd_wait_for_rwp();

            let ctlr = gicd_read(GICD_CTLR);
            if ctlr & GICD_CTLR_ARE_NS == 0 {
                crate::uart_puts("[GICv3] ARE_NS not set, trying ARE_S...\n");
                gicd_write(
                    GICD_CTLR,
                    GICD_CTLR_ARE_S | GICD_CTLR_ENABLE_G1A | GICD_CTLR_ENABLE_G1,
                );
                gicd_wait_for_rwp();
            }
        }
    }

    crate::uart_puts("[GICv3] Distributor initialized\n");
}

unsafe fn gicr_write64_at(base: usize, offset: usize, val: u64) {
    core::ptr::write_volatile((base + offset) as *mut u64, val);
}

fn init_gicv3_cpu() {
    crate::uart_puts("[GICv3] Redistributor + CPU interface init...\n");

    let vgic = unsafe { IS_VGIC };

    unsafe {
        find_this_cpu_redistributor();

        let waker = gicr_read(GICR_WAKER_OFF);
        crate::uart_puts("[GICv3] GICR_WAKER=0x");
        crate::uart_put_hex(waker as u64);
        crate::uart_puts("\n");

        if !vgic {
            gicr_write(GICR_WAKER_OFF, waker & !GICR_WAKER_PROCESSOR_SLEEP);

            let mut count = 1000000u32;
            while gicr_read(GICR_WAKER_OFF) & GICR_WAKER_CHILDREN_ASLEEP != 0 {
                count -= 1;
                if count == 0 {
                    crate::uart_puts("[GICv3] WARNING: Redistributor wakeup timeout\n");
                    break;
                }
                core::hint::spin_loop();
            }

            if count > 0 {
                crate::uart_puts("[GICv3] Redistributor woken up\n");
            }

            gicr_write(GICR_IGROUPR0_OFF, 0xFFFF_FFFF);
            gicr_write(GICR_ICACTIVER0_OFF, 0xFFFF_FFFF);
            gicr_write(GICR_ICENABLER0_OFF, 0xFFFF_FFFF);

            for i in (0..32).step_by(4) {
                gicr_write(GICR_IPRIORITYR0_OFF + i, GICD_INT_DEF_PRI_X4);
            }

            gicr_wait_for_rwp();
        } else {
            crate::uart_puts("[GICv3] VGIC mode: skipping Redistributor reconfiguration\n");
            let g0_before = gicr_read(GICR_IGROUPR0_OFF);
            gicr_write(GICR_IGROUPR0_OFF, 0xFFFF_FFFF);
            let g0_after = gicr_read(GICR_IGROUPR0_OFF);
            crate::uart_puts("[GICv3] VGIC: IGROUPR0 before=0x");
            crate::uart_put_hex(g0_before as u64);
            crate::uart_puts(" after=0x");
            crate::uart_put_hex(g0_after as u64);
            crate::uart_puts("\n");
        }

        icc_igrpen0_el1_write(ICC_IGRPEN1_EL1_ENABLE);
        crate::uart_puts("[GICv3] ICC_IGRPEN0_EL1=1 (Group 0 also enabled)\n");

        let sre = icc_sre_el1_read();
        crate::uart_puts("[GICv3] ICC_SRE_EL1=0x");
        crate::uart_put_hex(sre);
        crate::uart_puts("\n");

        if sre & ICC_SRE_EL1_SRE == 0 {
            icc_sre_el1_write(sre | 0x7);
            let sre2 = icc_sre_el1_read();
            if sre2 & ICC_SRE_EL1_SRE == 0 {
                crate::uart_puts("[GICv3] WARNING: ICC_SRE_EL1 SRE bit stuck at 0\n");
                crate::uart_puts("[GICv3] GIC system registers not available, interrupts may not work\n");
            } else {
                crate::uart_puts("[GICv3] ICC_SRE_EL1 enabled\n");
            }
        } else {
            crate::uart_puts("[GICv3] ICC_SRE_EL1 already enabled\n");
        }

        icc_pmr_el1_write(ICC_PMR_DEF_PRIO);

        icc_bpr1_el1_write(0);

        let ctlr = icc_ctlr_el1_read();
        icc_ctlr_el1_write(ctlr | ICC_CTLR_EL1_EOIMODE_DROP);
        crate::uart_puts("[GICv3] ICC_CTLR_EL1 EOI mode set to drop (mode 1)\n");

        icc_igrpen1_el1_write(ICC_IGRPEN1_EL1_ENABLE);

        let igrpen = icc_igrpen1_el1_read();
        crate::uart_puts("[GICv3] ICC_IGRPEN1_EL1=0x");
        crate::uart_put_hex(igrpen);
        crate::uart_puts("\n");
    }

    crate::uart_puts("[GICv3] CPU interface initialized\n");
}

fn init_gicv3() {
    crate::uart_puts("[GICv3] Initializing...\n");

    init_gicv3_dist();
    init_gicv3_cpu();

    crate::uart_puts("[GICv3] Initialized\n");
}

fn init_gicv2() {
    crate::uart_puts("[GICv2] Initializing...\n");

    let vgic = detect_vgic();
    unsafe {
        IS_VGIC = vgic;
    }
    if vgic {
        crate::uart_puts("[GICv2] VGIC detected, skipping Distributor reconfiguration\n");
    }

    let gicc_base: usize = 0x0801_0000;

    const GIC_CPU_CTRL: usize = 0x00;
    const GIC_CPU_PRIMASK: usize = 0x04;
    const GIC_DIST_TARGET: usize = 0x0800;

    unsafe {
        let typer = gicd_read(GICD_TYPER);
        let nr_irqs = (((typer & 0x1F) + 1) * 32) as usize;
        let nr_irqs = if nr_irqs > 1020 { 1020 } else { nr_irqs };
        crate::uart_puts("[GICv2] IRQ count: ");
        crate::uart_put_hex(nr_irqs as u64);
        crate::uart_puts("\n");

        if !vgic {
            gicd_write(GICD_CTLR, 0);
            mmio_write32(gicc_base, GIC_CPU_CTRL, 0);

            for i in (32..nr_irqs).step_by(32) {
                gicd_write(GICD_IGROUPR + (i / 8), 0xFFFF_FFFF);
            }

            for i in (32..nr_irqs).step_by(4) {
                mmio_write32(GICD_BASE, GICD_IPRIORITYR + i, GICD_INT_DEF_PRI_X4);
            }

            for i in (32..nr_irqs).step_by(4) {
                mmio_write32(GICD_BASE, GIC_DIST_TARGET + i, 0x01010101);
            }

            for i in (32..nr_irqs).step_by(16) {
                gicd_write(GICD_ICFGR + (i / 4), 0);
            }

            gicd_write(GICD_CTLR, 1);

            for i in (0..32).step_by(4) {
                gicd_write(GICD_IPRIORITYR + i, GICD_INT_DEF_PRI_X4);
            }
        }

        mmio_write32(gicc_base, GIC_CPU_PRIMASK, 0xF0);
        mmio_write32(gicc_base, GIC_CPU_CTRL, 1);
    }

    crate::uart_puts("[GICv2] Initialized\n");
}

pub fn init_gic(fdt_ptr: u64) {
    let _ = fdt_ptr;

    let version = detect_gic_version();
    unsafe {
        GIC_VERSION = version;
    }

    crate::uart_puts("[GIC] Detected version: GICv");
    crate::uart_putc(b'0' + version as u8);
    crate::uart_puts("\n");

    match version {
        3 => init_gicv3(),
        _ => init_gicv2(),
    }
}

pub fn init_gic_from_fdt(gicd_base: u64, gicr_base: u64, version: u32) {
    unsafe {
        GICD_BASE = gicd_base as usize;
        GICR_BASE = gicr_base as usize;
        GIC_VERSION = version;
    }

    crate::uart_puts("[GIC] FDT-provided: GICv");
    crate::uart_putc(b'0' + version as u8);
    crate::uart_puts(" GICD=0x");
    crate::uart_put_hex(gicd_base);
    crate::uart_puts(" GICR=0x");
    crate::uart_put_hex(gicr_base);
    crate::uart_puts("\n");

    match version {
        3 => init_gicv3(),
        _ => init_gicv2(),
    }
}

pub fn enable_irq(intid: u32) {
    let version = unsafe { GIC_VERSION };
    unsafe {
        if intid < 32 {
            let bit = 1u32 << intid;
            if version == 3 {
                gicr_write(GICR_ISENABLER0_OFF, bit);
            } else {
                gicd_write(GICD_ISENABLER, bit);
            }
        } else {
            let reg = (intid / 32) as usize;
            let bit = 1u32 << (intid % 32);
            gicd_write(GICD_ISENABLER + reg * 4, bit);
        }
    }
}

pub fn disable_irq(intid: u32) {
    let version = unsafe { GIC_VERSION };
    unsafe {
        if intid < 32 {
            let bit = 1u32 << intid;
            if version == 3 {
                gicr_write(GICR_ICENABLER0_OFF, bit);
            } else {
                gicd_write(GICD_ICENABLER, bit);
            }
        } else {
            let reg = (intid / 32) as usize;
            let bit = 1u32 << (intid % 32);
            gicd_write(GICD_ICENABLER + reg * 4, bit);
        }
    }
}

pub fn acknowledge_irq() -> u32 {
    let version = unsafe { GIC_VERSION };
    if version == 3 {
        let intid = unsafe { icc_iar1_el1_read() as u32 };
        unsafe { asm!("dsb sy") };
        intid
    } else {
        unsafe { mmio_read32(0x0801_0000, 0x00C) }
    }
}

pub fn end_of_interrupt(intid: u32) {
    let version = unsafe { GIC_VERSION };
    if version == 3 {
        unsafe {
            icc_eoir1_el1_write(intid as u64);
        }
    } else {
        unsafe { mmio_write32(0x0801_0000, 0x010, intid) }
    }
}

pub fn deactivate_irq(intid: u32) {
    let version = unsafe { GIC_VERSION };
    if version == 3 {
        unsafe {
            icc_dir_el1_write(intid as u64);
        }
    }
}

pub fn is_spurious(intid: u32) -> bool {
    intid >= INTID_SPURIOUS
}

pub fn alloc_ppi() -> Option<u32> {
    let id = PPI_NEXT.fetch_add(1, Ordering::SeqCst);
    if id < 32 {
        Some(id)
    } else {
        None
    }
}

pub fn disable() {
    unsafe { asm!("msr DAIFSet, #0xF") }
}

pub fn enable_and_hlt() {
    unsafe {
        asm!(
            "msr DAIFClr, #0xF",
            "wfi"
        );
    }
}

pub fn one_shot_tsc_deadline_mode(_period: u64) -> Option<u64> {
    None
}

pub fn one_shot_tsc_deadline_mode_reset() {}

#[cfg(feature = "oneshot-apic")]
pub fn one_shot(_ticks: u64) {}

#[cfg(feature = "oneshot-apic")]
pub fn one_shot_reset() {}

pub fn readback_igroupr0() -> u32 {
    let version = unsafe { GIC_VERSION };
    if version == 3 {
        unsafe { gicr_read(GICR_IGROUPR0_OFF) }
    } else {
        0
    }
}

pub fn readback_isenabler0() -> u32 {
    let version = unsafe { GIC_VERSION };
    if version == 3 {
        unsafe { gicr_read(GICR_ISENABLER0_OFF) }
    } else {
        0
    }
}

pub fn apic_timer_lvtt_setup(_vector: u8) {}
