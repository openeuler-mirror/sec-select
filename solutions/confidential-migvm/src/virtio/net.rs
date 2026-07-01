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

use crate::uart_puts;
use crate::uart_put_hex;
use crate::virtio::mmio_regs::*;
use crate::virtio::mmio_transport::VirtioMmioTransport;
use crate::virtio::virtio_pci::VirtioPciTransport;
use crate::virtio::{alloc_dma_pages, free_dma_pages};
use core::mem;
use core::ptr;

pub const VIRTQ_DESC_F_NEXT: u16 = 1;
pub const VIRTQ_DESC_F_WRITE: u16 = 2;
pub const VIRTQ_DESC_F_INDIRECT: u16 = 4;

pub const VIRTIO_NET_F_CSUM: u64 = 1 << 0;
pub const VIRTIO_NET_F_GUEST_CSUM: u64 = 1 << 1;
pub const VIRTIO_NET_F_MAC: u64 = 1 << 5;
pub const VIRTIO_NET_F_GSO: u64 = 1 << 6;
pub const VIRTIO_NET_F_GUEST_TSO4: u64 = 1 << 7;
pub const VIRTIO_NET_F_GUEST_TSO6: u64 = 1 << 8;
pub const VIRTIO_NET_F_GUEST_ECN: u64 = 1 << 9;
pub const VIRTIO_NET_F_GUEST_UFO: u64 = 1 << 10;
pub const VIRTIO_NET_F_HOST_TSO4: u64 = 1 << 11;
pub const VIRTIO_NET_F_HOST_TSO6: u64 = 1 << 12;
pub const VIRTIO_NET_F_HOST_ECN: u64 = 1 << 13;
pub const VIRTIO_NET_F_HOST_UFO: u64 = 1 << 14;
pub const VIRTIO_NET_F_MRG_RXBUF: u64 = 1 << 15;
pub const VIRTIO_NET_F_STATUS: u64 = 1 << 16;
pub const VIRTIO_NET_F_CTRL_VQ: u64 = 1 << 17;
pub const VIRTIO_NET_F_CTRL_RX: u64 = 1 << 18;
pub const VIRTIO_NET_F_CTRL_VLAN: u64 = 1 << 19;

pub const VIRTIO_NET_HDR_F_NEEDS_CSUM: u8 = 1;
pub const VIRTIO_NET_HDR_F_DATA_VALID: u8 = 2;

pub const VIRTIO_NET_GSO_NONE: u8 = 0;
pub const VIRTIO_NET_GSO_TCPV4: u8 = 1;
pub const VIRTIO_NET_GSO_UDP: u8 = 3;
pub const VIRTIO_NET_GSO_TCPV6: u8 = 4;

pub const NET_BUF_SIZE: usize = 1536;
pub const NET_QUEUE_SIZE: u16 = 128;

#[repr(C)]
pub struct VirtioNetHdr {
    pub flags: u8,
    pub gso_type: u8,
    pub hdr_len: u16,
    pub gso_size: u16,
    pub csum_start: u16,
    pub csum_offset: u16,
}

#[repr(C)]
pub struct VirtqDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

#[repr(C)]
pub struct VirtqUsedElem {
    pub id: u32,
    pub len: u32,
}

pub struct VirtQueue {
    pub desc: u64,
    pub avail: u64,
    pub used: u64,
    pub num: u16,
    pub free_desc: u16,
    pub last_used_idx: u16,
    dma_addr: u64,
    dma_pages: usize,
}

impl VirtQueue {
    pub fn new(queue_size: u16, is_legacy: bool) -> Option<Self> {
        if is_legacy {
            Self::new_legacy(queue_size)
        } else {
            Self::new_modern(queue_size)
        }
    }

    fn new_legacy(queue_size: u16) -> Option<Self> {
        let num = queue_size;
        let desc_bytes = num as usize * mem::size_of::<VirtqDesc>();
        let avail_bytes = mem::size_of::<u16>() * 2 + num as usize * mem::size_of::<u16>();
        let used_bytes = mem::size_of::<u16>() * 2 + num as usize * mem::size_of::<VirtqUsedElem>();

        let desc_pages = (desc_bytes + 0xFFF) >> 12;
        let avail_pages = (avail_bytes + 0xFFF) >> 12;
        let used_pages = (used_bytes + 0xFFF) >> 12;
        let total_pages = desc_pages + avail_pages + used_pages;

        let dma_addr = alloc_dma_pages(total_pages)?;
        unsafe { ptr::write_bytes(dma_addr as *mut u8, 0, total_pages * 0x1000); }

        let desc = dma_addr;
        let avail = dma_addr + (desc_pages as u64 * 0x1000);
        let used = dma_addr + ((desc_pages + avail_pages) as u64 * 0x1000);

        let vq = VirtQueue { desc, avail, used, num, free_desc: 0, last_used_idx: 0, dma_addr, dma_pages: total_pages };
        vq.init_descriptors();
        Some(vq)
    }

    fn new_modern(queue_size: u16) -> Option<Self> {
        let num = queue_size;
        let desc_bytes = num as usize * mem::size_of::<VirtqDesc>();
        let avail_bytes = mem::size_of::<u16>() * 2 + num as usize * mem::size_of::<u16>();
        let used_bytes = mem::size_of::<u16>() * 2 + num as usize * mem::size_of::<VirtqUsedElem>();

        let avail_off = (desc_bytes + 1) & !1;
        let used_off = (avail_off + avail_bytes + 7) & !7;
        let total_bytes = used_off + used_bytes;
        let total_pages = (total_bytes + 0xFFF) >> 12;

        let dma_addr = alloc_dma_pages(total_pages)?;
        unsafe { ptr::write_bytes(dma_addr as *mut u8, 0, total_pages * 0x1000); }

        let desc = dma_addr;
        let avail = dma_addr + avail_off as u64;
        let used = dma_addr + used_off as u64;

        let vq = VirtQueue { desc, avail, used, num, free_desc: 0, last_used_idx: 0, dma_addr, dma_pages: total_pages };
        vq.init_descriptors();
        Some(vq)
    }

    fn init_descriptors(&self) {
        for i in 0..self.num {
            unsafe {
                let desc = (self.desc as *mut VirtqDesc).add(i as usize);
                ptr::write_volatile(&mut (*desc).next, if i + 1 < self.num { i + 1 } else { 0 });
            }
        }
    }

    fn desc_ptr(&self, idx: u16) -> *mut VirtqDesc {
        unsafe { (self.desc as *mut VirtqDesc).add(idx as usize) }
    }

    pub(crate) fn desc_ptr_pub(&self, idx: u16) -> *mut VirtqDesc {
        self.desc_ptr(idx)
    }

    fn avail_idx_ptr(&self) -> *mut u16 {
        unsafe { (self.avail as *mut u16).add(1) }
    }

    fn avail_ring_ptr(&self) -> *mut u16 {
        unsafe { (self.avail as *mut u16).add(2) }
    }

    fn used_idx_ptr(&self) -> *mut u16 {
        unsafe { (self.used as *mut u16).add(1) }
    }

    fn used_ring_ptr(&self) -> *mut VirtqUsedElem {
        unsafe { (self.used as *mut u8).add(4) as *mut VirtqUsedElem }
    }

    pub fn avail_add(&self, head_idx: u16) {
        unsafe {
            let avail_idx = ptr::read_volatile(self.avail_idx_ptr());
            let ring = self.avail_ring_ptr();
            ptr::write_volatile(ring.add((avail_idx % self.num) as usize), head_idx);
            core::arch::asm!("dsb st", options(nomem, nostack));
            ptr::write_volatile(self.avail_idx_ptr(), avail_idx.wrapping_add(1));
        }
    }

    pub fn get_used_buf(&mut self) -> Option<(u32, u32)> {
        let used_idx = unsafe { ptr::read_volatile(self.used_idx_ptr()) };
        if self.last_used_idx == used_idx {
            return None;
        }
        let elem = unsafe {
            ptr::read_volatile(self.used_ring_ptr().add((self.last_used_idx % self.num) as usize))
        };
        self.last_used_idx = self.last_used_idx.wrapping_add(1);
        Some((elem.id, elem.len))
    }

    pub fn poll_used_imm(&self) -> u16 {
        unsafe { ptr::read_volatile(self.used_idx_ptr()) }
    }
}

impl Drop for VirtQueue {
    fn drop(&mut self) {
        free_dma_pages(self.dma_addr, self.dma_pages);
    }
}

pub(crate) struct RxBuf {
    dma: u64,
    desc_idx: u16,
}

impl RxBuf {
    pub(crate) fn dma(&self) -> u64 {
        self.dma
    }
}

pub enum NetTransport {
    Mmio(VirtioMmioTransport),
    Pci(VirtioPciTransport),
}

impl NetTransport {
    pub fn notify_queue(&self, idx: u16) {
        match self {
            NetTransport::Mmio(t) => t.notify_queue(idx),
            NetTransport::Pci(t) => t.notify_queue(idx),
        }
    }

    pub fn read_interrupt_status(&self) -> u32 {
        match self {
            NetTransport::Mmio(t) => t.read_interrupt_status(),
            NetTransport::Pci(t) => t.read_interrupt_status(),
        }
    }
}

pub struct VirtioNet {
    pub transport: NetTransport,
    pub(crate) rx: VirtQueue,
    pub tx: VirtQueue,
    pub mac: [u8; 6],
    pub features: u64,
    pub(crate) rx_bufs: [Option<RxBuf>; NET_QUEUE_SIZE as usize],
    rx_buf_next: u16,
    pub(crate) tx_buf_dma: u64,
}

impl VirtioNet {
    pub fn init(mut transport: VirtioMmioTransport) -> Option<Self> {
        uart_puts("\n[VirtIO Net] === Net device init ===\n");

        if transport.device_id() != VIRTIO_SUBSYSTEM_NET {
            uart_puts("[VirtIO Net] ERROR: not a net device\n");
            return None;
        }

        match transport.init_device(VIRTIO_SUBSYSTEM_NET) {
            Ok(()) => uart_puts("[VirtIO Net] init_device OK\n"),
            Err(_) => { uart_puts("[VirtIO Net] init_device FAILED\n"); return None; }
        }

        let device_features = transport.get_features();
        let driver_features = device_features
            & (VIRTIO_NET_F_MAC | VIRTIO_NET_F_STATUS | VIRTIO_F_VERSION_1);

        uart_puts("[VirtIO Net] Device features: 0x");
        uart_put_hex(device_features);
        uart_puts(" Driver: 0x");
        uart_put_hex(driver_features);
        uart_puts("\n");

        let negotiated = match transport.negotiate_features(driver_features) {
            Ok(f) => f,
            Err(_) => { uart_puts("[VirtIO Net] Feature negotiation FAILED\n"); return None; }
        };

        let rx_max = transport.get_queue_max_size(0);
        let tx_max = transport.get_queue_max_size(1);
        let queue_size = NET_QUEUE_SIZE.min(rx_max).min(tx_max);
        if queue_size == 0 {
            uart_puts("[VirtIO Net] ERROR: queue not available\n");
            return None;
        }

        let is_legacy = transport.is_legacy();
        uart_puts("[VirtIO Net] Mode: ");
        uart_puts(if is_legacy { "legacy" } else { "modern" });
        uart_puts(" qsize=");
        uart_put_hex(queue_size as u64);
        uart_puts("\n");

        let rx = VirtQueue::new(queue_size, is_legacy)?;
        let tx = VirtQueue::new(queue_size, is_legacy)?;

        match transport.setup_queue(0, rx.desc, rx.avail, rx.used, queue_size) {
            Ok(()) => uart_puts("[VirtIO Net] RX queue setup OK\n"),
            Err(_) => { uart_puts("[VirtIO Net] RX queue setup FAILED\n"); return None; }
        }
        match transport.setup_queue(1, tx.desc, tx.avail, tx.used, queue_size) {
            Ok(()) => uart_puts("[VirtIO Net] TX queue setup OK\n"),
            Err(_) => { uart_puts("[VirtIO Net] TX queue setup FAILED\n"); return None; }
        }

        transport.driver_ok();

        let tx_buf_dma = alloc_dma_pages(1)?;
        unsafe { ptr::write_bytes(tx_buf_dma as *mut u8, 0, 0x1000); }

        let mut mac: [u8; 6] = [0; 6];
        for i in 0..6u64 { mac[i as usize] = transport.read_config8(i); }

        uart_puts("[VirtIO Net] MAC: ");
        for i in 0..6 {
            if mac[i] < 0x10 { uart_puts("0"); }
            uart_put_hex(mac[i] as u64);
            if i < 5 { uart_puts(":"); }
        }
        uart_puts("\n");

        const NONE_BUF: Option<RxBuf> = None;
        let rx_bufs: [Option<RxBuf>; NET_QUEUE_SIZE as usize] = [NONE_BUF; NET_QUEUE_SIZE as usize];

        let mut net = VirtioNet {
            transport: NetTransport::Mmio(transport), rx, tx, mac, features: negotiated,
            rx_bufs, rx_buf_next: 0, tx_buf_dma,
        };

        net.fill_rx_buffers();

        uart_puts("[VirtIO Net] === Init complete ===\n");
        Some(net)
    }

    pub fn init_pci(transport: VirtioPciTransport) -> Option<Self> {
        uart_puts("\n[VirtIO Net PCI] === Net device init ===\n");

        let device_features = transport.get_features();
        let driver_features = device_features
            & (VIRTIO_NET_F_MAC | VIRTIO_NET_F_STATUS | VIRTIO_F_VERSION_1);

        uart_puts("[VirtIO Net PCI] Device features: 0x");
        uart_put_hex(device_features);
        uart_puts(" Driver: 0x");
        uart_put_hex(driver_features);
        uart_puts("\n");

        let negotiated = transport.negotiate_features(driver_features)?;

        let rx_max = transport.get_queue_max_size(0);
        let tx_max = transport.get_queue_max_size(1);
        let queue_size = NET_QUEUE_SIZE.min(rx_max).min(tx_max);
        if queue_size == 0 {
            uart_puts("[VirtIO Net PCI] ERROR: queue not available\n");
            return None;
        }

        uart_puts("[VirtIO Net PCI] Mode: modern qsize=");
        uart_put_hex(queue_size as u64);
        uart_puts("\n");

        let rx = VirtQueue::new(queue_size, false)?;
        let tx = VirtQueue::new(queue_size, false)?;

        uart_puts("[VirtIO Net PCI] RX desc=0x");
        uart_put_hex(rx.desc);
        uart_puts(" TX desc=0x");
        uart_put_hex(tx.desc);
        uart_puts("\n");

        if !transport.setup_queue(0, rx.desc, rx.avail, rx.used, queue_size) {
            uart_puts("[VirtIO Net PCI] RX queue setup FAILED\n");
            return None;
        }
        if !transport.setup_queue(1, tx.desc, tx.avail, tx.used, queue_size) {
            uart_puts("[VirtIO Net PCI] TX queue setup FAILED\n");
            return None;
        }

        transport.driver_ok();
        uart_puts("[VirtIO Net PCI] setup_queue OK, DRIVER_OK set\n");

        let tx_buf_dma = alloc_dma_pages(1)?;
        unsafe { ptr::write_bytes(tx_buf_dma as *mut u8, 0, 0x1000); }

        let mut mac: [u8; 6] = [0; 6];
        for i in 0..6u64 { mac[i as usize] = transport.read_config8(i); }

        uart_puts("[VirtIO Net PCI] MAC: ");
        for i in 0..6 {
            if mac[i] < 0x10 { uart_puts("0"); }
            uart_put_hex(mac[i] as u64);
            if i < 5 { uart_puts(":"); }
        }
        uart_puts("\n");

        const NONE_BUF: Option<RxBuf> = None;
        let rx_bufs: [Option<RxBuf>; NET_QUEUE_SIZE as usize] = [NONE_BUF; NET_QUEUE_SIZE as usize];

        let mut net = VirtioNet {
            transport: NetTransport::Pci(transport), rx, tx, mac, features: negotiated,
            rx_bufs, rx_buf_next: 0, tx_buf_dma,
        };

        net.fill_rx_buffers();
        net.transport.notify_queue(0);

        uart_puts("[VirtIO Net PCI] === Init complete ===\n");

        let test_ok = net.tx_send_test();
        uart_puts("[VirtIO Net PCI] TX test: ");
        uart_puts(if test_ok { "OK\n" } else { "FAIL\n" });

        let rx_used = unsafe { core::ptr::read_volatile(net.rx.used_idx_ptr()) };
        uart_puts("[VirtIO Net PCI] RX used_idx after init: 0x");
        uart_put_hex(rx_used as u64);
        uart_puts(" last=0x");
        uart_put_hex(net.rx.last_used_idx as u64);
        uart_puts("\n");

        Some(net)
    }

    fn tx_send_test(&mut self) -> bool {
        let arp_pkt: [u8; 42] = [
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xfe, 0xed, 0xde, 0xad, 0xbe, 0xef,
            0x08, 0x06,
            0x00, 0x01, 0x08, 0x00, 0x06, 0x04, 0x00, 0x01,
            0xfe, 0xed, 0xde, 0xad, 0xbe, 0xef,
            0x00, 0x00, 0x00, 0x00,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0x00, 0x00, 0x00, 0x00,
        ];
        self.tx_send(&arp_pkt)
    }

    fn fill_rx_buffers(&mut self) {
        let batch = self.rx.num.min(64);

        for _ in 0..batch {
            let buf_dma = match alloc_dma_pages(1) {
                Some(a) => a,
                None => { uart_puts("[VirtIO Net] RX buf alloc FAILED\n"); return; }
            };
            unsafe { ptr::write_bytes(buf_dma as *mut u8, 0, 0x1000); }

            let d0 = self.rx_buf_next;
            self.rx_buf_next = (self.rx_buf_next + 1) % self.rx.num;

            unsafe {
                let desc = &mut *self.rx.desc_ptr(d0);
                desc.addr = buf_dma;
                desc.len = 0x1000u32;
                desc.flags = VIRTQ_DESC_F_WRITE;
                desc.next = 0;
            }

            self.rx_bufs[d0 as usize] = Some(RxBuf { dma: buf_dma, desc_idx: d0 });
            self.rx.avail_add(d0);
        }
    }

    pub(crate) fn recycle_rx_buf(&mut self, desc_idx: u16) {
        let buf = match &self.rx_bufs[desc_idx as usize] {
            Some(b) => RxBuf { dma: b.dma, desc_idx: b.desc_idx },
            None => {
                let dma = alloc_dma_pages(1).unwrap();
                RxBuf { dma, desc_idx }
            }
        };

        unsafe {
            let desc = &mut *self.rx.desc_ptr(buf.desc_idx);
            desc.addr = buf.dma;
            desc.len = 0x1000u32;
            desc.flags = VIRTQ_DESC_F_WRITE;
            desc.next = 0;
        }
        self.rx_bufs[desc_idx as usize] = Some(buf);
        self.rx.avail_add(desc_idx);
    }

    pub fn rx_poll(&mut self) -> Option<(*const u8, usize)> {
        let (desc_idx, pkt_len) = self.rx.get_used_buf()?;

        let buf = self.rx_bufs[desc_idx as usize].take()?;

        let hdr = buf.dma as *const VirtioNetHdr;
        let data = (buf.dma + mem::size_of::<VirtioNetHdr>() as u64) as *const u8;

        let hdr_len = unsafe { u16::from_le(ptr::read_volatile(&(*hdr).hdr_len)) } as usize;
        let data_len = pkt_len as usize - mem::size_of::<VirtioNetHdr>();

        self.rx_bufs[desc_idx as usize] = Some(buf);

        let skip = if hdr_len > 0 { hdr_len } else { 0 };
        Some((data, data_len.saturating_sub(skip)))
    }

    pub fn rx_recycle(&mut self, desc_idx: u16) {
        self.recycle_rx_buf(desc_idx);
    }

    pub fn tx_send(&mut self, data: &[u8]) -> bool {
        let hdr_len = mem::size_of::<VirtioNetHdr>() as u32;
        let pkt_len = data.len() as u32;

        if pkt_len > 0x1000 - hdr_len { return false; }

        unsafe {
            let hdr_ptr = self.tx_buf_dma as *mut VirtioNetHdr;
            ptr::write_bytes(hdr_ptr, 0, 1);
            (*hdr_ptr).flags = 0;
            (*hdr_ptr).gso_type = VIRTIO_NET_GSO_NONE;
            (*hdr_ptr).hdr_len = 0;
            (*hdr_ptr).gso_size = 0;
            (*hdr_ptr).csum_start = 0;
            (*hdr_ptr).csum_offset = 0;

            let pkt_ptr = (self.tx_buf_dma + hdr_len as u64) as *mut u8;
            ptr::copy_nonoverlapping(data.as_ptr(), pkt_ptr, pkt_len as usize);
        }

        let d0: u16 = 0;
        let d1 = unsafe { (*self.tx.desc_ptr(0)).next };

        unsafe {
            let desc0 = &mut *self.tx.desc_ptr(d0);
            desc0.addr = self.tx_buf_dma;
            desc0.len = hdr_len;
            desc0.flags = VIRTQ_DESC_F_NEXT;
            desc0.next = d1;

            let desc1 = &mut *self.tx.desc_ptr(d1);
            desc1.addr = self.tx_buf_dma + hdr_len as u64;
            desc1.len = pkt_len;
            desc1.flags = 0;
            desc1.next = 0;
        }

        self.tx.avail_add(d0);

        unsafe { core::arch::asm!("dsb st", options(nomem, nostack)); }
        self.transport.notify_queue(1);

        for _ in 0..5000000 { core::hint::spin_loop(); }

        self.tx.get_used_buf().is_some()
    }
}
