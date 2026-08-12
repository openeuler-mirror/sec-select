# OpenClaw-CCA 环境搭建指南

本文档介绍如何在 RBS 服务器（非虚机）和 CCA/vCCA 安全容器（虚机）两套环境中完成 OpenClaw-CCA 所需的全部依赖安装与配置。

> 本文档假设 RBS 服务器运行在 `192.168.1.1`，以下命令中涉及 IP 的部分请按实际环境替换。

---

## 目录

- [1. 适用范围与软件版本](#1-适用范围与软件版本)
- [2. RBS 服务器环境配置](#2-rbs-服务器环境配置)
  - [2.1 安装主要软件包](#21-安装主要软件包)
  - [2.2 安装 GTA-Server 依赖](#22-安装-gta-server-依赖)
  - [2.3 配置 GTA-Server 环境变量 (`/etc/attestation_server/.env`)](#23-配置-gta-server-环境变量)
  - [2.4 生成 GTA-Server 密钥对](#24-生成-gta-server-密钥对)
  - [2.5 生成 GTA-Server TLS 证书](#25-生成-gta-server-tls-证书)
  - [2.6 安装 RBS 依赖](#26-安装-rbs-依赖)
  - [2.7 配置 RBS (`/etc/rbs/rbs.yaml`)](#27-配置-rbs-etcrbsyaml)
  - [2.8 生成 RBS 相关密钥](#28-生成-rbs-相关密钥)
  - [2.9 安装并配置 openBao（RBS Resource 存储后端）](#29-安装并配置-openbaorbs-resource-存储后端)
  - [2.10 启动服务](#210-启动服务)
- [3. 准备安全资源（RBS Server 侧）](#3-准备安全资源rbs-server-侧)
  - [3.1 安装 rbs-cli](#31-安装-rbs-cli)
  - [3.2 准备验证策略（Rego）](#32-准备验证策略rego)
  - [3.3 在 openBao 中写入秘密](#33-在-openbao-中写入秘密)
  - [3.4 在 RBS 中创建资源策略](#34-在-rbs-中创建资源策略)
  - [3.5 在 RBS 中注册资源](#35-在-rbs-中注册资源)
- [4. 安全容器内（虚机）环境配置](#4-安全容器内虚机环境配置)
  - [4.1 安装软件包](#41-安装软件包)
  - [4.2 配置 Attestation Agent (`/etc/attestation_agent/agent_config.yaml`)](#42-配置-attestation-agent-etcauth_agent_configyaml)
  - [4.3 使能 CCA（仅硬件 CCA 需要执行）](#43-使能-cca仅硬件-cca-需要执行)
  - [4.4 测试 RBS 与 Attestation Server 的连通性](#44-测试-rbs-与-attestation-server-的连通性)
  - [4.5 准备 attester 密钥并测试 RBS 接口](#45-准备-attester-密钥并测试-rbs-接口)

---

## 1. 适用范围与软件版本

OpenEuler 24.03 SP4 LTS 最小安装。本文基于以下软件包版本（`0.0.1-30.oe2403`）提供构建教程。

| 软件包 | 大小 | 更新日期 |
|---|---|---|
| global-trust-authority-agent-0.0.1-30.oe2403sp4.x86_64.rpm | 20.9 MiB | 2026-Jun-30 15:38 |
| global-trust-authority-cli-0.0.1-30.oe2403sp4.x86_64.rpm | 19.7 MiB | 2026-Jun-30 15:40 |
| global-trust-authority-debuginfo-0.0.1-30.oe2403sp4.x86_64.rpm | 425.4 MiB | 2026-Jun-30 15:41 |
| global-trust-authority-debugsource-0.0.1-30.oe2403sp4.x86_64.rpm | 13.3 MiB | 2026-Jun-30 15:38 |
| global-trust-authority-key-manager-0.0.1-30.oe2403sp4.x86_64.rpm | 3.8 MiB | 2026-Jun-30 15:38 |
| global-trust-authority-server-0.0.1-30.oe2403sp4.x86_64.rpm | 38.2 MiB | 2026-Jun-30 15:40 |
| globaltrustauthority-rbs-cli-0.0.1-2.oe2403sp4.x86_64.rpm | 4.5 MiB | 2026-Jun-30 15:39 |
| globaltrustauthority-rbs-debuginfo-0.0.1-2.oe2403sp4.x86_64.rpm | 182.7 MiB | 2026-Jun-30 15:39 |
| globaltrustauthority-rbs-debugsource-0.0.1-2.oe2403sp4.x86_64.rpm | 10.7 MiB | 2026-Jun-30 15:41 |
| globaltrustauthority-rbs-rbc-devel-0.0.1-2.oe2403sp4.x86_64.rpm | 7.5 MiB | 2026-Jun-30 15:38 |
| globaltrustauthority-rbs-rbs-0.0.1-2.oe2403sp4.x86_64.rpm | 8.5 MiB | 2026-Jun-30 15:40 |

---

## 2. RBS 服务器环境配置

### 2.1 安装主要软件包

```bash
sudo dnf install global-trust-authority-server globaltrustauthority-rbs-rbs
# global-trust-authority-key-manager 可以不需要
```

### 2.2 安装 GTA-Server 依赖

GTA-Server 需要 MySQL 和 Redis 作为数据库与缓存后端，同时需要生成 FSK / NSK / TSK 三组非对称密钥用于证据签名验证。

```bash
sudo dnf install mysql-server redis cjson
sudo systemctl enable --now mysqld
sudo systemctl enable --now redis

# 创建密钥存放目录
mkdir -p /etc/attestation_server/keys

# 生成 FSK（File Signing Key）和 NSK（Nonce Signing Key），均使用 RSA-PSS 3072 位
openssl genpkey -algorithm RSA-PSS -pkeyopt rsa_keygen_bits:3072 -out /etc/attestation_server/keys/fsk_private_key.pem
openssl rsa -in /etc/attestation_server/keys/fsk_private_key.pem -pubout -out /etc/attestation_server/keys/fsk_public_key.pem
openssl genpkey -algorithm RSA-PSS -pkeyopt rsa_keygen_bits:3072 -out /etc/attestation_server/keys/nsk_private_key.pem
openssl rsa -in /etc/attestation_server/keys/nsk_private_key.pem -pubout -out /etc/attestation_server/keys/nsk_public_key.pem

# 生成 TSK（Token Signing Key），使用 RSA 4096 位
openssl genrsa -out /etc/attestation_server/keys/tsk_private_key.pem 4096
openssl rsa -in /etc/attestation_server/keys/tsk_private_key.pem -pubout -out /etc/attestation_server/keys/tsk_public_key.pem

# 更改 MySQL root 默认密码（如已有密码可跳过）并初始化数据库
mysql -u root << EOF
create USER 'ra_user'@'localhost' IDENTIFIED BY 'ra_user_password';
CREATE DATABASE RA;
GRANT ALL PRIVILEGES ON RA.* TO 'ra_user'@'localhost';
FLUSH PRIVILEGES;
EOF
```

### 2.3 配置 GTA-Server 环境变量

GTA-Server 通过 `/etc/attestation_server/.env` 文件读取数据库连接信息和 HTTPS 开关。以下配置关闭 HTTPS（便于内部测试），并指向上面创建的 MySQL 数据库。

```bash
DB_USER=ra_user
DB_PASSWORD=ra_user_password

HTTPS_SWITCH=0

MYSQL_DATABASE_URL=mysql://ra_user:ra_user_password@127.0.0.1:3306/RA
```

| 变量 | 说明 | 示例值 |
|------|------|--------|
| `DB_USER` | MySQL 数据库用户名 | `ra_user` |
| `DB_PASSWORD` | MySQL 数据库密码 | `ra_user_password` |
| `HTTPS_SWITCH` | 是否启用 HTTPS（`0` 关闭，`1` 开启） | `0` |
| `MYSQL_DATABASE_URL` | MySQL 连接串，格式 `mysql://<user>:<password>@<host>:<port>/<db>` | `mysql://ra_user:ra_user_password@127.0.0.1:3306/RA` |

> **生产环境建议**：将 `HTTPS_SWITCH` 设为 `1` 并配置 TLS 证书（见 [2.5](#25-生成-gta-server-tls-证书)）。

### 2.4 生成 GTA-Server 密钥对

上一节已生成 FSK / NSK / TSK 的公私钥。这些密钥的作用：

| 密钥 | 用途 | 算法 |
|------|------|------|
| FSK | File Signing Key，签名文件度量 | RSA-PSS 3072 |
| NSK | Nonce Signing Key，签名 nonce | RSA-PSS 3072 |
| TSK | Token Signing Key，签名 attestation token | RSA 4096 |

> TSK 的公钥 (`tsk_public_key.pem`) 需要拷贝到 RBS，用于 RBS 验证 GTA 签发的 token（见 [2.8](#28-生成-rbs-相关密钥)）。

### 2.5 生成 GTA-Server TLS 证书

如果 `HTTPS_SWITCH=1`，需要生成 CA 证书和服务器证书。

```bash
mkdir -p /etc/attestation_server/certs

# 1. 生成自签名 CA 证书
sudo openssl req -x509 -newkey rsa:3072 \
    -out /etc/attestation_server/certs/ca.crt \
    -keyout /etc/attestation_server/certs/ca.key \
    -noenc -subj "/CN=GTA"

# 2. 生成服务器私钥和 CSR
sudo openssl genrsa -out /etc/attestation_server/certs/server.key 3072
sudo openssl req -new \
    -key /etc/attestation_server/certs/server.key \
    -out /etc/attestation_server/certs/server.csr \
    -subj "/CN=GTA-Server" \
    -addext "subjectAltName=IP:127.0.0.1"

# 3. 使用 CA 签发服务器证书
sudo openssl x509 -req \
    -in /etc/attestation_server/certs/server.csr \
    -CA /etc/attestation_server/certs/ca.crt \
    -CAkey /etc/attestation_server/certs/ca.key \
    -CAcreateserial \
    -out /etc/attestation_server/certs/server.crt \
    -copy_extensions copy
```

| 文件 | 说明 |
|------|------|
| `ca.crt` / `ca.key` | CA 证书与私钥 |
| `server.key` | 服务器私钥 |
| `server.crt` | 服务器证书 |
| `server.csr` | 中间文件（签发后可删除） |

> 如果 `subjectAltName` 中的 IP 与实际服务器 IP 不同，请替换 `127.0.0.1` 为实际 IP，例如 `IP:192.168.1.1`。

### 2.6 安装 RBS 依赖

RBS 使用 SQLite 作为本地状态数据库。

```bash
sudo dnf install sqlite

# 验证数据库文件可正常访问并创建数据库文件
sqlite3 /var/lib/rbs/rbs.db "SELECT 1;"
```

### 2.7 配置 RBS (`/etc/rbs/rbs.yaml`)

`rbs.yaml` 是 RBS 的主配置文件，控制监听地址、证明后端、资源存储后端。以下分两部分说明：先配置基础证明链路，再配置资源存储。

#### 2.7.1 基础配置（证明后端）

默认使用 `vi` 打开配置文件；也可按个人习惯改用 `vim` 或 `nano`：

```bash
sudo vi /etc/rbs/rbs.yaml
```

修改为以下内容（关闭 JWT/JWKS 认证，使用公钥直接验证 token）：

```yaml
rest:
  listen_addr: "127.0.0.1:6666"   # 请在实际部署时更改为 "0.0.0.0:6666" 并配置防火墙策略
  https:
    enabled: false
auth:
  attest_token:
    public_key_path: "/etc/rbs/attest_pub.pem"
    # 注释以下一行（不使用 JWKS 文件）
    # jwks_file: "/etc/rbs/attest.jwk"
attestation:
  backends:
    gta:
      rest:
        base_url: "http://127.0.0.1:8080"
        credentials:
          # 注释以下两行（当前版本不需要 API Key）
          # main_api_key: "${MAIN_API_KEY}"
          # sub_api_key: "${SUB_API_KEY}"
```

| 字段 | 说明 | 示例值 |
|------|------|--------|
| `auth.attest_token.public_key_path` | GTA 的 TSK 公钥路径，RBS 用它验证 token 签名 | `/etc/rbs/attest_pub.pem` |
| `auth.attest_token.jwks_file` | JWKS 文件路径（与 `public_key_path` 二选一，本文不使用） | 注释掉 |
| `attestation.backends.gta.rest.base_url` | GTA-Server 的 REST API 地址 | `http://127.0.0.1:8080` |
| `attestation.backends.gta.rest.credentials.main_api_key` | 主 API Key（预留，当前注释） | 注释掉 |
| `attestation.backends.gta.rest.credentials.sub_api_key` | 副 API Key（预留，当前注释） | 注释掉 |

> **适配要点**：
> - 如果 GTA-Server 运行在其他主机，将 `base_url` 中的 `127.0.0.1` 替换为 GTA-Server 的实际 IP。
> - 如果 GTA-Server 启用了 HTTPS（`HTTPS_SWITCH=1`），将 `base_url` 改为 `https://...`，并确保 RBS 主机信任 GTA 的 CA 证书。

#### 2.7.2 完整配置（含资源存储后端）

下方展示 RBS 的最终完整配置，供后续步骤参考。请先完成 2.7.1 的基础配置；待 2.9 完成 openBao 初始化并取得实际 Root Token 后，再按照 2.9.5 的说明修改现有配置，使其包含 `rest.listen_addr` 和 `resource` 部分。不要将下方内容直接追加到配置文件末尾：

```yaml
rest:
  listen_addr: "127.0.0.1:6666"   # 请在实际部署时更改为 "0.0.0.0:6666" 并配置防火墙策略
  https:
    enabled: false
attestation:
  backends:
    gta:
      rest:
        base_url: "http://127.0.0.1:8080"
        credentials:
          # main_api_key: "${MAIN_API_KEY}"
          # sub_api_key: "${SUB_API_KEY}"
auth:
  attest_token:
    public_key_path: "/etc/rbs/attest_pub.pem"
    # jwks_file: "/etc/rbs/attest.jwk"
resource:
  default_provider: vault
  backends:
    # local:
    #   type: local
    vault:
      type: vault
      url: "http://127.0.0.1:8200"
      token: "s.u6O7REadDh4C5RwxCkfIdfOh"   # openBao 的 Root Token（见 2.9）
      mount_path: "secret"
    # ca:
    #   type: ca
    #   url: "https://ca-server:8443"
    #   token: "${CA_TOKEN}"
    #   default_profile: "server"
```

| 字段 | 说明 | 示例值 |
|------|------|--------|
| `rest.listen_addr` | RBS REST API 监听地址 | `127.0.0.1:6666`（本机）或 `0.0.0.0:6666`（允许远程访问） |
| `resource.default_provider` | 默认资源存储后端名称 | `vault` |
| `resource.backends.vault.type` | 后端类型 | `vault`（即 openBao） |
| `resource.backends.vault.url` | openBao 服务地址 | `http://127.0.0.1:8200` |
| `resource.backends.vault.token` | openBao 访问令牌（Root Token 或有 secret 读写权限的 Token） | `s.u6O7REadDh4C5RwxCkfIdfOh` |
| `resource.backends.vault.mount_path` | KV 存储挂载路径 | `secret` |
| `resource.backends.local.type` | 本地文件后端（可选，本文不使用） | `local` |
| `resource.backends.ca.type` | CA 后端（可选，本文不使用） | `ca` |

> **适配要点**：
> - `listen_addr` 默认绑定 `127.0.0.1`，仅供本机测试。生产环境或需要让虚机远程访问时，改为 `0.0.0.0:6666`，并通过防火墙放行 6666 端口。
> - `vault.token` 必须是 openBao 初始化时生成的 Root Token（或具有 secret 读写权限的子 Token）。该 Token 也出现在 [2.9](#29-安装并配置-openbaorbs-resource-存储后端) 中 `bao operator init` 的输出中。
> - 如果 openBao 启用了 TLS，`url` 改为 `https://...`，并确保 RBS 主机信任 openBao 的 CA 证书。

### 2.8 生成 RBS 相关密钥

RBS 需要一对管理员密钥（用于 rbs-cli 认证）和 GTA 的 TSK 公钥（用于验证 attestation token）。

```bash
# 1. 生成 RBS 管理员私钥和公钥（用于 rbs-cli 生成访问令牌）
openssl genrsa -out /etc/rbs/admin.pem 4096
openssl rsa -in /etc/rbs/admin.pem -pubout -out /etc/rbs/admin_pub.pem

# 2. 将 GTA-Server 的 TSK 公钥拷贝到 RBS 配置目录
#    RBS 用 TSK 公钥验证 GTA 签发的 attestation token 的签名
cp /etc/attestation_server/keys/tsk_public_key.pem /etc/rbs/attest_pub.pem
```

| 文件 | 说明 |
|------|------|
| `/etc/rbs/admin.pem` | RBS 管理员私钥，rbs-cli 用它生成访问令牌 |
| `/etc/rbs/admin_pub.pem` | RBS 管理员公钥，RBS Server 用它验证令牌签名 |
| `/etc/rbs/attest_pub.pem` | GTA 的 TSK 公钥，RBS 用它验证 attestation token 签名 |

> **适配要点**：`attest_pub.pem` 必须与 GTA-Server 的 `tsk_public_key.pem` 一致，否则 RBS 无法验证 token 签名，导致 attestation 失败。

### 2.9 安装并配置 openBao（RBS Resource 存储后端）

openBao（Vault 的开源分支）作为 RBS 的 KV 存储后端，用于存放 passphrase 等秘密资源。

#### 2.9.1 安装 openBao

```bash
# AMD64 平台
curl -fsSL -O https://github.com/openbao/openbao/releases/download/v2.6.1/openbao_2.6.1_linux_amd64.rpm
sudo dnf install ./openbao_2.6.1_linux_amd64.rpm

# ARM64 平台
curl -fsSL -O https://github.com/openbao/openbao/releases/download/v2.6.1/openbao_2.6.1_linux_arm64.rpm
sudo dnf install ./openbao_2.6.1_linux_arm64.rpm
```

#### 2.9.2 配置 openBao

默认使用 `vi` 编辑配置文件，禁用 HTTPS，启用 HTTP（本机访问）；也可按个人习惯改用 `vim` 或 `nano`：

```bash
vi /etc/openbao/openbao.hcl
```

修改后的文件内容如下：

```hcl
ui = true

storage "file" {
  path = "/opt/openbao/data"
}

# HTTP listener（本机访问，禁用 TLS）
listener "tcp" {
  address     = "127.0.0.1:8200"
  tls_disable = 1
}

# HTTPS listener（生产环境推荐启用，需配置 TLS 证书）
# listener "tcp" {
#   address       = "0.0.0.0:8200"
#   tls_cert_file = "/opt/openbao/tls/tls.crt"
#   tls_key_file  = "/opt/openbao/tls/tls.key"
# }
```

| 字段 | 说明 | 示例值 |
|------|------|--------|
| `ui` | 是否启用 Web UI | `true` |
| `storage "file".path` | 数据存储目录 | `/opt/openbao/data` |
| `listener "tcp".address` | 监听地址 | `127.0.0.1:8200` |
| `listener "tcp".tls_disable` | 是否禁用 TLS | `1`（禁用）或 `0`（启用） |
| `listener "tcp".tls_cert_file` | TLS 证书路径（TLS 启用时必填） | `/opt/openbao/tls/tls.crt` |
| `listener "tcp".tls_key_file` | TLS 私钥路径（TLS 启用时必填） | `/opt/openbao/tls/tls.key` |

> **适配要点**：
> - 如果 RBS 和 openBao 不在同一主机，将 `address` 改为 `0.0.0.0:8200` 并启用 TLS。
> - 生产环境应使用实际 X509 证书，而非自签名证书。

#### 2.9.3 启动和初始化 openBao

```bash
sudo systemctl enable --now openbao
export BAO_ADDR=http://127.0.0.1:8200
bao operator init
```

`bao operator init` 会输出 5 个 Unseal Key 和 1 个 Root Token，**请妥善保存**：

```
Unseal Key 1: 4guvN68/un4SntwLjqyn4OTzoRlNQ7tAgpAcGtESHtzV
Unseal Key 2: 1SMpSQSIIVEBuk1BhGZO3tdg8r+ENtl1vZi5VwAqlQyo
Unseal Key 3: BUwiJEAWSzFXyhRIoyoiHapjwYI5I6JlRVKxn3wMOn2T
Unseal Key 4: u7X+Jc0zkVPQNvwE7Zcyvagi8mW0mKnY6PhSQvwmTUrY
Unseal Key 5: znEXrSzvLha690pmyjL+oh9e2uNhob1zTtvg8mESygs/

Initial Root Token: s.l3J24O59v2NkrQ8fli7u5rEP
```

#### 2.9.4 解密 openBao

使用任意 3 个 Unseal Key 解密（默认需要 3 次）：

```bash
bao operator unseal 4guvN68/un4SntwLjqyn4OTzoRlNQ7tAgpAcGtESHtzV  # Unseal Key 1
bao operator unseal 1SMpSQSIIVEBuk1BhGZO3tdg8r+ENtl1vZi5VwAqlQyo  # Unseal Key 2
bao operator unseal BUwiJEAWSzFXyhRIoyoiHapjwYI5I6JlRVKxn3wMOn2T  # Unseal Key 3
```

> **重要**：妥善保存 Root Token。下一步需要使用该 Token 配置 RBS 的 resource backend。

#### 2.9.5 配置 RBS Resource 存储后端

openBao 初始化并解封后，回到 `/etc/rbs/rbs.yaml`，在 2.7.1 的基础配置上继续修改。不要把下方配置直接追加到文件末尾；如果相关字段已经存在，请修改其值，并确保最终配置结构与 2.7.2 的完整示例一致。

默认使用 `vi` 打开配置文件；也可按个人习惯改用 `vim` 或 `nano`：

```bash
sudo vi /etc/rbs/rbs.yaml
```

确认配置中包含以下 `resource` 部分，并将`url`替换为实际openbao运行地址（HTTP或HTTPS），`<OPENBAO_ROOT_TOKEN>` 替换为 `bao operator init` 生成的实际 Root Token：

```yaml
resource:
  default_provider: vault
  backends:
    vault:
      type: vault
      url: "http://127.0.0.1:8200"
      token: "<OPENBAO_ROOT_TOKEN>"
      mount_path: "secret"
```

> **注意**：这里只列出需要在此步骤确认的字段，不代表完整的 `rbs.yaml`。请保留 2.7.1 中已有的 `rest`、`auth` 和 `attestation` 配置。

#### 2.9.6 登录并启用 KV 存储

```bash
# 使用 Root Token 登录
bao login

# 启用 KV v2 秘密引擎，挂载路径为 secret（与 rbs.yaml 中 mount_path 一致）
bao secrets enable --path=secret kv-v2
```

> **适配要点**：`--path=secret` 必须与 `/etc/rbs/rbs.yaml` 中 `resource.backends.vault.mount_path` 的值一致。如果你使用了其他挂载路径（如 `kv`），需要同步修改两处配置。

### 2.10 启动服务

```bash
sudo systemctl enable --now attestation_server
sudo systemctl enable --now rbs
```

验证服务状态：

```bash
systemctl status attestation_server
systemctl status rbs
```

---

## 3. 准备安全资源（RBS Server 侧）

### 3.1 安装 rbs-cli

在 RBS 服务器上安装 rbs-cli，用于管理资源策略和资源：

```bash
sudo dnf install globaltrustauthority-rbs-cli
```

### 3.2 准备验证策略（Rego）

RBS 使用 OPA/Rego 策略验证 attestation evidence。准备一个基础验证策略：

```bash
cat > rego << 'EOF'
package verification

default attestation_valid = false
attestation_valid {
    input.status == "pass"
}

result = {"policy_matched": attestation_valid}
EOF
```

> **说明**：以上是一个最简单的策略，仅检查 attestation 状态是否为 `pass`。在实际 OpenClaw-CCA 部署中，策略由 `gen_policy.py` 脚本根据基线 JWT 自动生成，包含对 REM/RIM 值的精确匹配。请参考 [使用手册](usage_guide.md) 中的步骤二。

### 3.3 在 openBao 中写入秘密

```bash
# 语法: bao kv put secret/<repository>/<resource-type>/<resource-name> key=value
bao kv put secret/default/secret/mysecret username=private-username
```

> **路径规则**：`secret/default/secret/mysecret` 对应 RBS 资源 URI `vault/default/secret/mysecret`，其中：
> - `vault` — provider 名称（与 `rbs.yaml` 中 `resource.backends.vault` 一致）
> - `default` — repository 名称
> - `secret` — resource type
> - `mysecret` — resource name

### 3.4 在 RBS 中创建资源策略

```bash
export RBS_SERVER=http://192.168.1.1:6666
export ACCESS_KEY=$(rbs-cli token gen --private-key-file /etc/rbs/admin.pem)

rbs-cli -b ${RBS_SERVER} -t ${ACCESS_KEY} res-policy create \
    --name policy-01 \
    --content @./rego

# 保存上一步生成的 policy-id，如 c28a6e63-b0b2-4fdd-9832-4d297f28e31e
```

| 参数 | 说明 |
|------|------|
| `-b` | RBS 服务地址 |
| `-t` | 访问令牌（由 admin 私钥签名生成） |
| `--name` | 策略名称 |
| `--content` | Rego 策略文件路径（`@` 前缀表示从文件读取） |

### 3.5 在 RBS 中注册资源

```bash
rbs-cli -b ${RBS_SERVER} -t ${ACCESS_KEY} res create \
    --uri vault/default/secret/mysecret \
    --policy-id c28a6e63-b0b2-4fdd-9832-4d297f28e31e
```

| 参数 | 说明 | 对应 openBao 路径 |
|------|------|-------------------|
| `--uri` | 资源 URI | `vault/default/secret/mysecret` |
| `--policy-id` | 绑定的策略 ID | 步骤 [3.4](#34-在-rbs-中创建资源策略) 返回的 ID |

> 注册后资源的 URI 为 `vault/default/secret/mysecret`，虚机内的 rbc-cli 通过此 URI 获取资源。

---

## 4. 安全容器内（虚机）环境配置

### 4.1 安装软件包

在安全容器（CCA Realm 或 vCCA 虚机）内安装 attestation agent 和 rbc-cli：

```bash
dnf install global-trust-authority-agent globaltrustauthority-rbs-rbc-devel
```

### 4.2 配置 Attestation Agent (`/etc/attestation_agent/agent_config.yaml`)

Attestation Agent 的配置文件位于 `/etc/attestation_agent/agent_config.yaml`，控制证据采集行为。

#### 4.2.1 修复字段名（兼容性）

由于版本差异，配置文件中可能存在 `ccel_data_path` 字段，需要替换为 `boot_log_file_path`：

```bash
sed -i 's|ccel_data_path|boot_log_file_path|' /etc/attestation_agent/agent_config.yaml
```

#### 4.2.2 配置项说明

默认使用 `vi` 编辑配置文件；也可按个人习惯改用 `vim` 或 `nano`：

```bash
vi /etc/attestation_agent/agent_config.yaml
```

需要关注的关键配置项：

| 配置项 | 说明 | 适配方法 |
|--------|------|----------|
| `server` 部分 | GTA-Server 地址 | 改为对应 HTTP 地址（如 `http://192.168.1.1:8080`）；若 GTA 启用了 HTTPS，需导入 CA 证书 |
| `plugins` 中 `name: "cca"` 的条目 | 硬件 CCA attester 插件配置 | 硬件 CCA：`enabled: true`；virtCCA：`enabled: false` |
| `plugins` 中 `name: "virt_cca"` 的条目 | virtCCA attester 插件配置 | virtCCA：`enabled: true`；硬件 CCA：`enabled: false` |
| `boot_log_file_path` | 启动日志路径（原 `ccel_data_path`） | 通常保持默认值 |

> **适配要点**：
> - `server` 部分的地址必须指向 GTA-Server（不是 RBS），端口默认为 `8080`。
> - 使用硬件 CCA 时，将 `plugins` 列表中 `name: "cca"` 条目的 `enabled` 设为 `true`，并将 `name: "virt_cca"` 条目的 `enabled` 设为 `false`。
> - 使用 virtCCA 时，将 `plugins` 列表中 `name: "virt_cca"` 条目的 `enabled` 设为 `true`，并将 `name: "cca"` 条目的 `enabled` 设为 `false`。
> - 硬件 CCA 与 virtCCA 插件不要同时启用；其他不使用的 attester 插件也应保持 `false`。
> - 如果 GTA-Server 启用了 HTTPS，需要在 agent 中配置 CA 证书路径，使 agent 信任 GTA 的证书。

### 4.3 使能 CCA（仅硬件 CCA 需要执行）

> **vCCA 用户跳过此步骤**。vCCA 不需要加载 `arm_cca_guest` 内核模块。

以 `root` 身份运行以下命令，加载 CCA 内核模块并测试证据采集：

```bash
modprobe tsm
modprobe arm_cca_guest
mount -t configfs none /sys/kernel/config

# 测试 CCA 证据采集
export report=/sys/kernel/config/tsm/report/report0
mkdir -p $report
dd if=/dev/urandom bs=64 count=1 > $report/inblob
hexdump -C $report/outblob
hexdump -C $report/auxblob
```

如果 `outblob` 和 `auxblob` 有非零输出，说明 CCA 硬件证明功能正常。

### 4.4 测试 RBS 与 Attestation Server 的连通性

```bash
export RBS_SERVER=http://192.168.1.1:6666
curl -X GET ${RBS_SERVER}/rbs/v0/challenge
# 应返回 json 消息 {"nonce": xxx}
```

如果返回 nonce，说明 RBS 服务正常且网络连通。

### 4.5 准备 attester 密钥并测试 RBS 接口

```bash
# 1. 生成 attester 密钥对（公钥绑定到 evidence，私钥用于处理返回的资源）
openssl ecparam -name prime256v1 -genkey -out attester.key
openssl ec -in attester.key -pubout -out attester.pub

# 2. 准备一次性 nonce
rbc-cli -b ${RBS_SERVER} challenge > nonce

# 3. 获取并验证 evidence
rbc-cli -b ${RBS_SERVER} collect-evidence \
    --nonce @./nonce \
    --attester-pubkey @./attester.pub > evidence

# 4. 通过 evidence 远程获取安全资源
rbc-cli -b ${RBS_SERVER} get-resource \
    --uri vault/default/secret/mysecret \
    --evidence @evidence

# 或者使用私钥文件方式
rbc-cli -b ${RBS_SERVER} get-resource \
    --uri vault/default/secret/mysecret \
    --evidence @evidence \
    --private-key-file attester.key
```

| 步骤 | 说明 |
|------|------|
| `challenge` | 从 RBS 获取一次性随机数（nonce） |
| `collect-evidence` | 采集 TEE 证据并绑定 attester 公钥，输出 evidence |
| `get-resource` | 提交 evidence 至 RBS，通过验证后使用对应私钥处理返回的资源 |

> **验证成功**：如果 `get-resource` 返回了你在 [3.3](#33-在-openbao-中写入秘密) 中写入的秘密内容，说明整个证明链路正常工作。接下来请按照 [使用手册](usage_guide.md) 进行 OpenClaw-CCA 的完整部署。
