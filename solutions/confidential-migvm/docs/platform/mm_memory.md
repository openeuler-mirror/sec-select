# 内存布局与 MMU

> 链接脚本: `aarch64-qemu.ld`, 页表: `src/mm/paging.rs`

## 内存布局

```
0x40080000  ┌─────────────────┐
            │  .text (ROX)     │  EL1 只读可执行
            ├─────────────────┤
            │  .rodata (RO)    │  EL1 只读不可执行
            ├─────────────────┤
            │  .data (RW)      │  EL1 读写
            ├─────────────────┤
            │  .bss (RW)       │  EL1 读写 (零初始化)
            ├─────────────────┤
            │  Page Tables     │  0x80000 (512KB)
            │   L0+L1          │    8KB
            │   L2 tables      │    512×4KB = 2MB
            │   L3 (dynamic)   │    余量
            ├─────────────────┤
            │  Heap            │  0x700000 (7MB)
            ├─────────────────┤
            │  Stack           │  0x200000 (2MB，向低地址增长)
            └─────────────────┘
```

## 页表映射

```
VA                   PA                    Size      Attr
0x00000000-0x3FFFFFFF 0x00000000-0x3FFFFFFF  1GB      Device-nGnRE (L1 block)
0x40000000-0x40200000 identity              2MB      4KB pages × 512 (首 2MB)
0x40200000-0x7FFFFFFF identity              1022MB   2MB blocks × 511 (其余)
```

### MAIR_EL1 配置

| Attr | 值 | 含义 |
|------|-----|------|
| Attr0 | 0x44 | Normal, Inner/Outer WB WA |
| Attr1 | 0xFF | Normal, Inner/Outer WB WA, no allocate |
| Attr2 | 0x04 | Device-nGnRE |

### TCR_EL1 配置

| 字段 | 值 | 含义 |
|------|-----|------|
| T0SZ | 30 | 1GB 虚拟地址空间 |
| IRGN0 | 01 | Inner WB WA |
| ORGN0 | 01 | Outer WB WA |
| SH0 | 11 | Inner Shareable |
| TG0 | 00 | 4KB granule |
| IPS | 010 | 36-bit 物理地址 |

## MMU 加固（2026-06-05）

### Before

1GB L1 block（device）+ 512×2MB L2 blocks（RAM），所有内存 RWX，EL0 可访问。

### After

首 2MB 用 4KB 页区分权限，其余用 2MB 块。

### 权限矩阵

| 区域 | AP[2:1] | PXN | UXN | DBM | 效果 |
|------|---------|-----|-----|-----|------|
| `.text` (0x40080000) | 10（EL1 RO） | — | ✅ | — | EL1 只读可执行 |
| `.rodata` | 10（EL1 RO） | ✅ | ✅ | — | EL1 只读不可执行 |
| `.data/.bss/heap/stack` | 11（EL1 RW） | ✅ | ✅ | ✅ | EL1 读写 |
| 2MB RAM blocks (0x40200000+) | 11（EL1 RW） | ✅ | ✅ | ✅ | EL1 读写 |
| Device MMIO | 11（EL1 RW） | ✅ | ✅ | ✅ | Device-nGnRE |
| DMA pool (VA 0x70000000) | 11（EL1 RW） | ✅ | ✅ | ✅ | NC, shared |

### Linker 符号

- `__text_start/_end`, `__rodata_start/_end`, `__data_start/_end`: 段边界
- `__data_end` 包含 `.bss`（页表/堆/栈在其后，直接 RW 2MB 块）
- `PTE_AP_RW_EL1 = (1<<7)|(1<<6)`: EL0 不可访问

### TLB flush

`init_page_tables()` 用 `tlbi vmalle1` 刷新全部 TLB，覆盖 ASM `setup_mmu` 的临时映射。

### 权限常量

| 常量 | bits | 对应内核 |
|------|------|---------|
| `PROT_KERNEL_ROX` | AF+SH_INNER+NORMAL+UXN+AP_RO | `_PAGE_KERNEL_ROX` |
| `PROT_KERNEL_RO` | AF+SH_INNER+NORMAL+PXN+UXN+AP_RO | `_PAGE_KERNEL_RO` |
| `PROT_KERNEL_RW` | AF+SH_INNER+NORMAL+PXN+UXN+DBM+AP_RW_EL1 | `_PAGE_KERNEL` |
| `PROT_DEVICE` | AF+DEVICE_nGnRE+PXN+UXN+DBM+AP_RW_EL1 | `PROT_DEVICE_nGnRE` |

## 运行时 VA 映射（PCIe/VirtIO 多设备 + DMA Pool）

```
VA                   PA                    Attr              用途
0x50000000           0x4010000000          Device-nGnRE       ECAM (PCIe 配置空间)
0x60000000           0x10000000            Normal NC          Net BAR4
0x61000000           0x11000000            Normal NC          Vsock BAR4
0x70000000           (IPA)                 Normal / NS        DMA Shared Pool (2MB)
```

### BAR4 内 VirtIO PCI 寄存器布局

```
BAR4 0x60000000:
  0x60000000-0x60000FFF  Common Config
  0x60001000-0x60001FFF  ISR Status
  0x60002000-0x60002FFF  Device Config
  0x60003000-0x60003FFF  Notify (每个 queue 4 字节)
```

## 启动流程

```
QEMU bootrom (0x40000000)
  → ldr x0, [DTB] → ldr x4, [内核入口] → br x4

_start (exception.S)
  → EL2→EL1 降级 → CPACR FP/SIMD → BSS 清零 → 栈=__stack_end → VBAR

setup_mmu (exception.S)
  → 淸零 L0+L1 → L0→L1 → L1[0]=Device 1GB block → L1[1]→L2 table
  → L2 512×2MB RAM → TTBR0 → TCR → MAIR → SCTLR M+A

_start_rust_impl (src/main.rs)
  → Heap 7MB → Logger → FDT parse → GIC init → Timer init
  → Memory map → 机密计算检测 → PCIe 枚举 → DHCP → 事件循环
```

## 文件

| 文件 | 功能 |
|------|------|
| `aarch64-qemu.ld` | 链接脚本 |
| `src/mm/paging.rs` | 页表管理 |
| `src/mm/heap.rs` | 堆分配器 |
| `src/mm/shared.rs` | 共享内存管理 |
| `src/mm/layout.rs` | 运行时内存布局 |
| `src/arch/exception.S` | 启动汇编 + MMU setup |
