#!/bin/bash
set -euo pipefail

# 默认值
VFS_FILE="/root/vfs"
VFS_SIZE_GB=1
MOUNT_POINT="/opt/openclaw-data"
LOOP_DEVICE=""
REAL_DEVICE=""

usage() {
    echo "用法（推荐，无需空闲磁盘，自动创建虚拟磁盘文件）："
    echo "  $0 <key_uri>"
    echo "  $0 <key_uri> --mount <挂载路径>      （默认: $MOUNT_POINT）"
    echo "  $0 <key_uri> --size <GB>             （默认: ${VFS_SIZE_GB}GB）"
    echo ""
    echo "用法（已有空闲块设备，如 /dev/vdb）："
    echo "  $0 <key_uri> --device <设备路径> [--mount <挂载路径>]"
}

if [[ "$(id -u)" -ne 0 ]]; then
    echo "错误：请以 root 身份执行此脚本"
    exit 1
fi

KEY_URI="${1:-}"
if [[ -z "$KEY_URI" ]]; then
    usage
    exit 1
fi
shift

while [[ $# -gt 0 ]]; do
    case "$1" in
        --device|-d) REAL_DEVICE="${2:?--device 需要指定路径}"; shift 2 ;;
        --mount|-m)  MOUNT_POINT="${2:?--mount 需要指定路径}"; shift 2 ;;
        --size|-s)   VFS_SIZE_GB="${2:?--size 需要指定大小（GB）}"; shift 2 ;;
        --help|-h)   usage; exit 0 ;;
        *) echo "错误：未知参数 $1"; usage; exit 1 ;;
    esac
done

source /etc/openclaw-cca/config

# ── 模式 A：使用已有块设备 ─────────────────────────────────────────────────
if [[ -n "$REAL_DEVICE" ]]; then
    [[ -b "$REAL_DEVICE" ]] || { echo "错误：$REAL_DEVICE 不是块设备"; exit 1; }
    echo "=== 使用块设备 $REAL_DEVICE 创建加密卷 ==="
    /usr/local/sbin/openclaw-rbc-unlock.sh --create "$KEY_URI" "$REAL_DEVICE" "$MOUNT_POINT"
    echo ""
    echo "=== 加密卷创建完成，已挂载于 $MOUNT_POINT ==="
    echo ""
    echo "后续步骤："
    echo "  1. 编辑 /etc/systemd/system/openclaw-luks-unlock.service，填入："
    echo "       Environment=\"KEY_URI=$KEY_URI\""
    echo "       Environment=\"DEVICE=$REAL_DEVICE\""
    echo "       Environment=\"MOUNT_POINT=$MOUNT_POINT\""
    echo "  2. 执行："
    echo "       systemctl daemon-reload"
    echo "       systemctl enable openclaw-luks-unlock.service"
    exit 0
fi

# ── 模式 B：自动创建虚拟磁盘文件（默认）──────────────────────────────────
echo "=== OpenClaw 加密存储创建向导 ==="
echo ""
echo "将创建一个 ${VFS_SIZE_GB}GB 的加密虚拟磁盘文件。"
echo "  虚拟磁盘文件：$VFS_FILE"
echo "  挂载路径：    $MOUNT_POINT"
echo ""

# 检查是否已存在
if [[ -f "$VFS_FILE" ]]; then
    echo "错误：$VFS_FILE 已存在。"
    echo "  若要重新创建，请先备份并删除该文件，或使用 --size 指定不同路径。"
    exit 1
fi


# 检查磁盘空间
AVAILABLE_KB=$(df --output=avail / | tail -1)
REQUIRED_KB=$(( VFS_SIZE_GB * 1024 * 1024 ))
if [[ "$AVAILABLE_KB" -lt "$REQUIRED_KB" ]]; then
    echo "错误：磁盘空间不足。需要 ${VFS_SIZE_GB}GB，当前可用 $(( AVAILABLE_KB / 1024 / 1024 ))GB。"
    exit 1
fi

# 失败时自动清理
_cleanup() {
    echo ""
    echo "创建失败，正在清理..."
    losetup --detach "$LOOP_DEVICE" 2>/dev/null || true
    [[ -f "$VFS_FILE" ]] && rm -f "$VFS_FILE" && echo "  已删除 $VFS_FILE"
}
trap _cleanup ERR

# 1. 创建虚拟磁盘文件
echo "正在分配 ${VFS_SIZE_GB}GB 磁盘空间（稍候）..."
dd if=/dev/zero of="$VFS_FILE" bs=1M count=$(( VFS_SIZE_GB * 1024 )) status=progress 2>&1
echo "  完成：$VFS_FILE"

# 2. 绑定 loop 设备
echo "绑定虚拟磁盘到 loop 设备..."
LOOP_DEVICE=$(losetup --find --show "$VFS_FILE")
echo "  完成：$LOOP_DEVICE → $VFS_FILE"

# 3. 通过 RBS 远程证明获取密钥，格式化并挂载加密卷
echo ""
echo "--- 远程证明与加密卷初始化 ---"
echo "本步骤将由 RBS（Resource Broker Service，资源代理服务）验证本机可信执行环境（CCA TEE），"
echo "验证通过后 RBS 才会释放加密密钥，整个过程约需 10~30 秒。"
echo ""
/usr/local/sbin/openclaw-rbc-unlock.sh --create "$KEY_URI" "$LOOP_DEVICE" "$MOUNT_POINT"

# 4. 将 VFS 路径持久化到 config，供重启后 --open 自动重新绑定 loop 设备
grep -q '^VFS_FILE=' /etc/openclaw-cca/config 2>/dev/null \
    && sed -i "s|^VFS_FILE=.*|VFS_FILE=$VFS_FILE|" /etc/openclaw-cca/config \
    || echo "VFS_FILE=$VFS_FILE" >> /etc/openclaw-cca/config

losetup --detach "$LOOP_DEVICE"

trap - ERR
echo ""
echo "=== 加密卷创建完成，已挂载于 $MOUNT_POINT ==="
echo ""
echo "后续步骤："
echo "  1. 执行以下命令配置 systemd 服务："
echo ""
echo "       sudo systemctl edit --full openclaw-luks-unlock.service"
echo ""
echo "     将 REPLACE_ME 替换为以下实际值："
echo "       Environment=\"KEY_URI=$KEY_URI\""
echo "       Environment=\"DEVICE=$LOOP_DEVICE\""
echo "       Environment=\"MOUNT_POINT=$MOUNT_POINT\""
echo ""
echo "  2. 配置 OpenClaw:"
echo ""
echo "       sudo nano $MOUNT_POINT/.openclaw/openclaw.json"
echo ""
echo "  3. 启用并重启验证："
echo ""
echo "       sudo systemctl daemon-reload"
echo "       sudo systemctl enable openclaw-luks-unlock.service"
echo "       sudo reboot"
echo ""
echo "  注意：$VFS_FILE 是加密存储的数据文件，请勿删除。"
