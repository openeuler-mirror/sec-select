# OpenClaw-CCA

在 ARM CCA（Confidential Compute Architecture，机密计算架构）Realm 中运行 [OpenClaw](https://github.com/nicholasgriffintn/openclaw) LLM 网关，通过远程证明将 LUKS 加密卷的解锁与 Realm 完整性度量绑定 — 只有通过验证的 Realm 实例才能解锁加密卷。

## 概述

OpenClaw-CCA 将 OpenClaw 的全部持久化数据加密存储在 LUKS2 加密卷中，解密口令不落盘。口令由 [RBS](https://gitcode.com/openeuler/globaltrustauthority-rbs)（Resource Broker Service，资源代理服务）在 Realm 通过 [GTA](https://gitcode.com/openeuler/global-trust-authority)（Global Trust Authority，全局信任中心）远程证明验证后按策略下发。

加密卷中存放：

- **配置数据**（`openclaw.json`）：API Key、LLM 服务端点、工具/技能配置
- **长期记忆**：用户对话历史、用户画像、知识库与向量嵌入
- **会话状态**：持久化会话快照、缓存中间结果
- **运行时元数据**：日志、审计记录、Skill workspace

## 信任链

```
Realm 启动
  └─ extend-rem3 度量关键组件（losetup / cryptsetup / rbc-cli / openclaw / shadow hash）
       └─ rbc-cli collect-evidence 采集含 REM3 的 TEE evidence
            └─ RBS 验证 evidence 与策略匹配
                 └─ 下发 passphrase → cryptsetup luksOpen → openclaw 启动
```

## 环境

| 项目 | 说明 |
|------|------|
| 操作系统 | openEuler 24.03-SP4 |
| 硬件平台 | 鲲鹏 950 (Kunpeng 950) |
| 可信执行环境 | ARM CCA Realm（同时支持 vCCA） |

## 依赖项目

| 项目 | 作用 |
|------|------|
| [GTA](https://gitcode.com/openeuler/global-trust-authority) @ `3f58d58` | 硬件可信验证的远程证明，负责验证节点完整性 |
| [RBS](https://gitcode.com/openeuler/globaltrustauthority-rbs) @ `8e3d7f7` | 策略驱动的可信资源分发，按策略释放密钥等敏感资源 |

## 目录结构

```
openclaw-cca/
├── src/
│   └── extend_tools/
│       └── attest.c                  # extend-rem3：通过 ioctl 扩展 REM3 寄存器
├── Makefile                          # 构建与安装
├── docs/
│   ├── usage_guide.md                # 完整部署手册
│   └── images/demo.png               # 信任链示意图
├── scripts/
│   ├── openclaw-init.sh              # 首次初始化
│   ├── openclaw-create-volume.sh     # 创建 LUKS2 加密卷
│   ├── openclaw-rbc-unlock.sh        # 通过 RBC 证明解锁加密卷
│   ├── gen_policy.py                 # 从 JWT 基线生成 OPA/Rego 策略
│   └── policy_template/
│       ├── cca.rego                  # CCA 证明策略模板
│       └── vcca.rego                 # vCCA 证明策略模板
├── skills/
│   ├── openclaw-cca-attest/          # Agent 技能：CCA 证明流程
│   └── openclaw-vcca-attest/         # Agent 技能：vCCA 证明流程
└── systemd/
    ├── openclaw-luks-unlock.service  # 开机自动解锁加密卷
    └── openclaw.service              # OpenClaw 网关服务
```

## 快速开始

### 前置条件

- ARM CCA Realm（或 vCCA）
- `rbc-cli` 已安装于 `/usr/bin/rbc-cli`
- `cryptsetup`、`make`、`gcc`、`openssl`、`jq`
- RBS 服务已部署并可从 Realm 内网络访问
- OpenClaw 二进制已安装

### 构建与安装

```bash
git clone https://gitcode.com/openeuler/sec-select.git
cd solution/openclaw-cca

# 将 RBS_BASE_URL 替换为实际 RBS 服务地址
sed -i 's|YOUR_RBS_URL_HERE|<your_rbs_url>|' scripts/openclaw-rbc-unlock.sh

make
sudo make install
```

安装后的文件：

| 路径 | 说明 |
|------|------|
| `/usr/local/bin/extend-rem3` | 将组件哈希扩展至 REM3 寄存器 |
| `/usr/local/sbin/openclaw-rbc-unlock.sh` | RBC 远程证明与加密卷解锁 |
| `/usr/local/sbin/openclaw-init.sh` | 首次初始化 |
| `/usr/local/sbin/openclaw-create-volume.sh` | 创建 LUKS2 加密卷 |
| `/etc/systemd/system/openclaw-luks-unlock.service` | 开机自动解锁服务 |

### 部署

完整的逐步部署指南（含策略生成、加密卷创建、systemd 配置）见 [docs/usage_guide.md](docs/usage_guide.md)。

总体流程：

1. **初始化**：`sudo openclaw-init.sh` — 将组件度量至 REM3，生成基线 JWT
2. **生成策略**：`python3 scripts/gen_policy.py /tmp/baseline_jwt.txt` — 产出 OPA/Rego 策略，上传至 RBS
3. **注册密钥**：生成随机口令，绑定策略上传至 RBS，记录返回的 `key_uri`
4. **创建加密卷**：`sudo openclaw-create-volume.sh <key_uri>` — 创建 LUKS2 加密卷
5. **配置 systemd**：在 unlock 服务中填入 `KEY_URI`/`DEVICE`/`MOUNT_POINT`
6. **写入 API Key**：编辑 `/opt/openclaw-data/openclaw.json`
7. **启用并重启**：`systemctl enable openclaw-luks-unlock.service && reboot`

## 工作原理

### REM3 度量

`extend-rem3`（由 `attest.c` 构建）通过 `/dev/attest` 的 ioctl 接口扩展 REM3 — CCA Realm 中的一个累加寄存器，只能扩展不可重置（重启后归零）。按顺序度量以下组件：

1. `/usr/local/bin/extend-rem3`
2. `/sbin/losetup`
3. `/usr/sbin/cryptsetup`
4. `/usr/bin/rbc-cli`
5. OpenClaw 二进制
6. 当前用户的 `/etc/shadow` 密码哈希（绑定 VM 实例身份）

### 证明流程

1. 生成临时 RSA-4096 密钥对
2. `rbc-cli challenge` 向 RBS 请求挑战随机数（nonce）
3. `rbc-cli collect-evidence` 采集含 REM/RIM 值的 TEE evidence，以 attester 私钥签名
4. `rbc-cli get-resource` 提交 evidence 至 RBS；RBS 以 OPA/Rego 策略评估 evidence
5. 策略匹配通过（所有 REM/RIM 值与基线一致），RBS 下发 LUKS 口令
6. `cryptsetup luksOpen` 使用口令解锁加密卷

### CCA 与 vCCA 对比

方案同时支持硬件 CCA 和虚拟 CCA（vCCA）：

| 方面 | CCA | vCCA |
|------|-----|------|
| 策略模板 | `cca.rego` | `vcca.rego` |
| JWT 字段路径 | `cca.realm_token` | `virt_cca.realm_token` |
| 度量键名 | `cca_rpv`、`cca_rim`、`cca_rem[0-3]` | `vcca_rpv`、`vcca_rim`、`vcca_rem[0-3]` |
| 内核模块 | 需加载 `arm_cca_guest` | 不需要 |

### 更新后重新初始化

度量清单中的任何组件（尤其是 OpenClaw 二进制）更新后，REM3 值将变化，RBS 策略失效，重启后 unlock 服务将失败。更新后必须重新走完整初始化流程，生成新的基线和策略。

## 故障恢复

若启动时 unlock 服务因 RBS 不可达而失败，等 RBS 恢复后直接重启服务即可，无需重启 Realm：

```bash
systemctl start openclaw-luks-unlock.service
```

脚本使用 tmpfs 标记文件（`/run/openclaw-rem3-extended`）防止重试时重复 extend REM3，确保度量值与 RBS 策略一致。标记文件存于 tmpfs，重启后自动消失。

## 许可证

[木兰宽松许可证，第 2 版 (MulanPSL-2.0)](http://license.coscl.org.cn/MulanPSL2)
