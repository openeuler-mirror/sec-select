#!/bin/bash
set -euo pipefail

MODE="${1:-}"

MODULE_NAME="attest"
ATTEST_KO_PATH="/root/${MODULE_NAME}.ko"
RBS_BASE_URL="YOUR_RBS_URL_HERE"  # 替换为实际 RBS URL

load_cca_attest_module() {
    modprobe tsm
    modprobe arm_cca_guest
    mount -t configfs none /sys/kernel/config
}

extend_rem3() {
    local OPENCLAW_BIN="$1"
    # REM3 是累加寄存器，每次启动只能 extend 一次。
    # 用 tmpfs 标记防止手动重试时重复 extend 导致 REM3 值与 RBS 策略不符。
    # /run 位于 tmpfs，重启后自动消失，下次启动会重新 extend。
    local REM3_FLAG="/run/openclaw-rem3-extended"
    if [[ -f "$REM3_FLAG" ]]; then
        echo "  REM3 已在本次启动中完成扩展，跳过重复操作。"
        return 0
    fi

    if lsmod | grep -q "^${MODULE_NAME}\b"; then
        echo "  ${MODULE_NAME} 内核模块已加载。"
    else
        echo "  正在加载 ${MODULE_NAME} 内核模块..."
        insmod "$ATTEST_KO_PATH"
    fi

    local FILE_TARGETS=(
        "/usr/local/bin/extend-rem3"
        "/sbin/losetup"
        "/usr/sbin/cryptsetup"
        "/usr/bin/rbc-cli"
        "$OPENCLAW_BIN"
    )

    echo "  正在将关键二进制文件的度量哈希扩展至 REM3 寄存器..."
    for f in "${FILE_TARGETS[@]}"; do
        local hex
        hex=$(sha256sum "$f" | awk '{print $1}')
        echo "    ↳ $(basename "$f")"
        extend-rem3 "$hex"
    done

    # 提取当前用户在 /etc/shadow 中的 hash 字段，绑定 VM 实例身份
    echo "  正在将实例身份哈希扩展至 REM3 寄存器..."
    local SHADOW_HASH
    SHADOW_HASH=$(awk -F: -v user="$(whoami)" '$1==user{print $2}' /etc/shadow)
    local hex
    hex=$(printf '%s' "$SHADOW_HASH" | sha256sum | awk '{print $1}')
    extend-rem3 "$hex"

    touch "$REM3_FLAG"
    echo "  ✅ REM3 扩展完成。"
}

gen_ephemeral_keypair() {
    openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:4096 \
        -out /tmp/oc-attester-priv.pem 2>/dev/null
    openssl pkey -in /tmp/oc-attester-priv.pem \
        -pubout -out /tmp/oc-attester-pub.pem 2>/dev/null
}

cleanup_ephemeral() {
    rm -f /tmp/oc-attester-priv.pem /tmp/oc-attester-pub.pem \
          /tmp/oc-nonce.txt /tmp/oc-evidence.json
}

get_passphrase() {
    local KEY_URI="$1"

    echo "  [1/4] 生成临时非对称密钥对（RSA-4096）..." >&2
    gen_ephemeral_keypair

    load_cca_attest_module
    echo "  [2/4] 向 RBS 请求挑战随机数（Challenge）..." >&2
    rbc-cli -b ${RBS_BASE_URL} \
        challenge -o /tmp/oc-nonce.txt

    echo "  [3/4] 收集 CCA 可信执行环境证明证据..." >&2
    rbc-cli -b ${RBS_BASE_URL} \
        collect-evidence \
        --nonce @/tmp/oc-nonce.txt \
        --attester-pubkey @/tmp/oc-attester-pub.pem \
        -o /tmp/oc-evidence.json

    echo "  [4/4] 向 RBS 提交证明证据，申请解密密钥..." >&2
    local PASSPHRASE
    PASSPHRASE=$(rbc-cli -b ${RBS_BASE_URL} \
        get-resource \
        --uri "$KEY_URI" \
        --evidence @/tmp/oc-evidence.json \
        --private-key-file /tmp/oc-attester-priv.pem \
        | jq -r '.content')
    cleanup_ephemeral
    if [[ -z "$PASSPHRASE" || "$PASSPHRASE" == "null" ]]; then
        echo "  ❌ RBS 拒绝请求或返回密钥为空，远程证明未通过。" >&2
        return 1
    fi
    echo "  ✅ RBS 远程证明验证通过，加密密钥已获取。" >&2
    printf '%s\n' "$PASSPHRASE"
}

case "$MODE" in

  --init)
    OPENCLAW_BIN="${2:?用法: $0 --init <openclaw_bin>}"
    # extend_rem3 "$OPENCLAW_BIN"

    gen_ephemeral_keypair

    load_cca_attest_module
    rbc-cli -b ${RBS_BASE_URL} \
        challenge -o /tmp/oc-nonce.txt

    rbc-cli -b ${RBS_BASE_URL} \
        collect-evidence \
        --nonce @/tmp/oc-nonce.txt \
        --attester-pubkey @/tmp/oc-attester-pub.pem \
        -o /tmp/oc-evidence.json

    rbc-cli -b ${RBS_BASE_URL} \
        get-token \
        --evidence @/tmp/oc-evidence.json \
        -o /tmp/baseline_jwt.txt

    cleanup_ephemeral
    echo "baseline has been generated at /tmp/baseline_jwt.txt, please create a policy and upload it to RBS"
    ;;

  --create)
    KEY_URI="${2:?用法: $0 --create <key_uri> <device> <mount_point>}"
    DEVICE="${3:?}"
    MOUNT_POINT="${4:?}"
    # REM3 已由调用方（openclaw-init.sh）extend 完毕，此处不重复 extend
    echo "正在通过 RBS（Resource Broker Service）远程证明服务获取加密密钥..."
    PASSPHRASE=$(get_passphrase "$KEY_URI")
    echo ""
    echo "正在格式化加密卷（LUKS2）..."
    printf '%s' "$PASSPHRASE" | cryptsetup luksFormat --type luks2 --batch-mode --key-file=- "$DEVICE"
    echo "正在开启加密卷..."
    printf '%s' "$PASSPHRASE" | cryptsetup luksOpen --key-file=- "$DEVICE" openclaw-data
    echo "正在创建 ext4 文件系统..."
    mkfs.ext4 /dev/mapper/openclaw-data
    mkdir -p "$MOUNT_POINT"
    echo "正在挂载加密卷至 $MOUNT_POINT..."
    mount /dev/mapper/openclaw-data "$MOUNT_POINT"
    ;;

  --open)
    KEY_URI="${2:?用法: $0 --open <key_uri> <device> <mount_point>}"
    DEVICE="${3:?}"
    MOUNT_POINT="${4:?}"
    # 重启后 REM3 归零，必须重新 extend
    source /etc/openclaw-cca/config
    # loop 设备重启后自动解绑，需在 luksOpen 前重新绑定
    if [[ "$DEVICE" == /dev/loop* ]] && [[ -n "${VFS_FILE:-}" ]]; then
        losetup "$DEVICE" &>/dev/null || losetup "$DEVICE" "$VFS_FILE"
    fi
    echo "正在扩展 REM3 可信度量寄存器..."
    # extend_rem3 "$OPENCLAW_BIN"
    echo ""
    echo "正在通过 RBS（Resource Broker Service）远程证明服务获取加密密钥..."
    PASSPHRASE=$(get_passphrase "$KEY_URI")
    echo ""
    echo "正在开启加密卷..."
    printf '%s' "$PASSPHRASE" | cryptsetup luksOpen --key-file=- "$DEVICE" openclaw-data
    mkdir -p "$MOUNT_POINT"
    echo "正在挂载加密卷至 $MOUNT_POINT..."
    mount /dev/mapper/openclaw-data "$MOUNT_POINT"
    echo "✅ 加密卷已开启并挂载至 $MOUNT_POINT"
    ;;

  *)
    echo "用法: $0 {--init <openclaw_bin>|--create <key_uri> <device> <mount_point>|--open <key_uri> <device> <mount_point>}"
    exit 1
    ;;
esac
