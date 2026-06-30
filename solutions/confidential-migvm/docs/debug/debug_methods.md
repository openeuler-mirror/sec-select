# 调试方法论与参考

> 关键技术问题排查记录和调试技巧汇总。

## 调试方法论

### QEMU 源码追踪

在 QEMU `hw/virtio/virtio-pci.c` 中追踪 Common Config read/write handler：

```c
// virtio_pci_common_read() — 每个 case 对应一个寄存器偏移
case VIRTIO_PCI_COMMON_Q_NOFF:  // 0x1E
    val = vdev->queue_sel;      // ← 关键：返回 queue_sel，不是硬编码值
    break;
```

`vdev->queue_sel` 由 `Q_SELECT` 写入 handler 更新，但只在 Guest 写到**正确偏移**时才执行。通过阅读 QEMU 源码直接定位到寄存器偏移错误。

### ESR/FAR 异常解码

```
ESR: 0x96000061  →  EC=0b100101 (Data Abort from lower EL)
                   DFSC=0b100001 (Alignment Fault)
FAR: 0x60003001  →  故障地址（奇数，确认是 strh 对齐问题）
```

### VirtIO PCI Common Config 寄存器对照表

```
偏移  宽度   寄存器          访问方式
0x14  8-bit  STATUS          读/写
0x15  8-bit  CFGGENERATION   只读
0x16  16-bit Q_SELECT        读/写
0x18  16-bit Q_SIZE          读/写
0x1A  16-bit Q_MSIX          读/写
0x1C  16-bit Q_ENABLE        读/写
0x1E  16-bit Q_NOFF          只读
0x20  32-bit Q_DESCLO        读/写
0x24  32-bit Q_DESCHI        读/写
0x28  32-bit Q_AVAILLO       读/写
0x2C  32-bit Q_AVAILHI       读/写
0x30  32-bit Q_USEDLO        读/写
0x34  32-bit Q_USEDHI        读/写
```

## 关键技术问题及解决

### 1. EL2→EL1 降级

QEMU `-machine virt` 默认从 EL2 启动内核。需要通过配置 `spsr_el2` 和 `elr_el2`，然后执行 `eret` 指令降到 EL1。

### 2. CPACR_EL1 FP/SIMD 陷阱

Rust 编译器会使用 SIMD 指令，但 aarch64 默认禁用 FP/SIMD 访问。需要在进入 Rust 代码前启用 CPACR_EL1 的 FPEN 位：

```asm
mrs x5, cpacr_el1
orr x5, x5, #0x300000   // FPEN = 0b11 (enable FP/SIMD)
msr cpacr_el1, x5
```

### 3. MMU 页表描述符格式（关键修复）

ARMv8 Block entry 的 bit 1 必须为 0（Table entry 才为 1）。错误地将 Block entry 的 bit 1 设为 1 会导致 Translation Fault。

| 描述符类型 | bit 0 (Valid) | bit 1 |
|-----------|---------------|-------|
| Invalid   | 0             | x     |
| Block     | 1             | **0** |
| Table     | 1             | **1** |

修正：L1 Block `0x611` → `0x610`，L2 Block `0x703` → `0x701`

### 4. MAIR_EL1 设备属性

MAIR_EL1 中 Attr2 必须为 0x04（Device-nGnRE），否则设备内存访问失败。

```
错误: MAIR_EL1 = 0x0000_0000_0000_FF44  (Attr2=0x00)
正确: MAIR_EL1 = 0x0000_0000_0004_FF44  (Attr2=0x04)
```

### 5. VirtIO Common Config 寄存器偏移错误

`write_queue_select` 写到 `0x14`（STATUS 寄存器）而非 `0x16`（Q_SELECT）。每个寄存器独立按正确宽度访问：
- STATUS: 8-bit @ 0x14
- Q_SELECT: 16-bit @ 0x16
- Q_SIZE: 16-bit @ 0x18
- Q_ENABLE: 16-bit @ 0x1C
- Q_NOFF: 16-bit @ 0x1E

### 6. Notify 对齐 + multiplier

`queue_notify_off=1`，notify 地址 `0x60003001`（奇数），`strh` 触发 Alignment Fault。改为 `strb`。

QEMU `virtio_pci_notify_write` 计算 `queue = addr / 4`，每个 queue 占 4 字节。需要 `addr = base + off * multiplier`（multiplier=4）。

### 7. used_ring_ptr 偏移量错误

`used` 内存布局为 `flags(2B) + idx(2B) + ring[]`，ring 在 offset 4。`ptr.add(1)` 在 `*mut VirtqUsedElem` 上加 8 字节，需要改为 `(self.used as *mut u8).add(4)`。

### 8. GICv3 初始化 SYNC Exception

KVM 模式下 GIC 初始化触发 SYNC exception（ESR=0x9600010）。

根因：GICR_SGI_OFFSET 错误（应为 0x10000），Redistributor 未初始化，CPU Interface 未配置，Device 区域未覆盖 Redistributor 地址空间。

### 9. GIC 版本检测兼容性

使用 `GICD_PIDR2`（偏移 0xFFE8）在 GICv2 下触发 Translation fault。改用 `GICD_TYPER` 的 LPIS 位（bit 17）：GICv2=0, GICv3=1。

### 10. KVM VGIC 兼容性

KVM VGIC 拦截 Distributor/Redistributor 寄存器写入。最终方案：基于 MIDR_EL1 检测 VGIC，跳过被拦截的寄存器配置，仅配置 CPU Interface 系统寄存器。

### 11. TokenGranule struct 字段顺序

与内核 `struct virtcca_cvm_token_granule` 字段顺序不一致（`head/ipa` 反了、`count` 为 `u32` 而非 `u64`），导致 TMM 读到错位的 offset/size 值。

### 12. TSI attestation COSE sign 23

根因：平台 BIOS 版本未预置加解密模块私钥，刷新 BIOS 后解决。

## 参考代码

| 路径 | 说明 |
|------|------|
| `../qemu/hw/virtio/virtio-pci.c` | QEMU virtio-pci Common Config + Notify handler |
| `../qemu/include/standard-headers/linux/virtio_pci.h` | Common Config 寄存器偏移定义 |
| `../qemu/hw/arm/virt.c` | QEMU aarch64 virt 机器 PCIe 布局 |
| `../kernel/drivers/virtio/virtio_mmio.c` | Linux MMIO 驱动参考 |
| `../kernel/arch/arm64/include/asm/arch_timer.h` | Generic Timer 内核寄存器访问 |
| `../kernel/arch/arm64/include/asm/archrandom.h` | RNDR 内核实现 |
| `../kernel/arch/arm64/kvm/virtcca_cvm_exit.c` | VirtCCA timer 上下文同步 |
| `../kernel/arch/arm64/kvm/arch_timer.c` | KVM timer 模拟层 |
