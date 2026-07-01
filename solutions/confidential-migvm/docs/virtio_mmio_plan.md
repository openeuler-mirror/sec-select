# VirtIO AArch64 适配方案

## 目标

在 aarch64 payload 中实现 VirtIO 传输层，使 MigVM 通过 PCIe 同时访问 VirtIO-Net 和 VirtIO-Vsock 设备。

## 总体策略

- **MMIO 传输层**：已完成 modern 模式 TX 验证，代码保留供参考
- **PCIe 传输层**：✅ 已完成。ECAM 枚举 + Capability 解析 + Queue setup + Doorbell 全部打通，KVM + GICv3 模式下 DHCP 获取 IP + Vsock 连接成功
- **网络栈**：✅ smoltcp + DHCPv4 已集成并通过验证
- **Vsock 传输**：✅ connect/send/recv 已实现，KVM + GICv3 下与 host socat 通信成功
- **多设备支持**：✅ net + vsock 双 PCI 设备独立 BAR 映射，无冲突

---

## 实测进度

### PCIe 传输层

| 步骤 | 状态 | 说明 |
|------|------|------|
| FDT ECAM 解析 | ✅ | saved_addr_cells 修复 |
| ECAM VA 重映射 | ✅ | 0x4010000000 → 0x50000000 |
| 32-bit 对齐 ECAM 读 | ✅ | 8/16-bit → 32-bit 移位提取 |
| PCI 总线枚举 | ✅ | vendor=0x1AF4 device=0x1041 |
| BAR4 分配 (无固件) | ✅ | ECAM 写入 BAR4=0x10000000(64-bit) |
| BAR4 mapping (net) | ✅ | VA=0x60000000, AttrIndx=0 (Normal NC), PA=0x10000000 |
| BAR4 mapping (vsock) | ✅ | VA=0x61000000, PA=0x11000000 (独立地址空间) |
| Virtio PCI Cap 解析 | ✅ | common/notify/device/ISR 地址正确（含 bar 字段） |
| STATUS RESET→ACK→DRIVER | ✅ | 8/16-bit 独立访问正确偏移 |
| FEATURES_OK | ✅ | net: 0x10130BF8024, vsock: 0x130000002 |
| Queue setup | ✅ | net: RX+TX, vsock: RX+TX (desc/avail/used 64-bit) |
| QUEUE_ENABLE | ✅ | 16-bit 写 0x1c |
| DRIVER_OK | ✅ | status 0x0F |
| Doorbell (notify) | ✅ | off×mult 地址公式修复 |
| smoltcp + DHCP | ✅ | IP=10.0.2.15 Router=10.0.2.2 |
| Vsock connect | ✅ | CID:5 → 2 port:4052 OP_RESPONSE |
| Vsock send | ✅ | "hello from vsock" → socat 收到 |

---

## 当前实测输出（KVM + GICv3，2026-06-04 ✅ 全部通过）

### Net + Vsock 双设备

```
[VirtIO PCI] probe va=0x60000000 → 0x0 (net BAR, PA=0x10000000)
[VirtIO Net PCI] === Net device init ===
[VirtIO Net PCI] TX test: OK

[VirtIO PCI] probe va=0x61000000 → 0x0 (vsock BAR, PA=0x11000000)
[VirtIO Vsock PCI] Guest CID: 5
[VirtIO Vsock PCI] === Init complete ===

[PCI] VirtIO-net init OK
[PCI] VirtIO-vsock init OK

[Net] DHCP OK: 10.0.2.15
[Service] TCP listening on 10.0.2.15:5001 + :5002
[Service] Vsock connected
[Service] === Entering event loop ===
```

## 调试环境

| 项目 | 值 |
|------|-----|
| 运行脚本 (net only) | `sh_script/qemu_aarch64.sh run` |
| 运行脚本 (net + vsock) | `sh_script/qemu_aarch64.sh vsock` |
| Vsock 参数 | `QEMU_KVM=yes QEMU_VSOCK_CID=5 QEMU_TIMEOUT=60` |
| Vsock QEMU 命令 | `-device virtio-net-pci,netdev=net0,disable-legacy=on -device vhost-vsock-pci,guest-cid=3` |
| 串口日志 | `/tmp/migvm_yes_gicv3.log` (run) / `/tmp/migvm_yes_gicv3_vsock.log` (vsock) |
| 内核镜像 | `target/aarch64-unknown-none/release/migvm.bin` (~107KB raw binary) |
| PCIe ECAM PA | 0x4010000000 → VA 0x50000000 (Device-nGnRE) |
| Net BAR4 | PA 0x10000000 → VA 0x60000000 (Normal NC) |
| Vsock BAR4 | PA 0x11000000 → VA 0x61000000 (Normal NC) |

### 核心源文件

```
src/
├── mm/paging.rs             map_bar_space: 0x701 (Normal WB WA!)
├── pci.rs                   ECAM + 总线枚举 + BAR 探测
├── virtio/virtio_pci.rs     BAR 编程 + Cap 解析 + notify_queue (含调试日志)
├── virtio/net.rs            VirtioNet::init_pci + VirtQueue + RX/TX
├── network.rs               smoltcp DHCPv4
├── fdt.rs                   FDT 解析 (PcieInfo + ranges)
└── src/main.rs              main: FDT→GIC→MMIO→PCI→DHCP
```

---

## 关键修复记录

### 1. ECAM 地址超出 VA 范围
- **问题**: PA `0x4010000000` 超出 T0SZ=30 的 16GB VA 空间
- **修复**: 重映射到 VA `0x50000000`，L1 block entry `0x609`
- **涉及**: paging.rs, pci.rs

### 2. FDT #address-cells 覆盖
- **问题**: PCIe 节点 `#address-cells=3` 导致 ECAM reg=0
- **修复**: saved_addr_cells / saved_size_cells
- **涉及**: fdt.rs

### 3. TLB 刷新漏掉
- **问题**: 页表写入后 MMU 走旧 TLB
- **修复**: `dsb ishst` + `tlbi vmalle1is` + `dsb ish` + `isb`
- **涉及**: paging.rs

### 4. ECAM 8/16-bit → Data Abort
- **问题**: Device-nGnRE 不允许 sub-32-bit 访问
- **修复**: 全部 ECAM 读取改为 32-bit 对齐 + 移位提取
- **涉及**: pci.rs

### 5. Virtio common config 16-bit 寄存器
- **问题**: offset 0x14/0x16/0x18/0x1A/0x1C/0x1E 不在 4 字节边界
- **修复**: 相邻 16-bit 寄存器配对为 32-bit RMW 访问
- **涉及**: virtio_pci.rs

### 6. BAR 未编程（无固件模式）
- **问题**: `-kernel` 直接启动无 UEFI/固件，PCIe BAR 全部为 0
- **修复**: 检测 bar_pa==0 时通过 ECAM 写入 FDT ranges mmio_base 到 BAR4
- **涉及**: virtio_pci.rs

### 7. 64-bit BAR 只写低 32 位
- **问题**: 原代码 `PCI_BAR0 + 4*4` 只写 BAR4 低 32 位
- **修复**: 检测 is_64bit 后同时写入 BAR4+4 高 32 位
- **涉及**: virtio_pci.rs

### 8. probe 误判 0x0 为错误
- **问题**: `device_feature_select=0` 被错误当作 BAD
- **修复**: 只拒绝 0xFFFFFFFF
- **涉及**: virtio_pci.rs

### 9. notify_queue 内存排序 + 地址公式错误
- **问题 1**: Normal WB WA 映射下 write_queue_select 和 read_notify_off 间无屏障，读到旧值
- **问题 2**: `notify_base + off * notify_mult` 错误，queue_notify_off 已是最终 offset
- **修复**: 加 `dmb sy` 屏障 + 改为 `notify_base + off`
- **结果**: `queue_notify_off` 仍然始终返回 0（因为 queue_sel 寄存器从未被正确写入）

### 10. Common Config 寄存器偏移错误（🔴 根因）
- **问题**: `write_queue_select` 写到 `0x14`（STATUS 寄存器）而非 `0x16`（Q_SELECT 寄存器）。原来的 32-bit RMW 将 STATUS+CFGGENERATION+Q_SELECT 三个寄存器当作一个 u32 操作，QEMU 的 STATUS handler 只看 `val & 0xFF`，`vdev->queue_sel` 从未被更新
- **QEMU 寄存器布局**:
  ```
  0x14: STATUS (8-bit)      0x15: CFGGENERATION (8-bit)
  0x16: Q_SELECT (16-bit)   0x18: Q_SIZE (16-bit)
  0x1A: Q_MSIX (16-bit)     0x1C: Q_ENABLE (16-bit)
  0x1E: Q_NOFF (16-bit)     0x20+: Q_DESC/AVAIL/USED (64-bit)
  ```
- **修复**: 每个寄存器独立按正确宽度访问正确偏移——status 用 8-bit `0x14`、queue_select 用 16-bit `0x16`、queue_size 用 16-bit `0x18`、queue_enable 用 16-bit `0x1c`、notify_off 用 16-bit `0x1e`
- **涉及**: virtio_pci.rs

### 11. Notify 对齐异常
- **问题**: `queue_notify_off=1`, notify addr = `0x60003001`（奇数），`strh` 触发 Alignment Fault（ESR `0x96000061` → DFSC=0x21 对齐错误）。KVM stage-2 将 PCI MMIO 映射为 Device 内存，禁止非对齐访问
- **修复**: notify 写入从 `u16`(`strh`) 改为 `u8`(`strb`)。QEMU 通过地址偏移判定 queue 编号，不依赖写入值大小
- **涉及**: virtio_pci.rs

### 12. Notify 地址缺少 multiplier
- **问题**: QEMU `virtio_pci_notify_write` 计算 `queue = addr / 4`，每个 queue 的 notify 子区域占 4 字节。`addr = base + off`（off=1）→ QEMU `1/4=0` 将 TX 通知发给了 queue 0 (RX)
- **修复**: `addr = base + off * notify_mult`，notify_mult=4，addr=`0x60003004` → QEMU `4/4=1` → queue 1 (TX) ✅
- **涉及**: virtio_pci.rs

### 13. used_ring_ptr 偏移量错误
- **问题**: `used` 内存布局为 `flags(2B) + idx(2B) + ring[]`，ring 在 offset 4。原代码 `ptr.add(1)` 在 `*mut VirtqUsedElem` 上加 8 字节 → 读到 ring[0].len 而非 ring[0]，导致 `get_used_buf` 返回垃圾数据，触发 panic
- **修复**: `(self.used as *mut u8).add(4) as *mut VirtqUsedElem`
- **涉及**: virtio/net.rs

### 14. 多 PCI 设备 BAR VA/PA 冲突（🔴 多设备根因）
- **问题**: 两个 VirtIO PCI 设备都硬编码 `va_base=0x60000000` 和 `bar_pa=mmio_base`（同一个 PA）。vsock init 调用 `map_bar_space` 覆盖了 net 的 BAR 映射，导致 net 的 Common Config 访问实际操作的是 vsock 设备
- **修复**:
  - `VirtioPciTransport::init` 新增 `bar_va` 参数
  - net: `bar_va=0x60000000`, `bar_pa=0x10000000`
  - vsock: `bar_va=0x61000000`, `bar_pa=0x11000000`（PA 间隔 16MB）
- **涉及**: virtio_pci.rs, src/main.rs

### 15. RX kick 时序问题
- **问题**: `fill_rx_buffers()` 后没有显式 `notify_queue(0)`，DHCP 阶段 QEMU 不知道 RX 缓冲区可用。之前单设备时靠 TX notify(1) side-effect 触发 QEMU 检查 RX（偶然工作），多设备时不再可靠
- **保留的必要 notify**:
  - `VirtioNet::init_pci()` 末尾：fill RX → notify(0)（VirtIO 标准要求）
  - `service_with_tcp()` 入口：DHCP 后重新创建 Interface → notify(0)
- **涉及**: virtio/net.rs, network.rs

### 16. 编译警告清零
- 删除 `virtio_pci.rs` 中 unused `use crate::mm::paging`
- 删除 `pci.rs` 中 unused `ECAM_MAX_PER_2MB`
- 删除 `vsock.rs` 中 unused `VSOCK_EVENT_QUEUE`
- 给 `fdt.rs` 中 `child_addr_lo` 加 `_` 前缀
- 给 `mmio_transport.rs` 中 `read8/write8` 加 `#[allow(dead_code)]`
- 给 `virtio_pci.rs` 中 `read_reg0/read_u32` 加 `#[allow(dead_code)]`

---

## 总结

### 成功的调试方法论

1. **读 QEMU 源码**：在 QEMU `hw/virtio/virtio-pci.c` 中追踪 Common Config 的 read/write handler，发现寄存器按字节偏移独立分发（非 32-bit 整体操作），直接定位到 `write_queue_select` 写错偏移的根因
2. **ESR/FAR 解码**：同步异常时打印 ESR_EL1 和 FAR_EL1，解析 EC（Exception Class）和 DFSC（Data Fault Status Code），精确定位对齐错误类型
3. **逐步验证**：每次修改只改一处，编译运行观察日志变化，避免多变量干扰

### 修改文件汇总

| 文件 | 修改 |
|------|------|
| `mm/paging.rs` | 无实质变更（保持 `0x701` Normal NC）、DMA 共享池 `init_dma_shared_pool()` |
| `virtio/virtio_pci.rs` | 寄存器偏移修复、notify multiplier、u8 写入、`bar_va` 参数化、`read_config64` |
| `virtio/net.rs` | `used_ring_ptr` 偏移修复、`notify_queue(0)` RX kick |
| `virtio/vsock.rs` | **新增**：VirtIO Vsock 驱动（connect/send/recv/VirtQueue） |
| `virtio/mod.rs` | `pub mod vsock`、DMA 双模式 allocator |
| `src/main.rs` | 多设备枚举（net + vsock）、`start_migration_service` 调用 |
| `network.rs` | `run_dhcp_and_keep`、`start_migration_service`（TCP 5001/5002 + Vsock 事件循环） |
| `pci.rs` | 删除 unused const |
| `fdt.rs` | unused 变量加 `_` 前缀 |
| `mmio_transport.rs` | `#[allow(dead_code)]` 标记 |
| `sh_script/qemu_aarch64.sh` | 新增 `vsock` 子命令 + `QEMU_VSOCK_CID` 环境变量 |
| `rust-toolchain` | channel 修复（`1.88.0` → `stable`） |

### 当前多设备 VA/PA 映射表

```
VA                    PA                   设备
0x50000000            0x4010000000          ECAM (PCIe 配置空间)
0x60000000            0x10000000            Net BAR4 (Common/Notify/ISR/Device)
0x61000000            0x11000000            Vsock BAR4
0x70000000            (预留)                DMA Shared Pool (CCA Realm 用)
```

### 当前状态

- KVM + GICv3: ✅ 全链路通过（ECAM→双 BAR→双设备→Net DHCP→Vsock Connect→TCP 双端口监听）
- IP: `10.0.2.15`, Router: `10.0.2.2`, Vsock CID:5
- 编译: 0 warnings
- 测试脚本: `run`（net only）、`vsock`（net + vsock）
