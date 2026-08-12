# OpenClaw-CCA 配置参考手册

本文档汇总 OpenClaw-CCA 涉及的全部配置文件、环境变量和命令行参数，作为快速查阅的参考。

---

## 目录

- [1. 配置文件总览](#1-配置文件总览)
- [2. RBS 服务器侧配置](#2-rbs-服务器侧配置)
  - [2.1 `/etc/attestation_server/.env`](#21-etcauth_serverenv)
  - [2.2 `/etc/rbs/rbs.yaml`](#22-etcrbsrbsyaml)
  - [2.3 `/etc/openbao/openbao.hcl`](#23-etcauth_openbaoopenbaohcl)
  - [2.4 密钥与证书文件](#24-密钥与证书文件)
- [3. 虚机（Realm）侧配置](#3-虚机realm侧配置)
  - [3.1 `scripts/openclaw-rbc-unlock.sh` 中的 RBS_BASE_URL](#31-scriptsopenclaw-rbc-unlocksh-中的-rbs_base_url)
  - [3.2 `/etc/openclaw-cca/config`](#32-etcauth_openclaw-ccaconfig)
  - [3.3 `/etc/attestation_agent/agent_config.yaml`](#33-etcauth_agentagent_configyaml)
- [4. systemd 服务配置](#4-systemd-服务配置)
  - [4.1 `openclaw-luks-unlock.service`](#41-openclaw-luks-unlockservice)
  - [4.2 `openclaw.service`](#42-openclawservice)
- [5. 命令行工具参数](#5-命令行工具参数)
  - [5.1 `extend-rem3`](#51-extend-rem3)
  - [5.2 `openclaw-init.sh`](#52-openclaw-initsh)
  - [5.3 `openclaw-create-volume.sh`](#53-openclaw-create-volumesh)
  - [5.4 `openclaw-rbc-unlock.sh`](#54-openclaw-rbc-unlocksh)
  - [5.5 `gen_policy.py`](#55-gen_policypy)
  - [5.6 `rbs-cli`](#56-rbs-cli)
- [6. Rego 策略模板](#6-rego-策略模板)
  - [6.1 CCA 策略 (`cca.rego`)](#61-cca-策略-ccarego)
  - [6.2 vCCA 策略 (`vcca.rego`)](#62-vcca-策略-vccarego)
- [7. CCA 与 vCCA 适配速查](#7-cca-与-vcca-适配速查)
- [8. Makefile 变量](#8-makefile-变量)

---

## 1. 配置文件总览

| 配置文件 | 所在位置 | 用途 | 所属环境 |
|----------|----------|------|----------|
| `/etc/attestation_server/.env` | RBS 服务器 | GTA-Server 环境变量 | RBS Server |
| `/etc/rbs/rbs.yaml` | RBS 服务器 | RBS 主配置 | RBS Server |
| `/etc/openbao/openbao.hcl` | RBS 服务器 | openBao 配置 | RBS Server |
| `/etc/attestation_agent/agent_config.yaml` | 虚机 | Attestation Agent 配置 | Realm |
| `/etc/openclaw-cca/config` | 虚机 | OpenClaw-CCA 运行时配置 | Realm |
| `scripts/openclaw-rbc-unlock.sh` 中的 `RBS_BASE_URL` | 虚机 | RBS 服务地址 | Realm |
| `/etc/systemd/system/openclaw-luks-unlock.service` | 虚机 | 开机解锁服务 | Realm |
| `/etc/systemd/system/openclaw.service` | 虚机 | OpenClaw 网关服务 | Realm |

---

## 2. RBS 服务器侧配置

### 2.1 `/etc/attestation_server/.env`

GTA-Server 的环境变量配置文件。

```bash
DB_USER=ra_user
DB_PASSWORD=ra_user_password
HTTPS_SWITCH=0
MYSQL_DATABASE_URL=mysql://ra_user:ra_user_password@127.0.0.1:3306/RA
```

| 变量 | 类型 | 说明 | 适配方法 |
|------|------|------|----------|
| `DB_USER` | string | MySQL 数据库用户名 | 与 MySQL 中创建的用户一致 |
| `DB_PASSWORD` | string | MySQL 数据库密码 | 与 MySQL 中创建的密码一致 |
| `HTTPS_SWITCH` | `0`/`1` | HTTPS 开关。`0` 关闭（HTTP），`1` 开启（HTTPS） | 生产环境建议 `1`；若开启需配置 TLS 证书 |
| `MYSQL_DATABASE_URL` | string | MySQL 连接串，格式 `mysql://<user>:<password>@<host>:<port>/<db>` | 与 `DB_USER`/`DB_PASSWORD` 保持一致 |

### 2.2 `/etc/rbs/rbs.yaml`

RBS 主配置文件，控制监听地址、证明后端、资源存储后端。

```yaml
rest:
  listen_addr: "127.0.0.1:6666"
  https:
    enabled: false
auth:
  attest_token:
    public_key_path: "/etc/rbs/attest_pub.pem"
    # jwks_file: "/etc/rbs/attest.jwk"
attestation:
  backends:
    gta:
      rest:
        base_url: "http://127.0.0.1:8080"
        credentials:
          # main_api_key: "${MAIN_API_KEY}"
          # sub_api_key: "${SUB_API_KEY}"
resource:
  default_provider: vault
  backends:
    vault:
      type: vault
      url: "http://127.0.0.1:8200"
      token: "s.u6O7REadDh4C5RwxCkfIdfOh"
      mount_path: "secret"
```

| 配置路径 | 类型 | 说明 | 适配方法 |
|----------|------|------|----------|
| `rest.listen_addr` | string | RBS REST API 监听地址 | 本机测试：`127.0.0.1:6666`；远程访问：`0.0.0.0:6666` |
| `rest.https.enabled` | boolean | RBS REST API 是否启用 HTTPS | 本机 HTTP 测试：`false`；生产环境建议启用 HTTPS 并配置证书 |
| `auth.attest_token.public_key_path` | string | GTA TSK 公钥路径，用于验证 token 签名 | 必须与 GTA-Server 的 `tsk_public_key.pem` 一致 |
| `auth.attest_token.jwks_file` | string | JWKS 文件路径（二选一） | 与 `public_key_path` 二选一，本文不使用 |
| `attestation.backends.gta.rest.base_url` | string | GTA-Server REST API 地址 | 指向 GTA-Server，如 `http://127.0.0.1:8080`；若 GTA 启用 HTTPS 则改为 `https://` |
| `attestation.backends.gta.rest.credentials.main_api_key` | string | 主 API Key（预留） | 当前版本注释掉 |
| `attestation.backends.gta.rest.credentials.sub_api_key` | string | 副 API Key（预留） | 当前版本注释掉 |
| `resource.default_provider` | string | 默认资源存储后端名称 | 与 `resource.backends` 中的 key 一致 |
| `resource.backends.vault.type` | string | 后端类型 | `vault`（openBao） |
| `resource.backends.vault.url` | string | openBao 服务地址 | 如 `http://127.0.0.1:8200`；若启用 TLS 则改为 `https://` |
| `resource.backends.vault.token` | string | openBao 访问令牌 | 使用 Root Token 或有 secret 读写权限的子 Token |
| `resource.backends.vault.mount_path` | string | KV 存储挂载路径 | 与 openBao 中 `bao secrets enable --path=<mount_path>` 一致 |
| `resource.backends.local.type` | string | 本地文件后端（可选） | `local` |
| `resource.backends.ca.type` | string | CA 后端（可选） | `ca` |

### 2.3 `/etc/openbao/openbao.hcl`

openBao 配置文件。

```hcl
ui = true

storage "file" {
  path = "/opt/openbao/data"
}

listener "tcp" {
  address     = "127.0.0.1:8200"
  tls_disable = 1
}
```

| 配置路径 | 类型 | 说明 | 适配方法 |
|----------|------|------|----------|
| `ui` | bool | 是否启用 Web UI | `true` 或 `false` |
| `storage "file".path` | string | 数据存储目录 | 通常 `/opt/openbao/data` |
| `listener "tcp".address` | string | 监听地址 | 本机：`127.0.0.1:8200`；远程：`0.0.0.0:8200` |
| `listener "tcp".tls_disable` | `0`/`1` | TLS 开关 | 本机测试：`1`；远程访问：`0`（需配置证书） |
| `listener "tcp".tls_cert_file` | string | TLS 证书路径 | TLS 启用时必填 |
| `listener "tcp".tls_key_file` | string | TLS 私钥路径 | TLS 启用时必填 |

### 2.4 密钥与证书文件

| 文件路径 | 用途 | 生成方式 |
|----------|------|----------|
| `/etc/attestation_server/keys/fsk_private_key.pem` | FSK 私钥（File Signing Key） | `openssl genpkey -algorithm RSA-PSS -pkeyopt rsa_keygen_bits:3072` |
| `/etc/attestation_server/keys/fsk_public_key.pem` | FSK 公钥 | `openssl rsa -in fsk_private_key.pem -pubout` |
| `/etc/attestation_server/keys/nsk_private_key.pem` | NSK 私钥（Nonce Signing Key） | `openssl genpkey -algorithm RSA-PSS -pkeyopt rsa_keygen_bits:3072` |
| `/etc/attestation_server/keys/nsk_public_key.pem` | NSK 公钥 | `openssl rsa -in nsk_private_key.pem -pubout` |
| `/etc/attestation_server/keys/tsk_private_key.pem` | TSK 私钥（Token Signing Key） | `openssl genrsa -out tsk_private_key.pem 4096` |
| `/etc/attestation_server/keys/tsk_public_key.pem` | TSK 公钥 | `openssl rsa -in tsk_private_key.pem -pubout` |
| `/etc/attestation_server/certs/ca.crt` | GTA CA 证书 | `openssl req -x509 -newkey rsa:3072 ...` |
| `/etc/attestation_server/certs/ca.key` | GTA CA 私钥 | 同上 |
| `/etc/attestation_server/certs/server.key` | GTA 服务器私钥 | `openssl genrsa -out server.key 3072` |
| `/etc/attestation_server/certs/server.crt` | GTA 服务器证书 | 由 CA 签发 |
| `/etc/rbs/admin.pem` | RBS 管理员私钥 | `openssl genrsa -out admin.pem 4096` |
| `/etc/rbs/admin_pub.pem` | RBS 管理员公钥 | `openssl rsa -in admin.pem -pubout` |
| `/etc/rbs/attest_pub.pem` | GTA TSK 公钥（拷贝自 GTA-Server） | `cp /etc/attestation_server/keys/tsk_public_key.pem /etc/rbs/attest_pub.pem` |

> **关键依赖**：`/etc/rbs/attest_pub.pem` 必须与 `/etc/attestation_server/keys/tsk_public_key.pem` 内容一致，否则 RBS 无法验证 attestation token 签名。

---

## 3. 虚机（Realm）侧配置

### 3.1 `scripts/openclaw-rbc-unlock.sh` 中的 RBS_BASE_URL

```bash
RBS_BASE_URL="YOUR_RBS_URL_HERE"  # 替换为实际 RBS URL
```

| 变量 | 类型 | 说明 | 适配方法 |
|------|------|------|----------|
| `RBS_BASE_URL` | string | RBS REST API 地址 | `make install` 前通过 `sed` 替换为实际地址，如 `http://192.168.1.1:6666` |

> **其他相关变量**（脚本内部使用，通常无需修改）：

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `MODULE_NAME` | `attest` | 内核模块名称 |
| `ATTEST_KO_PATH` | `/root/attest.ko` | 内核模块文件路径 |
| `REM3_FLAG` | `/run/openclaw-rem3-extended` | REM3 已扩展标记文件（tmpfs） |

### 3.2 `/etc/openclaw-cca/config`

OpenClaw-CCA 运行时配置文件，由 `openclaw-init.sh` 和 `openclaw-create-volume.sh` 自动生成，被其他脚本通过 `source` 读取。

```bash
OPENCLAW_BIN=/opt/nodejs/bin/openclaw
VFS_FILE=/root/vfs
```

| 字段 | 类型 | 说明 | 由谁写入 |
|------|------|------|----------|
| `OPENCLAW_BIN` | string | openclaw 二进制路径 | `openclaw-init.sh` |
| `VFS_FILE` | string | 虚拟磁盘文件路径 | `openclaw-create-volume.sh`（仅 VFS 模式） |

> 该文件被 `openclaw.service` 通过 `EnvironmentFile` 指令加载。

### 3.3 `/etc/attestation_agent/agent_config.yaml`

Attestation Agent 配置文件，控制证据采集行为。

```yaml
# GTA-Server 地址（指向 GTA，不是 RBS）
server:
  base_url: "http://192.168.1.1:8080"
  # 若 GTA 启用 HTTPS，使用 https:// 并配置 CA 证书

# 插件开关
plugins:
  - name: "cca"
    enabled: true   # CCA 模式设为 true，vCCA 模式设为 false
  - name: "virt_cca"
    enabled: false  # vCCA 模式设为 true，CCA 模式设为 false
  - name: "tpm"
    enabled: false
  # 其他插件条目的 enabled 保持 false

# 启动日志路径（原 ccel_data_path）
boot_log_file_path: "/sys/kernel/config/tsm/report/report0"
```

| 配置项 | 说明 | 适配方法 |
|--------|------|----------|
| `server.base_url` | GTA-Server 地址 | 改为 GTA-Server 的实际地址（HTTP 或 HTTPS） |
| `plugins` 中 `name: "cca"` 的条目 | CCA 插件配置 | CCA 模式：`enabled: true`；vCCA 模式：`enabled: false` |
| `plugins` 中 `name: "virt_cca"` 的条目 | vCCA 插件配置 | vCCA 模式：`enabled: true`；CCA 模式：`enabled: false` |
| `boot_log_file_path` | 启动日志路径（原 `ccel_data_path`） | 通常保持默认 |

> **兼容性修复**：如果配置文件中存在 `ccel_data_path`，需替换为 `boot_log_file_path`：
> ```bash
> sed -i 's|ccel_data_path|boot_log_file_path|' /etc/attestation_agent/agent_config.yaml
> ```

---

## 4. systemd 服务配置

### 4.1 `openclaw-luks-unlock.service`

开机自动解锁加密卷的 systemd 服务。

```ini
[Unit]
Description=OpenClaw LUKS Unlock via RBC Attestation
DefaultDependencies=no
After=local-fs.target network-online.target
Wants=network-online.target

[Service]
Type=oneshot
RemainAfterExit=yes

# 部署时填写
Environment="KEY_URI=REPLACE_ME"
Environment="DEVICE=REPLACE_ME"
Environment="MOUNT_POINT=REPLACE_ME"

ExecStart=/usr/local/sbin/openclaw-rbc-unlock.sh --open ${KEY_URI} ${DEVICE} ${MOUNT_POINT}

[Install]
WantedBy=multi-user.target
```

| 环境变量 | 说明 | VFS 模式示例值 | 块设备模式示例值 |
|----------|------|----------------|------------------|
| `KEY_URI` | RBS 资源 URI | `vault/default/secret/openclaw` | 同左 |
| `DEVICE` | 加密卷设备路径 | `/dev/loop7`（loop 设备） | `/dev/vdb`（真实块设备） |
| `MOUNT_POINT` | 挂载路径 | `/opt/openclaw-data` | 同左 |

| 指令 | 说明 |
|------|------|
| `DefaultDependencies=no` | 禁用默认依赖，确保在早期启动阶段运行 |
| `After=local-fs.target network-online.target` | 等待本地文件系统和网络就绪后启动 |
| `Wants=network-online.target` | 拉起网络在线目标 |
| `Type=oneshot` | 一次性任务，执行完毕后标记为 active |
| `RemainAfterExit=yes` | 进程退出后仍保持 active 状态 |

### 4.2 `openclaw.service`

OpenClaw LLM 网关服务（可选），在加密卷解锁后自动启动 OpenClaw。

```ini
[Unit]
Description=OpenClaw LLM Gateway
After=openclaw-luks-unlock.service
Requires=openclaw-luks-unlock.service

[Service]
Type=simple
EnvironmentFile=/etc/openclaw-cca/config

# 部署时填写
Environment="OPENCLAW_HOME=REPLACE_ME"

ExecStart=/bin/sh -c 'exec "$OPENCLAW_BIN" --config "$OPENCLAW_HOME/openclaw.json"'
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

| 环境变量 | 说明 | 示例值 |
|----------|------|--------|
| `OPENCLAW_HOME` | OpenClaw 数据目录（加密卷挂载路径） | `/opt/openclaw-data` |
| `OPENCLAW_BIN` | openclaw 二进制路径（从 EnvironmentFile 加载） | `/opt/nodejs/bin/openclaw` |

| 指令 | 说明 |
|------|------|
| `After=openclaw-luks-unlock.service` | 在 unlock 服务之后启动 |
| `Requires=openclaw-luks-unlock.service` | 强依赖 unlock 服务 |
| `EnvironmentFile` | 从 `/etc/openclaw-cca/config` 加载环境变量（`OPENCLAW_BIN`、`VFS_FILE`） |
| `Restart=on-failure` | 失败时自动重启 |
| `RestartSec=5` | 重启间隔 5 秒 |

---

## 5. 命令行工具参数

### 5.1 `extend-rem3`

将哈希值扩展至 REM3 寄存器。

```bash
extend-rem3 <hex_data>
```

| 参数 | 类型 | 说明 |
|------|------|------|
| `hex_data` | string | 十六进制字符串，最多 128 字符（64 字节），不足部分右侧自动补零 |

> 内部通过 ioctl 调用 `/dev/attest` 设备，扩展 REM 索引 3（`REM_INDEX=3`）。

### 5.2 `openclaw-init.sh`

首次初始化脚本，无命令行参数。

```bash
sudo openclaw-init.sh
```

执行流程：
1. 检测 openclaw 二进制路径
2. 写入 `/etc/openclaw-cca/config`
3. 调用 `openclaw-rbc-unlock.sh --init` 采集基线 evidence
4. 输出 `/tmp/baseline_jwt.txt`

### 5.3 `openclaw-create-volume.sh`

创建 LUKS2 加密卷。

```bash
sudo openclaw-create-volume.sh <key_uri> [选项]
```

| 参数 | 类型 | 说明 | 默认值 |
|------|------|------|--------|
| `key_uri` | string | RBS 资源 URI | 必填 |
| `--size <GB>` | int | 虚拟磁盘大小（GB） | `1` |
| `--mount <路径>` | string | 挂载路径 | `/opt/openclaw-data` |
| `--device <设备路径>` | string | 使用已有块设备 | 无（默认创建 VFS） |

### 5.4 `openclaw-rbc-unlock.sh`

RBC 远程证明与加密卷操作脚本。

```bash
openclaw-rbc-unlock.sh <模式> [参数]
```

| 模式 | 参数 | 说明 |
|------|------|------|
| `--init` | `<openclaw_bin>` | 采集基线 evidence，生成 `/tmp/baseline_jwt.txt` |
| `--create` | `<key_uri> <device> <mount_point>` | 通过 RBS 获取 passphrase，创建并挂载 LUKS2 加密卷 |
| `--open` | `<key_uri> <device> <mount_point>` | 通过 RBS 获取 passphrase，解锁并挂载已有加密卷（用于重启后自动解锁） |

### 5.5 `gen_policy.py`

从基线 JWT 生成 OPA/Rego 策略。

```bash
python3 gen_policy.py [--type cca|vcca] <jwt 或文件路径>
```

| 参数 | 类型 | 说明 | 默认值 |
|------|------|------|--------|
| `--type` | `cca`/`vcca` | 策略类型 | `cca` |
| `<jwt 或文件路径>` | string | JWT 字符串或包含 JWT 的文件路径 | 必填 |

输出：base64 编码的 Rego 策略。

### 5.6 `rbs-cli`

RBS 命令行客户端。

#### token gen - 生成访问令牌

```bash
rbs-cli token gen --private-key-file <私钥路径> [--role <角色>]
```

| 参数 | 说明 | 示例 |
|------|------|------|
| `--private-key-file` | RBS 管理员私钥路径 | `/etc/rbs/admin.pem` |
| `--role` | 角色（可选） | `Administrator` |

#### res-policy create - 创建资源策略

```bash
rbs-cli -b <RBS_URL> -t <TOKEN> res-policy create --name <名称> --content <策略文件>
```

| 参数 | 说明 |
|------|------|
| `-b` | RBS 服务地址 |
| `-t` | 访问令牌 |
| `--name` | 策略名称 |
| `--content` | Rego 策略文件路径（`@` 前缀表示从文件读取） |

#### res create - 注册资源

```bash
rbs-cli -b <RBS_URL> -t <TOKEN> res create \
    --provider-name <提供者> \
    --repository-name <仓库名> \
    --resource-type <类型> \
    --resource-name <名称> \
    --policy-id <策略ID>
```

| 参数 | 说明 | 示例 |
|------|------|------|
| `--provider-name` | 资源提供者名称 | `vault` |
| `--repository-name` | 仓库名称 | `default` |
| `--resource-type` | 资源类型 | `secret` |
| `--resource-name` | 资源名称 | `openclaw` |
| `--policy-id` | 绑定的策略 ID | `c28a6e63-...` |

#### challenge / collect-evidence / get-resource / get-token

```bash
# 获取 nonce
rbc-cli -b <RBS_URL> challenge -o <输出文件>

# 采集证据
rbc-cli -b <RBS_URL> collect-evidence \
    --nonce @<nonce文件> \
    --attester-pubkey @<公钥文件> \
    -o <输出文件>

# 获取资源
rbc-cli -b <RBS_URL> get-resource \
    --uri <资源URI> \
    --evidence @<evidence文件> \
    --private-key-file <私钥文件>

# 获取 token（用于基线生成）
rbc-cli -b <RBS_URL> get-token \
    --evidence @<evidence文件> \
    -o <输出文件>
```

---

## 6. Rego 策略模板

### 6.1 CCA 策略 (`cca.rego`)

```rego
package verification

predefined_values := {
    "cca_rpv": "",    # Realm Personalization Value（由 gen_policy.py 自动填充）
    "cca_rim": "",    # Realm Initial Measurement（由 gen_policy.py 自动填充）
    "cca_rem0": "",   # REM[0]
    "cca_rem1": "",   # REM[1]
    "cca_rem2": "",   # REM[2]
    "cca_rem3": ""    # REM[3]（包含 OpenClaw 组件度量）
}

default attestation_valid = false

attestation_valid {
    input.cca.realm_token.cca_rpv == predefined_values.cca_rpv
    input.cca.realm_token.cca_rim == predefined_values.cca_rim
    input.cca.realm_token.cca_rem0 == predefined_values.cca_rem0
    input.cca.realm_token.cca_rem1 == predefined_values.cca_rem1
    input.cca.realm_token.cca_rem2 == predefined_values.cca_rem2
    input.cca.realm_token.cca_rem3 == predefined_values.cca_rem3
}

result = {"policy_matched": attestation_valid}
```

**输入路径**：`input.cca.realm_token.<key>`

### 6.2 vCCA 策略 (`vcca.rego`)

```rego
package verification

predefined_values := {
    "vcca_rpv": "",
    "vcca_rim": "",
    "vcca_rem0": "",
    "vcca_rem1": "",
    "vcca_rem2": "",
    "vcca_rem3": ""
}

default attestation_valid = false

attestation_valid {
    input.virt_cca.realm_token.vcca_rpv == predefined_values.vcca_rpv
    input.virt_cca.realm_token.vcca_rim == predefined_values.vcca_rim
    input.virt_cca.realm_token.vcca_rem0 == predefined_values.vcca_rem0
    input.virt_cca.realm_token.vcca_rem1 == predefined_values.vcca_rem1
    input.virt_cca.realm_token.vcca_rem2 == predefined_values.vcca_rem2
    input.virt_cca.realm_token.vcca_rem3 == predefined_values.vcca_rem3
}

result = {"policy_matched": attestation_valid}
```

**输入路径**：`input.virt_cca.realm_token.<key>`

---

## 7. CCA 与 vCCA 适配速查

| 配置项 | CCA 模式 | vCCA 模式 |
|--------|----------|-----------|
| 内核模块 | `modprobe tsm && modprobe arm_cca_guest` | 不需要 |
| Agent 插件 | `plugins` 中 `name: "cca"` 的条目启用，`name: "virt_cca"` 的条目禁用 | `plugins` 中 `name: "cca"` 的条目禁用，`name: "virt_cca"` 的条目启用 |
| 策略模板 | `cca.rego` | `vcca.rego` |
| gen_policy.py | `gen_policy.py <jwt>`（默认） | `gen_policy.py --type vcca <jwt>` |
| JWT 字段路径 | `cca.realm_token` | `virt_cca.realm_token` |
| 度量键名前缀 | `cca_` | `vcca_` |
| Rego 输入路径 | `input.cca.realm_token.*` | `input.virt_cca.realm_token.*` |
| attest skill | `openclaw-cca-attest` | `openclaw-vcca-attest` |

---

## 8. Makefile 变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `CC` | `gcc` | C 编译器 |
| `CFLAGS` | `-O2 -Wall -Wextra` | 编译选项 |
| `DESTDIR` | （空） | 安装根目录前缀（用于打包） |
| `BIN_DIR` | `$(DESTDIR)/usr/local/bin` | 可执行文件安装目录 |
| `SBIN_DIR` | `$(DESTDIR)/usr/local/sbin` | 管理脚本安装目录 |
| `UNIT_DIR` | `$(DESTDIR)/etc/systemd/system` | systemd unit 安装目录 |

### 安装目标说明

| Makefile 目标 | 说明 |
|---------------|------|
| `all` | 编译 `extend-rem3` |
| `install` | 编译并安装所有文件到系统目录 |
| `uninstall` | 删除已安装的文件 |
| `clean` | 清理编译产物 |

### 安装后的文件清单

| 安装路径 | 源文件 | 权限 |
|----------|--------|------|
| `/usr/local/bin/extend-rem3` | `src/extend_tools/attest.c` | `0550` |
| `/usr/local/sbin/openclaw-rbc-unlock.sh` | `scripts/openclaw-rbc-unlock.sh` | `0550` |
| `/usr/local/sbin/openclaw-init.sh` | `scripts/openclaw-init.sh` | `0550` |
| `/usr/local/sbin/openclaw-create-volume.sh` | `scripts/openclaw-create-volume.sh` | `0550` |
| `/etc/systemd/system/openclaw-luks-unlock.service` | `systemd/openclaw-luks-unlock.service` | `0640` |

> **注意**：`openclaw.service` 未包含在 Makefile 的 install 目标中。如需使用，需手动拷贝：
> ```bash
> sudo cp systemd/openclaw.service /etc/systemd/system/
> ```
