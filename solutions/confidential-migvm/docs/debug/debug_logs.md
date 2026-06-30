# Debug Log 参考

> 各种环境下的完整启动输出日志，用于回归对比。

## 日志文件路径

| 模式 | 日志文件 |
|------|---------|
| KVM + GICv3 | `/tmp/migvm_kvm_gicv3.log` |
| TCG + GICv3 | `/tmp/migvm_tcg_gicv3.log` |
| VirtCCA + KVM + GICv3 | `/tmp/migvm_virtcca_vsock.log` |

## KVM + GICv3（QEMU 虚拟机）

```
=== MigVM AArch64 Boot ===
FDT ptr: 0x0000000048000000
Current EL: 0x0000000000000001
[OK] Heap initialized (7MB)
INFO: Logger initialized
[OK] Logger initialized
[FDT] Parsed device tree:
[FDT]   Memory @ 0x40000000, size=0x40000000 (1024MB)
[FDT]   GICv3: GICD=0x08000000 (0x10000), GICR=0x080a0000 (0xf60000)
[FDT]   PCIe ECAM @ 0x4010000000 ...
...
[OK] FDT parsed
[GIC] FDT-provided: GICv3 GICD=0x08000000 GICR=0x080A0000
[GICv3] Initializing...
[GICv3] Distributor init...
[GIC] MIDR_EL1=0x480FD020
[GIC] Unknown/virtual CPU, assuming VGIC present
[GICv3] VGIC detected, skipping Distributor reconfiguration
[GICv3] VGIC: IGROUPR0 before=0x0 after=0xFFFFFFFF
[GICv3] ICC_IGRPEN0_EL1=1 (Group 0 also enabled)
[GICv3] ICC_IGRPEN1_EL1=0x1
[GICv3] ICC_CTLR_EL1 EOI mode set to drop (mode 1)
[GICv3] CPU interface initialized
[GICv3] Initialized
[OK] GIC initialized
[OK] Memory map initialized
[Time] CNTFRQ=0x5F5E100 (100MHz) CNTVCT=0x16A294 [Time] Poll-based tick started (1ms)
[OK] Timer/System tick initialized
[VM] Running in normal VM mode
INFO: MigVM AArch64 booted successfully

=== Boot Complete ===

[VirtIO Net PCI] TX test: OK
[Net] DHCP OK: 10.0.2.15
...
[Service] === Entering event loop ===
[Service] ms=0x07D0     ← 2000ms
[Service] ms=0x0FA0     ← 4000ms
...
[Service] ms=0x7530 halt  ← 30000ms
```

## TCG + GICv3（QEMU 虚拟机）

```
=== MigVM AArch64 Boot ===
...
[Service] TCP listening on 10.0.2.15:5001 + :5002
[Service] === Entering event loop ===
...
```

## VirtCCA + KVM + GICv3（✅ 真实硬件已验证，2026-06-04）

```
=== MigVM AArch64 Boot ===
FDT ptr: 0x0000000048000000
Current EL: 0x0000000000000001
[OK] Heap initialized (7MB)
[OK] Logger initialized
[FDT] Parsed device tree:
[FDT]   Memory @ 0x40000000, size=0x40000000 (1024MB)
[FDT]   GICv3: GICD=0x08000000 (0x10000), GICR=0x080a0000 (0xf60000)
...
[OK] FDT parsed
[GIC] FDT-provided: GICv3 GICD=0x08000000 GICR=0x080A0000
[GIC] MIDR_EL1=0x480FD020
[GIC] Unknown/virtual CPU, assuming VGIC present
[GICv3] VGIC detected, skipping Distributor reconfiguration
[GICv3] Initialized
[OK] GIC initialized
[OK] Memory map initialized
[VirtCCA] Running in VirtCCA guest mode
[VirtCCA] TSI version: 0x00010000
[VirtCCA] IPA bits=0x2B algo=0x0
[VirtCCA] DMA shared pool initialized
[VirtCCA] DMA pool PTEs marked NS (bit 5)
[VirtCCA] tsi_sec_mem_unmap notify = 0xC400019C
INFO: MigVM AArch64 booted successfully

=== Boot Complete ===

[PCI] ECAM @ 0x4010000000
[PCI] Found 0x3 devices
[PCI] VirtIO-net (modern) at 0:1
[VirtIO Net PCI] TX test: OK
[VirtIO Net PCI] MAC: 52:54:00:12:34:56
[PCI] VirtIO-net init OK
[PCI] VirtIO-vsock (modern) at 0:2
[VirtIO Vsock PCI] Guest CID: 3
[PCI] VirtIO-vsock init OK
```

### VirtCCA TSI 接口探测（2026-06-05 最终验证）

```
[TSI] === Probing TSI interfaces ===
[TSI] A.1 tsi_attestation_token_init (static challenge, x4..x7=0):
[TSI]     chal_ipa=0x4009B0C0
[TSI]     rc=0x0000000000000000 token_ub=0x0000000000001000
[TSI] A.2 tsi_attestation_token_continue:
[TSI]     struct_pa 0x4009D000 buf_ipa=0x4009E000
[TSI]     rc=0x0000000000000000 nbytes=0x0000000000000591
[TSI]     token_head=(1425 bytes CBOR/COSE)
[TSI] B. tsi_device_cert:
[TSI]    rc=0x0000000000000000 size=0x00000000000003DC
[TSI] C. tsi_peek_binding_list:
[TSI]    rc=0x0000000000000002 (expected: no binding)
[TSI] D. MigVM/Checksum (all expected fail with rd=0):
[TSI]    get_attr rc=0x1 set_slot rc=0x1 checksum_init rc=0x1 checksum_loop rc=0x1
[TSI] E. All measurement slots:
[TSI]    slot=0x0000000000000000 rc=0x0000000000000000 RIM=0x44...
...
[TSI] === TSI probe done ===
```

## 日志文件维护说明

| 日期 | 变更 |
|------|------|
| 2026-06-05 | 添加 TRNG ChaCha20 + MMU 4KB 页后的运行输出 |
| 2026-06-04 | VirtCCA TSI 11/11 全通过输出 |
| 2026-06-04 | Timer Poll 驱动精度验证输出 |
