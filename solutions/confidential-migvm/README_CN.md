# MigVM AArch64

面向 CCA（ARM v9 Realm）与 VirtCCA（ARM v8 TrustZone）机密虚拟机迁移的独立裸机载荷（payload）。

## 快速开始

```bash
# 编译
cargo build --release --target aarch64-unknown-none

# 在 QEMU 上运行
bash sh_script/qemu.sh run

# VirtCCA 客户机模式
cargo build --release --features virtcca --target aarch64-unknown-none
bash sh_script/qemu.sh virtcca

# CCA Realm 模式（待硬件就绪）
cargo build --release --features cca --target aarch64-unknown-none
```

## 功能特性

| Feature | 说明 |
|---------|------|
| `default` | 普通 QVM 虚拟机（非机密计算） |
| `virtcca` | VirtCCA 客户机模式（ARM v8 TrustZone S-EL2，TSI SMC 调用） |
| `cca` | CCA Realm 模式（ARM v9，RSI SMC 调用） |

## 架构

```
AArch64 启动 (exception.S) → MMU 初始化 → FDT 解析 → GICv3 初始化 → 定时器初始化
    → PCIe 枚举 → VirtIO-net + VirtIO-vsock → smoltcp DHCP → TCP 5001/5002
```

## 文档

- [移植概览](docs/aarch64_port_summary.md)
- [代码流程](docs/CODE_FLOW.md)
- [VirtIO 方案](docs/virtio_mmio_plan.md)
- [GIC 控制器](docs/platform/gic.md)
- [定时器 + TRNG](docs/platform/timer_trng.md)
- [内存 / MMU](docs/platform/mm_memory.md)
- [CCA / VirtCCA](docs/platform/cca_virtcca.md)
- [调试日志](docs/debug/debug_logs.md)
- [调试方法](docs/debug/debug_methods.md)

## 环境要求

- Rust nightly 工具链，目标平台 `aarch64-unknown-none`
- QEMU 8.2+（需提供 `qemu-system-aarch64`）
- VirtCCA 运行环境：KVM 启用 `kvm-type=cvm` + TMM 固件

## 验证状态

| 环境 | DHCP | TCP | 定时器 | TSI | MMU |
|------|------|-----|--------|-----|-----|
| KVM + GICv3 | ✅ | ✅ | ✅ 30s | N/A | ✅ |
| VirtCCA | ✅ | ✅ | ✅ | ✅ 11/11 | ✅ |
| CCA Realm | ⚠️ | ⚠️ | ⚠️ | ⚠️ 代码就绪 | ⚠️ |

## 许可证

本项目采用双重许可：

- **BSD-2-Clause-Patent** — 适用于保留原始 Intel 版权的文件
  （通用存根 / 标准模板：`src/acpi.rs`、`src/mm/layout.rs`、`src/mm/heap.rs`）。
- **Mulan Permissive Software License v2（木兰宽松许可证 v2，Mulan PSL v2）** — 适用于
  所有其它源文件（AArch64 特有实现），持有华为技术有限公司版权。

完整许可证文本请参见 [LICENSE](LICENSE) 文件。每个源文件头部声明了其适用的许可证。

> 本项目基于 Intel x86 机密虚拟机迁移项目衍生，针对 ARMv8/ARMv9 CCA/VirtCCA 机密虚拟机迁移场景进行了适配与扩展。
> 架构对比与移植细节详见 [移植概览](docs/aarch64_port_summary.md)。
