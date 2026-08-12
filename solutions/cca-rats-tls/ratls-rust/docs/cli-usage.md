# CCA RA-TLS CLI 使用说明

本文档说明 `ratls-sample` 提供的 `cca-server` 和 `cca-client` 命令行参数、输入限制及常见测试方式。

## 构建与查看帮助

```bash
cargo build -p ratls-sample

target/debug/cca-server --help
target/debug/cca-client --help
```

也可以直接通过 Cargo 运行：

```bash
cargo run -p ratls-sample --bin cca-server -- --help
cargo run -p ratls-sample --bin cca-client -- --help
```

CLI 会在监听或连接网络前检查参数关系、输入文件和策略文件。校验失败时退出，不执行 TLS 握手。

## 公共网络参数

| 参数 | 作用 | 默认值 | 限制 |
| --- | --- | --- | --- |
| `-i, --ip <IP>` | 服务端监听地址或客户端连接地址 | 服务端和客户端均为 `127.0.0.1` | 只接受合法 IPv4 或 IPv6，不接受 DNS 主机名 |
| `-p, --port <PORT>` | TCP 监听或连接端口 | `1234` | `1..=65535`，不允许 0 |

## 本端动态证书参数

客户端与服务端共用以下参数：

| 参数 | 作用 | 默认值和限制 |
| --- | --- | --- |
| `--cert-algo <CERT_ALGO>` | 动态叶子私钥算法 | 默认 `ecc256`；可选 `ecc256`、`rsa3072` |
| `--issuer-private-key <PEM_FILE>` | 用于签发动态叶子证书的 CA 私钥 PEM | 必须是未加密私钥；文件大小 `1..=65536` 字节 |
| `--issuer-certificate-chain <PEM_FILE>` | 与 CA 私钥匹配的证书链 PEM bundle | 文件大小 `1..=1048576` 字节 |
| `--subject-alt-name <TYPE:VALUE>` | 写入动态叶子证书的 SAN；可以重复配置 | 最多 64 项；每项最多 2048 个 UTF-8 字节 |

证书生成规则：

- 私钥和证书链都不配置：生成临时叶子私钥和动态自签名证书。
- 私钥和证书链都配置：生成临时叶子私钥，并由给定 CA 动态签发。
- 只配置其中一项：CLI 直接报错。
- CA 私钥必须与 CA 证书匹配，CA 证书必须有效并允许签发证书。

SAN 支持以下格式：

```text
DNS:server.example.com
DNS:*.example.com
IP:192.0.2.10
IP:2001:db8::10
URI:spiffe://example.org/service
```

DNS 值为 ASCII，最大 253 字节；通配符只能是最左侧的完整 label，例如 `*.example.com`。

## 标准 TLS 对端证书校验参数

| 参数 | 作用 | 限制 |
| --- | --- | --- |
| `--verify-peer-certificate` | 在 RA 校验外开启 CA 信任链校验 | 开启后必须至少选择系统 CA 或自定义 CA |
| `--use-system-ca` | 加载 OpenSSL 默认系统 CA | 必须与 `--verify-peer-certificate` 一起使用 |
| `--trusted-ca-chain <PEM_FILE>` | 加载自定义可信 CA PEM bundle | 必须与校验开关一起使用；文件大小 `1..=4194304` 字节 |
| `--expected-peer-name <DNS_OR_IP>` | 校验证书中的对端 DNS 名称或 IP | 必须与校验开关一起使用；最大 253 字节；不允许通配符 |

系统 CA 和自定义 CA 可以同时配置，两者会合并为同一个信任集合。服务端校验客户端证书时必须同时开启 `--mutual`。

CLI 不允许在关闭 `--verify-peer-certificate` 时单独填写系统 CA、自定义 CA 或预期名称，避免用户误以为已经启用标准 TLS 校验。

未开启标准 CA 校验时，库仍会执行证书签名、有效期、`BasicConstraints CA:FALSE`、`KeyUsage digitalSignature`、角色 EKU 和 RA evidence 校验。当前不主动执行 CRL 或 OCSP 吊销检查。

## cca-server 参数

| 参数 | 作用 | 默认值和限制 |
| --- | --- | --- |
| `-1, --once` | 处理一个连接后退出 | 默认持续监听 |
| `-m, --mutual` | 开启双向 RA-TLS，要求并验证客户端 CCA evidence | 服务端使用 RIM、platform policy 或 TLS 客户端证书校验时必须开启 |
| `--ima-log <FILE>` | 客户端请求 IMA log 时读取的文件 | 默认 `/sys/kernel/security/ima/binary_runtime_measurements` |
| `--ccel-table <FILE>` | 客户端请求 CCEL ACPI table 时读取的文件 | 默认 `/sys/firmware/acpi/tables/CCEL` |
| `--event-log <FILE>` | 客户端请求 measured boot event log 时读取的文件 | 默认 `/sys/firmware/acpi/tables/data/CCEL` |
| `--rootfs-key <FILE>` | 保存客户端发送的 rootfs/FDE key | 默认 `/root/rootfs_key.bin`；父目录必须存在，目标不能是目录 |
| `--rim <HEX>` | 要求客户端已验证的 CCA RIM 与给定值相同 | 必须开启 `--mutual`；2～128 个十六进制字符，长度必须为偶数 |
| `-P, --platform <JSON>` | 使用 JSON policy 校验客户端 platform SW components | 必须开启 `--mutual`；普通可读文件，`1..=1 MiB` |
| `--max-key <BYTES>` | 允许接收的 rootfs/FDE key 最大长度 | 默认 65536 字节；范围 `1..=1048576` |
| `-l, --log-level <LEVEL>` | 日志级别 | 默认 `error`；可选 `debug`、`info`、`warn`、`error`、`fatal`、`none` |

IMA、CCEL 和 event log 使用默认路径时不会在服务端启动阶段检查；只有客户端实际请求时才读取。每个返回文件必须是普通非空文件，且不能超过 10 MiB。

## cca-client 参数

| 参数 | 作用 | 默认值和限制 |
| --- | --- | --- |
| `-M, --message <TEXT>` | 发送给服务端并等待 echo 的 UTF-8 消息 | 默认 `hello CCA`；`1..=4096` 字节 |
| `--message-file <FILE>` | 将二进制文件作为 echo 消息发送 | 普通可读文件，`1..=4096` 字节；与 `--message` 互斥 |
| `-m, --mutual` | 开启双向 RA-TLS，由客户端也生成 CCA evidence | 服务端也必须开启 `--mutual` |
| `-I, --ima-log` | 请求并解析服务端 IMA binary log | 可单独使用 |
| `-d, --digest <FILE>` | 使用 IMA digest baseline 校验 IMA log | 必须同时使用 `--ima-log`；文件 `1..=10 MiB` |
| `-g, --bootlog` | 请求 CCEL table 和 measured boot event log，将 registry 1/2 的重放值与 CCA REM[0]/REM[1] 比较 | 最大接收长度由 `--max-log` 控制 |
| `-f, --firmware <JSON>` | 使用 firmware baseline 校验已绑定的 boot log | 必须同时使用 `--bootlog`；文件 `1..=1 MiB` |
| `--rim <HEX>` | 要求服务端已验证的 CCA RIM 与给定值相同 | 2～128 个十六进制字符，长度必须为偶数 |
| `-P, --platform <JSON>` | 使用 JSON policy 校验服务端 platform SW components | 普通可读文件，`1..=1 MiB` |
| `-k, --fdekey <FILE>` | RA-TLS 握手成功后向服务端发送 rootfs/FDE key | 普通可读非空文件，最大 1 MiB；别名 `--fde-key` |
| `--max-log <BYTES>` | IMA、CCEL 或 boot log 单帧最大接收长度 | 默认和最大均为 10485760 字节（10 MiB）；最小 1 字节 |
| `-l, --log-level <LEVEL>` | 日志级别 | 默认 `error`；可选 `debug`、`info`、`warn`、`error`、`fatal`、`none` |

`--fdekey` 不强制依赖 RIM、platform 或 firmware policy；只要 RA-TLS 握手及用户配置的校验通过，客户端就会发送文件。

## 策略文件基础校验

策略文件会在联网前解析：

- Firmware baseline 必须是合法 JSON，`hash_alg` 当前为 `sha-256`，GRUB、`grub.cfg`、kernel 和 initramfs 摘要必须是合法 SHA-256 十六进制值。
- IMA digest baseline 每个有效行格式为 `<sha1|sha256> <hex-digest> <path>`，最多 100000 项，不能为空。
- Platform policy 必须是合法 JSON，包含非空 `measure_value`，measurement 和非通配符 signer ID 必须是合法十六进制值。

## 常用命令

### 单向动态自签名

服务端：

```bash
cargo run -p ratls-sample --bin cca-server -- \
  --ip 0.0.0.0 \
  --port 1234 \
  --once
```

客户端：

```bash
cargo run -p ratls-sample --bin cca-client -- \
  --ip 127.0.0.1 \
  --port 1234 \
  --message "hello CCA"
```

### CA 签发服务端证书并校验名称

服务端：

```bash
cargo run -p ratls-sample --bin cca-server -- \
  --once \
  --issuer-private-key server-ca.key \
  --issuer-certificate-chain server-ca.crt \
  --subject-alt-name DNS:server.test
```

客户端：

```bash
cargo run -p ratls-sample --bin cca-client -- \
  --ip 127.0.0.1 \
  --verify-peer-certificate \
  --trusted-ca-chain server-ca.crt \
  --expected-peer-name server.test
```

### 使用系统 CA 与自定义 CA

```bash
cargo run -p ratls-sample --bin cca-client -- \
  --ip 192.0.2.10 \
  --verify-peer-certificate \
  --use-system-ca \
  --expected-peer-name server.example.com
```

### 双向 RA-TLS 与双向证书校验

服务端使用 Server CA 签发自己的证书，并信任 Client CA：

```bash
cargo run -p ratls-sample --bin cca-server -- \
  --once --mutual \
  --issuer-private-key server-ca.key \
  --issuer-certificate-chain server-ca.crt \
  --subject-alt-name DNS:server.test \
  --verify-peer-certificate \
  --trusted-ca-chain client-ca.crt \
  --expected-peer-name client.test
```

客户端使用 Client CA 签发自己的证书，并信任 Server CA：

```bash
cargo run -p ratls-sample --bin cca-client -- \
  --ip 127.0.0.1 --mutual \
  --issuer-private-key client-ca.key \
  --issuer-certificate-chain client-ca.crt \
  --subject-alt-name DNS:client.test \
  --verify-peer-certificate \
  --trusted-ca-chain server-ca.crt \
  --expected-peer-name server.test
```

### IMA 与 boot log

```bash
cargo run -p ratls-sample --bin cca-client -- \
  --ip 127.0.0.1 \
  --ima-log \
  --digest ima-baseline.txt

cargo run -p ratls-sample --bin cca-client -- \
  --ip 127.0.0.1 \
  --bootlog \
  --firmware firmware-baseline.json
```

### 准备 firmware baseline

当前 sample 只接受 SHA-256，文件格式如下：

```json
{
  "hash_alg": "sha-256",
  "grub": "<64 个十六进制字符>",
  "grub.cfg": "<64 个十六进制字符>",
  "kernels": [
    {
      "version": "<可选说明>",
      "kernel": "<可选，64 个十六进制字符>",
      "initramfs": "<可选，64 个十六进制字符>"
    }
  ]
}
```

`version` 仅用于说明，不参与匹配。GRUB 必须出现在
`EV_EFI_BOOT_SERVICES_APPLICATION` 的 SHA-256 摘要中，`grub.cfg`、kernel 和
initramfs 则按 sample 的 `EV_IPL` 事件识别规则提取。使用 `--bootlog` 时，客户端先
用 SHA-256 重放 registry 1/2，并分别与已验证的 CCA REM[0]/REM[1] 比较；只有重放
通过后才使用上述基线。event log 中只有 SHA-384 或 SHA-512、缺少所需 SHA-256
摘要，或者重放值不一致时，当前 sample 会拒绝校验或无法匹配基线。

### 发送 FDE key

服务端选择输出文件：

```bash
cargo run -p ratls-sample --bin cca-server -- \
  --once \
  --rootfs-key /tmp/rootfs_key.bin \
  --max-key 65536
```

客户端发送文件：

```bash
cargo run -p ratls-sample --bin cca-client -- \
  --ip 127.0.0.1 \
  --fdekey ./rootfs_key.bin
```

## 常见参数错误

以下命令会在联网前被拒绝：

```bash
# 只填写 CA，但没有开启 TLS 对端证书校验
cca-client --trusted-ca-chain root-ca.pem

# 开启校验但没有任何信任根
cca-client --verify-peer-certificate

# 服务端在非 mTLS 模式校验客户端证书
cca-server --verify-peer-certificate --use-system-ca

# 同时填写文本消息和文件消息
cca-client --message hello --message-file message.bin

# RIM 不是偶数长度的十六进制字符串
cca-client --rim abc

# firmware baseline 缺少 boot log
cca-client --firmware firmware-baseline.json
```
