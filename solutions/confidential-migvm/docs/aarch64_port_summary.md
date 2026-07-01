# MigVM AArch64 移植总结

> **子文档索引**:
> - 业务逻辑: [CODE_FLOW.md](CODE_FLOW.md)
> - GIC 中断: [platform/gic.md](platform/gic.md)
> - Timer + TRNG: [platform/timer_trng.md](platform/timer_trng.md)
> - 内存/MMU: [platform/mm_memory.md](platform/mm_memory.md)
> - CCA/VirtCCA: [platform/cca_virtcca.md](platform/cca_virtcca.md)
> - 调试日志: [debug/debug_logs.md](debug/debug_logs.md)
> - 调试方法: [debug/debug_methods.md](debug/debug_methods.md)
> - VirtIO 设备: [virtio_mmio_plan.md](virtio_mmio_plan.md)
> - 完整启动输出: `/tmp/migvm_kvm_gicv3.log` / `/tmp/migvm_virtcca_vsock.log`

## 快速开始

```bash
# 单次运行（自动检测 KVM）
bash sh_script/qemu_aarch64.sh run

# VirtCCA 真机模式
bash sh_script/qemu_aarch64.sh virtcca

# GDB 调试（端口 1234）
bash sh_script/qemu_aarch64.sh debug

# 仅编译
bash sh_script/qemu_aarch64.sh build
```

| 环境变量 | 默认值 | 说明 |
|------|--------|------|
| `QEMU_KVM` | `auto` | `auto`/`yes`/`no` |
| `QEMU_GIC` | `auto`(→3) | `2`/`3` |
| `QEMU_TIMEOUT` | `60` | QEMU 超时秒数 |
| `QEMU_FEATURES` | *(空)* | `cca` / `virtcca` |

### 编译 Feature 选项

| Feature | QEMU 命令 | 机密计算模式 | SMC 接口 | 内存共享 |
|---------|----------|------------|---------|---------|
| *(默认)* | `sh_script/qemu_aarch64.sh run` | 普通 VM | 无 | 无 |
| `cca` | `QEMU_FEATURES=cca ... run` | CCA Realm | RSI | IPA 高位 |
| `virtcca` | `... virtcca` | VirtCCA CVM | TSI | PTE bit 5 |

## 项目目标

将基于 TDX 的机密虚拟机热迁移系统从 x86_64 移植到 aarch64，同时支持两类机密计算平台：
1. **CCA (Confidential Compute Architecture)**：ARM v9 Realm，RSI 调用，⚠️ 代码已实现，待硬件验证
2. **VirtCCA**：ARM v8 TrustZone S-EL2，TSI 调用，✅ 真机全链路验证（11/11 TSI 接口）

## 当前完成状态（2026-06-05）

| 阶段 | 内容 | 状态 |
|------|------|------|
| 1 | FDT 解析 + GIC + MMU + Boot | ✅ |
| 2 | VirtIO PCIe 传输层（net + vsock 双设备） | ✅ |
| 3 | smoltcp DHCP + TCP 双端口监听 | ✅ |
| 4 | VirtIO Vsock connect/send/recv | ✅ |
| 5 | 多设备 BAR 映射冲突解决 | ✅ |
| 6 | CCA/VirtCCA 编译选项隔离 | ✅ |
| 7 | VirtCCA 真机验证（Net DHCP + Vsock 全链路） | ✅ |
| 8 | TSI 接口探测（11/11 SMC 通路正常，含 attestation token） | ✅ |
| 9 | ARM Generic Timer 时钟（Poll 驱动，100MHz，KVM+VirtCCA 验证） | ✅ |
| 10 | GICv3 VGIC 中断控制器 + ISR 调试 | ✅ |
| 11 | TSI attestation token（根因：BIOS 未预置私钥，刷新后解决） | ✅ |
| 12 | ARM TRNG 随机数（ChaCha20 软件 PRNG，CNTVCT 种子） | ✅ |
| 13 | MMU 加固（4KB 页首 2MB × ROX/RO/RW + AP bits EL1 only） | ✅ |
| 14 | CCA/Realm 实机验证 | ⚠️ 待实机 |
| 15 | 双核启动（PSCI CPU_ON） | 🔲 暂缓（TDX 也未支持） |
| 16 | 业务流程对接（SocketMsg + session.rs + CODE_FLOW.md） | 🔲 下一步 |
| 17 | TDH 接口适配 | 🔲 后续 |

## 架构差异对比

### x86_64/CCA/VirtCCA 三平台对比

| 组件 | x86_64 (TDX) | aarch64 CCA (ARM v9) | aarch64 VirtCCA (ARM v8) |
|------|-------------|----------------------|--------------------------|
| 启动入口 | 多核 16-bit 实模式 | 单核 EL2→EL1 | 单核 EL2→EL1 |
| 页表 | 4/5 级，4KB/2MB/1GB | 4 级，4KB/2MB/1GB 块 | 4 级，4KB/2MB/1GB 块 |
| 中断控制器 | APIC | GICv3 | GICv3 |
| 系统定时器 | TSC + APIC Timer | ARM Generic Timer | ARM Generic Timer |
| 串口 | COM1 (I/O port) | PL011 (MMIO) | PL011 (MMIO) |
| 硬件描述 | ACPI | FDT | FDT |
| PCIe 配置 | MMIO + PIO | ECAM | ECAM |
| 机密计算 | TDX (TDCALL) | CCA/Realm (SMC→RSI) | VirtCCA (SMC→TSI) |
| 内存共享 | TDX Shared bit | IPA 高位 (PROT_NS_SHARED) | **PTE bit 5 (CVM_PTE_NS)** |
| 远程认证 | TDX Quote | Realm Token (RSI) | Attestation Token + Device Cert (TSI) |
| 迁移支持 | TDX Migration TD | ❌ | ✅ TSI MigVM |

### 模块复用度分析

| 模块 | 复用度 | 说明 |
|------|--------|------|
| MigTD 业务逻辑 (session.rs) | **95%+** | 仅底层接口不同 |
| vmcall-raw 传输层 | **90%** | GHCI buffer 完全复用 |
| VirtIO VirtQueue | **80%** | 队列逻辑复用 |
| rust_std_stub / async_runtime | **100%** | 直接复用 |
| td-payload (runtime) | **0%** | 架构完全不同，独立实现 |
| FDT 解析 | **0%** | aarch64 独有 |

## TDX MigTD 架构分析

### x86 TDX 原生模式（bare-metal, no_std）

```
MigTD 业务逻辑层 (src/migtd/src/bin/migtd/main.rs)
  ├── runtime_main() → handle_pre_mig() → wait_for_request()
  ├── exchange_msk() → 远程证明密钥交换
  └── report_status() → 报告迁移状态

迁移会话层 (src/migtd/src/migration/session.rs)
  ├── wait_for_request() → tdvmcall_migtd_waitforrequest
  ├── report_status() → tdvmcall_migtd_reportstatus
  └── exchange_msk() → 密钥交换协议

传输层
  模式 A: vmcall-raw (TCP)  — GHCI 1.5 buffer 格式
  模式 B: virtio-vsock (Vsock)  — 3 VirtQueue: RX/TX/Event

底层: TDX Module / APIC / PCI / UART / VirtIO PCI
```

### aarch64 目标架构（bare-metal, no_std）

```
MigTD 业务逻辑层（复用 x86）
  → session.rs: wait_for_request() / exchange_msk() / report_status()

传输层
  方案 A: vmcall-raw → smoltcp TCP/IP
  方案 B: virtio-vsock → VirtIO MMIO/PIC vsock 驱动

VirtIO 框架层（新写 MMIO 传输 + 复用 VirtQueue）
FDT 设备发现层（✅ 已完成）

机密计算底层接口
  CCA: rsi.rs → SMC 封装（⚠️ 未实机验证）
  VirtCCA: tsi.rs → SMC 封装（✅ 实机 11/11 通过）

精简 Runtime (no_std)
  deps/td-payload-aarch64/
  ├── arch/exception.S → EL2→EL1, MMU, 异常向量表
  ├── arch/apic.rs → GICv2/v3 + VGIC 检测
  ├── arch/timer.rs → ARM Generic Timer
  ├── time.rs → 系统时钟 (poll-driven)
  ├── trng.rs → ChaCha20 PRNG
  ├── mm/ → 页表, 堆, 共享内存
  └── network.rs → smoltcp DHCP + TCP
```

### 通信方案对比

| 维度 | TCP (smoltcp + VirtIO Net) | Vsock (VirtIO Vsock) |
|------|---------------------------|---------------------|
| 协议栈复杂度 | 高（ARP + IP + TCP） | 低（vsock 流式协议） |
| 与 x86 AzCVMEmu 对齐 | ✅ 完全对齐 | ❌ 需额外适配 |
| 上层代码复用 | tdx_emu.rs 几乎直接复用 | 需写新的 vsock 传输层 |
| 推荐场景 | 与 x86 AzCVMEmu 互通 | 快速验证迁移流程 |

## 开发策略

### 双轨并行

```
轨道 1: bare-metal (最终目标)
  FDT → VirtIO PCI → smoltcp TCP/Vsock → 迁移服务 → 完整功能

轨道 2: AzCVMEmu (快速调试)
  Linux 用户态 + TCP 通信 → 上层逻辑快速验证
```

**推荐先走 TCP 路线**，与 x86 AzCVMEmu 对齐，后续可切换到 Vsock。

## 后续计划

### 优先级 0：底层基础设施（✅ 基本完成）

| 组件 | 状态 |
|------|------|
| 系统时钟（Poll 驱动 ARM Timer） | ✅ |
| Timer 中断路径（根因：TMM `vtimer_adjust`，Poll 最优） | ✅ |
| TRNG 随机数（ChaCha20 PRNG） | ✅ |
| MMU 加固（4KB 页 + 区段权限 + EL1 only） | ✅ |
| BAR 64-bit 回退验证 | ⚠️ 不阻塞 |

### 优先级 1：TSI 调试 ✅ 已解决

11/11 TSI 接口全部通过。`attestation_token_continue` 返回 `rc=0, nbytes=0x591`。

根因：平台 BIOS 未预置加解密模块私钥，刷新 BIOS 后解决。辅助修复：`TokenGranule` struct 字段顺序对齐内核。

### 优先级 2：业务流程对接 🔲 下一步

1. `NetHandle` / `VsockHandle` 封装 → 统一 `send/recv`
2. `SocketMsg` (300 bytes packed) VSOCK 协议实现
3. `session.rs` 对接 `wait_for_request()` / `exchange_msk()` / `report_status()`
4. async executor 驱动 CODE_FLOW.md 状态机
5. VirtCCA 真机跑完整 Migrate 全链路

### 后续低优先级

| 事项 | 说明 |
|------|------|
| rats-tls 集成 | x86 侧已有 `rustls_impl/`，aarch64 直接复用 |
| GHCI 1.5 buffer 格式 | vmcall_raw 的 header 解析 |
| TDCALL → SMC 抽象层 | `td_call()` rax-based 分发 |
| SPDM 远程证明 | TSI token → SPDM evidence 转换 |
| CCA/Realm 实机验证 | 代码就绪，待硬件 |
| GICv3 ITS | MSI/MSI-X 中断翻译 |

## 文件清单

| 文件 | 功能 |
|------|------|
| `src/arch/exception.S` | 汇编启动：EL2→EL1，BSS 清零，栈，MMU，异常向量表 |
| `src/fdt.rs` | FDT 解析器：两阶段，提取 Memory/GIC/VirtIO MMIO |
| `src/lib.rs` | UART、panic、异常处理（ESR/FAR/SMC/EOI Mode 1） |
| `src/tsi.rs` | VirtCCA TSI SMC 封装（11/11 接口 ✅） |
| `src/rsi.rs` | CCA RSI SMC 封装（⚠️ 未实机验证） |
| `src/arch/apic.rs` | GICv2/v3 + VGIC 检测 + CPU Interface |
| `src/arch/timer.rs` | ARM Timer 硬件驱动 |
| `src/time.rs` | 系统时钟：poll 驱动 |
| `src/trng.rs` | ChaCha20 PRNG |
| `src/mm/paging.rs` | 页表管理（4KB 页 + 2MB 块） |
| `src/pci.rs` | PCIe ECAM 枚举 |
| `src/virtio/virtio_pci.rs` | VirtIO PCI 传输层 |
| `src/virtio/net.rs` | VirtIO Net 驱动 |
| `src/virtio/vsock.rs` | VirtIO Vsock 驱动 |
| `src/network.rs` | smoltcp DHCP + TCP + Vsock 事件循环 |
| `aarch64-qemu.ld` | 链接脚本 |
