---
name: openclaw-cca-attest
description: Use when verifying CCA Realm integrity, inspecting current REM/RIM measurement values, or diagnosing RBS policy mismatches in an OpenClaw deployment. Triggers include "查看度量值", "check REM3", "验证 Realm 完整性", "attestation failed", or before generating a new policy after component updates.
---

# openclaw-attest

## Overview

对 CCA Realm 执行完整远程证明流程，解析并展示当前 REM/RIM 度量值。可用于初始化前确认基线、组件更新后验证新值，以及排查 RBS 策略不匹配问题。

## When to Use

- 首次部署前，查看度量基线（之后配合 `gen_policy.py` 生成策略）
- 更新了度量清单中的组件（openclaw 二进制、rbc-cli 等），确认新的 REM/RIM 值
- RBS 校验失败（`get-resource` 返回 403），对比实际度量值与策略预期值
- 手动调试证明流程

**不适用于：** 解锁加密卷（用 `openclaw-rbc-unlock.sh --open`）、生成策略文件（用 `scripts/gen_policy.py`）

---

## Implementation

运行 `openclaw-attest/scripts/attest.sh`，或按以下步骤手动执行：

### 环境变量

```bash
export RBS_BASE_URL=https://your-rbs-host   # 必填
```

### 步骤一：生成临时密钥对

```bash
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:4096 \
    -out /tmp/oc-attester-priv.pem 2>/dev/null
openssl pkey -in /tmp/oc-attester-priv.pem \
    -pubout -out /tmp/oc-attester-pub.pem 2>/dev/null
```

### 步骤二：加载 CCA attest 模块

```bash
modprobe tsm
modprobe arm_cca_guest
mount -t configfs none /sys/kernel/config
```

### 步骤三：获取 nonce

```bash
rbc-cli -b ${RBS_BASE_URL} challenge -o /tmp/oc-nonce.txt
```

### 步骤四：采集证据

```bash
rbc-cli -b ${RBS_BASE_URL} \
    collect-evidence \
    --nonce @/tmp/oc-nonce.txt \
    --attester-pubkey @/tmp/oc-attester-pub.pem \
    -o /tmp/oc-evidence.json
```

### 步骤五：获取 token（attest）

```bash
rbc-cli -b ${RBS_BASE_URL} \
    get-token \
    --evidence @/tmp/oc-evidence.json \
    -o /tmp/baseline_jwt.txt
```

### 步骤六：解析并展示 REM/RIM 值

从 JWT payload 的 `cca.realm_token` 字段提取度量值（解析逻辑参考 `scripts/gen_policy.py`）：

```python
import sys, json, base64

def b64url_decode(s):
    s += "=" * (-len(s) % 4)
    return base64.urlsafe_b64decode(s)

with open("/tmp/baseline_jwt.txt") as f:
    token = f.read().strip()

payload = json.loads(b64url_decode(token.split(".")[1]))
realm = payload.get("cca", {}).get("realm_token", {})

FIELDS = [
    ("cca_rim",  "RIM  "),
    ("cca_rem0", "REM[0]"),
    ("cca_rem1", "REM[1]"),
    ("cca_rem2", "REM[2]"),
    ("cca_rem3", "REM[3]"),
    ("cca_rpv",  "RPV  "),
]
for key, label in FIELDS:
    print(f"  {label}: {realm.get(key, '(未找到)')}")
```

### 清理临时文件

```bash
rm -f /tmp/oc-attester-priv.pem /tmp/oc-attester-pub.pem \
      /tmp/oc-nonce.txt /tmp/oc-evidence.json
```

---

## Prerequisites

| 依赖 | 说明 |
|------|------|
| `RBS_BASE_URL` | RBS 服务地址（环境变量，必填） |
| `rbc-cli` | 已安装于 `/usr/bin/rbc-cli` |
| `openssl`, `python3` | 标准工具 |
| ARM CCA 驱动 | Realm 环境 |

## Common Mistakes

| 问题 | 原因 | 处理 |
|------|------|------|
| `rbc-cli challenge` 超时 | RBS 不可达 | 确认网络连通性 |
| `cca.realm_token 未找到` | token 结构不符 | 确认 RBS 支持 CCA Realm 证明 |
| REM[3] 与预期不符 | extend 次数异常 | 检查 `/run/openclaw-rem3-extended`，需重启后重试 |
