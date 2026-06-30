# MIGVM Agent 代码流程与用法总结（Baremetal Rust 重写指导版）

## 1. 概述

migcvm-agent 是运行在机密虚拟机（MIG-CVM）内部的迁移代理程序，在热迁移过程中负责：

- 通过 **VSOCK** 与宿主机（QEMU）通信，接收迁移指令
- 通过 **RATS-TLS**（基于远程证明的 TLS）在源端和目标端之间建立安全信道
- 通过 **TSI**（Trusted Service Interface，`/dev/tsi`）与底层可信固件交互，完成密钥获取和注入
- 完成远程证明（token 验证、RIM 对比、IMA 度量验证、固件状态验证等）
- 迁移完成后，启动 **完整性校验线程** 持续校验迁移内存

---

## 2. 关键常量速查表

从多个头文件中汇总，Rust 重写时必须保持一致：

| 常量 | 值 | 来源 | 说明 |
|------|-----|------|------|
| `CLIENT_AGENT_PORT` | `9000` | socket_agent.h | 源端 VSOCK 监听端口 |
| `SERVER_AGENT_PORT` | `9001` | socket_agent.h | 目的端 VSOCK 监听端口 |
| `MIGCVM_PORT` | `1234` | ima_measure.h | RATS-TLS TCP 通信端口 |
| `MIGCVM_CID` | `VMADDR_CID_ANY` | socket_agent.h | VSOCK CID（绑定任意） |
| `MAX_PAYLOAD_SIZE` | `256` | socket_agent.h | 字符串负载最大长度 |
| `MAX_BIND_VM` | `256` | migcvm_tsi.h | 最大绑定的 VM 数量 |
| `REM_COUNT` | `4` | rem.h | REM 条目数量 |
| `REM_LENGTH_BYTES` | `32` | rem.h | 每个 REM 长度（SHA256） |
| `SHA256_SIZE` | `32` | ima_measure.h | SHA256 摘要长度 |
| `SHA512_SIZE` | `64` | ima_measure.h | SHA512 摘要长度 |
| `MAX_MEASUREMENT_SIZE` | `SHA512_SIZE` (64) | ima_measure.h | 最大度量值（RIM 等） |
| `CHALLENGE_SIZE` | `64` | migcvm_tsi.h | 证明挑战值长度 |
| `REPORT_MAX_LENGTH` | 隐含约 4KB | migcvm_tsi.h | attestation token 最大长度 |
| `ATTEST_MAX_TOKEN_SIZE` | `4096` | token_parse.h | token 解析最大长度 |
| `VIRTCCA_TOKEN_SIZE` | `4104` | rats_tls_handler.c | 内部 token buf 大小 |
| `IMA_READ_BLCOK_SIZE` | `1024` | ima_measure.h | IMA 日志读取块大小 |
| `MAX_IMA_LOG_SIZE` | `1GB` | ima_measure.h | IMA 日志最大大小 |
| `TSI_MAGIC` | `'T'` (0x54) | migcvm_tsi.h | TSI ioctl magic number |
| `QUEUE_IOCTL_MAGIC` | `'q'` (0x71) | integrity_check | 队列 ioctl magic number |

---

## 3. 精确数据结构（`#[repr(C, packed)]` 对齐）

### 3.1 VSOCK 消息协议

```rust
// 来源: inc/socket/socket_agent.h
// 注意: #pragma pack(push, 1) — 1 字节对齐，无填充！

#[repr(C, packed)]
pub struct SocketPayload {
    pub char_payload: [u8; 256],       // offset 0,   size 256
    pub ull_payload: u64,              // offset 256, size 8
} // total: 264 bytes

#[repr(C, packed)]
pub struct SocketMsg {
    pub payload: SocketPayload,        // offset 0,   size 264
    pub session_id: u64,               // offset 264, size 8
    pub cmd: [u8; 16],                 // offset 272, size 16
    pub payload_type: u32,             // offset 288, size 4
    pub payload_char_len: u32,         // offset 292, size 4
    pub success: u32,                  // offset 296, size 4
} // total: 300 bytes
```

**重要：** C 结构体是 `#pragma pack(push, 1)` 的，Rust 端必须用 `#[repr(C, packed)]` 确保字节级兼容。

### 3.2 Payload 类型枚举

```rust
pub const PAYLOAD_TYPE_NONE: u32 = 0;
pub const PAYLOAD_TYPE_CHAR: u32 = 1;
pub const PAYLOAD_TYPE_ULL:  u32 = 2;
pub const VSOCK_MSG_ACK:     u32 = 0xb;
```

### 3.3 迁移信息结构（ioctl 交互）

```rust
// 来源: inc/migcvm_tsi/migcvm_tsi.h
// 注意: 这些结构通过 ioctl 与内核交互，必须精确匹配

pub const MAX_BIND_VM: usize = 256;

#[repr(C)]
pub struct PendingGuestRd {
    pub guest_rd: [u64; MAX_BIND_VM],   // 256 * 8 = 2048 bytes
}

#[repr(C)]
pub struct MigrationInfo {
    pub msk: [u64; 4],                  // 4 * 8 = 32 bytes (offset 0)
    pub rand_iv: [u64; 4],              // 4 * 8 = 32 bytes (offset 32)
    pub tag: [u64; 2],                  // 2 * 8 = 16 bytes (offset 64)
    pub pending_guest_rds: u64,         // 指针, 8 bytes (offset 80)
    pub slot_status: u16,               // 2 bytes      (offset 88)
    pub set_key: u8,                    // 1 byte       (offset 90)
    // 隐式 padding: 5 bytes 使结构体 8 字节对齐
    // total: ~96 bytes
}

#[repr(C)]
pub struct VirtccaMigInfo {
    pub ops: u64,           // OP_MIGRATE_GET_ATTR=0 / SET_SLOT=1 / PEEK_RDS=2
    pub mig_info: u64,      // 指针 → MigrationInfo
    pub guest_rd: u64,      // 用于 OP_MIGRATE_SET_SLOT
    pub size: u64,
}
```

### 3.4 Attestation Token 命令结构

```rust
// 来源: inc/migcvm_tsi/migcvm_tsi.h

#[repr(C)]
pub struct CvmAttestationCmd {
    pub challenge: [u8; 64],                                  // 64 bytes
    pub token: [u8; 4096 * 2], // GRANULE_SIZE * MAX_TOKEN_GRANULE_COUNT = 8192
    pub token_size: u64,                                      // 8 bytes
}
```

### 3.5 Slot 状态枚举

```rust
pub const SLOT_IS_EMPTY:   u16 = 0;
pub const SLOT_NOT_BINDED: u16 = 1;
pub const SLOT_IS_BINDED:  u16 = 2;
pub const SLOT_IS_READY:   u16 = 3;
```

### 3.6 ioctl 命令码（精确计算）

```rust
// _IOWR(TSI_MAGIC, 1, cvm_attestation_cmd_t)  → TMM_GET_ATTESTATION_TOKEN
// _IOR(TSI_MAGIC, 2, cca_dev_cert_t)           → TMM_GET_DEV_CERT
// _IOWR(TSI_MAGIC, 3, virtcca_mig_info_t)      → TMM_GET_MIGRATION_INFO
// _IOW(TSI_MAGIC, 4, virtcca_migvm_checksum_info_t) → TMM_GET_MIGVM_MEM_CHECKSUM

// Linux ioctl 编码：
// _IOWR(type, nr, size) = _IOC(_IOC_READ|_IOC_WRITE, type, nr, size)
// _IOC(dir, type, nr, size) = ((dir) << _IOC_DIRSHIFT) | ((type) << _IOC_TYPESHIFT)
//                            | ((nr) << _IOC_NRSHIFT) | ((size) << _IOC_SIZESHIFT)
// 其中 _IOC_READ=2, _IOC_WRITE=1, → dir=3 for _IOWR
// _IOC_TYPESHIFT=8, _IOC_NRSHIFT=0, _IOC_SIZESHIFT=16

// TSI_MAGIC = 'T' = 0x54
// TMM_GET_ATTESTATION_TOKEN = _IOWR(0x54, 1, sizeof(cvm_attestation_cmd_t))
// TMM_GET_MIGRATION_INFO     = _IOWR(0x54, 3, sizeof(virtcca_mig_info_t))

// 对于 baremetal，如果直接通过寄存器/SMC 调用，ioctl 编码可能不适用，
// 但 op 编号 (1=token, 2=dev_cert, 3=migration, 4=checksum) 是确定的
```

### 3.7 迁移密钥包（MSK 传输格式）

```rust
// 来源: rats_tls_handler.c rats_tls_client_startup() 末尾
// migrate_key_package[10] 布局:
//
// migrate_key_package[0..4]  = msk[0..4]      // 4 个 u64, 共 32 bytes
// migrate_key_package[4..8]  = rand_iv[0..4]   // 4 个 u64, 共 32 bytes
// migrate_key_package[8..10] = tag[0..2]       // 2 个 u64, 共 16 bytes
// 总计: 10 * 8 = 80 bytes

pub const MIGRATE_KEY_PACKAGE_SIZE: usize = 10 * 8; // 80 bytes
```

---

## 4. 整体架构与生命周期

```
┌─────────────────────────────────────────────────────────────────┐
│  main()                                                          │
│  ├── CPU 亲和性: 绑定主线程到 CPU 0                               │
│  ├── rim_initialize(): 获取 attestation token → 提取 RIM          │
│  ├── parse_input(): 解析 -c/-s/-r 参数                           │
│  ├── open("/dev/vsock") + ioctl(IOCTL_VM_SOCKETS_GET_LOCAL_CID)  │
│  │   → 获取本机 vsock_cid                                        │
│  ├── 分配 server_args + client_args (两个独立的 mig_agent_args)    │
│  │                                                               │
│  ├── pthread_create(server_thread)                                │
│  │   └── server_thread_func()                                    │
│  │       ├── CPU 亲和性 CPU 0                                     │
│  │       ├── mig_agent_init(args)                                │
│  │       ├── socket_agent_start_with_handler(                     │
│  │       │     port=SERVER_AGENT_PORT(9001),                     │
│  │       │     handler=ras_tls_handler_server)                   │
│  │       └── [死循环 accept VSOCK 连接]                           │
│  │                                                               │
│  ├── pthread_create(client_thread)                               │
│  │   └── client_thread_func()                                    │
│  │       ├── CPU 亲和性 CPU 0                                     │
│  │       ├── socket_agent_start_with_handler(                     │
│  │       │     port=CLIENT_AGENT_PORT(9000),                     │
│  │       │     handler=ras_tls_handler_client)                   │
│  │       └── [死循环 accept VSOCK 连接]                           │
│  │                                                               │
│  └── pthread_join(server) + pthread_join(client)                 │
│      → 正常退出 (实际不会，因为有死循环)                            │
└─────────────────────────────────────────────────────────────────┘
```

**要点：**
- 同一个二进制同时运行在源端和目的端，角色由 QEMU 发送的 VSOCK 指令决定
- Server 线程（9001）和 Client 线程（9000）是**相互独立**的，分别处理不同角色的请求
- 每台 VM 上，只有对应角色的线程会被 QEMU 触发

---

## 5. RIM 初始化流程

```rust
// 位置: migcvm-agent.c rim_initialize()
fn rim_initialize() -> Option<[u8; MAX_MEASUREMENT_SIZE]> {
    // 1. 打开 /dev/tsi
    let ctx = tsi_new_ctx()?;  // open("/dev/tsi", O_RDWR)
    
    // 2. 准备 challenge
    let mut challenge = [0xFFu8; 64]; // 全 0xFF
    let challenge_len = 64;
    
    // 3. 获取 attestation token
    //    ioctl(fd, TMM_GET_ATTESTATION_TOKEN, &cmd)
    //    cmd.challenge = challenge
    //    → cmd.token 收到 token 数据, cmd.token_size = 实际长度
    
    // 4. 解析 CBOR token
    //    parse_cca_attestation_token(&parsed_token, token, token_len)
    //    → 提取 parsed_token.cvm_token.rim (qbuf_t { ptr, len })
    
    // 5. 复制 RIM 到 g_rim_ref
    //    memcpy(g_rim_ref, parsed_token.cvm_token.rim.ptr, min(rim.len, MAX_MEASUREMENT_SIZE))
    //    g_rim_ref_size = parsed_token.cvm_token.rim.len
}
```

**注意：** RIM 也可以在命令行通过 `-r` 参数显式提供，此时不进行 token 提取。

---

## 6. 详细流程：源端（Client）

### 6.1 VSOCK 消息接收（ras_tls_handler_client）

```
QEMU → VSOCK port 9000
  msg.cmd = "START_CLIENT"       (16 bytes, null-terminated)
  msg.payload_type = PAYLOAD_TYPE_NONE? (实际代码用 payload_decode_all)
  msg.payload.char_payload = 目标 IP 字符串
  msg.payload.ull_payload = guest_rd  (来自 QEMU)
```

**处理流程：**

```
ras_tls_handler_client(msg, conn_fd, args)
│
├── 步骤 1: 解析消息
│   payload_decode_all(msg, &payload)
│   host_srv_ip = payload.char_payload  (目标 IP)
│   args.guest_rd = payload.ull_payload
│
├── 步骤 2: 创建 TSI 上下文
│   tsi_new_ctx() → open("/dev/tsi", O_RDWR)
│
├── 步骤 3: 校验 guest_rd 合法性
│   get_migration_binded_rds(ctx, migvm_info, attest_info)
│   → ioctl(TMM_GET_MIGRATION_INFO, OP_MIGRATE_PEEK_RDS)
│   → 遍历 attest_info.pending_guest_rds.guest_rd[0..MAX_BIND_VM]
│   → 查找是否有匹配的 guest_rd
│   → 不匹配则返回错误
│
├── 步骤 4: 获取迁移密钥信息
│   get_migration_info_and_mask(ctx, migvm_info, attest_info)
│   → ioctl(TMM_GET_MIGRATION_INFO, OP_MIGRATE_GET_ATTR)
│   → attest_info.msk, attest_info.rand_iv, attest_info.tag 被填充
│   → attest_info.set_key 被设置为 true (表示这是源端)
│   → 复制到 args: args.msk = attest_info.msk, 同理 tag, rand_iv
│
├── 步骤 5: RATS-TLS 客户端启动
│   rats_tls_client_startup(args)
│   → 见 6.2 节详细流程
│
├── 步骤 6: 设置源端 slot 就绪
│   attest_info.slot_status = SLOT_IS_READY
│   attest_info.set_key = true
│   set_migration_bind_slot_and_mask(ctx, migvm_info, attest_info)
│   → ioctl(TMM_GET_MIGRATION_INFO, OP_MIGRATE_SET_SLOT)
│
├── 步骤 7: 回复 ACK
│   ack_msg.cmd = "START_CLIENT_ACK"
│   ack_msg.payload_type = VSOCK_MSG_ACK
│   ack_msg.session_id = msg.session_id  (原样返回)
│   ack_msg.success = (ret == TSI_SUCCESS) ? 1 : 0
│   writen(conn_fd, &ack_msg, sizeof(ack_msg))
│   shutdown(conn_fd, SHUT_WR)
│   readn(conn_fd, tmp, 8)  -- 等待对端关闭
│
└── 步骤 8: 清理
    释放 attest_info, migvm_info, ctx, host_srv_ip
    清零 args 中的敏感数据 (guest_rd, msk, tag, rand_iv)
```

### 6.2 RATS-TLS 客户端详细流程

```
rats_tls_client_startup(args)  -- 在 src/rats_tls_handler/rats_tls_handler.c
│
├── 1. 配置 static_args (因为 user_callback 需要跨生命周期访问)
│      static mig_agent_args static_args;
│      memcpy(&static_args, args, sizeof(mig_agent_args));
│
├── 2. 配置 RATS-TLS conf
│      conf.attester_type = ""
│      conf.verifier_type = ""
│      conf.tls_type = "openssl"
│      conf.crypto_type = "openssl"
│      conf.cert_algo = RATS_TLS_CERT_ALGO_RSA_3072_SHA256
│      conf.flags = mutual? RATS_TLS_CONF_FLAGS_MUTUAL : 0
│      conf.custom_claims = &static_claim  (name="mig_agent_args", value=&static_args)
│
├── 3. TCP connect → 目标 IP:1234 (MIGCVM_PORT)
│
├── 4. rats_tls_init + rats_tls_set_verification_callback(handle, user_callback)
│      → user_callback 详见 6.3 节
│
├── 5. rats_tls_negotiate(handle, sockfd)
│      TLS 握手过程中会触发 user_callback 进行远程证明验证
│
├── 6. [可选] 平台 SW 组件验证 (verify_platform_components)
│      从 verifier context 获取 token → 解析 → 加载 ref JSON → verify_platform_sw_components
│
├── 7. [可选] IMA 度量验证 (deal_ima)
│      发送 "REQUEST_IMA_LOG" → 接收 IMA 日志 → 写入文件
│      → 解析 PCR index → 从 token REM 获取参考值 → ima_measure 验证
│
├── 8. [可选] 固件状态验证 (use_firmware / dump_eventlog)
│      发送 "REQUEST_CCEL_TABLE" → 接收并保存 CCEL ACPI 表
│      发送 "REQUEST_EVENT_LOG"  → 接收并保存 Event Log
│      → event_log_init → event_log_replay (计算 REM)
│      → 对比 token 中的 REM[0..3]
│      → 创建 firmware_log_state → extract → verify
│
├── 9. [可选] 全盘加密密钥 (deal_fde_key)
│      发送 "ENABLE_FDE_TOKEN" → 读取 rootfs_key_file → 发送给 Server
│
├── 10. ★ MSK 密钥发送 ★
│      发送 "MIG_MSK_SEND"
│      接收 "MSK_ACK"
│      构造 migrate_key_package[10]:
│        [0..4]  = args.msk[0..4]
│        [4..8]  = args.rand_iv[0..4]
│        [8..10] = args.tag[0..2]
│      发送 migrate_key_package (80 bytes)
│
└── 11. 启动完整性校验线程
       integrity_socket_t params = { guest_rd, sockfd, handle }
       pthread_create → io_thread (detached)
       返回 ret
```

### 6.3 user_callback 验证流程

```
user_callback(args) → bool
│
├── 1. 获取 evidence
│   ev = (rtls_evidence_t *)args
│   cca_token_buf = ev->cca.evidence (大小检查: <= VIRTCCA_TOKEN_SIZE=4104)
│
├── 2. 解析 CCA attestation token
│   parse_cca_attestation_token(&token, cca_token_buf, buf_size)
│   token 结构:
│     cvm_token: { challenge, rpv, rim, rem[4], hash_algo_id, pub_key, pub_key_hash_algo_id }
│     platform_token: { profile, challenge, implementation_id, instance_id, config,
│                       lifecycle, sw_components, verification_service, hash_algo_id }
│     cvm_cose: COSE_Sign1 信封 (CVM token)
│     platform_cose: COSE_Sign1 信封 (Platform token, 可选)
│
├── 3. 检测 AIK 证书类型 → 配置证书链
│   detect_aik_cert_type(DEFAULT_AIK_CERT_PEM_FILENAME)
│   configure_cert_info_by_type(&cert_info, aik_cert_type)
│
├── 4. 验证 token 签名
│   verify_cca_token_signatures(&cert_info, platform_cose, cvm_cose,
│       pub_key, platform_challenge, pub_key_hash_algo_id)
│
├── 5. ★ 验证 RIM ★
│   if token.cvm_token.rim.len != g_rim_ref_size
│      || memcmp(g_rim_ref, token.cvm_token.rim.ptr, rim.len) != 0
│      → return false
│
└── 6. 返回 true
```

---

## 7. 详细流程：目的端（Server）

### 7.1 VSOCK 消息接收（ras_tls_handler_server）

```
QEMU → VSOCK port 9001
  msg.cmd = "START_SERVER"
  msg.payload_type = PAYLOAD_TYPE_ULL
  msg.payload.ull_payload = guest_rd
```

**处理流程：**

```
ras_tls_handler_server(msg, conn_fd, args)
│
├── 1. 解析 guest_rd
│   payload_decode_one_type(msg, &payload)
│   args.guest_rd = payload.ull_payload
│
├── 2. 发送 ACK
│   ack_msg.cmd = "START_SERVER_ACK"
│   ack_msg.payload_type = VSOCK_MSG_ACK
│   ack_msg.session_id = msg.session_id
│   ack_msg.success = (args.guest_rd != 0) ? 1 : 0
│   writen(conn_fd, &ack_msg, sizeof(ack_msg))
│   shutdown(conn_fd, SHUT_WR)
│   readn(conn_fd, tmp, 8)  -- 等待对端关闭
│
└── 3. 启动 RATS-TLS Server
    if ack_msg.success == 1:
        rats_tls_server_startup(args)
```

### 7.2 RATS-TLS 服务端详细流程

```
rats_tls_server_startup(args)
│
├── 1. 配置 RATS-TLS conf
│      conf.attester_type = ""   (服务端不产生证据)
│      conf.verifier_type = ""   (服务端不验证对方)
│      conf.tls_type = "openssl"
│      conf.crypto_type = "openssl"
│      conf.cert_algo = RATS_TLS_CERT_ALGO_RSA_3072_SHA256
│      conf.flags |= RATS_TLS_CONF_FLAGS_SERVER
│      conf.flags |= RATS_TLS_CONF_FLAGS_MUTUAL (if mutual)
│
├── 2. 创建 TCP socket
│      socket(AF_INET, SOCK_STREAM)
│      setsockopt(SO_REUSEADDR)
│      setsockopt(SO_KEEPALIVE + TCP_KEEPIDLE=30 + TCP_KEEPINTVL=10 + TCP_KEEPCNT=5)
│
├── 3. bind(listen_ip, port=1234) + listen(5)
│
├── 4. accept() → 等待源端连接
│
├── 5. rats_tls_init(handle) + rats_tls_set_verification_callback(handle, NULL)
│      注意: Server 端 user_callback 为 NULL!
│
├── 6. rats_tls_negotiate(handle, connd) → TLS 握手
│
└── 7. deal_client_req(handle, args)  ★ 核心请求处理循环 ★
        (详见 7.3 节)
```

### 7.3 deal_client_req 请求处理状态机

**协议：** Server 端以循环（递归）方式处理多个 Client 请求，每次处理一个请求后递归调用自身：

```
deal_client_req(handle, args)
│
├── rats_tls_receive → 接收请求字符串 (max 256 bytes)
│
├── match buf:
│
│   "REQUEST_CCEL_TABLE" → send_ccel_data(handle)
│   │   读取 /sys/firmware/acpi/tables/CCEL → rats_tls_transmit
│   │   → return deal_client_req(handle, args)  ★ 递归 ★
│   │
│   "REQUEST_EVENT_LOG" → send_event_log(handle)
│   │   读取 /sys/firmware/acpi/tables/data/CCEL
│   │   → 先发送 size_t(8 bytes): event_log_size
│   │   → 再分块发送 event_log 数据
│   │   → return deal_client_req(handle, args)  ★ 递归 ★
│   │
│   "ATTESTATION_PASS" → 
│       发送 "Attestation Passed, Switching Root..."
│       → return RATS_TLS_ERR_NONE  ★ 终止递归，成功返回 ★
│   │
│   "REQUEST_IMA_LOG" → send_ima_log(handle)
│   │   读取 /sys/kernel/security/ima/binary_runtime_measurements
│   │   → 先发送 ima_size (8 bytes)
│   │   → 再分块发送 IMA 日志
│   │   → return deal_client_req(handle, args)  ★ 递归 ★
│   │
│   "ENABLE_FDE_TOKEN" → deal_rootfs_key(handle)
│   │   接收 rootfs key 数据 → 保存到 /root/rootfs_key.bin
│   │   → return deal_client_req(handle, args)  ★ 递归 ★
│   │   → 如果成功，继续等待 "ATTESTATION_PASS"
│   │   → 最终返回 0x68 (LUKS 完成标记)
│   │
│   "MIG_MSK_SEND" → 回复 "MSK_ACK" → receive_and_save_msk(handle, args)
│       (详见 7.4 节)
│       → return ret  ★ 终止 ★
│
│   其他 → 回复 "Attestation Failed, Continue..."
│        → return ENCLAVE_ATTESTER_ERR_UNKNOWN
```

**关键点：**
- `deal_client_req` 是**递归**的：处理完非终止请求后递归调用自己
- 终止请求：`TOKEN`（返回 0）、`MIG_MSK_SEND`（返回 ret）
- 请求**顺序**由 Client 端决定，必须与 6.2 节中的发送顺序一致

### 7.4 receive_and_save_msk 详细流程

```
receive_and_save_msk(handle, args)
│
├── 1. 创建 TSI 上下文
│   tsi_new_ctx() → open("/dev/tsi", O_RDWR)
│
├── 2. 接收密钥包
│   rats_tls_receive(handle, migrate_key_package, 80)
│   migrate_key_package 布局:
│     [0..4]  = msk[0..4]
│     [4..8]  = rand_iv[0..4]
│     [8..10] = tag[0..2]
│
├── 3. 校验 guest_rd 合法性
│   get_migration_binded_rds(ctx, migvm_info, attest_info)
│   → 遍历 attest_info.pending_guest_rds.guest_rd[0..MAX_BIND_VM]
│   → 确认 args.guest_rd 在绑定列表中
│
├── 4. 填充 migration_info
│   attest_info.set_key = false  (这是目的端)
│   attest_info.msk     = migrate_key_package[0..4]
│   attest_info.rand_iv = migrate_key_package[4..8]
│   attest_info.tag     = migrate_key_package[8..10]
│   attest_info.slot_status = SLOT_IS_READY
│
├── 5. 写入 TSI
│   set_migration_bind_slot_and_mask(ctx, migvm_info, attest_info)
│   → ioctl(TMM_GET_MIGRATION_INFO, OP_MIGRATE_SET_SLOT)
│
└── 6. 清理敏感数据 + 释放资源
```

---

## 8. 完整性校验线程

RATS-TLS 完成后，Client 端启动一个 detach 的完整性校验线程：

```rust
// 位置: rats_tls_handler.c rats_tls_client_startup() 末尾
// 以及 integrity_check_handler.c

struct IntegritySocket {
    guest_rd: u64,       // 迁移的 VM 标识
    socket_fd: i32,      // RATS-TLS socket fd
    handle: *mut Handle, // RATS-TLS handle
}

// 线程入口: io_thread(params)
// 功能:
// - 打开 /dev/tsi 和 /dev/migvm_queue_mem
// - 通过 epoll 监听 RATS-TLS socket + TSI fd
// - 从 /dev/migvm_queue_mem 共享内存队列读取数据 → 通过 RATS-TLS 发送
// - 从 RATS-TLS 接收数据 → 写入 /dev/migvm_queue_mem
// - 使用 TMM_GET_MIGVM_MEM_CHECKSUM ioctl 进行内存校验和计算
// - 使用 16 个 CPU(0..15) 作为工作线程，CPU 16 作为主线程
```

**注意：** baremetal 重写时，完整性校验线程可能不适用（取决于是否有共享内存队列机制），但需要了解其存在。

---

## 9. VSOCK Socket Agent 核心模式

```rust
// 来源: src/socket/host_socket_agent.c

struct SocketAgentCfg {
    args: *mut MigAgentArgs,
    cid: u64,        // VSOCK CID (VMADDR_CID_ANY = 0xFFFFFFFF)
    port: u32,       // 9000 或 9001
    backlog: i32,    // 128
}

fn socket_agent_start_with_handler(cfg, handler) {
    // 1. socket(AF_VSOCK, SOCK_STREAM, 0)
    //    AF_VSOCK = 40 (Linux)
    //    sockaddr_vm = { svm_family: AF_VSOCK, svm_cid: cfg.cid, svm_port: cfg.port }
    
    // 2. setsockopt(SO_REUSEADDR)
    
    // 3. bind(listen_fd, sockaddr_vm)
    
    // 4. listen(listen_fd, cfg.backlog)
    
    // 5. loop {
    //      conn_fd = accept(listen_fd, &peer_sa, &peer_len)
    //      getsockname(conn_fd) → 获取 local port (用于日志)
    //      
    //      readn(conn_fd, &msg, sizeof(SocketMsg))  -- 读取 300 bytes
    //      
    //      handler(&msg, conn_fd, cfg.args)         -- 调用消息处理器
    //      
    //      close(conn_fd)
    //    }
}

// 可靠 I/O 原语:
fn readn(fd, buf, n) {
    // 循环 read() 直到读满 n 字节或 EOF
    // 处理 EINTR (中断重试)
    // read 返回 0 表示对端关闭
}

fn writen(fd, buf, n) {
    // 循环 write() 直到写满 n 字节
    // 处理 EINTR (中断重试)
}
```

**重要模式：ACK 后的 graceful shutdown**

每次 handler 回复 ACK 后，都使用以下模式等待对端关闭：

```c
writen(conn_fd, &ack_msg, sizeof(ack_msg));  // 发送 ACK
shutdown(conn_fd, SHUT_WR);                   // 关闭写端
char tmp[8];
readn(conn_fd, tmp, sizeof(tmp));             // 读取对端的 close 信号
// 然后 close(conn_fd)
```

---

## 10. 通信矩阵（完整版）

```
┌──────────────────────────────────────────────────────────────────┐
│                      三方通信关系                                  │
├────────────┬─────────────────┬──────────────────┬────────────────┤
│   通信方    │      协议        │      端口        │     方向       │
├────────────┼─────────────────┼──────────────────┼────────────────┤
│ QEMU→源端  │ VSOCK (SOCK_STREAM)│   9000         │ 宿主机→VM     │
│ QEMU→目的端│ VSOCK (SOCK_STREAM)│   9001         │ 宿主机→VM     │
│ 源端→目的端 │ TCP (TLS)        │   1234          │ VM→VM        │
├────────────┴─────────────────┴──────────────────┴────────────────┤
│ VSOCK 消息格式: 二进制 SocketMsg (300 bytes, packed)              │
│ TCP 消息格式:  RATS-TLS 加密信道 + 字符串协议命令                   │
│ TLS 认证:      RATS_TLS_CERT_ALGO_RSA_3072_SHA256                 │
│ 远程证明:      CCA attestation token (CBOR/COSE)                  │
└──────────────────────────────────────────────────────────────────┘
```

---

## 11. TSI ioctl 接口清单

| 操作 | ioctl 类型 | nr | 输入结构 | 输出 | 说明 |
|------|-----------|----|---------|------|------|
| `get_attestation_token` | `_IOWR` | 1 | `cvm_attestation_cmd_t` | token + token_size | 获取证明 token |
| `get_dev_cert` | `_IOR` | 2 | — | `cca_dev_cert_t` | 获取设备证书 |
| `get_migration_info` | `_IOWR` | 3 | `virtcca_mig_info_t` | 根据 ops 不同 | 迁移信息操作 |
| `get_migvm_mem_checksum` | `_IOW` | 4 | `virtcca_migvm_checksum_info_t` | — | 内存校验和 |

**`TMM_GET_MIGRATION_INFO` 的 ops 子命令：**

| ops 值 | 枚举名 | 说明 | 方向 |
|--------|--------|------|------|
| 0 | `OP_MIGRATE_GET_ATTR` | 获取 MSK/IV/Tag（源端用） | 内核→用户 |
| 1 | `OP_MIGRATE_SET_SLOT` | 设置 slot 状态和密钥 | 用户→内核 |
| 2 | `OP_MIGRATE_PEEK_RDS` | 查看绑定的 guest_rd 列表 | 内核→用户 |

---

## 12. 文件路径依赖

Rust 重写时这些路径可能需要适配 baremetal 环境（如改用直接读取内存/ACPI 表）：

| C 代码路径 | 用途 | 角色 |
|-----------|------|------|
| `/dev/tsi` | TSI 设备 | 双方 |
| `/dev/vsock` | VSOCK 设备（获取 CID） | 双方 |
| `/sys/kernel/security/ima/binary_runtime_measurements` | IMA 日志 | Server 读取 |
| `/sys/firmware/acpi/tables/CCEL` | CCEL ACPI 表 | Server 读取 |
| `/sys/firmware/acpi/tables/data/CCEL` | Event Log | Server 读取 |
| `binary_runtime_measurements` (当前目录) | IMA 日志缓存 | Client 写入 |
| `/root/rootfs_key.bin` | 文件加密密钥 | Server 写入 |
| `/dev/migvm_queue_mem` | 迁移内存队列 | 完整性校验 |

---

## 13. 状态机总览

### 13.1 源端状态

```
[等待 START_CLIENT]
    │ VSOCK msg 到达
    ▼
[初始化 TSI + 校验 guest_rd]
    │ guest_rd 合法
    ▼
[获取 MSK/IV/Tag from TSI]
    │ OP_MIGRATE_GET_ATTR
    ▼
[RATS-TLS connect → 目的端]
    │ TLS 握手 + 远程证明
    ▼
[可选验证: IMA → Platform → Firmware → FDE]
    │ 按序执行
    ▼
[发送 "MIG_MSK_SEND" → 等待 "MSK_ACK" → 发送密钥包]
    │ 80 bytes: msk[4]+iv[4]+tag[2]
    ▼
[设置 slot_status=READY (OP_MIGRATE_SET_SLOT)]
    │
    ▼
[回复 "START_CLIENT_ACK" to QEMU]
    │
    ▼
[启动完整性校验线程 → 后台运行]
```

### 13.2 目的端状态

```
[等待 START_SERVER]
    │ VSOCK msg 到达
    ▼
[回复 "START_SERVER_ACK" to QEMU]
    │ success = (guest_rd != 0)
    ▼
[启动 RATS-TLS TCP Server :1234]
    │ accept()
    ▼
[RATS-TLS 握手]
    │
    ▼
[deal_client_req 循环]
    ├── REQUEST_CCEL_TABLE → 发送 → 循环
    ├── REQUEST_EVENT_LOG  → 发送 → 循环
    ├── REQUEST_IMA_LOG    → 发送 → 循环
    ├── ENABLE_FDE_TOKEN   → 接收 → 循环
    ├── MIG_MSK_SEND       → 回复 ACK → receive_and_save_msk → 结束
    └── ATTESTATION_PASS   → 回复 → 结束
```

---

## 14. 错误处理策略

| 错误类型 | 处理方式 |
|---------|---------|
| VSOCK 连接失败 | 继续 accept 下一个连接（死循环） |
| guest_rd 不在绑定列表 | `ack_msg.success = 0`，应答失败 |
| TSI ioctl 失败 | `goto out`，清理资源，回复失败 ACK |
| RATS-TLS 握手失败 | `goto err`，关闭 socket，清理资源 |
| RIM 验证失败 | `user_callback` 返回 false，TLS 握手失败 |
| MSK 发送/接收失败 | 回复错误，不阻塞后续操作 |
| IMA 验证失败 | `goto err`，终止流程 |

**关键原则：** 任何敏感数据（msk, tag, rand_iv, guest_rd）在流程结束后立即 `memset` 清零。

---

## 15. Baremetal Rust 重写建议

### 15.1 需要重写的模块（按优先级）

| 优先级 | 模块 | Rust 替代方案 |
|--------|------|-------------|
| P0 | VSOCK Socket Agent | 直接操作 VSOCK 设备（`/dev/vsock` 或 hypercall） |
| P0 | TSI 接口 | 直接 ioctl 或 SMC 调用 |
| P0 | RATS-TLS Client | 需要 TLS 库 + 远程证明集成 |
| P0 | RATS-TLS Server | 同上 |
| P1 | Token 解析 | CBOR/COSE 解析（可用 `ciborium` + `coset` crate） |
| P1 | RIM 验证 | 字节比较 |
| P1 | MSK 密钥交换 | TCP + TLS + 二进制序列化 |
| P2 | IMA 验证 | IMA 日志解析器 |
| P2 | Event Log 重放 | TPM2 event log 解析 + SHA256 哈希 |
| P3 | 完整性校验线程 | 依赖具体 baremetal 环境 |

### 15.2 可简化的部分

1. **网络 IP 发现**：baremetal 环境下 IP 可能由启动参数传入，可简化 `get_local_ipv4` 的复杂回退逻辑
2. **CPU 亲和性**：baremetal 下可直接控制 CPU 分配，不需要 `pthread_setaffinity_np`
3. **文件路径**：IMA 日志、CCEL 表等可能通过直接内存访问或 ACPI 表读取
4. **pthread**：可用 `async`/`task` 替代线程模型

### 15.3 不变的核心逻辑

1. **VSOCK 消息协议**：`SocketMsg` 的 300 字节 packed 二进制格式不能变
2. **TSI ioctl 接口**：`MigrationInfo` 结构的字段布局和 ioctl 命令码不能变
3. **MSK 密钥包格式**：`migrate_key_package[10]` 的 80 字节布局不能变
4. **RATS-TLS 请求/响应协议**：字符串命令和交互顺序不能变
5. **deal_client_req 的递归状态机**：请求处理顺序和终止条件不能变

---

## 16. 编译依赖（参考）

```
CMakeLists.txt 中的链接依赖:
  - attest (静态库): token_parse, token_validate, platform_verify,
                     ima_measure, event_log, rem, firmware_state,
                     binary_blob, verify, config, utils
  - migcvm-tsi (静态库): migcvm_tsi
  - m (libm)
  - OpenSSL::Crypto
  - t_cose (CBOR/COSE 签名验证)
  - qcbor (CBOR 解析)
  - rats-tls (远程证明 TLS 框架)
```

---

## 17. 完整迁移时序（精确版）

```
时间线 →

源端 CVM                                    目的端 CVM
─────────                                   ─────────
main():                                     main():
  rim_initialize()                            rim_initialize()
  parse_input()                              parse_input()
  open /dev/vsock, 获取 CID                   open /dev/vsock, 获取 CID
  start server_thread(port=9001)             start server_thread(port=9001)
    [空闲，等待 VSOCK]                          [空闲，等待 VSOCK]
  start client_thread(port=9000)             start client_thread(port=9000)
    [空闲，等待 VSOCK]                          [空闲，等待 VSOCK]
                                             
                        ◄──────────────── QEMU VSOCK → 目的端 9001
                        QEMU 发送: cmd="START_SERVER"
                                   payload.ull_payload = guest_rd
                                             
                        ras_tls_handler_server:
                          args.guest_rd = guest_rd
                          ack { "START_SERVER_ACK", success=1 }
                          ────────────────► QEMU
                          
                          rats_tls_server_startup:
                            TCP bind :1234
                            listen(5)
                            [等待源端连接...]

QEMU VSOCK → 源端 9000 ────────────────────►
  发送: cmd="START_CLIENT"
        payload.char_payload = 目标IP
        payload.ull_payload = guest_rd

ras_tls_handler_client:
  tsi_new_ctx() → /dev/tsi
  get_migration_binded_rds()
    → ioctl(OP_MIGRATE_PEEK_RDS)
  ✓ guest_rd 合法
  
  get_migration_info_and_mask()
    → ioctl(OP_MIGRATE_GET_ATTR)
  ← msk, rand_iv, tag 就绪
  set_key = true (源端标识)

  rats_tls_client_startup:
    TCP connect → 目的端:1234 ──────────────► accept()
    ┌─ RATS-TLS 握手 ────────────────────────► rats_tls_negotiate()
    │    交换 evidence
    │    user_callback:
    │      parse token
    │      verify signatures (AIK cert chain)
    │      verify RIM ✓
    │    
    │  [可选] IMA 验证:
    │    "REQUEST_IMA_LOG" ─────────────────► send_ima_log()
    │    ◄─────────────────────────────── ima_size + ima_data
    │    ima_measure() ✓
    │    
    │  [可选] 平台 SW 验证:
    │    verify_platform_sw_components()
    │    
    │  [可选] 固件验证:
    │    "REQUEST_CCEL_TABLE" ──────────────► send_ccel_data()
    │    ◄─────────────────────────────── CCEL table
    │    "REQUEST_EVENT_LOG" ───────────────► send_event_log()
    │    ◄─────────────────────────────── event_log_size + data
    │    event_log_replay() → 计算 REM
    │    compare REM[0..3] ✓
    │    
    │  [可选] FDE 密钥:
    │    "ENABLE_FDE_TOKEN" ────────────────► deal_rootfs_key()
    │    rootfs_key_file ───────────────────► 保存到 /root/rootfs_key.bin
    │    
    │  ★ MSK 交换 ★:
    │    "MIG_MSK_SEND" ────────────────────►
    │    ◄──────────────────────────────── "MSK_ACK"
    │    migrate_key_package[10] ───────────► receive_and_save_msk()
    │      msk[4]+iv[4]+tag[2]                 校验 guest_rd ✓
    │      共 80 bytes                          set_migration_bind_slot_and_mask()
    │                                             → ioctl(OP_MIGRATE_SET_SLOT)
    │                                             slot_status = READY
    └─ 握手完成 ────────────────────────────►
    
    [启动完整性校验线程]
  
  set_migration_bind_slot_and_mask()
    → ioctl(OP_MIGRATE_SET_SLOT)
    slot_status = READY (源端)
  
  ack { "START_CLIENT_ACK", success=1 }
  ────────────────────────────────────────► QEMU

═════════════════════════════════════════════════════════════
  此时源端和目的端 slot 状态均为 SLOT_IS_READY
  QEMU 收到双方的 ACK 后，开始实际的热迁移
═════════════════════════════════════════════════════════════
```
