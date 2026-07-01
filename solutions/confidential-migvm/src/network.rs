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
use crate::virtio::net::{VirtioNet, VirtioNetHdr, VIRTQ_DESC_F_NEXT};
use smoltcp::phy::{Device, DeviceCapabilities, RxToken, TxToken};
use smoltcp::time::Instant;

pub fn tick() {}

pub fn now() -> Instant {
    Instant::from_millis(crate::time::now_ms() as i64)
}

pub struct NetDevice {
    inner: VirtioNet,
    last_rx_desc: Option<u16>,
}

impl NetDevice {
    pub fn new(inner: VirtioNet) -> Self {
        NetDevice { inner, last_rx_desc: None }
    }

    pub fn inner(&self) -> &VirtioNet {
        &self.inner
    }

    pub fn recycle_last_rx(&mut self) {
        if let Some(idx) = self.last_rx_desc.take() {
            self.inner.recycle_rx_buf(idx);
        }
    }
}

pub struct VirtioRxToken {
    data: *const u8,
    len: usize,
}

impl RxToken for VirtioRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let slice = unsafe {
            core::slice::from_raw_parts_mut(self.data as *mut u8, self.len)
        };
        f(slice)
    }
}

pub struct VirtioTxToken {
    net: *mut VirtioNet,
}

impl TxToken for VirtioTxToken {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let net = unsafe { &mut *self.net };
        let hdr_len = core::mem::size_of::<VirtioNetHdr>();

        let d0: u16 = 0;
        let d1 = unsafe { (*net.tx.desc_ptr_pub(0)).next };

        unsafe {
            let hdr_ptr = net.tx_buf_dma as *mut VirtioNetHdr;
            core::ptr::write_bytes(hdr_ptr, 0, 1);
            (*hdr_ptr).flags = 0;
            (*hdr_ptr).gso_type = 0;
            (*hdr_ptr).hdr_len = 0;
            (*hdr_ptr).gso_size = 0;
            (*hdr_ptr).csum_start = 0;
            (*hdr_ptr).csum_offset = 0;

            let pkt = core::slice::from_raw_parts_mut(
                (net.tx_buf_dma + hdr_len as u64) as *mut u8,
                len,
            );
            let result = f(pkt);

            let desc0 = &mut *net.tx.desc_ptr_pub(d0);
            desc0.addr = net.tx_buf_dma;
            desc0.len = hdr_len as u32;
            desc0.flags = VIRTQ_DESC_F_NEXT;
            desc0.next = d1;

            let desc1 = &mut *net.tx.desc_ptr_pub(d1);
            desc1.addr = net.tx_buf_dma + hdr_len as u64;
            desc1.len = len as u32;
            desc1.flags = 0;
            desc1.next = 0;

            core::arch::asm!("dsb st", options(nomem, nostack));
            net.tx.avail_add(d0);
            net.transport.notify_queue(1);

            for _ in 0..5000000 {
                core::hint::spin_loop();
            }
            net.tx.get_used_buf();
            result
        }
    }
}

impl Device for NetDevice {
    type RxToken<'a> = VirtioRxToken;
    type TxToken<'a> = VirtioTxToken;

    fn receive(
        &mut self,
        _timestamp: Instant,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let (desc_idx_u32, pkt_len) = self.inner.rx.get_used_buf()?;
        let desc_idx = desc_idx_u32 as u16;

        let buf = self.inner.rx_bufs[desc_idx as usize].take()?;

        let data =
            (buf.dma() + core::mem::size_of::<VirtioNetHdr>() as u64) as *const u8;

        let data_len = pkt_len as usize - core::mem::size_of::<VirtioNetHdr>();

        self.inner.rx_bufs[desc_idx as usize] = Some(buf);
        self.last_rx_desc = Some(desc_idx);

        let rx = VirtioRxToken { data, len: data_len };
        let tx = VirtioTxToken {
            net: &mut self.inner as *mut VirtioNet,
        };
        Some((rx, tx))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(VirtioTxToken {
            net: &mut self.inner as *mut VirtioNet,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = 1500;
        caps.max_burst_size = Some(1);
        caps
    }
}

pub fn run_dhcp_and_keep(net: VirtioNet, max_ticks: usize) -> Option<(smoltcp::wire::Ipv4Address, crate::virtio::net::VirtioNet)> {
    use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
    use smoltcp::socket::dhcpv4::Socket as Dhcpv4Socket;
    use smoltcp::wire::{EthernetAddress, HardwareAddress};

    let mut device = NetDevice::new(net);

    let hw_addr = EthernetAddress(device.inner().mac);
    let mut config = Config::new(HardwareAddress::Ethernet(hw_addr));
    config.random_seed = crate::trng::u64_seed();

    let mut iface = Interface::new(config, &mut device, now());

    let dhcp_socket = Dhcpv4Socket::new();
    let mut socket_storage = [SocketStorage::EMPTY; 4];
    let mut sockets = SocketSet::new(&mut socket_storage[..]);
    let dhcp_handle = sockets.add(dhcp_socket);
    let mut rx_count: u64 = 0;

    for i in 0..max_ticks {
        tick();
        let t = now();

        let had_rx = device.inner.rx.poll_used_imm() != device.inner.rx.last_used_idx;
        if had_rx {
            rx_count += 1;
            if rx_count <= 5 {
                uart_puts("[Net] RX packet #");
                uart_put_hex(rx_count);
                uart_puts("\n");
            }
        }

        iface.poll(t, &mut device, &mut sockets);

        let event = {
            let dhcp = sockets.get_mut::<Dhcpv4Socket>(dhcp_handle);
            if i == 0 {
                dhcp.reset();
            }
            dhcp.poll()
        };

        match event {
            Some(smoltcp::socket::dhcpv4::Event::Configured(config)) => {
                let ip_addr = config.address.address();
                let b = ip_addr.as_bytes();
                uart_puts("\n[Net] DHCP OK: ");
                uart_put_hex(b[0] as u64);
                uart_puts(".");
                uart_put_hex(b[1] as u64);
                uart_puts(".");
                uart_put_hex(b[2] as u64);
                uart_puts(".");
                uart_put_hex(b[3] as u64);
                uart_puts("\n");

                if let Some(router) = config.router {
                    let rb = router.as_bytes();
                    uart_puts("[Net] Router: ");
                    uart_put_hex(rb[0] as u64);
                    uart_puts(".");
                    uart_put_hex(rb[1] as u64);
                    uart_puts(".");
                    uart_put_hex(rb[2] as u64);
                    uart_puts(".");
                    uart_put_hex(rb[3] as u64);
                    uart_puts("\n");
                }

                // Drop iface/sockets to release device borrow, extract inner
                drop(sockets);
                drop(iface);
                return Some((ip_addr, device.inner));
            }
            Some(smoltcp::socket::dhcpv4::Event::Deconfigured) => {
                if i == 0 {
                    uart_puts("[Net] DHCP init, waiting for server...\n");
                }
            }
            None => {}
        }
    }

    uart_puts("[Net] DHCP timeout (");
    uart_put_hex(rx_count);
    uart_puts(" RX pkts)\n");
    None
}

pub fn start_migration_service(net: VirtioNet, vsock_dev: Option<crate::virtio::vsock::VsockDevice>) -> ! {
    let dhcp_result = run_dhcp_and_keep(net, 600000);

    match dhcp_result {
        Some((ip_addr, net)) => service_with_tcp(net, vsock_dev, ip_addr),
        None => service_vsock_only(vsock_dev),
    }
}

fn service_vsock_only(vsock_dev: Option<crate::virtio::vsock::VsockDevice>) -> ! {
    uart_puts("[Service] DHCP failed, TCP disabled, net device lost\n");
    let mut vs = vsock_dev;
    if let Some(ref mut v) = vs {
        v.connect();
    }
    let mut vpc: u64 = 0;
    let mut last_ms: u64 = 0;
    loop {
        tick();
        vpc += 1;
        if vpc >= 500 {
            vpc = 0;
            if let Some(ref mut v) = vs {
                if v.connected {
                    if let Some((_d, _p)) = v.recv() {
                        uart_puts("[VSOCK] received data\n");
                    }
                }
            }
        }

        let cur_ms = crate::time::now_ms();
        if cur_ms >= 30000 {
            break;
        }
        if last_ms != cur_ms && cur_ms % 2000 == 0 {
            last_ms = cur_ms;
            uart_puts("[Service] ms=");
            uart_put_hex(cur_ms);
            uart_puts("\n");
        }
    }
    uart_puts("[Service] ms=");
    uart_put_hex(crate::time::now_ms());
    uart_puts(" halt\n");
    loop {
        core::hint::spin_loop();
    }
}

fn service_with_tcp(
    net: VirtioNet,
    vsock_dev: Option<crate::virtio::vsock::VsockDevice>,
    ip_addr: smoltcp::wire::Ipv4Address,
) -> ! {
    use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
    use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpBuf};
    use smoltcp::wire::{EthernetAddress, HardwareAddress};
    use alloc::vec::Vec;

    let mut device = NetDevice::new(net);
    let hw_addr = EthernetAddress(device.inner().mac);
    let mut config = Config::new(HardwareAddress::Ethernet(hw_addr));
    config.random_seed = crate::trng::u64_seed();
    let mut iface = Interface::new(config, &mut device, now());
    device.inner().transport.notify_queue(0);

    let mut t1r = Vec::new(); t1r.resize(4096, 0u8);
    let mut t1w = Vec::new(); t1w.resize(4096, 0u8);
    let mut t2r = Vec::new(); t2r.resize(4096, 0u8);
    let mut t2w = Vec::new(); t2w.resize(4096, 0u8);
    let tc1 = TcpSocket::new(TcpBuf::new(&mut t1r[..]), TcpBuf::new(&mut t1w[..]));
    let tc2 = TcpSocket::new(TcpBuf::new(&mut t2r[..]), TcpBuf::new(&mut t2w[..]));
    let mut ss = [SocketStorage::EMPTY; 4];
    let mut sockets = SocketSet::new(&mut ss[..]);
    let t1h = sockets.add(tc1);
    let t2h = sockets.add(tc2);
    sockets.get_mut::<TcpSocket>(t1h).listen(5001u16).ok();
    sockets.get_mut::<TcpSocket>(t2h).listen(5002u16).ok();

    uart_puts("[Service] TCP listening on ");
    let b = ip_addr.as_bytes();
    uart_put_hex(b[0] as u64); uart_puts(".");
    uart_put_hex(b[1] as u64); uart_puts(".");
    uart_put_hex(b[2] as u64); uart_puts(".");
    uart_put_hex(b[3] as u64);
    uart_puts(":5001 + :5002\n");

    let mut vsock = vsock_dev;
    if let Some(ref mut vs) = vsock {
        uart_puts("[Service] Vsock connecting to CID:2 port:4052\n");
        if vs.connect() {
            uart_puts("[Service] Vsock connected\n");
        } else {
            uart_puts("[Service] Vsock connect failed\n");
            vsock = None;
        }
    }

    let mut ebuf = [0u8; 2048];
    let mut vpc: u64 = 0;
    let mut last_ms: u64 = 0;
    uart_puts("[Service] === Entering event loop ===\n");

    loop {
        tick();
        let t = now();
        iface.poll(t, &mut device, &mut sockets);

        {
            let s = sockets.get_mut::<TcpSocket>(t1h);
            if s.can_recv() {
                let mut n: usize = 0;
                let _ = s.recv(|d| {
                    n = d.len().min(ebuf.len());
                    ebuf[..n].copy_from_slice(&d[..n]);
                    (n, ())
                });
                if n > 0 {
                    uart_puts("[TCP:5001] "); uart_put_hex(n as u64); uart_puts("b\n");
                    let _ = s.send(|d| {
                        let x = n.min(d.len());
                        d[..x].copy_from_slice(&ebuf[..x]);
                        (x, ())
                    });
                }
            } else if !s.is_open() {
                let _ = s.listen(5001u16);
            }
        }

        {
            let s = sockets.get_mut::<TcpSocket>(t2h);
            if s.can_recv() {
                let mut n: usize = 0;
                let _ = s.recv(|d| {
                    n = d.len().min(ebuf.len());
                    ebuf[..n].copy_from_slice(&d[..n]);
                    (n, ())
                });
                if n > 0 {
                    uart_puts("[TCP:5002] "); uart_put_hex(n as u64); uart_puts("b\n");
                    let _ = s.send(|d| {
                        let x = n.min(d.len());
                        d[..x].copy_from_slice(&ebuf[..x]);
                        (x, ())
                    });
                }
            } else if !s.is_open() {
                let _ = s.listen(5002u16);
            }
        }

        vpc += 1;
        if vpc >= 500 {
            vpc = 0;
            if let Some(ref mut vs) = vsock {
                if vs.connected {
                    if let Some((_d, _p)) = vs.recv() {
                        uart_puts("[VSOCK] received data\n");
                    }
                }
            }
        }

        let cur_ms = crate::time::now_ms();
        if cur_ms >= 30000 {
            break;
        }
        if last_ms != cur_ms && cur_ms % 2000 == 0 {
            last_ms = cur_ms;
            uart_puts("[Service] ms=");
            uart_put_hex(cur_ms);
            uart_puts("\n");
        }
    }
    uart_puts("[Service] ms=");
    uart_put_hex(crate::time::now_ms());
    uart_puts(" halt\n");
    loop {
        core::hint::spin_loop();
    }
}
