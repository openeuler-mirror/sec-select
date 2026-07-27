# API 接口参数要求

本文档描述 `ratls-api` 的公共参数契约，适用于 Rust API 和 C ABI。两套接口使用相同的业务语义和输入上限；C ABI 因使用裸指针，还需满足额外的内存安全要求。

当前项目处于开发阶段，ABI 版本为 `0.1.0`。修改本文中的限制时，必须同步修改 Rust 校验、C 头文件宏、测试和本文档。

## 输入长度和数量限制

所有长度均按**字节**计算，最大值本身允许使用；只有超过最大值才会失败。

| 参数 | 最大值 | Rust 常量 | C 宏 |
| --- | ---: | --- | --- |
| CA 签发私钥 PEM | 65,536 B（64 KiB） | `MAX_ISSUER_PRIVATE_KEY_PEM_SIZE` | `RATLS_MAX_ISSUER_PRIVATE_KEY_PEM_SIZE` |
| CA 证书链 PEM bundle | 1,048,576 B（1 MiB） | `MAX_ISSUER_CERTIFICATE_CHAIN_PEM_SIZE` | `RATLS_MAX_ISSUER_CERTIFICATE_CHAIN_PEM_SIZE` |
| 对端自定义可信 CA PEM bundle | 4,194,304 B（4 MiB） | `MAX_TRUSTED_CA_CHAIN_PEM_SIZE` | `RATLS_MAX_TRUSTED_CA_CHAIN_PEM_SIZE` |
| SAN 数量 | 64 项 | `MAX_SUBJECT_ALT_NAMES` | `RATLS_MAX_SUBJECT_ALT_NAMES` |
| 单项 SAN（包含 `DNS:`、`IP:` 或 `URI:` 前缀） | 2,048 B | `MAX_SUBJECT_ALT_NAME_LENGTH` | `RATLS_MAX_SUBJECT_ALT_NAME_LENGTH` |
| 预期对端 DNS 名称或文本 IP | 253 B | `MAX_EXPECTED_PEER_NAME_LENGTH` | `RATLS_MAX_EXPECTED_PEER_NAME_LENGTH` |
| 自定义 claim 数量 | 64 项 | `MAX_CUSTOM_CLAIMS` | `RATLS_MAX_CUSTOM_CLAIMS` |
| 单个自定义 claim 名称 | 128 B | `MAX_CUSTOM_CLAIM_NAME_LENGTH` | `RATLS_MAX_CUSTOM_CLAIM_NAME_LENGTH` |
| 单个自定义 claim 值 | 65,536 B（64 KiB） | `MAX_CUSTOM_CLAIM_VALUE_LENGTH` | `RATLS_MAX_CUSTOM_CLAIM_VALUE_LENGTH` |
| 全部自定义 claim 值之和 | 262,144 B（256 KiB） | `MAX_CUSTOM_CLAIMS_TOTAL_VALUE_LENGTH` | `RATLS_MAX_CUSTOM_CLAIMS_TOTAL_VALUE_LENGTH` |
| C ABI 的 `tls_type`、`crypto_type` | 64 B 以内必须找到结尾 `\0` | 仅 C ABI 内部限制 | 无公共宏 |

字符串在 Rust 中按 UTF-8 字节长度计算，不是按字符数量计算。C ABI 中的字符串也必须是有效 UTF-8；DNS、IP 和 URI 还受下文的格式限制。

发送和接收接口当前没有额外的应用数据长度上限。TLS 是字节流，一次发送可能只写入部分数据，调用方必须根据实际长度循环处理，并自行设计消息分帧。

## 公共配置要求

### 角色组合

| 模式 | `server` | `mutual` | `attester` | `verifier` |
| --- | ---: | ---: | --- | --- |
| 单向证明客户端 | `false` | `false` | 可不配置 | 必须配置 |
| 单向证明服务端 | `true` | `false` | 必须配置 | 可不配置 |
| 双向证明客户端 | `false` | `true` | 必须配置 | 必须配置 |
| 双向证明服务端 | `true` | `true` | 必须配置 | 必须配置 |

服务端只有在 `mutual=true` 时才能开启对客户端的标准 TLS 证书校验。

### 动态叶子证书

- `cert_algo` 只接受 RSA-3072/SHA-256 和 ECDSA P-256/SHA-256。该字段决定动态生成的**叶子私钥**算法。
- `issuer_private_key` 与 `issuer_certificate_chain` 都为空：生成临时叶子私钥，并动态生成自签名叶子证书。
- 两者都非空：生成临时叶子私钥，使用给定 CA 私钥和 CA 证书链动态签发叶子证书。
- 只配置私钥或只配置证书链：初始化失败。
- CA 私钥必须是未加密 PEM，当前不支持密码字段。
- CA 私钥必须与证书链中的 CA 证书匹配。CA 证书必须处于有效期内，包含 `CA:TRUE`，并允许 `keyCertSign`。
- RSA CA 私钥至少为 2048 位；EC CA 私钥支持 P-256、P-384、P-521，也支持 Ed25519 和 Ed448。X25519 和 X448 不能用于签名。
- 证书链的 PEM 排列顺序不限，可以包含或省略 Root CA；库会使用 OpenSSL 查找并校验签发关系。

### SAN

每个 SAN 必须使用以下一种格式：

- `DNS:server.example.com`
- `IP:192.0.2.10` 或合法 IPv6 地址
- `URI:spiffe://example.org/service`

DNS 名称必须是 ASCII，总长度不超过 253 字节；每个 label 为 1～63 字节，只允许字母、数字和连字符，且不能以连字符开头或结尾。SAN 允许只在最左侧使用完整 label 通配符，例如 `DNS:*.example.com`。

IP 必须能解析为合法 IPv4 或 IPv6。URI 必须是 ASCII，不得包含空格或控制字符，且必须包含合法 scheme 和非空的 scheme-specific 内容。

### 自定义 claims

- 名称不能为空、不能包含控制字符、不能重复。
- `pubkey-hash`、`client-key-share-hash` 和 `nonce` 是保留名称，应用不能使用。
- claim 值是任意二进制数据，不要求 UTF-8。
- 数量、单项长度和总长度必须同时满足上表限制。

### 标准 TLS 对端证书校验

当 `verify_peer_certificate=false` 时：

- 不进行 CA 信任链和对端名称校验。
- `use_system_ca`、`trusted_ca_chain` 和 `expected_peer_name` 被忽略。
- 仍然校验证书自签名、有效期、`BasicConstraints CA:FALSE`、`KeyUsage digitalSignature` 和与对端角色匹配的 EKU。
- RA evidence 及其握手绑定校验仍然必须通过。

当 `verify_peer_certificate=true` 时：

- `use_system_ca=true` 与非空 `trusted_ca_chain` 至少满足一项，否则初始化失败。
- 两项同时配置时，系统 CA 与自定义 CA 合并为同一个信任集合，任意一条合法信任链均可通过。
- `trusted_ca_chain` 必须是可由 OpenSSL 解析的 PEM CA bundle。
- `expected_peer_name` 为空时不校验名称；非空时必须是合法 DNS 名称或 IP。
- `expected_peer_name` 不允许通配符；客户端配置 DNS 名称时也会将其用于 SNI。
- 标准 TLS 校验和 RA 校验必须同时通过。
- 当前不主动执行 CRL 或 OCSP 吊销检查；加载系统 CA 仅表示加载系统信任根。

## Rust API 参数要求

Rust 配置类型为 `RaTlsConf`，初始化入口为 `rats_tls_init(conf)`：

- 输入的 `String` 和 `Vec` 所有权随 `conf` 传入，初始化期间完成校验。
- 参数不满足要求时返回 `RaTlsError::InvalidArgument`；PEM、证书或密码学内容无效时可能返回更具体的 OpenSSL 或数据错误。
- `rats_tls_set_verification_callback` 的回调只能接收已经通过内置密码学校验的 evidence；回调返回错误会拒绝连接。
- `rats_tls_negotiate` 需要可读写的已连接传输流。
- `rats_tls_transmit` 和 `rats_tls_receive` 只能在握手成功后使用，返回值是本次实际处理的字节数。

Rust 公共限制常量定义于 `ratls_api::api`。

## C ABI 附加要求

C 头文件为 `ratls-api/include/ratls_api.h`。除公共业务规则外，还必须满足以下要求：

### 指针与长度

- 调用 `ratls_conf_init(&conf)` 后再修改字段；不得手工猜测 `struct_size`。
- `ratls_init(NULL, &handle)` 可以使用默认配置；`handle_out` 不能为 `NULL`。
- 对任意 `ratls_buffer_t`，`len > 0` 时 `data` 必须非空，并指向至少 `len` 字节的可读连续内存。
- 数组长度大于 0 时数组指针必须非空，且数组中的每一项都必须有效。
- C 字符串必须以 `\0` 结尾并且是有效 UTF-8。`tls_type` 和 `crypto_type` 必须在前 64 字节内出现 `\0`。
- `ratls_transmit` 在 `data_len > 0` 时要求 `data != NULL`。
- `ratls_receive` 要求 `buffer != NULL` 且 `buffer_len > 0`。
- 非空裸指针是否真实可读或可写无法由库完整判断，调用方必须保证其有效性。

`ratls_init` 会复制配置字符串、PEM、SAN 和 claims；函数返回后，调用方可以释放这些输入内存。

### 句柄、socket 与回调生命周期

- `ratls_init` 成功返回的句柄最终必须由 `ratls_cleanup` 释放，同一句柄只能释放一次；`ratls_cleanup(NULL)` 合法。
- `ratls_negotiate_fd` 要求句柄有效、`fd >= 0`，并且 socket 已经完成 `connect()` 或 `accept()`。
- 库会 `dup(fd)`；原始 fd 仍由调用方关闭。
- evidence 结构体及其内部指针只在 verification callback 执行期间有效。需要在回调外使用时必须复制。
- `ratls_api_version()` 和 `ratls_last_error()` 返回库持有的借用指针，调用方不得释放。
- `ratls_last_error()` 是线程局部状态，其指针会在同一线程下一次错误状态更新后失效。

### 返回值

成功返回 `RATLS_SUCCESS`（0），失败返回负数：

| 返回值 | 含义 |
| --- | --- |
| `RATLS_INVALID_ARGUMENT` | 函数的直接参数无效 |
| `RATLS_INIT_ERROR` | 配置、PEM、证书或初始化失败 |
| `RATLS_NEGOTIATE_ERROR` | TLS/RA 握手失败 |
| `RATLS_TRANSMIT_ERROR` | 发送失败 |
| `RATLS_RECEIVE_ERROR` | 接收失败 |
| `RATLS_SYSTEM_ERROR` | 系统调用失败 |
| `RATLS_PANIC` | Rust panic 被 ABI 边界捕获 |

失败后应立即调用 `ratls_last_error()` 获取当前线程的具体错误信息。

## C 默认配置注意事项

`ratls_conf_init` 的默认值是：CCA attester/verifier、OpenSSL、ECDSA P-256、客户端、单向证明、动态自签名证书，并关闭标准 TLS CA 校验。

`RATLS_CERT_ALGO_DEFAULT` 的值为 3，表示由库选择默认算法；枚举值 0 实际表示 RSA-3072，不能用清零结构体代替 `ratls_conf_init`。
