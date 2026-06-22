#!/bin/bash
# 对 CCA Realm 执行远程证明，输出 REM/RIM 度量值。
# 环境变量：RBS_BASE_URL（必填）
set -euo pipefail

RBS_BASE_URL="${RBS_BASE_URL:-}"
if [[ -z "$RBS_BASE_URL" ]]; then
    echo "错误：未设置 RBS_BASE_URL" >&2
    exit 1
fi

WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

# ── 1. 生成临时密钥对 ────────────────────────────────────────────────────────
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:4096 \
    -out "$WORK_DIR/priv.pem" 2>/dev/null
openssl pkey -in "$WORK_DIR/priv.pem" \
    -pubout -out "$WORK_DIR/pub.pem" 2>/dev/null

# ── 2. 加载 CCA attest 模块 ──────────────────────────────────────────────────
modprobe tsm            2>/dev/null || true
modprobe arm_cca_guest  2>/dev/null || true
mount -t configfs none /sys/kernel/config 2>/dev/null || true

# ── 3. 获取 nonce ────────────────────────────────────────────────────────────
echo "正在获取 nonce..."
rbc-cli -b "$RBS_BASE_URL" challenge -o "$WORK_DIR/nonce.txt"

# ── 4. 采集证据 ──────────────────────────────────────────────────────────────
echo "正在采集 TEE evidence..."
rbc-cli -b "$RBS_BASE_URL" \
    collect-evidence \
    --nonce          @"$WORK_DIR/nonce.txt" \
    --attester-pubkey @"$WORK_DIR/pub.pem" \
    -o "$WORK_DIR/evidence.json"

# ── 5. 获取 token（attest）──────────────────────────────────────────────────
echo "正在请求 token..."
rbc-cli -b "$RBS_BASE_URL" \
    get-token \
    --evidence @"$WORK_DIR/evidence.json" \
    -o "$WORK_DIR/token.txt"

# ── 6. 解析并展示 REM/RIM 值 ─────────────────────────────────────────────────
python3 - "$WORK_DIR/token.txt" <<'PYEOF'
import sys, json, base64

def b64url_decode(s):
    s += "=" * (-len(s) % 4)
    return base64.urlsafe_b64decode(s)

with open(sys.argv[1]) as f:
    token = f.read().strip()

try:
    payload = json.loads(b64url_decode(token.split(".")[1]))
except Exception as e:
    print(f"错误：JWT 解析失败：{e}", file=sys.stderr)
    sys.exit(1)

realm = payload.get("cca", {}).get("realm_token")
if not realm:
    print("错误：JWT payload 中未找到 cca.realm_token 字段", file=sys.stderr)
    sys.exit(1)

FIELDS = [
    ("cca_rim",  "RIM  "),
    ("cca_rem0", "REM[0]"),
    ("cca_rem1", "REM[1]"),
    ("cca_rem2", "REM[2]"),
    ("cca_rem3", "REM[3]"),
    ("cca_rpv",  "RPV  "),
]

print("\n=== CCA Realm 度量值 ===")
for key, label in FIELDS:
    value = realm.get(key, "(未找到)")
    print(f"  {label}: {value}")
print()
PYEOF
