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

#![no_std]
#![no_main]

extern crate alloc;

use migvm::uart_puts;
use migvm::uart_put_hex;

const HEAP_SIZE: usize = 0x70_0000;

static mut FDT_INFO: Option<migvm::fdt::FdtInfo> = None;

#[no_mangle]
pub extern "C" fn _start_rust_impl(fdt_ptr: u64, _reserved: u64) -> ! {
    uart_puts("\n=== MigVM AArch64 Boot ===\n");

    uart_puts("FDT ptr: ");
    uart_put_hex(fdt_ptr);
    uart_puts("\n");

    let current_el: u64;
    unsafe {
        core::arch::asm!("mrs {}, CurrentEL", out(reg) current_el);
    }
    uart_puts("Current EL: ");
    uart_put_hex(current_el >> 2);
    uart_puts("\n");

    migvm::mm::heap::init_heap(HEAP_SIZE);
    uart_puts("[OK] Heap initialized (7MB)\n");

    migvm::logger::init_logger();
    log::info!("Logger initialized");
    uart_puts("[OK] Logger initialized\n");

    match migvm::fdt::parse_fdt(fdt_ptr) {
        Ok(info) => {
            migvm::fdt::print_fdt_info(&info);
            unsafe { FDT_INFO = Some(info) };
            uart_puts("[OK] FDT parsed\n");
        }
        Err(e) => {
            uart_puts("[FDT] Parse failed: ");
            uart_puts(e);
            uart_puts("\n");
        }
    }

    unsafe {
        if let Some(ref info) = FDT_INFO {
            if let Some(ref gic) = info.gic {
                migvm::arch::apic::init_gic_from_fdt(gic.gicd_base, gic.gicr_base, gic.version);
            } else {
                migvm::arch::apic::init_gic(fdt_ptr);
            }
        } else {
            migvm::arch::apic::init_gic(fdt_ptr);
        }
    }
    uart_puts("[OK] GIC initialized\n");

    unsafe {
        if let Some(ref info) = FDT_INFO {
            migvm::mm::init_memory_map_from_fdt(&info.memory);
        } else {
            migvm::mm::init_memory_map();
        }
    }
    uart_puts("[OK] Memory map initialized\n");

    migvm::time::init_sys_tick();
    uart_puts("[OK] Timer/System tick initialized\n");

    #[cfg(feature = "cca")]
    {
        if migvm::rsi::is_realm_world() {
            uart_puts("[CCA] Running in Realm (CCA guest) mode\n");
            uart_puts("[CCA] PROT_NS_SHARED = ");
            uart_put_hex(migvm::rsi::prot_ns_shared());
            uart_puts("\n");

            migvm::mm::accept_all_ram();
            uart_puts("[CCA] RAM memory accepted\n");

            migvm::mm::init_dma_shared_pool(0x40000000, 0x40000000);
        } else {
            uart_puts("[VM] Running in normal VM mode (no CCA)\n");
            migvm::virtio::init_dma_pool_normal();
        }
    }

    #[cfg(feature = "virtcca")]
    {
        if migvm::tsi::is_virtcca_world() {
            uart_puts("[VirtCCA] Running in VirtCCA guest mode\n");
            uart_puts("[VirtCCA] TSI version: 0x");
            uart_put_hex(migvm::tsi::tsi_version());
            uart_puts("\n");

            if let Some(cfg) = migvm::tsi::tsi_get_realm_config() {
                uart_puts("[VirtCCA] IPA bits=");
                uart_put_hex(cfg.ipa_bits);
                uart_puts(" algo=");
                uart_put_hex(cfg.algorithm);
                uart_puts("\n");
            }

            migvm::virtio::init_dma_pool_shared(0x70000000, 0x200000);
            uart_puts("[VirtCCA] DMA shared pool initialized\n");

            migvm::mm::paging::mark_range_shared_virtcca(0x70000000, 0x200000);
            uart_puts("[VirtCCA] DMA pool PTEs marked NS (bit 5)\n");

            #[cfg(feature = "virtcca")]
            probe_tsi_interfaces();

            let rc = migvm::tsi::tsi_sec_mem_unmap(0x70000000, 0x200000);
            uart_puts("[VirtCCA] tsi_sec_mem_unmap notify = 0x");
            uart_put_hex(rc);
            uart_puts("\n");
        } else {
            uart_puts("[VM] Running in normal VM mode (no VirtCCA)\n");
            migvm::virtio::init_dma_pool_normal();
        }
    }

    #[cfg(not(any(feature = "cca", feature = "virtcca")))]
    {
        uart_puts("[VM] Running in normal VM mode\n");
        migvm::virtio::init_dma_pool_normal();
    }

    log::info!("MigVM AArch64 booted successfully");
    uart_puts("\n=== Boot Complete ===\n");

    unsafe {
        if let Some(ref info) = FDT_INFO {
            if !info.virtio_devices.is_empty() {
                let mut devices = migvm::virtio::device::probe_devices(&info.virtio_devices);
                if !devices.is_empty() {
                    migvm::virtio::device::test_init_devices(&mut devices);
                }
            } else {
                uart_puts("[VirtIO] No MMIO devices found in FDT\n");
            }
        }
    }
    uart_puts("[OK] VirtIO MMIO probe done\n");

    unsafe {
        if let Some(ref info) = FDT_INFO {
            if let Some(ref pcie) = info.pcie {
                uart_puts("\n[PCI] ECAM @ 0x");
                uart_put_hex(pcie.ecam_base);
                uart_puts(" size=");
                uart_put_hex(pcie.ecam_size);
                uart_puts("\n");

                migvm::pci::init_ecam(pcie.ecam_base);
                let devices = migvm::pci::enumerate_bus(0);

                uart_puts("[PCI] Found ");
                uart_put_hex(devices.len() as u64);
                uart_puts(" devices\n");

                let mut net_dev: Option<migvm::virtio::net::VirtioNet> = None;
                let mut vsock_dev: Option<migvm::virtio::vsock::VsockDevice> = None;

                for dev in &devices {
                    migvm::pci::print_device(dev);

                    if dev.vendor_id == 0x1AF4 && dev.device_id >= 0x1000 && dev.device_id <= 0x107F {
                        let subsystem = dev.device_id - 0x1040;
                        let name = match subsystem {
                            1 => "net",
                            2 => "block",
                            3 => "console",
                            4 => "entropy",
                            9 => "9p",
                            _ => "virtio",
                        };
                        uart_puts("[PCI] VirtIO-");
                        uart_puts(name);
                        uart_puts(" (modern) at ");
                        uart_put_hex(dev.bus as u64);
                        uart_puts(":");
                        uart_put_hex(dev.dev as u64);
                        uart_puts("\n");

                        if subsystem == 1 {
                            if let Some(transport) = migvm::virtio::virtio_pci::VirtioPciTransport::init(dev, pcie.mmio_base, 0x60000000) {
                                if let Some(net) = migvm::virtio::net::VirtioNet::init_pci(transport) {
                                    uart_puts("[PCI] VirtIO-net init OK\n");
                                    net_dev = Some(net);
                                }
                            }
                        }

                        if subsystem == 19 {
                            let vsock_mmio = pcie.mmio_base + 0x1000000;
                            if let Some(transport) = migvm::virtio::virtio_pci::VirtioPciTransport::init(dev, vsock_mmio, 0x61000000) {
                                if let Some(vs) = migvm::virtio::vsock::VsockDevice::init_pci(transport, 2, 4052) {
                                    uart_puts("[PCI] VirtIO-vsock init OK\n");
                                    vsock_dev = Some(vs);
                                }
                            }
                        }
                    }
                }

                if let Some(net) = net_dev {
                    migvm::network::start_migration_service(net, vsock_dev);
                } else {
                    uart_puts("[PCI] No VirtIO-net device found\n");
                }
            } else {
                uart_puts("[PCI] No PCIe controller found in FDT\n");
            }
        }
    }
    uart_puts("[OK] PCI probe done\n");

    uart_puts("\nHello World from MigVM AArch64!\n");

    uart_puts("[Service] Starting hot-migration remote attestation key exchange service...\n");
    run_migration_service();
}

fn run_migration_service() -> ! {
    let mut loop_count: u64 = 0;
    loop {
        loop_count += 1;
        uart_puts("[Service] loop #");
        uart_put_hex(loop_count);
        uart_puts(" ms=");
        uart_put_hex(migvm::time::now_ms());
        uart_puts("\n");
        migvm::time::wait_ms(1000);
    }
}

#[cfg(feature = "virtcca")]
fn probe_tsi_interfaces() {
    use migvm::tsi;
    use core::arch::asm;

    #[repr(C, align(4096))]
    struct AlignedTokenBuf([u8; 0x2000]);

    #[repr(C, align(8))]
    struct TokenGranuleBuf {
        granule: tsi::TokenGranule,
        buf: AlignedTokenBuf,
    }

    static mut TOKEN_GRANULE_BUF: TokenGranuleBuf = TokenGranuleBuf {
        granule: tsi::TokenGranule {
            ipa: 0, head: 0, offset: 0, size: 0, num_wr_bytes: 0, count: 0,
        },
        buf: AlignedTokenBuf([0u8; 0x2000]),
    };
    static mut DEV_CERT_BUF: [u8; tsi::MAX_DEV_CERT_SIZE] = [0u8; tsi::MAX_DEV_CERT_SIZE];
    static mut MIGVM_BUF: [u64; 256] = [0u64; 256];
    static mut CHALLENGE_BUF: [u8; tsi::CHALLENGE_SIZE] = [0xFFu8; tsi::CHALLENGE_SIZE];

    uart_puts("\n[TSI] === Probing TSI interfaces ===\n");

    // === Token flow ===
    uart_puts("[TSI] A.1 tsi_attestation_token_init (static challenge, x4..x7=0):\n");
    let chal_ipa = unsafe { CHALLENGE_BUF.as_ptr() as u64 };
    uart_puts("[TSI]     chal_ipa=0x");
    uart_put_hex(chal_ipa);
    uart_puts("\n");

    unsafe { asm!("dsb sy"); }
    let (rc_init, token_ub) = tsi::tsi_attestation_token_init_raw(chal_ipa);
    unsafe { asm!("dsb sy"); }
    uart_puts("[TSI]     rc=");
    uart_put_hex(rc_init);
    uart_puts(" token_ub=");
    uart_put_hex(token_ub);
    uart_puts("\n");

    if rc_init == tsi::TSI_SUCCESS {
        let buf_ipa = unsafe { TOKEN_GRANULE_BUF.buf.0.as_ptr() as u64 };
        let granule_ptr: *mut tsi::TokenGranule = unsafe { core::ptr::addr_of_mut!(TOKEN_GRANULE_BUF.granule) };
        uart_puts("[TSI] A.2 tsi_attestation_token_continue:\n");
        uart_puts("[TSI]     struct_pa 0x");
        uart_put_hex(granule_ptr as u64);
        uart_puts(" buf_ipa=0x");
        uart_put_hex(buf_ipa);
        uart_puts("\n");

        unsafe {
            core::ptr::write_bytes(TOKEN_GRANULE_BUF.buf.0.as_mut_ptr(), 0, 0x2000);
            core::ptr::write_bytes(granule_ptr as *mut u8, 0, core::mem::size_of::<tsi::TokenGranule>());
            (*granule_ptr).ipa = buf_ipa;
            (*granule_ptr).head = buf_ipa;
            (*granule_ptr).offset = 0;
            (*granule_ptr).size = 0x1000;
            (*granule_ptr).num_wr_bytes = 0;
            (*granule_ptr).count = 0;
        }
        unsafe { asm!("dsb sy"); }

        let rc = tsi::tsi_attestation_token_continue(unsafe { &mut *granule_ptr });
        unsafe { asm!("dsb sy"); }
        let nbytes = unsafe { (*granule_ptr).num_wr_bytes };
        uart_puts("[TSI]     rc=");
        uart_put_hex(rc);
        uart_puts(" nbytes=");
        uart_put_hex(nbytes);
        uart_puts("\n");

        if rc == tsi::TSI_SUCCESS || rc == tsi::TSI_INCOMPLETE {
            let buf = unsafe { &TOKEN_GRANULE_BUF.buf.0 };
            let n = (nbytes as usize).min(64);
            uart_puts("[TSI]     token_head=");
            for i in 0..n { uart_put_hex(buf[i] as u64); }
            uart_puts("\n");
        }
        // Follow kernel: loop while INCOMPLETE
        let mut cur_rc = rc;
        unsafe {
            let max_granules = tsi::MAX_TOKEN_GRANULE_COUNT as u64;
            while cur_rc == tsi::TSI_INCOMPLETE && (*granule_ptr).count < max_granules {
                (*granule_ptr).count += 1;
                (*granule_ptr).ipa = (*granule_ptr).head + (*granule_ptr).count as u64 * 0x1000;
                (*granule_ptr).offset = 0;
                asm!("dsb sy");
                cur_rc = tsi::tsi_attestation_token_continue(&mut *granule_ptr);
                asm!("dsb sy");
                uart_puts("[TSI]     granule#");
                uart_put_hex((*granule_ptr).count as u64);
                uart_puts(" rc=");
                uart_put_hex(cur_rc);
                uart_puts(" nbytes=");
                uart_put_hex((*granule_ptr).num_wr_bytes);
                uart_puts("\n");
            }
        }
    } else {
        uart_puts("[TSI]     skip continue (init failed)\n");
    }

    // === Remaining interfaces ===
    uart_puts("[TSI] B. tsi_device_cert:\n");
    {
        let cert_ipa = unsafe { DEV_CERT_BUF.as_ptr() as u64 };
        let rc = tsi::tsi_device_cert_raw(cert_ipa, tsi::MAX_DEV_CERT_SIZE as u64);
        uart_puts("[TSI]    rc=");
        uart_put_hex(rc.x0);
        uart_puts(" size=");
        uart_put_hex(rc.x1);
        uart_puts("\n");
    }

    uart_puts("[TSI] C. tsi_peek_binding_list:\n");
    {
        let list_ipa = unsafe { MIGVM_BUF.as_ptr() as u64 };
        let rc = tsi::tsi_peek_binding_list(list_ipa);
        uart_puts("[TSI]    rc=");
        uart_put_hex(rc);
        uart_puts(" (expected: no binding)\n");
    }

    uart_puts("[TSI] D. MigVM/Checksum (all expected fail with rd=0):\n");
    let buf_ipa = unsafe { MIGVM_BUF.as_ptr() as u64 };
    uart_puts("[TSI]    get_attr rc=");
    uart_put_hex(tsi::tsi_migvm_get_attr(0, buf_ipa));
    uart_puts(" set_slot rc=");
    uart_put_hex(tsi::tsi_migvm_set_slot(0, buf_ipa));
    uart_puts(" checksum_init rc=");
    uart_put_hex(tsi::tsi_mig_integrity_checksum_init(0, 0x70000000));
    uart_puts(" checksum_loop rc=");
    uart_put_hex(tsi::tsi_mig_integrity_checksum_loop(0, 0));
    uart_puts("\n");

    uart_puts("[TSI] E. All measurement slots:\n");
    for slot in 0..tsi::MEASUREMENT_SLOT_NR {
        let mut mbuf = [0u8; tsi::MAX_MEASUREMENT_SIZE];
        let rc = tsi::tsi_measurement_read(slot, &mut mbuf);
        uart_puts("[TSI]    slot=");
        uart_put_hex(slot);
        uart_puts(" rc=");
        uart_put_hex(rc);
        if rc == tsi::TSI_SUCCESS && mbuf.iter().any(|b| *b != 0) {
            uart_puts(" RIM=");
            for i in 0..4 { uart_put_hex(mbuf[i] as u64); }
            uart_puts("...");
        }
        uart_puts("\n");
    }

    uart_puts("[TSI] === TSI probe done ===\n\n");
}
