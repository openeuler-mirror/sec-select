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

pub const VSOCK_QUEUE_SIZE: u16 = 64;

pub const VIRTIO_VSOCK_F_SEQPACKET: u64 = 1;

const VSOCK_RX_QUEUE: u16 = 0;
const VSOCK_TX_QUEUE: u16 = 1;

const TX_BUF_SIZE: u32 = 4096;
const RX_BUF_SIZE: u32 = 4096;

pub const VSOCK_OP_INVALID: u16 = 0;
pub const VSOCK_OP_REQUEST: u16 = 1;
pub const VSOCK_OP_RESPONSE: u16 = 2;
pub const VSOCK_OP_RST: u16 = 3;
pub const VSOCK_OP_SHUTDOWN: u16 = 4;
pub const VSOCK_OP_RW: u16 = 5;
pub const VSOCK_OP_CREDIT_UPDATE: u16 = 6;
pub const VSOCK_OP_CREDIT_REQUEST: u16 = 7;

pub const VSOCK_TYPE_STREAM: u16 = 1;

#[repr(C)]
pub struct VirtioVsockHdr {
    pub src_cid: u64,
    pub dst_cid: u64,
    pub src_port: u32,
    pub dst_port: u32,
    pub len: u32,
    pub ty: u16,
    pub op: u16,
    pub flags: u32,
    pub buf_alloc: u32,
    pub fwd_cnt: u32,
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

const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;

pub struct VsockVirtQueue {
    pub desc: u64,
    pub avail: u64,
    pub used: u64,
    pub num: u16,
    pub free_desc: u16,
    pub last_used_idx: u16,
    dma_addr: u64,
    dma_pages: usize,
}

impl VsockVirtQueue {
    pub fn new(queue_size: u16) -> Option<Self> {
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

        let vq = Self { desc, avail, used, num, free_desc: 0, last_used_idx: 0, dma_addr, dma_pages: total_pages };

        for i in 0..num {
            unsafe {
                let d = (vq.desc as *mut VirtqDesc).add(i as usize);
                ptr::write_volatile(&mut (*d).addr, 0);
                ptr::write_volatile(&mut (*d).len, 0);
                ptr::write_volatile(&mut (*d).flags, 0);
                ptr::write_volatile(&mut (*d).next, if i + 1 < num { i + 1 } else { 0 });
            }
        }

        Some(vq)
    }

    fn desc_ptr(&self, idx: u16) -> *mut VirtqDesc {
        unsafe { (self.desc as *mut VirtqDesc).add(idx as usize) }
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

    pub fn avail_add(&mut self, head_idx: u16) {
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

    pub fn alloc_desc_head(&mut self) -> Option<u16> {
        let head = self.free_desc;
        let d = unsafe { &*self.desc_ptr(head) };
        if d.next == 0 && head != 0 {
            return None;
        }
        self.free_desc = d.next;
        Some(head)
    }

    pub fn setup_rx_desc(&mut self, desc_idx: u16, buf_dma: u64, buf_len: u32) {
        unsafe {
            let d = &mut *self.desc_ptr(desc_idx);
            d.addr = buf_dma;
            d.len = buf_len;
            d.flags = VIRTQ_DESC_F_WRITE;
            d.next = 0;
        }
    }
}

impl Drop for VsockVirtQueue {
    fn drop(&mut self) {
        free_dma_pages(self.dma_addr, self.dma_pages);
    }
}

pub enum VsockTransport {
    Mmio(VirtioMmioTransport),
    Pci(VirtioPciTransport),
}

impl VsockTransport {
    pub fn notify_queue(&self, idx: u16) {
        match self {
            VsockTransport::Mmio(t) => t.notify_queue(idx),
            VsockTransport::Pci(t) => t.notify_queue(idx),
        }
    }

    pub fn read_interrupt_status(&self) -> u32 {
        match self {
            VsockTransport::Mmio(t) => t.read_interrupt_status(),
            VsockTransport::Pci(t) => t.read_interrupt_status(),
        }
    }
}

pub struct VsockDevice {
    pub transport: VsockTransport,
    rx: VsockVirtQueue,
    tx: VsockVirtQueue,
    pub guest_cid: u64,
    pub peer_cid: u64,
    pub peer_port: u32,
    pub local_port: u32,
    rx_buf_dma: u64,
    tx_buf_dma: u64,
    rx_buf_len: u32,
    rx_desc_idx: u16,
    pub connected: bool,
}

impl VsockDevice {
    pub fn init_pci(transport: VirtioPciTransport, peer_cid: u64, peer_port: u32) -> Option<Self> {
        uart_puts("\n[VirtIO Vsock PCI] === Vsock device init ===\n");

        let device_features = transport.get_features();
        let driver_features = device_features & VIRTIO_F_VERSION_1;

        uart_puts("[VirtIO Vsock PCI] Device features: 0x");
        uart_put_hex(device_features);
        uart_puts(" Driver: 0x");
        uart_put_hex(driver_features);
        uart_puts("\n");

        let _negotiated = transport.negotiate_features(driver_features)?;

        let guest_cid = transport.read_config64(0);
        uart_puts("[VirtIO Vsock PCI] Guest CID: ");
        uart_put_hex(guest_cid);
        uart_puts("\n");

        let rx_max = transport.get_queue_max_size(VSOCK_RX_QUEUE);
        let tx_max = transport.get_queue_max_size(VSOCK_TX_QUEUE);
        let queue_size = VSOCK_QUEUE_SIZE.min(rx_max).min(tx_max);
        if queue_size < 2 {
            uart_puts("[VirtIO Vsock PCI] ERROR: queue too small\n");
            return None;
        }

        uart_puts("[VirtIO Vsock PCI] qsize=");
        uart_put_hex(queue_size as u64);
        uart_puts("\n");

        let rx = VsockVirtQueue::new(queue_size)?;
        let tx = VsockVirtQueue::new(queue_size)?;

        if !transport.setup_queue(VSOCK_RX_QUEUE, rx.desc, rx.avail, rx.used, queue_size) {
            uart_puts("[VirtIO Vsock PCI] RX queue setup FAILED\n");
            return None;
        }
        if !transport.setup_queue(VSOCK_TX_QUEUE, tx.desc, tx.avail, tx.used, queue_size) {
            uart_puts("[VirtIO Vsock PCI] TX queue setup FAILED\n");
            return None;
        }

        transport.driver_ok();
        uart_puts("[VirtIO Vsock PCI] setup_queue OK, DRIVER_OK set\n");

        let rx_buf_dma = alloc_dma_pages(1)?;
        unsafe { ptr::write_bytes(rx_buf_dma as *mut u8, 0, 0x1000); }

        let tx_buf_dma = alloc_dma_pages(1)?;
        unsafe { ptr::write_bytes(tx_buf_dma as *mut u8, 0, 0x1000); }

        uart_puts("[VirtIO Vsock PCI] === Init complete ===\n");

        Some(VsockDevice {
            transport: VsockTransport::Pci(transport),
            rx,
            tx,
            guest_cid,
            peer_cid,
            peer_port,
            local_port: 0xFFFF,
            rx_buf_dma,
            tx_buf_dma,
            rx_buf_len: RX_BUF_SIZE,
            rx_desc_idx: 0,
            connected: false,
        })
    }

    fn submit_rx_buf(&mut self) {
        if self.rx_desc_idx == 0 {
            self.rx_desc_idx = match self.rx.alloc_desc_head() {
                Some(h) => h,
                None => return,
            };
        }
        self.rx.setup_rx_desc(self.rx_desc_idx, self.rx_buf_dma, self.rx_buf_len);
        let idx = self.rx_desc_idx;
        self.rx_desc_idx = 0;
        self.rx.avail_add(idx);
    }

    pub fn connect(&mut self) -> bool {
        self.submit_rx_buf();

        let hdr = self.tx_buf_dma as *mut VirtioVsockHdr;
        unsafe {
            ptr::write_bytes(hdr, 0, 1);
            (*hdr).src_cid = self.guest_cid;
            (*hdr).dst_cid = self.peer_cid;
            (*hdr).src_port = self.local_port;
            (*hdr).dst_port = self.peer_port;
            (*hdr).len = 0;
            (*hdr).ty = VSOCK_TYPE_STREAM;
            (*hdr).op = VSOCK_OP_REQUEST;
            (*hdr).buf_alloc = RX_BUF_SIZE;
            (*hdr).fwd_cnt = 0;
        }

        let d0 = match self.tx.alloc_desc_head() {
            Some(h) => h,
            None => return false,
        };

        let tx_hdr_len = mem::size_of::<VirtioVsockHdr>() as u32;
        unsafe {
            let desc = &mut *self.tx.desc_ptr(d0);
            desc.addr = self.tx_buf_dma;
            desc.len = tx_hdr_len;
            desc.flags = 0;
            desc.next = 0;
        }

        self.tx.avail_add(d0);
        self.transport.notify_queue(VSOCK_TX_QUEUE);

        for _ in 0..2000000 {
            core::hint::spin_loop();
        }

        if let Some((_desc_id, _len)) = self.tx.get_used_buf() {
        } else {
            uart_puts("[Vsock] connect: TX not consumed\n");
            return false;
        }

        self.submit_rx_buf();
        self.transport.notify_queue(VSOCK_RX_QUEUE);

        for _ in 0..2000000 {
            core::hint::spin_loop();
        }

        if let Some((_desc_id, len)) = self.rx.get_used_buf() {
            let hdr = unsafe { &*(self.rx_buf_dma as *const VirtioVsockHdr) };
            let op = hdr.op;
            uart_puts("[Vsock] connect response: op=");
            uart_put_hex(op as u64);
            uart_puts(" len=");
            uart_put_hex(len as u64);
            uart_puts("\n");

            if op == VSOCK_OP_RESPONSE {
                self.connected = true;
                uart_puts("[Vsock] Connected!\n");
                return true;
            }
        }

        uart_puts("[Vsock] connect: no response\n");
        false
    }

    pub fn send(&mut self, data: &[u8]) -> bool {
        if !self.connected || data.is_empty() {
            return false;
        }

        let hdr_len = mem::size_of::<VirtioVsockHdr>() as u32;
        let pkt_len = data.len() as u32;
        if pkt_len > TX_BUF_SIZE - hdr_len {
            return false;
        }

        let hdr = self.tx_buf_dma as *mut VirtioVsockHdr;
        unsafe {
            ptr::write_bytes(hdr, 0, 1);
            (*hdr).src_cid = self.guest_cid;
            (*hdr).dst_cid = self.peer_cid;
            (*hdr).src_port = self.local_port;
            (*hdr).dst_port = self.peer_port;
            (*hdr).len = pkt_len;
            (*hdr).ty = VSOCK_TYPE_STREAM;
            (*hdr).op = VSOCK_OP_RW;
            (*hdr).buf_alloc = RX_BUF_SIZE;
            (*hdr).fwd_cnt = 0;

            let payload = (self.tx_buf_dma + hdr_len as u64) as *mut u8;
            ptr::copy_nonoverlapping(data.as_ptr(), payload, pkt_len as usize);
        }

        let d0 = match self.tx.alloc_desc_head() {
            Some(h) => h,
            None => return false,
        };
        let d1 = match self.tx.alloc_desc_head() {
            Some(h) => h,
            None => return false,
        };

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
        self.transport.notify_queue(VSOCK_TX_QUEUE);

        for _ in 0..1000000 {
            core::hint::spin_loop();
        }

        self.tx.get_used_buf().is_some()
    }

    pub fn recv(&mut self) -> Option<(&[u8], u32)> {
        if !self.connected {
            return None;
        }

        self.submit_rx_buf();
        self.transport.notify_queue(VSOCK_RX_QUEUE);

        for _ in 0..500000 {
            core::hint::spin_loop();
        }

        let (_desc_id, len) = self.rx.get_used_buf()?;

        let hdr = unsafe { &*(self.rx_buf_dma as *const VirtioVsockHdr) };

        if hdr.op == VSOCK_OP_RW {
            let hdr_len = mem::size_of::<VirtioVsockHdr>() as u32;
            let payload_len = hdr.len.min(len - hdr_len);
            let payload = unsafe {
                core::slice::from_raw_parts(
                    (self.rx_buf_dma + hdr_len as u64) as *const u8,
                    payload_len as usize,
                )
            };
            return Some((payload, hdr.src_port));
        }

        if hdr.op == VSOCK_OP_SHUTDOWN || hdr.op == VSOCK_OP_RST {
            uart_puts("[Vsock] connection closed by peer\n");
            self.connected = false;
        }

        None
    }
}
