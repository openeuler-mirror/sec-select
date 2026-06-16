#!/bin/bash
set -euo pipefail

if [[ "$(id -u)" -ne 0 ]]; then
    echo "错误：请以 root 身份执行此脚本"
    exit 1
fi

echo "=== OpenClaw-CCA 首次初始化 ==="

OPENCLAW_BIN=$(which openclaw 2>/dev/null || true)
if [[ -n "$OPENCLAW_BIN" ]]; then
    echo "检测到 openclaw：$OPENCLAW_BIN"
else
    read -rp "未检测到 openclaw，请手动输入路径: " OPENCLAW_BIN
fi
[[ -x "$OPENCLAW_BIN" ]] || { echo "错误：路径不存在或不可执行"; exit 1; }

mkdir -p /etc/openclaw-cca
chmod 700 /etc/openclaw-cca
cat > /etc/openclaw-cca/config <<EOF
OPENCLAW_BIN=$OPENCLAW_BIN
EOF
chmod 600 /etc/openclaw-cca/config

/usr/local/sbin/openclaw-rbc-unlock.sh --init "$OPENCLAW_BIN"

echo ""
echo "=== 初始化完成 ==="
echo "基线文件：/tmp/baseline_jwt.txt"
echo ""
echo "请依次执行以下操作："
echo "  1. 根据 /tmp/baseline_jwt.txt，使用 gen_policy.py 脚本制作策略并上传至 RBS"
echo "  2. 自行生成 passphrase，上传至 RBS，记录返回的uri"
echo "  3. 在本次会话内（不要重启）执行："
echo "       openclaw-create-volume.sh <key_uri> [--size <GB>] [--mount <路径>] [--device <块设备>]"
echo ""
echo "警告：步骤 3 必须在不重启的情况下执行，否则 REM3 状态丢失需重新初始化。"
