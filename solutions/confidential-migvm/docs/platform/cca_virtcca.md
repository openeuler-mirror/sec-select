# CCA / VirtCCA 机密计算支持

> 详细实现: `src/rsi.rs` (CCA), `src/tsi.rs` (VirtCCA)


> ⚠️ **重要提示**：RSI/CCA Realm Guest 接口代码已实现，但**尚未在真实 CCA 硬件上验证**。
> 当前所有测试均在普通 QEMU KVM/TCG 虚拟机的 `[VM] Running in normal VM mode (no CCA)` 模式下完成。
> 以下内容为代码实现的设计文档，实际行为待 CCA 硬件到位后验证。

#### RSI (Realm Services Interface) 实现

| RSI 函数 | SMC FID | 功能 |
|----------|---------|------|
| `rsi_request_version` | 0xC4000190 | 请求 RSI ABI 版本 |
| `rsi_features` | 0xC4000191 | 查询 RSI 特性 |
| `rsi_measurement_read` | 0xC4000192 | 读取 Realm 测量值 |
| `rsi_measurement_extend` | 0xC4000193 | 扩展 Realm 测量值 |
| `rsi_attestation_token_init` | 0xC4000194 | 初始化认证令牌请求 |
| `rsi_attestation_token_continue` | 0xC4000195 | 继续获取认证令牌 |
| `rsi_get_realm_config` | 0xC4000196 | 获取 Realm 配置 |
| `rsi_ipa_state_set` | 0xC4000197 | 设置 IPA 状态（接受/共享/销毁） |
| `rsi_ipa_state_get` | 0xC4000198 | 获取 IPA 状态 |
| `rsi_host_call` | 0xC4000199 | 主机调用 |

#### CCA 内存管理

在 CCA Realm 环境中，内存有两种状态：
- **Protected (加密)**：Realm 私有内存，hypervisor 不可访问
- **Shared (解密)**：与 hypervisor 共享的内存，用于 I/O 通信

内存状态转换通过 RSI 调用实现：
- `accept_memory(start, end)` → `rsi_set_memory_range_protected`：将内存标记为 Realm 受保护
- `mark_shared(start, end)` → `rsi_set_memory_range_shared`：将内存标记为共享（解密）
- `mark_private(start, end)` → `rsi_set_memory_range_protected`：将共享内存恢复为受保护

#### SMC 安全调用机制

在非 CCA 环境中，SMC 调用会触发同步异常（因为 EL3 不存在）。系统实现了安全 SMC 检测机制：

1. 调用 SMC 前设置 `SMC_TESTING = true`
2. 若 SMC 触发同步异常（EC=0x17），异常处理程序：
   - 设置返回值 x0 = 0xFFFFFFFF（表示不支持）
   - 修改 ELR_EL1 跳过 SMC 指令（ELR += 4）
   - 清除 `SMC_TESTING` 标志
3. 若 SMC 正常返回，清除 `SMC_TESTING` 标志

#### CCA VirtIO 共享内存设计（⚠️ 未实机验证）

##### 背景

在 CCA Realm 中，Realm 私有内存受硬件保护，Host/Hypervisor 无法访问。VirtIO 作为一种半虚拟化 I/O 机制，需要 Host（QEMU）读写 Guest 内存中的 VirtQueue descriptor/avail/used ring 和数据缓冲区。

Linux 内核通过 **swiotlb bounce buffer** 机制解决此问题：在启动时预分配共享内存池，每个 DMA 操作前将数据从私有页 bounce-copy 到共享页，操作完成后再 copy 回来。这套机制通用但复杂。

MigVM 作为 bare-metal payload，场景远比 Linux 简单，**不需要完整的 swiotlb 机制**。

##### 设计思路：预分配共享 DMA 池

核心思想：**直接使用共享页作为 VirtIO DMA 缓冲区，消除 bounce-copy 开销。**

```
┌──────────────────────────────────────────────────────────────────┐
│                      Realm Private Memory                         │
│  ┌────────┐ ┌──────┐ ┌───────┐ ┌──────────────────────────────┐ │
│  │ .text  │ │ Heap │ │ Stack │ │  smoltcp buffers / app data   │ │
│  └────────┘ └──────┘ └───────┘ └──────────────────────────────┘ │
├──────────────────────────────────────────────────────────────────┤
│                      Realm Shared Memory (NS=1)                   │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │              DMA Shared Pool (固定大小, 如 2MB)               │ │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌───────────────┐  │ │
│  │  │ RX VQ    │ │ TX VQ    │ │ RX Bufs  │ │ TX Buf + Hdr  │  │ │
│  │  │ desc/avl │ │ desc/avl │ │ 64×4KB   │ │   1×4KB       │  │ │
│  │  │ /used    │ │ /used    │ │          │ │               │  │ │
│  │  └──────────┘ └──────────┘ └──────────┘ └───────────────┘  │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                              ▲                   │
│                                              │ Host 可访问        │
├──────────────────────────────────────────────┼───────────────────┤
│                    Hypervisor (QEMU)         │                   │
│   virtio-net-pci ── desc.addr ──────────────┘                   │
└──────────────────────────────────────────────────────────────────┘
```

##### 实现要点

**1. 启动时预分配共享池**（在 `VirtioNet::init_pci()` 之前）

```
从 FDT memory 区域末尾切出固定大小（如 2MB）作为 DMA Shared Pool：
  pool_pa = memory_base + memory_size - POOL_SIZE  (如 0x7FE00000)
  pool_va = 选择一个固定 VA（如 0x70000000）
  
  调用 rsi_set_memory_range_shared(pool_pa, pool_pa + POOL_SIZE)
  调用 map_shared_range(pool_va, pool_pa, POOL_SIZE)
```

**2. 将 `alloc_dma_pages` 改为从共享池分配**

当前 `alloc_dma_pages` 使用 `alloc::alloc::alloc()` 从私有堆分配。改为从一个简单的 bump allocator，从预分配的共享池 VA 区域分配：

```rust
static mut DMA_POOL_BASE: u64 = 0;
static mut DMA_POOL_OFFSET: usize = 0;
const DMA_POOL_SIZE: usize = 0x200000; // 2MB

pub fn alloc_dma_pages(num_pages: usize) -> Option<u64> {
    let size = num_pages * 0x1000;
    unsafe {
        let offset = DMA_POOL_OFFSET;
        if offset + size > DMA_POOL_SIZE {
            return None;
        }
        DMA_POOL_OFFSET = offset + size;
        let addr = DMA_POOL_BASE + offset as u64;
        core::ptr::write_bytes(addr as *mut u8, 0, size);
        Some(addr)
    }
}
```

**3. TX 数据路径：从私有内存 copy 到共享缓冲区**

普通 VirtIO TX 路径：
```
smoltcp 构造数据包 (私有内存)
    → 写入 VirtqDesc.addr (指向共享池中的 TX buffer)
    → avail_add + notify
```

如果 smoltcp 的数据在私有堆上，需要一个 copy 步骤。但由于 `VirtioTxToken::consume` 中我们直接将数据写入 `net.tx_buf_dma` 地址（该地址在共享池中），数据天然在共享内存中，**无需额外 bounce-copy**。

**4. RX 数据路径：从共享缓冲区读取**

```
QEMU 写入 RX buffer (共享池中) → used ring 更新 (共享池中)
    → Guest 从 used ring 读取 → 数据已在共享内存，直接可用
```

RX 数据已在共享池中，**无需 bounce-copy**。smoltcp 直接从共享池地址读取。

##### 对比 Linux swiotlb

| 方面 | Linux swiotlb | MigVM 简化方案 |
|------|--------------|---------------|
| 预分配 | 启动时分配 swiotlb 池（默认 64MB） | 启动时分配 DMA 共享池（约 2MB） |
| bounce 路径 | 每个 DMA 操作都 copy 两次（出/入） | **不 bounce**，数据直接在共享池 |
| 复杂度 | 拦截 DMA API，维护映射表 | 修改 `alloc_dma_pages` 为 bump allocator |
| 适用范围 | 所有 DMA 设备（块设备、网卡等） | 仅 VirtIO-Net（虚拟队列 + 网络缓冲区） |
| 代码量 | ~2000+ 行 | ~50 行 |

##### 内存预算（128 队列 × 2 个 VirtQueue × modern layout）

| 项目 | 计算 | 大小 |
|------|------|------|
| RX desc ring | 128 × 16 bytes | 2KB |
| RX avail ring | 2 + 128×2 = 258 bytes → align 2 | 260 bytes |
| RX used ring | 4 + 128×8 = 1028 bytes → align 8 | 1032 bytes |
| RX data buffers | 64 × 4KB | 256KB |
| TX desc ring | 128 × 16 bytes | 2KB |
| TX avail ring | 2 + 128×2 = 258 bytes → align 2 | 260 bytes |
| TX used ring | 4 + 128×8 = 1028 bytes → align 8 | 1032 bytes |
| TX header + buffer | 1 × 4KB | 4KB |
| **总计** | | **~270KB** |

2MB 池足够，还可为后续 Vsock 等留有余量。

##### 修改点

| 文件 | 修改 |
|------|------|
| `virtio/mod.rs` | `alloc_dma_pages` → 从共享池 bump-allocate，`init_dma_pool()` 新增 |
| `mm/paging.rs` | 新增 `map_shared_range` 调用（已有函数） |
| `rsi.rs` | 已有 `mark_shared` / `accept_memory` 等（无需修改） |
| `src/main.rs` | 在 VirtIO 初始化前调用 `init_dma_pool()` |
| `virtio/net.rs` | `fill_rx_buffers` → `alloc_dma_pages` 自动从共享池分配（无需修改） |

##### 开发步骤

1. 在 FDT 解析后，从 memory 区域末尾切出 DMA 共享池，记录 `pool_pa` / `pool_va`
2. 调用 `rsi_set_memory_range_shared()` + `map_shared_range()` 设置共享映射
3. 修改 `alloc_dma_pages` 使用共享池 bump allocator
4. `VirtioNet::init_pci()` 中原有的 `VirtQueue::new()` / `alloc_dma_pages()` 调用无需修改，自动从共享池分配
5. 在 CCA 实机上验证：QEMU 能正常读写 VirtQueue 和网络数据

##### TLS 数据安全分析（方案 B：TLS 直接写入共享池）

**威胁模型**：

在 CCA Realm 中，共享内存 (NS=1) 的页随时对 Host 可见：
- Realm 写入共享页 → Host 可实时读取
- Realm 读取共享页 → Host 可注入/篡改数据
- 无时序保护 —— Host 可在数据写入的任意时刻读取

**方案选择：TLS 直接在共享池上加密（零 bounce-copy）**

```
App data (私有堆 plaintext)
    │
    ▼ rats-tls::write(plaintext)
TLS 加密 (AES-GCM / ChaCha20-Poly1305, 内部 buffer 在私有堆)
    │
    ▼ ciphertext bytes
smoltcp socket.send(ciphertext)
    │ smoltcp 在栈上构造 TCP/IP header，payload 引用 ciphertext
    ▼
VirtioTxToken::consume → 写入 tx_buf_dma（共享池）
    │
    ▼ notify → QEMU 发送到网络
```

Host 在共享池中看到的数据：

```
[ TCP header | TLS ciphertext (AES-GCM) ]
                   ↑ 密文，无密钥无法解密
                   ↑ 与网络上传输的字节完全一致（TLS 本身就是为不可信网络设计的）
```

**安全性论证**：

| 问题 | 分析 |
|------|------|
| Host 能读到什么？ | TCP header（可忽略）+ TLS 密文 |
| TLS 密文泄露信息吗？ | 否 —— TLS 密文与网络上传输的字节相同，Host/网络始终可见 |
| Host 能篡改共享池数据吗？ | 能，但 TLS MAC 会检测篡改 → 连接断开 → rats-tls 收到 bad record |
| Host 能重放？ | 不能 —— TLS 有序列号 + nonce 保护 |
| 加密过程在私有内存？ | ✅ rats-tls 内部 buffer + AES 运算均在私有堆，仅最终密文输出到共享池 |
| smoltcp TCP header 泄露什么？ | seq/ack 编号、端口号——这些在网络层面始终是明文，TLS 只加密 payload |

**与方案 A 对比**：

| 维度 | 方案 A（私有加密 + copy 到共享池） | 方案 B（TLS 直接写入共享池） |
|------|--------------------------------|------------------------------|
| 加密在私有内存？ | ✅ | ✅ rats-tls 内部 buffer 在私有堆 |
| Host 看到的数据 | TLS 密文 | TLS 密文（相同） |
| 安全性 | ✅ | ✅（等价） |
| copy 次数 | 2+（TLS output → 中间 buffer → DMA buffer） | 1（smoltcp payload → DMA buffer） |
| 代码改动 | 需要中间 bounce buffer | **零改动** |
| 扩展性 | 每个 I/O 操作都要 bounce | 自动适配所有 VirtIO 设备 |

**结论**：当前 DMA 共享池机制完全支持方案 B，无需额外改动。约束条件：rats-tls 必须先加密再传给 smoltcp send（而非先 send 再 TLS wrap）——rats-tls 的设计天然满足此约束。

### VirtCCA Guest 支持（✅ 真机已验证）


> VirtCCA 基于 TrustZone S-EL2 构建，与 CCA Realm 功能类似，但实现机制完全不同：
> - CCA（ARM v9）使用 RSI（Realm Services Interface），内存共享通过 IPA 高位 `PROT_NS_SHARED` 标记
> - VirtCCA（ARM v8）使用 TSI（TMM Services Interface），内存共享通过 **PTE bit 5（CVM_PTE_NS）** 标记
>
> 两套接口通过 Rust `#[cfg(feature)]` 编译隔离，互不干扰。

#### CCA vs VirtCCA 对比

| 维度 | CCA (feature=cca) | VirtCCA (feature=virtcca) |
|------|-------------------|--------------------------|
| **架构** | ARM v9 Realm | ARM v8 TrustZone S-EL2 |
| **SMC 接口** | RSI (Realm Services Interface) | TSI (TMM Services Interface) |
| **SMC FID 基址** | `0xC4000000` | `0xC4000000`（相同） |
| **版本查询** | `rsi_request_version(0x190)` | `tsi_version()(0x190)` |
| **配置读取** | `rsi_get_realm_config(0x196)` | `tsi_get_realm_config(0x196)` |
| **测量扩展** | `rsi_measurement_extend(0x193)` | `tsi_measurement_extend(0x193)` |
| **认证令牌** | `rsi_attestation_token_*(0x194/0x195)` | `tsi_attestation_token_*(0x194/0x195)` |
| **设备证书** | ❌ RSI 无 | ✅ `tsi_device_cert(0x19A)` |
| **迁移属性** | ❌ RSI 无 | ✅ `tsi_migvm_get_attr(0x19D)` / `set_slot(0x19E)` |
| **绑定列表** | ❌ RSI 无 | ✅ `tsi_peek_binding_list(0x19F)` |
| **完整性校验** | ❌ RSI 无 | ✅ `tsi_mig_integrity_checksum_*(0x1A0/0x1A1)` |
| **内存共享机制** | IPA 高位标记 (`PROT_NS_SHARED = 1 << (ipa_bits-1)`) | PTE bit 5 (`CVM_PTE_NS = 1 << 5`) |
| **内存共享 SMC** | `rsi_ipa_state_set` → 变更 IPA 状态 | `tsi_sec_mem_unmap` → **仅通知 TMM**（可选） |
| **验证状态** | ⚠️ 代码已实现，待实机验证 | ✅ VirtCCA 真机全链路通过 |

#### VirtCCA QEMU 启动命令

```bash
bash sh_script/qemu_aarch64.sh virtcca

# 等效 QEMU 参数:
qemu-system-aarch64 \
  -M virt,usb=off,gic-version=host,accel=kvm,kvm-type=cvm -cpu host \
  -object tmm-guest,id=tmm0 \
  -m 1024 \
  -netdev user,id=net0 \
  -device virtio-net-pci,netdev=net0,disable-legacy=on \
  -device vhost-vsock-pci,guest-cid=3 \
  -kernel migvm.bin
```

#### VirtCCA 内存共享机制（PTE bit 5）

VirtCCA 的内存共享与 CCA 的根本不同在于 **不通过 SMC 调用变更内存状态**，而是通过 **PTE 属性位**：

```
VirtCCA (ARM v8): PTE bit 5 = CVM_PTE_NS (Non-Secure)
  - Guest stage-1 页表中设置 PTE bit 5
  - TMM stage-2 遍历 stage-1 页表，检查叶子 PTE 的 bit 5
  - bit 5 = 1 → stage-2 映射为 Non-Secure → Host 可访问
  - bit 5 = 0 → stage-2 映射为 Secure → Host 不可见

CCA (ARM v9): IPA 高位
  - PROT_NS_SHARED = 1 << (ipa_bits - 1)
  - IPA bit [ipa_bits-1] = 1 → Realm Non-Secure → Host 可访问
  - IPA bit [ipa_bits-1] = 0 → Realm Secure → Host 不可见
```

**实现**（`paging.rs:mark_range_shared_virtcca()`）：

```rust
#[cfg(feature = "virtcca")]
pub fn mark_range_shared_virtcca(vaddr: u64, size: u64) {
    let l1 = l1_table();
    // 直接操作 L1 表（.S setup_mmu 填充的 2MB block entries）
    // L1[(vaddr - 0x40000000) >> 21] |= (1 << 5)
    // 注意：不能操作 Rust map_range 创建的 L2 entries——它们是孤儿表，MMU 不遍历
}
```

**参考**：内核 `virtcca_cvm_guest.c:change_page_range_cvm()` — 设置 `PTE bit 5 = CVM_PTE_NS_MASK`。

#### VirtCCA TSI 接口探测结果（2026-06-04 实机测试，2026-06-05 更新）

> TMM version: 2.2 / TSI version: 1.0 / Attestation: enabled / Migration ability: set
> **最终状态**：11/11 接口 SMC 通路正常 ✅。`tsi_attestation_token_continue` 返回 `rc=0, nbytes=0x591`（1425 bytes CBOR/COSE token）。
>
> **根因**：早期的 COSE sign error 23 是**平台侧 BIOS 版本未预置加解密模块私钥**导致，刷新 BIOS 后问题消失。
> 同时修复了 `TokenGranule` struct 字段顺序与内核不一致的问题（`head/ipa` 反了、`count` 为 `u32` 而非 `u64`）。

在 VirtCCA 真机上逐接口探测 SMC 通路，结果如下（完整日志见 `[TSI] === Probing TSI interfaces ===` 章节）：

| # | TSI 接口 | SMC FID | rc | 判定 | 说明 |
|---|---------|---------|-----|------|------|
| 1 | `tsi_measurement_read(slot=0)` | `0xC4000192` | **0x0 ✅** | 正常 | RIM 数据 64 bytes |
| 2 | `tsi_measurement_extend(slot=1)` | `0xC4000193` | **0x0 ✅** | 正常 | REM 扩展成功 |
| 3 | `tsi_attestation_token_init(challenge)` | `0xC4000194` | **0x0 ✅** | 正常 | `token_ub=0x1000` (4KB) |
| 4 | `tsi_attestation_token_continue` | `0xC4000195` | **0x0 ✅** | 正常 | `nbytes=0x591` (1425 bytes CBOR/COSE token) |
| 5 | `tsi_device_cert` | `0xC400019A` | **0x0 ✅** | 正常 | AIK DER 证书 1169 bytes |
| 6 | `tsi_peek_binding_list` | `0xC400019F` | **0x2 (STATE)** | SMC 通路正常 | 预期：无迁移绑定 |
| 7 | `tsi_migvm_get_attr(rd=0)` | `0xC400019D` | **0x1 (INPUT)** | SMC 通路正常 | 预期：guest_rd=0 无上下文 |
| 8 | `tsi_migvm_set_slot(rd=0)` | `0xC400019E` | **0x1 (INPUT)** | SMC 通路正常 | 预期：guest_rd=0 无上下文 |
| 9 | `tsi_mig_integrity_checksum_init(rd=0)` | `0xC40001A0` | **0x1 (INPUT)** | SMC 通路正常 | 预期：guest_rd=0 无上下文 |
| 10 | `tsi_mig_integrity_checksum_loop(rd=0)` | `0xC40001A1` | **0x1 (INPUT)** | SMC 通路正常 | 预期：guest_rd=0 无上下文 |
| 11 | 全部 5 个 measurement slots | `0xC4000192` | **0x0 ✅** | 正常 | 仅 slot 0 有 RIM 数据 |

##### TSI SMC 调用规范（已验证的关键实现细节）

1. **SMCCC 1.1 约定**：`smc_full()` 必须显式设置 `x4-x7=0`，否则 TMM crypto 路径可能读取垃圾值
2. **`TokenGranule` 结构体** (`#[repr(C)]`)：
   ```rust
   #[repr(C)]
   pub struct TokenGranule {
       pub head: u64,          // offset 0:  整体 buffer 起始 IPA
       pub ipa: u64,           // offset 8:  当前 granule 的 IPA
       pub count: u64,         // offset 16: 已完成的 granule 计数
       pub offset: u64,        // offset 24: granule 内偏移
       pub size: u64,          // offset 32: 本次请求大小 (0x1000)
       pub num_wr_bytes: u64,  // offset 40: TMM 写回的字节数
   } // total: 48 bytes
   ```
   必须严格匹配内核 `struct virtcca_cvm_token_granule` 的字段顺序和大小。
   **早期错误**：字段顺序为 `ipa→head→offset→size→num_wr_bytes→count(u32)`，导致 TMM 内部看到的 `offset/size` 值错位（TMM 读到 `offset=0x1000, size=0`），签名引擎因 buffer 参数错误而失败。
3. **Buffer 连续性**：`TokenGranule` struct + token buffer（8KB）必须在物理内存中连续，放在同一个 `static` 分配块中
4. **Challenge buffer**：必须使用 static/bss buffer，避免栈 buffer 生命周期问题（TMM 可能在 SMC 返回后异步读取）
5. **SMC 调用方式**：`tsi_attestation_token_continue` 通过 x1/x2/x3 传 `(granule.ipa, granule.offset, granule.size)`，x4 建议传 struct PA 供 TMM 写回

##### token_continue COSE 签名错误（已解决）

早期 TMM 串口错误日志：
```
ERROR: tmm [attest_cvm_token_sign:905]: COSE signature encoding failed with error code: 23
ERROR: tmm [attest_token_continue_sign_state:1293]: cvm token signing failed, ret is 4.
```

**根因**：平台侧 BIOS 版本未预置加解密模块私钥（Attestation signing key），导致 TMM 内部的 COSE Sign1 签名因找不到对应私钥而失败。刷新 BIOS 后，Linux kernel guest 和我们的 payload 均正常通过 attestation。

**辅助修复**：`TokenGranule` struct 字段顺序与内核不一致（`head/ipa` 反了、`count` 为 `u32` 而非 `u64`），虽非根因但必须修正以保证 TMM 正确解析 granule 参数。**

### 关键技术问题及解决

