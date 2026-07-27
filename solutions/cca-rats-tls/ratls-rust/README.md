# ratls-rust

[English](README.en.md)

ratls-rust 是一个用 Rust 实现的 RATS-TLS 项目，当前聚焦 CCA 场景。它把远程证明
evidence 放进 TLS 证书中，让通信双方在建立 TLS 连接时完成身份和运行环境的校验。
项目同时提供 Rust API、可供 C 程序链接的动态库，以及一套可运行的 client/server
示例。

## 代码结构

```text
ratls-rust/
├── ratls-api/                     # 可复用的 RATS-TLS 核心库
│   ├── include/
│   │   └── ratls_api.h            # C 动态库的公共头文件
│   └── src/
│       ├── api/                   # 初始化、协商、收发等 Rust API
│       ├── attesters/cca/         # 从 Linux TSM 接口采集 CCA evidence
│       ├── verifiers/cca/         # CCA token、证书链和 claims 验证
│       ├── core/                  # evidence、证书、DICE/CBOR claims 等基础数据结构
│       ├── crypto_wrappers/       # OpenSSL 加密能力封装
│       ├── tls_wrappers/          # OpenSSL TLS 协商和证书处理
│       └── ffi.rs                 # 对外导出的 C ABI 实现
│
├── ratls-sample/                  # 可直接运行的示例程序
│   └── src/
│       ├── bin/
│       │   ├── cca-client.rs      # 示例客户端
│       │   └── cca-server.rs      # 示例服务端
│       └── common/                # 帧协议、IMA、CCEL、event log、策略校验等示例逻辑
│
├── Cargo.toml                     # Rust workspace 配置
└── README.md                      # 项目说明
```

## 编译与命令

当前依赖：

- Rust 1.75+
- OpenSSL 3.5+
- `gcc` 或 `clang`
- `pkg-config`

编译整个项目：

```sh
cargo build
```

编译 Release 版本：

```sh
cargo build --release
```

Release 产物：

```text
target/release/libratls_api.so
target/release/cca-client
target/release/cca-server
```

当前 sample 的命令如下。

### cca-server

完整的参数作用、输入限制、依赖关系和命令示例见
[CLI 使用说明](docs/cli-usage.md)。

```text
Usage: cca-server [OPTIONS]

Options:
  -i, --ip <IP>                  [default: 127.0.0.1]
  -p, --port <PORT>              [default: 1234]
  -1, --once
  -m, --mutual
      --ima-log <IMA_LOG>
                                  [default: /sys/kernel/security/ima/binary_runtime_measurements]
      --ccel-table <CCEL_TABLE>  [default: /sys/firmware/acpi/tables/CCEL]
      --event-log <EVENT_LOG>
                                  [default: /sys/firmware/acpi/tables/data/CCEL]
      --rootfs-key <ROOTFS_KEY>  [default: /root/rootfs_key.bin]
      --rim <RIM>
  -P, --platform <PLATFORM>
      --max-key <MAX_KEY>        [default: 65536]
      --cert-algo <CERT_ALGO>    [default: ecc256] [possible values: rsa3072, ecc256]
      --issuer-private-key <PEM_FILE>
      --issuer-certificate-chain <PEM_FILE>
      --subject-alt-name <TYPE:VALUE>
      --verify-peer-certificate
      --use-system-ca
      --trusted-ca-chain <PEM_FILE>
      --expected-peer-name <DNS_OR_IP>
  -l, --log-level <LOG_LEVEL>
                                  [default: error]
                                  [debug, info, warn, error, fatal, none]
  -h, --help
```

### cca-client

```text
Usage: cca-client [OPTIONS]

Options:
  -i, --ip <IP>                  [default: 127.0.0.1]
  -p, --port <PORT>              [default: 1234]
  -M, --message <MESSAGE>        [default: "hello CCA"]
      --message-file <MESSAGE_FILE>
  -m, --mutual
  -I, --ima-log
  -g, --bootlog
  -f, --firmware <FIRMWARE>
  -d, --digest <DIGEST>
      --rim <RIM>
  -P, --platform <PLATFORM>
  -k, --fdekey <FDE_KEY>
      --max-log <MAX_LOG>        [default: 10485760]
      --cert-algo <CERT_ALGO>    [default: ecc256] [possible values: rsa3072, ecc256]
      --issuer-private-key <PEM_FILE>
      --issuer-certificate-chain <PEM_FILE>
      --subject-alt-name <TYPE:VALUE>
      --verify-peer-certificate
      --use-system-ca
      --trusted-ca-chain <PEM_FILE>
      --expected-peer-name <DNS_OR_IP>
  -l, --log-level <LOG_LEVEL>
                                  [default: error]
                                  [debug, info, warn, error, fatal, none]
  -h, --help
```

`--firmware` 必须和 `--bootlog` 一起使用，`--digest` 必须和 `--ima-log`
一起使用。服务端的 `--rim` 和 `--platform` 用于校验客户端证明，因此必须启用
`--mutual`。

可使用以下命令查看：

```sh
cargo run -p ratls-sample --bin cca-server -- --help
cargo run -p ratls-sample --bin cca-client -- --help
```

## 示例程序操作

下面的命令默认在项目根目录执行。服务端需要能采集 CCA evidence 的环境，也就是系统
提供 Linux TSM report interface。

启动服务端：

```sh
cargo run -p ratls-sample --bin cca-server
```

只处理一个客户端连接后退出：

```sh
cargo run -p ratls-sample --bin cca-server -- --once
```

指定监听地址和端口：

```sh
cargo run -p ratls-sample --bin cca-server -- \
  --ip 0.0.0.0 \
  --port 1234 \
  --once
```

客户端连接服务端并发送消息：

```sh
cargo run -p ratls-sample --bin cca-client -- \
  --ip <server-ip> \
  --port 1234 \
  --message 'hello CCA'
```

双向证明时，服务端和客户端都需要加上 `--mutual`：

```sh
# server
cargo run -p ratls-sample --bin cca-server -- --once --mutual

# client
cargo run -p ratls-sample --bin cca-client -- --ip <server-ip> --mutual
```

### 使用 CA 动态签发并校验 TLS 证书

下面生成一套仅用于测试的 CA：

```sh
openssl genpkey -algorithm RSA \
  -pkeyopt rsa_keygen_bits:3072 \
  -out /tmp/ratls-test-ca.key

openssl req -x509 -new \
  -key /tmp/ratls-test-ca.key \
  -sha256 -days 3650 \
  -subj "/CN=RA-TLS Test CA" \
  -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign" \
  -out /tmp/ratls-test-ca.crt
```

服务端使用该 CA 动态签发包含 RA evidence 的证书：

```sh
cargo run -p ratls-sample --bin cca-server -- \
  --once \
  --issuer-private-key /tmp/ratls-test-ca.key \
  --issuer-certificate-chain /tmp/ratls-test-ca.crt \
  --subject-alt-name DNS:server.test \
  --log-level debug
```

客户端信任测试 CA，并校验证书名称：

```sh
cargo run -p ratls-sample --bin cca-client -- \
  --ip <server-ip> \
  --verify-peer-certificate \
  --trusted-ca-chain /tmp/ratls-test-ca.crt \
  --expected-peer-name server.test \
  --log-level debug
```

`--subject-alt-name` 可以重复填写，支持 `DNS:`、`IP:` 和 `URI:`。使用系统信任根时
将 `--trusted-ca-chain` 替换为 `--use-system-ca`；也可以同时指定两者。

测试双向 TLS 证书校验时，双方都使用测试 CA 签发本端证书，并信任对端 CA：

```sh
# server
cargo run -p ratls-sample --bin cca-server -- \
  --once --mutual \
  --issuer-private-key /tmp/ratls-test-ca.key \
  --issuer-certificate-chain /tmp/ratls-test-ca.crt \
  --verify-peer-certificate \
  --trusted-ca-chain /tmp/ratls-test-ca.crt

# client
cargo run -p ratls-sample --bin cca-client -- \
  --ip <server-ip> --mutual \
  --issuer-private-key /tmp/ratls-test-ca.key \
  --issuer-certificate-chain /tmp/ratls-test-ca.crt \
  --verify-peer-certificate \
  --trusted-ca-chain /tmp/ratls-test-ca.crt
```

服务端默认读取以下文件：

```text
IMA log:       /sys/kernel/security/ima/binary_runtime_measurements
CCEL table:    /sys/firmware/acpi/tables/CCEL
CCA event log: /sys/firmware/acpi/tables/data/CCEL
rootfs key:    /root/rootfs_key.bin
```

使用自定义路径：

```sh
cargo run -p ratls-sample --bin cca-server -- \
  --once \
  --ima-log <ima-log-path> \
  --ccel-table <ccel-table-path> \
  --event-log <event-log-path> \
  --rootfs-key <output-key-path>
```

其他常用客户端操作：

```sh
# 从文件读取消息
cargo run -p ratls-sample --bin cca-client -- \
  --ip <server-ip> --message-file <message-file>

# 请求 IMA log，并按 digest baseline 校验
cargo run -p ratls-sample --bin cca-client -- \
  --ip <server-ip> --ima-log --digest <baseline.json>

# 请求 CCEL 和 event log，并按 firmware baseline 校验
cargo run -p ratls-sample --bin cca-client -- \
  --ip <server-ip> --bootlog --firmware <firmware-baseline.json>

# 校验 Realm Initial Measurement 和 platform policy
cargo run -p ratls-sample --bin cca-client -- \
  --ip <server-ip> --rim <hex-rim> --platform <platform-policy.json>

# 通过 TLS 通道发送 rootfs key
cargo run -p ratls-sample --bin cca-client -- \
  --ip <server-ip> --fdekey <key-file>
```

使用 `--bootlog` 时，客户端会把 event log 的 registry 1/2 重放后，分别与已经通过
CCA evidence 密码学校验的 `cca_realm_rem0`、`cca_realm_rem1` 比较。只有重放结果
一致，才会提取 firmware state 并继续执行 `--firmware` 基线校验。

## 开发者集成

公共头文件在 `ratls-api/include/ratls_api.h`。构建 C 动态库：

```sh
cargo build --release -p ratls-api
```

生成的文件：

```text
target/release/libratls_api.so
ratls-api/include/ratls_api.h
```

C 程序编译时需要包含头文件并链接动态库：

```sh
cc app.c \
  -Iratls-api/include \
  -Ltarget/release \
  -lratls_api \
  -Wl,-rpath,'$ORIGIN/target/release' \
  -o app
```

也可以不写入 rpath，改为在运行前设置动态库搜索路径：

```sh
export LD_LIBRARY_PATH=target/release:$LD_LIBRARY_PATH
./app
```

调用顺序：

```text
ratls_conf_init
    ↓
ratls_init
    ↓
ratls_set_verification_callback（可选）
    ↓
ratls_negotiate_fd
    ↓
ratls_transmit / ratls_receive
    ↓
ratls_cleanup
```

推荐通过 `ratls_conf_init()` 初始化配置，不要依赖 `{0}`：证书算法的数值 `0`
表示 RSA-3072，而不是默认算法。初始化后再根据角色覆盖必要字段：

```c
ratls_conf_t conf;
ratls_conf_init(&conf);

/* 单向证明客户端：验证服务端，本地不生成 evidence。 */
conf.server = 0;
conf.attester_type = RATLS_ATTESTER_NONE;
conf.verifier_type = RATLS_VERIFIER_CCA;

/* 单向证明服务端则使用：
 * conf.server = 1;
 * conf.attester_type = RATLS_ATTESTER_CCA;
 * conf.verifier_type = RATLS_VERIFIER_NONE;
 */
```

### 动态证书签发与标准 TLS CA 校验

默认情况下，库保持原有行为：自动生成叶子私钥，并动态自签包含 RA evidence 的
X.509 证书。要改为由用户 CA 动态签发，需要把未加密 CA 私钥和对应证书链的 PEM
数据放进配置：

```c
/* issuer_key_pem/issuer_chain_pem 由应用读取或从 Secret Manager 获取。 */
conf.certificate.issuer_private_key = (ratls_buffer_t) {
    .data = issuer_key_pem,
    .len = issuer_key_pem_len,
};
conf.certificate.issuer_certificate_chain = (ratls_buffer_t) {
    .data = issuer_chain_pem,
    .len = issuer_chain_pem_len,
};
```

两项都为空表示动态自签；两项都非空表示动态 CA 签发；只配置其中一项会导致
`ratls_init()` 失败。私钥必须是未加密 PEM，证书顺序不限，Rust 会使用 OpenSSL
寻找与私钥匹配的 CA 证书并构建签发链。

动态证书的 SAN 是可选配置：

```c
const char *sans[] = {
    "DNS:server.example.com",
    "IP:192.0.2.10",
    "URI:spiffe://example/service",
};
conf.certificate.subject_alt_names = sans;
conf.certificate.subject_alt_names_len = sizeof(sans) / sizeof(sans[0]);
```

启用标准 TLS 对端证书校验：

```c
conf.tls_verify.verify_peer_certificate = 1;

/* 加载系统 CA。 */
conf.tls_verify.use_system_ca = 1;

/* 也可同时加入私有 CA；两类信任根会合并使用。 */
conf.tls_verify.trusted_ca_chain = (ratls_buffer_t) {
    .data = trusted_ca_pem,
    .len = trusted_ca_pem_len,
};

/* 可选；NULL/空字符串表示只校验证书链。 */
conf.tls_verify.expected_peer_name = "server.example.com";
```

如果不使用系统 CA，则必须提供 `trusted_ca_chain`：

```c
conf.tls_verify.verify_peer_certificate = 1;
conf.tls_verify.use_system_ca = 0;
conf.tls_verify.trusted_ca_chain.data = trusted_ca_pem;
conf.tls_verify.trusted_ca_chain.len = trusted_ca_pem_len;
```

服务端开启对端 TLS 证书校验时必须同时开启 mTLS：

```c
conf.server = 1;
conf.mutual = 1;
conf.tls_verify.verify_peer_certificate = 1;
```

此时服务端使用 `trusted_ca_chain`/系统 CA 验证客户端证书，不再直接跳过 OpenSSL
证书校验。客户端与服务端的签发配置和信任配置相互独立，因此 mTLS 可以使用不同
的客户端 CA 和服务端 CA。

所有 PEM、SAN 和名称数据都会在 `ratls_init()` 中复制；初始化返回后，调用方可以
释放原始内存。关闭 `verify_peer_certificate` 时，不校验 CA 信任根和对端名称，
其余 `tls_verify` 字段全部忽略；证书签名、有效期、约束、KeyUsage、角色 EKU 和
RA evidence 绑定校验仍然执行。
当前功能不启用 CRL 或 OCSP 吊销检查；`use_system_ca` 只加载 OpenSSL 的系统信任
根，不能据此假定系统会自动完成证书吊销检查。

`rats_tls_init()` 和 C 接口 `ratls_init()` 会统一校验输入。主要上限为：签发私钥
64 KiB、签发证书链 1 MiB、自定义可信 CA 4 MiB、SAN 64 项、custom claim 64 项、
单个 claim 值 64 KiB、全部 claim 值合计 256 KiB。C 头文件提供对应的
`RATLS_MAX_*` 宏。无效算法、重复或保留 claim 名、非法 SAN、非法对端 DNS 名称及
超限输入都会在初始化阶段被拒绝。

`ratls_negotiate_fd` 接收一个已经建立连接的 TCP socket fd。库会复制该 fd，调用方
仍然负责关闭原始 fd。`ratls_transmit` 和 `ratls_receive` 提供的是 TLS 字节流，不带
消息边界；应用需要自行定义长度字段、换行符或其他 framing 协议。

### C ABI 冒烟测试

`ratls-api/tests/c_abi_smoke.c` 用来确认公共 C 头文件、动态库导出符号、默认配置、
句柄初始化/释放和主要参数边界能够从 C 正常使用。它不建立网络连接，也不执行真实
CCA evidence 或 TLS 握手。

编译并运行：

```sh
cargo build --release -p ratls-api

cc -std=c11 -Wall -Wextra -Werror \
  ratls-api/tests/c_abi_smoke.c \
  -Iratls-api/include \
  -Ltarget/release \
  -lratls_api \
  -o /tmp/c_abi_smoke

LD_LIBRARY_PATH=target/release /tmp/c_abi_smoke
echo $?
```

返回 `0` 表示全部检查通过；返回 `1`～`17` 表示对应检查点失败。Cargo 不会自动编译
或执行 `.c` 文件，因此 `cargo test` 不包含这个测试。

### C 客户端和服务端示例

完整的 C 客户端和服务端示例位于：

```text
ratls-api/examples/c/ratls_client.c
ratls-api/examples/c/ratls_server.c
```

这两个示例可以直接编译并配套运行。先构建 Rust 动态库，再编译 C 程序：

```sh
cargo build --release -p ratls-api

cc -std=c11 -Wall -Wextra ratls-api/examples/c/ratls_server.c \
  -Iratls-api/include -Ltarget/release -lratls_api \
  -Wl,-rpath,'$ORIGIN/target/release' -o ratls-c-server

cc -std=c11 -Wall -Wextra ratls-api/examples/c/ratls_client.c \
  -Iratls-api/include -Ltarget/release -lratls_api \
  -Wl,-rpath,'$ORIGIN/target/release' -o ratls-c-client
```

服务端必须在能够通过 Linux TSM report interface 采集 CCA evidence 的环境运行：

```sh
./ratls-c-server 1234
```

另一个终端启动客户端：

```sh
./ratls-c-client 127.0.0.1 1234
```

命令格式为：

```text
ratls-c-server [PORT]
ratls-c-client [SERVER_IPV4] [PORT]
```

当前示例具有以下边界：

- 服务端必须能够访问 Linux TSM report interface，例如
  `/sys/kernel/config/tsm/report/report0`，否则无法生成真实 CCA evidence。
- 当前 C 示例只支持 IPv4。
- 服务端只接受一个连接，处理完成后退出。
- 默认使用动态自签名 RA-TLS 证书，并关闭标准 CA 信任链校验；证书基础校验和 CCA
  evidence 校验仍然执行。
- 客户端 verification callback 会打印已验证的 CCA claims，然后直接返回接受。
  生产代码仍需按业务要求检查 RIM、TCB、软件组件或 custom claims。
- CA 私钥、证书链、自定义信任根和名称校验目前需要在 C 源码中设置，示例没有为它们
  提供命令行参数。
- 这两个 C 示例直接使用 TLS 字节流，而 Rust sample 使用四字节长度头的 frame
  协议，因此 C 客户端和服务端应配套测试，不应直接与 Rust sample 混用。

如果运行时找不到 `libratls_api.so`，可以显式设置动态库搜索路径：

```sh
LD_LIBRARY_PATH=target/release ./ratls-c-server 1234
LD_LIBRARY_PATH=target/release ./ratls-c-client 127.0.0.1 1234
```

出错后可以调用 `ratls_last_error()` 读取当前线程的错误信息。返回的字符串由库管理，
下一次 FFI 调用后可能失效；如果需要长期保存，应用应自行复制。

`ratls_transmit()` 可能发生短写，完整发送一段数据时必须根据 `written_out` 循环。
`ratls_receive()` 返回成功且 `read_out == 0` 表示对端已经关闭 TLS 流。

业务验证回调在内置 CCA 证书链、签名和握手绑定校验成功后执行：

```c
static int verify(const ratls_verified_evidence_t *evidence, void *user_data)
{
    (void)user_data;
    printf("CCA claims: %.*s\n",
           (int)evidence->claims_json.len,
           (const char *)evidence->claims_json.data);

    /* 在这里校验 RIM、TCB、软件组件或 custom claims。 */
    return 1; /* 非 0 接受，0 拒绝连接。 */
}

ratls_set_verification_callback(handle, verify, NULL);
```

回调参数及其中的所有 buffer 都只在本次回调执行期间有效，不能保存其指针；
需要延后使用时必须复制数据。

## 二次开发与新场景集成

先判断新需求属于哪一层：

```text
新的 TEE / evidence 格式
    ├── attesters/       # 采集本地 evidence
    ├── verifiers/       # 验证对端 evidence
    └── core/            # evidence、claims、证书扩展的数据结构

新的 TLS 或加密实现
    ├── tls_wrappers/    # TLS 协商和证书加载
    └── crypto_wrappers/ # 密钥、证书、签名和哈希操作

新的业务校验逻辑
    ├── ratls_set_verification_callback
    └── ratls-sample/    # 示例协议、日志和业务策略
```

### 新增 attester 和 verifier

当前 CCA 实现在：

```text
ratls-api/src/attesters/cca/
ratls-api/src/verifiers/cca/
```

新增一种 TEE 或 evidence 格式时，按相同结构添加：

```text
ratls-api/src/attesters/<type>/
ratls-api/src/verifiers/<type>/
```

attester 负责从平台接口采集 evidence 并放进 RA-TLS 证书扩展；verifier 负责取出
对端 evidence，校验证书链、签名、nonce、ClientHello `key_share` 哈希和 claims。
完成实现后，还要在 attester 和 verifier registry 中注册名称，应用才能通过配置选择它：

```c
ratls_conf_t conf = {
    .attester_type = RATLS_ATTESTER_CCA,
    .verifier_type = RATLS_VERIFIER_CCA,
    .tls_type = "openssl",
    .crypto_type = "openssl",
};
```

### 自定义 claims 和业务策略

只需绑定业务信息时，不需要新增 attester。可以通过
`ratls_conf_t.custom_claims` 传入 workload 名称、镜像 digest、配置 hash、服务版本等。
不要把私钥、token 或 rootfs key 的原文放进 custom claims。

verifier 只验证 evidence 是否有效；业务层再决定这个有效的 evidence 是否符合服务
要求。业务策略放在 verification callback 中：

```c
int verify_callback(const ratls_verified_evidence_t *evidence, void *user_data) {
    /* 检查 claims_json 或 custom_claims。 */
    /* 返回非零：接受对端；返回零：拒绝对端。 */
    return 1;
}
```

IMA、RIM、platform policy 和 firmware baseline 的示例校验可以参考 `ratls-sample/`。

### 新增 TLS 或 crypto wrapper

需要替换 OpenSSL 时，分别在以下位置实现并注册新名称：

```text
ratls-api/src/tls_wrappers/<type>/
ratls-api/src/crypto_wrappers/<type>/
```

配置时选择新实现：

```c
.tls_type = "<type>",
.crypto_type = "<type>",
```

TLS wrapper 负责协商和证书加载；crypto wrapper 负责密钥、证书、签名和哈希操作。

开发完成后至少运行：

```sh
cargo fmt
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```

如果改了 C ABI，还需要同步更新：

```text
ratls-api/src/ffi.rs
ratls-api/include/ratls_api.h
README.md
README.en.md
```
