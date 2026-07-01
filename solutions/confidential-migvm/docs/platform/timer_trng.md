# ARM Generic Timer + TRNG 随机数

> Timer: `src/arch/timer.rs` + `src/time.rs`, TRNG: `src/trng.rs`

## Timer 架构

### 概述

ARM Generic Timer 是 ARM v8 的标准系统定时器。本方案使用 **virtual timer (CNTV)**：
- EL1 可直接访问
- QEMU/KVM 均完整支持
- 内核也使用 CNTV（`ARCH_TIMER_VIRT_PPI`）

### Poll 驱动设计（当前实现）

KVM VGIC PPI 27 中断不触发（根因见下文），采用 poll `CNTV_CTL_EL0.ISTAT` 位驱动系统时钟：

```
now_ms() 被调用（由 smoltcp event loop 高频触发）
  → poll_and_update()
    → 读 CNTV_CTL_EL0
    → ISTAT=1? → IMASK + TVAL + ENABLE + SYS_TICK_MS++
    → ISTAT=0? → 直接返回
```

### 寄存器

| 寄存器 | 汇编 | 用途 |
|--------|------|------|
| `CNTFRQ_EL0` | `mrs x, cntfrq_el0` | 计数器频率（QEMU 100MHz） |
| `CNTVCT_EL0` | `mrs x, cntvct_el0` | 虚拟计数器当前值 |
| `CNTV_TVAL_EL0` | `msr cntv_tval_el0, x` | 相对超时值（32-bit signed） |
| `CNTV_CVAL_EL0` | `msr cntv_cval_el0, x` | 绝对超时值 |
| `CNTV_CTL_EL0` | `mrs/msr cntv_ctl_el0, x` | bit0=ENABLE, bit1=IMASK, bit2=ISTAT |

### EL2 前置条件

在 `drop_to_el1` 中配置：

```asm
mrs x5, cnthctl_el2
orr x5, x5, #3          // CNTHCTL_EL1PCTEN + CNTHCTL_EL1PCEN
msr cnthctl_el2, x5
msr cntvoff_el2, xzr     // 清零虚拟偏移
```

### 系统时钟 API (`time.rs`)

| 函数 | 功能 | 对应 x86 |
|------|------|---------|
| `init_sys_tick()` | 读 CNTFRQ，设 1ms TVAL，arm timer | `init_sys_tick()` |
| `now_ms() -> u64` | 返回 ms 计数（内调 poll_and_update） | `SYS_TICK` |
| `wait_ms(ms)` | busy-wait 延迟 | — |

### 精度验证

KVM+GICv3 30 秒测试（每 2000ms 一行）：

```
[Service] ms=0x07D0     ← 2000ms,  Δ=0x7D0 ✓
[Service] ms=0x0FA0     ← 4000ms,  Δ=0x7D0 ✓
...
[Service] ms=0x7530 halt  ← 30000ms
```

**每间隔精确 0x7D0 (2000ms)，无抖动。**

### 文件

| 文件 | 功能 |
|------|------|
| `src/arch/timer.rs` | 硬件驱动：init, schedule_timeout_us, PPI 27 IRQ 回调（中断路径保留） |
| `src/time.rs` | 系统时钟：poll 驱动, now_ms(), wait_ms() |
| `src/arch/apic.rs` | GIC 扩展：VGIC IGROUPR0 诊断, ICC_IGRPEN0_EL1 |

---

## 内核/QEMU Timer 参考

### 内核初始化（`drivers/clocksource/arm_arch_timer.c`）

| 步骤 | 操作 | 与我们的对比 |
|------|------|------------|
| 1. 读频率 | `read_sysreg(cntfrq_el0)` | ✅ |
| 2. 选 PPI | `ARCH_TIMER_VIRT_PPI` (27) | ✅ |
| 3. set_next_event | `cntvct + evt → cntv_cval_el0`（只用 CVAL） | ⚠️ 我们偏好 TVAL |
| 4. handler | 读 `cntv_ctl → ISTAT → IMASK` | ✅ |

### QEMU 模拟（`target/arm/helper.c`）

- **TVAL write**: `cval = cntvct - offset + sextract64(value, 0, 32)` — signed 32-bit
- **CTL read**: bit 2 = ISTAT (count >= cval）
- **IRQ**: `(ctl & 6) == 4` = ISTAT=1 且 IMASK=0
- **QEMU PPI 映射**: `ARCH_TIMER_VIRT_IRQ = 27`（`hw/arm/virt.c`）

---

## VirtCCA KVM Timer 上下文同步

### 核心函数：`update_arch_timer_irq_lines()`

每次 vCPU 从 TMM 退出后调用（[virtcca_cvm_exit.c](../kernel/arch/arm64/kvm/virtcca_cvm_exit.c#L17)）：

```
vCPU exit from TMM
  → update_arch_timer_irq_lines(vcpu, is_wfx)
    ├─ __vcpu_sys_reg(CNTV_CTL_EL0)  = run->exit.cntv_ctl   // TMM 视图
    ├─ __vcpu_sys_reg(CNTV_CVAL_EL0) = run->exit.cntv_cval
    ├─ if (unmask_ctl): CNTV_CTL &= ~IT_MASK                // WFI 出口
    └─ kvm_cvm_timers_update(vcpu)
         ├─ cvm_timer_irq_can_fire = (ctl & (IMASK|ENABLE)) == ENABLE
         └─"not loaded"路径: cval <= (phys - cntvoff) → IRQ
```

### IRQ 触发条件

`cvm_timer_irq_can_fire`: `(ctl & (IMASK|ENABLE)) == ENABLE` — ENABLE=1 且 IMASK=0

### 根因：PPI 27 不触发

TMM 的 `vtimer_adjust` 在 S-EL2 层设置 IMASK → `cvm_timer_irq_can_fire` 返回 false → PPI 27 不注入。

### 对 Poll 驱动的影响

- `mrs cntv_ctl_el0` 读取的是 `__vcpu_sys_reg(CNTV_CTL_EL0)` — 与 IRQ 路径同一存储
- Poll 只检查 ISTAT、不依赖 `cvm_timer_irq_can_fire` → 天然绕过 TMM IMASK
- 每次 VirtIO MMIO access → vCPU exit → `update_arch_timer_irq_lines` → ISTAT 刷新
- **结论：Poll 在 VirtCCA 下是正确且最优方案**

---

## TRNG 随机数

### 内核参考（`arch/arm64/include/asm/archrandom.h`）

```c
asm volatile(
    __mrs_s("%0", SYS_RNDR_EL0) "\n"   // s3_3_c2_c4_0
    "cset %w1, ne\n"                    // NZCV≠0000 → failure
    : "=r" (*v), "=r" (ok) :: "cc");
```

- `SYS_RNDR_EL0` = `sys_reg(3, 3, 2, 4, 0)` = `s3_3_c2_c4_0` ← 我们的编码 ✅
- `SYS_RNDRRS_EL0` = `sys_reg(3, 3, 2, 4, 1)` = `s3_3_c2_c4_1`
- KVM: `arch/arm64/kvm/trng.c` 以 SMCCC TRNG 标准接口虚拟化

### 当前实现（`trng.rs`）

**纯 ChaCha20 PRNG**（CNTVCT 种子）：

```rust
fn chacha20_rand() -> u64 {
    let mut state = CHACHA_SEED.load(Ordering::Relaxed);
    if state == 0 { state = cntvct_raw(); }  // 首轮 CNTVCT 种子
    state = chacha20_round(state, 1);
    CHACHA_SEED.store(state, Ordering::Relaxed);
    state
}
```

| API | 说明 |
|-----|------|
| `u64_seed()` → smoltcp `config.random_seed` | DHCP 已通过 |
| `fill_random(buf)` | 通用随机字节填充 |
| `rand_u64()` | 单次 64-bit |

**RNDR 硬件检测暂未启用**：QEMU/KVM 环境下 `ID_AA64ISAR0_EL1` 和 `RNDR` 均触发 UNDEF 异常。VirtCCA 真机上 TMM 可能透传 RNDR，待后续验证。

### VirtCCA 时钟/随机数配置

- **`vtimer_adjust`**: TMI ABI version minor ≥ threshold 时启用，TMM 在 S-EL2 拦截 vtimer IRQ
- **RNDR**: 未在 `virtcca_cvm_guest.c` 中找到特殊处理 → 依赖 TMM 透传
- **CNTFRQ**: 使用 EL1 读取实际值（真机约 12.5MHz，QEMU KVM 62.5MHz）
