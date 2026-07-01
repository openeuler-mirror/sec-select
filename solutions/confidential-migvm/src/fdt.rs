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

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

const FDT_MAGIC: u32 = 0xD00DFEED;
const FDT_BEGIN_NODE: u32 = 0x00000001;
const FDT_END_NODE: u32 = 0x00000002;
const FDT_PROP: u32 = 0x00000003;
const FDT_NOP: u32 = 0x00000004;
const FDT_END: u32 = 0x00000009;

#[repr(C)]
struct FdtHeader {
    magic: u32,
    totalsize: u32,
    off_dt_struct: u32,
    off_dt_strings: u32,
    off_mem_rsvmap: u32,
    version: u32,
    last_comp_version: u32,
    boot_cpuid_phys: u32,
    size_dt_strings: u32,
    size_dt_struct: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct VirtioMmioDevice {
    pub base: u64,
    pub size: u64,
    pub irq: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct GicInfo {
    pub gicd_base: u64,
    pub gicd_size: u64,
    pub gicr_base: u64,
    pub gicr_size: u64,
    pub version: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct MemRegion {
    pub base: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct PcieInfo {
    pub ecam_base: u64,
    pub ecam_size: u64,
    pub mmio_base: u64,
    pub mmio_size: u64,
    pub bus_start: u8,
    pub bus_end: u8,
}

#[derive(Debug)]
pub struct FdtInfo {
    pub memory: Vec<MemRegion>,
    pub virtio_devices: Vec<VirtioMmioDevice>,
    pub gic: Option<GicInfo>,
    pub pcie: Option<PcieInfo>,
    pub interrupt_cells: u32,
}

impl fmt::Display for VirtioMmioDevice {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "VirtIO MMIO @ 0x{:08x}, size=0x{:x}, irq={}",
            self.base, self.size, self.irq
        )
    }
}

impl fmt::Display for GicInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "GICv{}: GICD=0x{:08x} (0x{:x}), GICR=0x{:08x} (0x{:x})",
            self.version, self.gicd_base, self.gicd_size, self.gicr_base, self.gicr_size
        )
    }
}

impl fmt::Display for MemRegion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Memory @ 0x{:08x}, size=0x{:x} ({}MB)",
            self.base,
            self.size,
            self.size / (1024 * 1024)
        )
    }
}

struct RawProp {
    name: String,
    data: *const u8,
    len: usize,
}

struct CollectedNode {
    props: Vec<RawProp>,
    children: Vec<CollectedNode>,
}

pub struct FdtParser {
    data: *const u8,
    struct_offset: usize,
    strings_offset: usize,
    struct_size: usize,
}

impl FdtParser {
    pub fn new(fdt_ptr: u64) -> Result<Self, &'static str> {
        if fdt_ptr == 0 {
            return Err("FDT pointer is NULL");
        }

        let data = fdt_ptr as *const u8;
        let header = unsafe { &*(data as *const FdtHeader) };

        let magic = u32::from_be(header.magic);
        if magic != FDT_MAGIC {
            crate::uart_puts("[FDT] Invalid magic: 0x");
            crate::uart_put_hex(magic as u64);
            crate::uart_puts("\n");
            return Err("Invalid FDT magic");
        }

        let struct_offset = u32::from_be(header.off_dt_struct) as usize;
        let strings_offset = u32::from_be(header.off_dt_strings) as usize;
        let struct_size = u32::from_be(header.size_dt_struct) as usize;

        Ok(Self {
            data,
            struct_offset,
            strings_offset,
            struct_size,
        })
    }

    pub fn parse(&self) -> Result<FdtInfo, &'static str> {
        let struct_start = unsafe { self.data.add(self.struct_offset) };
        let struct_end = unsafe { self.data.add(self.struct_offset + self.struct_size) };
        let mut offset = 0usize;

        let token = self.read_u32(struct_start, offset);
        if token != FDT_BEGIN_NODE {
            return Err("FDT structure does not start with FDT_BEGIN_NODE");
        }
        offset += 4;
        offset = self.skip_string(struct_start, offset);

        let root = self.collect_node(struct_start, struct_end, &mut offset);

        let mut info = FdtInfo {
            memory: Vec::new(),
            virtio_devices: Vec::new(),
            gic: None,
            pcie: None,
            interrupt_cells: 3,
        };

        self.process_tree(&root, &mut info, 2, 2);

        Ok(info)
    }

    fn collect_node(
        &self,
        base: *const u8,
        end: *const u8,
        offset: &mut usize,
    ) -> CollectedNode {
        let mut node = CollectedNode {
            props: Vec::new(),
            children: Vec::new(),
        };

        loop {
            if unsafe { base.add(*offset) } >= end {
                break;
            }

            let token = self.read_u32(base, *offset);
            *offset += 4;

            match token {
                FDT_BEGIN_NODE => {
                    *offset = self.skip_string(base, *offset);
                    let child = self.collect_node(base, end, offset);
                    node.children.push(child);
                }
                FDT_END_NODE => {
                    break;
                }
                FDT_PROP => {
                    let prop_len = self.read_u32(base, *offset) as usize;
                    *offset += 4;
                    let name_off = self.read_u32(base, *offset) as usize;
                    *offset += 4;

                    let name = self.get_string_name(name_off);
                    let data = unsafe { base.add(*offset) };
                    *offset += align4(prop_len);

                    node.props.push(RawProp {
                        name,
                        data,
                        len: prop_len,
                    });
                }
                FDT_NOP => {}
                FDT_END => break,
                _ => {}
            }
        }

        node
    }

    fn process_tree(
        &self,
        node: &CollectedNode,
        info: &mut FdtInfo,
        parent_addr_cells: u32,
        parent_size_cells: u32,
    ) {
        let mut addr_cells = parent_addr_cells;
        let mut size_cells = parent_size_cells;
        let mut local_interrupt_cells: Option<u32> = None;

        let saved_addr_cells = parent_addr_cells;
        let saved_size_cells = parent_size_cells;

        for prop in &node.props {
            match prop.name.as_str() {
                "#address-cells" if prop.len >= 4 => {
                    addr_cells = self.read_u32_from(prop.data);
                }
                "#size-cells" if prop.len >= 4 => {
                    size_cells = self.read_u32_from(prop.data);
                }
                "#interrupt-cells" if prop.len >= 4 => {
                    local_interrupt_cells = Some(self.read_u32_from(prop.data));
                }
                _ => {}
            }
        }

        let compatible = self.find_prop_str(&node.props, "compatible");
        let device_type = self.find_prop_str(&node.props, "device_type");

        if device_type.as_deref() == Some("memory") {
            if let Some((data, len)) = self.find_prop_raw(&node.props, "reg") {
                if let Some(region) = self.parse_reg(data, len, addr_cells, size_cells) {
                    info.memory.push(region);
                }
            }
        } else if compatible.as_deref() == Some("virtio,mmio") {
            if let Some((data, len)) = self.find_prop_raw(&node.props, "reg") {
                if let Some(region) = self.parse_reg(data, len, addr_cells, size_cells) {
                    let irq = self.parse_irq(&node.props, info.interrupt_cells);
                    info.virtio_devices.push(VirtioMmioDevice {
                        base: region.base,
                        size: region.size,
                        irq,
                    });
                }
            }
        } else if compatible.as_deref() == Some("arm,gic-v3") {
            if let Some((data, len)) = self.find_prop_raw(&node.props, "reg") {
                let mut gic = GicInfo {
                    gicd_base: 0,
                    gicd_size: 0,
                    gicr_base: 0,
                    gicr_size: 0,
                    version: 3,
                };
                self.parse_gic_reg(data, len, addr_cells, size_cells, &mut gic);
                info.gic = Some(gic);
            }
            if let Some(ic) = local_interrupt_cells {
                info.interrupt_cells = ic;
            }
        } else if compatible.as_deref() == Some("arm,cortex-a15-gic") {
            if let Some((data, len)) = self.find_prop_raw(&node.props, "reg") {
                let mut gic = GicInfo {
                    gicd_base: 0,
                    gicd_size: 0,
                    gicr_base: 0,
                    gicr_size: 0,
                    version: 2,
                };
                self.parse_gic_reg(data, len, addr_cells, size_cells, &mut gic);
                info.gic = Some(gic);
            }
            if let Some(ic) = local_interrupt_cells {
                info.interrupt_cells = ic;
            }
        } else if let Some(compat) = &compatible {
            if compat.contains("pci-host-ecam-generic") || compat.contains("pci-host-ecam") {
                if let Some((data, len)) = self.find_prop_raw(&node.props, "reg") {
                    let mut pcie = PcieInfo {
                        ecam_base: 0, ecam_size: 0,
                        mmio_base: 0, mmio_size: 0,
                        bus_start: 0, bus_end: 255,
                    };
                    let entry_size = (saved_addr_cells + saved_size_cells) as usize * 4;
                    if len >= entry_size {
                        pcie.ecam_base = self.read_cells(data, saved_addr_cells);
                        pcie.ecam_size = self.read_cells(
                            unsafe { data.add(saved_addr_cells as usize * 4) }, saved_size_cells);
                    }
                    if let Some((rdata, rlen)) = self.find_prop_raw(&node.props, "ranges") {
                        let child_cells = 3;
                        let parent_cells = saved_addr_cells;
                        let size_cells_parsed = saved_size_cells;
                        let entry = (child_cells + parent_cells + size_cells_parsed) as usize * 4;
                        let mut off = 0usize;
                        while off + entry <= rlen {
                            let hi = self.read_u32_from(unsafe { rdata.add(off) });
                            let space_code = (hi >> 24) & 3;
                            if space_code == 2 {
                                let _child_addr_lo = self.read_u32_from(unsafe { rdata.add(off + 4) }) as u64;
                                let parent = self.read_cells(unsafe { rdata.add(off + 12) }, parent_cells);
                                pcie.mmio_base = parent;
                                pcie.mmio_size = self.read_cells(
                                    unsafe { rdata.add(off + 12 + parent_cells as usize * 4) },
                                    size_cells_parsed);
                                break;
                            }
                            off += entry;
                        }
                    }
                    info.pcie = Some(pcie);
                }
            }
        }

        for child in &node.children {
            self.process_tree(child, info, addr_cells, size_cells);
        }
    }

    fn find_prop_str<'a>(&'a self, props: &'a [RawProp], name: &str) -> Option<String> {
        for prop in props {
            if prop.name == name {
                return Some(self.read_string_from(prop.data, prop.len));
            }
        }
        None
    }

    fn find_prop_raw(&self, props: &[RawProp], name: &str) -> Option<(*const u8, usize)> {
        for prop in props {
            if prop.name == name {
                return Some((prop.data, prop.len));
            }
        }
        None
    }

    fn parse_reg(
        &self,
        data: *const u8,
        len: usize,
        addr_cells: u32,
        size_cells: u32,
    ) -> Option<MemRegion> {
        let entry_size = (addr_cells + size_cells) as usize * 4;
        if len < entry_size || entry_size == 0 {
            return None;
        }

        let base = self.read_cells(data, addr_cells);
        let size = self.read_cells(unsafe { data.add(addr_cells as usize * 4) }, size_cells);

        Some(MemRegion { base, size })
    }

    fn parse_gic_reg(
        &self,
        data: *const u8,
        len: usize,
        addr_cells: u32,
        size_cells: u32,
        gic: &mut GicInfo,
    ) {
        let entry_size = (addr_cells + size_cells) as usize * 4;
        if len < entry_size {
            return;
        }

        gic.gicd_base = self.read_cells(data, addr_cells);
        gic.gicd_size = self.read_cells(unsafe { data.add(addr_cells as usize * 4) }, size_cells);

        if len >= entry_size * 2 {
            let second = unsafe { data.add(entry_size) };
            gic.gicr_base = self.read_cells(second, addr_cells);
            gic.gicr_size = self.read_cells(unsafe { second.add(addr_cells as usize * 4) }, size_cells);
        }
    }

    fn parse_irq(&self, props: &[RawProp], interrupt_cells: u32) -> u32 {
        if let Some((data, len)) = self.find_prop_raw(props, "interrupts") {
            if interrupt_cells == 3 && len >= 12 {
                let _gic_type = self.read_u32_from(data);
                let spi_num = self.read_u32_from(unsafe { data.add(4) });
                let _trigger = self.read_u32_from(unsafe { data.add(8) });
                return spi_num + 32;
            } else if len >= 4 {
                return self.read_u32_from(data);
            }
        }
        0
    }

    fn read_cells(&self, data: *const u8, cells: u32) -> u64 {
        match cells {
            0 => 0,
            1 => unsafe { core::ptr::read_unaligned(data as *const u32).to_be() as u64 },
            2 => {
                let hi = unsafe { core::ptr::read_unaligned(data as *const u32).to_be() as u64 };
                let lo = unsafe { core::ptr::read_unaligned(data.add(4) as *const u32).to_be() as u64 };
                (hi << 32) | lo
            }
            _ => 0,
        }
    }

    fn read_u32(&self, base: *const u8, offset: usize) -> u32 {
        unsafe { core::ptr::read_unaligned(base.add(offset) as *const u32).to_be() }
    }

    fn read_u32_from(&self, data: *const u8) -> u32 {
        unsafe { core::ptr::read_unaligned(data as *const u32).to_be() }
    }

    fn read_string_from(&self, data: *const u8, max_len: usize) -> String {
        let mut s = String::new();
        for i in 0..max_len {
            let b = unsafe { *data.add(i) };
            if b == 0 {
                break;
            }
            s.push(b as char);
        }
        s
    }

    fn skip_string(&self, base: *const u8, offset: usize) -> usize {
        let mut o = offset;
        loop {
            let b = unsafe { *base.add(o) };
            o += 1;
            if b == 0 {
                break;
            }
        }
        align4(o)
    }

    fn get_string_name(&self, offset: usize) -> String {
        let str_base = unsafe { self.data.add(self.strings_offset) };
        let mut s = String::new();
        let mut i = 0usize;
        loop {
            let b = unsafe { *str_base.add(offset + i) };
            if b == 0 {
                break;
            }
            s.push(b as char);
            i += 1;
        }
        s
    }
}

fn align4(v: usize) -> usize {
    (v + 3) & !3
}

pub fn parse_fdt(fdt_ptr: u64) -> Result<FdtInfo, &'static str> {
    let ptr = if fdt_ptr != 0 {
        fdt_ptr
    } else {
        match find_fdt_in_ram() {
            Some(p) => p,
            None => return Err("FDT pointer is NULL and FDT not found in RAM"),
        }
    };
    let parser = FdtParser::new(ptr)?;
    parser.parse()
}

fn find_fdt_in_ram() -> Option<u64> {
    let known_addr: u64 = 0x4800_0000;
    let magic_bytes: [u8; 4] = FDT_MAGIC.to_be_bytes();
    let ptr = known_addr as *const u8;
    let slice = unsafe { core::slice::from_raw_parts(ptr, 4) };
    if slice == magic_bytes {
        crate::uart_puts("[FDT] Found FDT at loader addr 0x");
        crate::uart_put_hex(known_addr);
        crate::uart_puts("\n");
        return Some(known_addr);
    }

    let ram_start: u64 = 0x4000_0000;
    let ram_end: u64 = 0x4800_0000;

    crate::uart_puts("[FDT] Scanning low RAM for FDT magic...\n");

    let mut addr = ram_start;
    while addr < ram_end {
        let ptr = addr as *const u8;
        let slice = unsafe { core::slice::from_raw_parts(ptr, 4) };
        if slice == magic_bytes {
            crate::uart_puts("[FDT] Found FDT magic at 0x");
            crate::uart_put_hex(addr);
            crate::uart_puts("\n");
            return Some(addr);
        }
        addr += 0x1000;
    }

    None
}

pub fn print_fdt_info(info: &FdtInfo) {
    crate::uart_puts("[FDT] Parsed device tree:\n");

    for region in &info.memory {
        crate::uart_puts("[FDT]   ");
        crate::uart_puts(&alloc::format!("{}\n", region));
    }

    if let Some(ref gic) = info.gic {
        crate::uart_puts("[FDT]   ");
        crate::uart_puts(&alloc::format!("{}\n", gic));
    }

    crate::uart_puts("[FDT]   VirtIO MMIO devices: ");
    crate::uart_put_hex(info.virtio_devices.len() as u64);
    crate::uart_puts("\n");
    for dev in &info.virtio_devices {
        crate::uart_puts("[FDT]     ");
        crate::uart_puts(&alloc::format!("{}\n", dev));
    }
    if let Some(ref pcie) = info.pcie {
        crate::uart_puts("[FDT]   PCIe ECAM @ ");
        crate::uart_put_hex(pcie.ecam_base);
        crate::uart_puts(" size=");
        crate::uart_put_hex(pcie.ecam_size);
        crate::uart_puts(" bus=");
        crate::uart_put_hex(pcie.bus_start as u64);
        crate::uart_puts("-");
        crate::uart_put_hex(pcie.bus_end as u64);
        crate::uart_puts(" mmio=0x");
        crate::uart_put_hex(pcie.mmio_base);
        crate::uart_puts(" size=0x");
        crate::uart_put_hex(pcie.mmio_size);
        crate::uart_puts("\n");
    }
}
