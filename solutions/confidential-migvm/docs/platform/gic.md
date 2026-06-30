# ARM GIC 中断控制器

> 详细实现: `src/arch/apic.rs`

## 版本自动检测

通过读取 `GICD_TYPER` 寄存器的 LPIS 位（bit 17）自动检测 GIC 版本：
- **GICv2**：LPIS=0，使用 MMIO 方式访问 CPU Interface（GICC_BASE）
- **GICv3**：LPIS=1，使用系统寄存器（ICC_*_EL1）访问 CPU Interface

## GICv3 初始化流程

### 1. VGIC 检测（通过 MIDR_EL1）

读取 MIDR_EL1 寄存器，检查 Implementer 和 PartNum 字段：
- 若为已知物理 ARM CPU（implementer=0x41，partnum 为 A57/A53/A72 等）→ 裸机模式，执行完整 GIC 配置
- 若为未知/虚拟 CPU → VGIC 模式，跳过 Distributor/Redistributor 配置，仅配置 CPU Interface 系统寄存器

### 2. Distributor 初始化（仅裸机模式）

- 禁用 Distributor（GICD_CTLR=0），等待 RWP
- 配置 SPI 中断分组（GICD_IGROUPR）、优先级（GICD_IPRIORITYR）
- 设置 SPI 路由到当前 CPU（GICD_IROUTER，基于 MPIDR 亲和性）
- 启用亲和路由（ARE_NS/ARE_S）和中断组（Group0/Group1）

### 3. Redistributor 初始化（仅裸机模式）

- 基于 MPIDR 亲和性匹配定位当前 CPU 的 Redistributor（遍历 GICR_TYPER）
- 唤醒 Redistributor（清除 GICR_WAKER.ProcessorSleep，等待 ChildrenAsleep 清零）
- 配置 SGI/PPI 分组（GICR_IGROUPR0）、优先级（GICR_IPRIORITYR0）

### 4. CPU Interface 初始化（裸机和 VGIC 模式均执行）

- 启用系统寄存器接口（ICC_SRE_EL1.SRE=1）
- 设置优先级阈值（ICC_PMR_EL1=0xF0）
- 配置 EOI Mode 1（ICC_CTLR_EL1.EOImode=1，优先级 Drop + Deactivation 分离）
- 设置二进制点寄存器（ICC_BPR1_EL1=0）
- 启用 Group1 中断（ICC_IGRPEN1_EL1=1）

## 中断处理流程（GICv3 EOI Mode 1）

```
IRQ → ICC_IAR1_EL1 (acknowledge, 获取 INTID)
    → ICC_EOIR1_EL1 (drop priority, 优先级恢复)
    → 处理中断
    → ICC_DIR_EL1 (deactivate, 中断去激活)
```

## GICv2 初始化流程

1. 禁用 Distributor 和 CPU Interface
2. 配置 SPI 中断分组（GICD_IGROUPR）、优先级（GICD_IPRIORITYR）
3. 设置 SPI 目标 CPU（GICD_TARGET）
4. 配置中断触发方式（GICD_ICFGR）
5. 启用 Distributor（GICD_CTLR=1）
6. 配置 SGI/PPI 优先级
7. 设置优先级阈值（GICC_PRIMASK=0xF0）
8. 启用 CPU Interface（GICC_CTRL=1）

## KVM VGIC PPI 中断问题（已确认根因）

KVM VGIC 模式下，GICR_IGROUPR0/ISENABLER0 的 MMIO 写入虽然被接受（readback 确认生效），但 PPI 的注入路径与裸机 GIC 不同：

- `IGROUPR0 before=0x0 after=0xFFFFFFFF`：写入生效
- `ISENABLER after=0x08000000`：PPI 27 bit 置位
- `ICC_IGRPEN0_EL1=1` + `ICC_IGRPEN1_EL1=1`：双组启用
- **但 CNTV timer 超时后 PPI 27 中断仍不被 CPU 接收**

**根因**: VirtCCA TMM 的 `vtimer_adjust` 机制在 S-EL2 层设置了 IMASK，导致 KVM 的 `cvm_timer_irq_can_fire` 返回 false → PPI 27 IRQ 不被注入。详见 [Timer 文档](timer_trng.md#virtcca-kvm-timer-上下文同步分析)。

当前采用 **Poll 驱动**（直接读 `CNTV_CTL_EL0.ISTAT` 位）绕过此限制。

## 文件清单

| 文件 | 功能 |
|------|------|
| `src/arch/apic.rs` | GICv2/GICv3 自动检测、VGIC 检测、Distributor/Redistributor/CPU Interface 初始化、EOI Mode 1、IGROUPR0 诊断 |
| `src/arch/idt.rs` | 中断回调注册与分发 |
| `src/lib.rs` | 异常处理：ESR/FAR 诊断、SMC 安全检测、EOI Mode 1 中断处理 |
