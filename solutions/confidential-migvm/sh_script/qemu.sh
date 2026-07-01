#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PAYLOAD_DIR="$SCRIPT_DIR/.."
BUILD_DIR="$PAYLOAD_DIR/target/aarch64-unknown-none/release"
ELF="$BUILD_DIR/migvm"
KERNEL="$BUILD_DIR/migvm.bin"

export RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static
export RUSTUP_UPDATE_ROOT=https://mirrors.ustc.edu.cn/rust-static/rustup
export PATH="$HOME/.cargo/bin:$PATH"

TIMEOUT_SEC="${QEMU_TIMEOUT:-60}"

# Default QEMU binary. /bin/qemu-kvm is suitable on aarch64 hosts (it links to
# qemu-system-aarch64 and uses KVM). On x86 hosts it only runs x86_64 guests,
# so to run this aarch64 payload under TCG emulation set:
#   QEMU_BIN=/usr/bin/qemu-system-aarch64
QEMU_BIN="${QEMU_BIN:-/bin/qemu-kvm}"
QEMU_FEATURES="${QEMU_FEATURES:-}"

VIRTIO_DEVICES="${VIRTIO_DEVICES:--netdev user,id=net0 -device virtio-net-pci,netdev=net0,disable-legacy=on}"

detect_kvm() {
    if [ -w /dev/kvm ] 2>/dev/null; then
        return 0
    fi
    if [ -d /sys/devices/virtual/misc/kvm ] 2>/dev/null; then
        local major minor
        major=$(cat /sys/devices/virtual/misc/kvm/dev 2>/dev/null | cut -d: -f1)
        minor=$(cat /sys/devices/virtual/misc/kvm/dev 2>/dev/null | cut -d: -f2)
        if [ -n "$major" ] && [ -n "$minor" ]; then
            mknod /dev/kvm c "$major" "$minor" 2>/dev/null \
                && chmod 666 /dev/kvm 2>/dev/null \
                && return 0 || true
        fi
    fi
    return 1
}

build() {
    local features_opt=""
    if [ -n "$QEMU_FEATURES" ]; then
        features_opt="--features $QEMU_FEATURES"
        echo "=== Building MigVM AArch64 (features: $QEMU_FEATURES) ==="
    else
        echo "=== Building MigVM AArch64 ==="
    fi
    (cd "$PAYLOAD_DIR" && cargo build --release --features default $features_opt --target aarch64-unknown-none)
    if [ ! -f "$ELF" ]; then
        echo "ERROR: Build failed - ELF not found"
        exit 1
    fi
    objcopy -O binary "$ELF" "$KERNEL"
    echo "[OK] Build successful: $(stat -c%s "$KERNEL") bytes (raw binary)"
}

run_test() {
    local test_name="$1"
    local machine_opt="$2"
    local accel_opt="$3"
    local log_file="$4"

    echo "--- $test_name ---"
    rm -f "$log_file"

    timeout $TIMEOUT_SEC $QEMU_BIN \
        $machine_opt \
        $accel_opt \
        -m 1024 \
        $VIRTIO_DEVICES \
        -monitor none \
        -display none \
        -serial file:"$log_file" \
        -kernel "$KERNEL" 2>/dev/null || true

    if [ -f "$log_file" ] && [ -s "$log_file" ]; then
        local boot_ok=no
        local gic_ver=""
        if grep -q "MigVM AArch64 Boot" "$log_file"; then
            if grep -q "Boot Complete" "$log_file"; then
                boot_ok=yes
            fi
        fi
        gic_ver=$(grep "Detected version:" "$log_file" | head -1 || true)

        if [ "$boot_ok" = "yes" ]; then
            echo "[PASS] Boot successful  $gic_ver"
        else
            echo "[FAIL] Boot incomplete or crashed"
            echo "  Last lines:"
            tail -5 "$log_file" | sed 's/^/    /'
        fi
    else
        echo "[FAIL] No serial output"
    fi
    echo ""
}

cmd_verify() {
    echo "========================================="
    echo "  MigVM AArch64 Verification"
    echo "========================================="
    echo ""

    if ! command -v $QEMU_BIN &>/dev/null; then
        echo "ERROR: $QEMU_BIN not found"
        exit 1
    fi
    echo "[OK] QEMU: $($QEMU_BIN --version | head -1)"

    local kvm_available=no
    if detect_kvm; then
        kvm_available=yes
        echo "[OK] /dev/kvm accessible"
    else
        echo "[WARN] /dev/kvm not accessible, KVM tests will be skipped"
    fi

    echo ""
    echo "[INFO] Architecture: $(uname -m)"
    if [ -f /proc/cpuinfo ]; then
        local cpu_part cpu_impl
        cpu_part=$(grep "CPU part" /proc/cpuinfo | head -1 | awk '{print $NF}')
        cpu_impl=$(grep "CPU implementer" /proc/cpuinfo | head -1 | awk '{print $NF}')
        if [ -n "$cpu_part" ]; then
            echo "[INFO] CPU implementer: $cpu_impl, part: $cpu_part"
        fi
    fi

    if [ -n "$VIRTIO_DEVICES" ]; then
        echo "[INFO] VirtIO devices: $VIRTIO_DEVICES"
    fi

    echo ""
    build
    echo ""

    if [ "$kvm_available" = "yes" ]; then
        echo "========================================="
        echo "  KVM + GICv3"
        echo "========================================="
        echo ""
        run_test "KVM + GICv3" \
            "-machine virt,gic-version=3 -cpu host" \
            "-accel kvm" \
            "/tmp/migvm_kvm_gicv3.log"
    else
        echo "[SKIP] KVM tests (no /dev/kvm access)"
        echo ""
    fi

    echo "========================================="
    echo "  TCG + GICv3"
    echo "========================================="
    echo ""
    run_test "TCG + GICv3" \
        "-machine virt,gic-version=3 -cpu cortex-a57" \
        "-accel tcg" \
        "/tmp/migvm_tcg_gicv3.log"

    echo "========================================="
    echo "  Summary"
    echo "========================================="
    echo ""
    echo "Log files:"
    for f in /tmp/migvm_kvm_gicv3.log /tmp/migvm_tcg_gicv3.log; do
        if [ -f "$f" ]; then
            echo "  $f"
        fi
    done
    echo ""
}

cmd_run() {
    local use_kvm="${QEMU_KVM:-auto}"
    local gic_version="${QEMU_GIC:-3}"

    local cpu_opt accel_opt
    if [ "$use_kvm" = "auto" ]; then
        if detect_kvm; then
            use_kvm=yes
        else
            use_kvm=no
        fi
    fi

    if [ "$use_kvm" = "yes" ]; then
        if ! detect_kvm; then
            echo "ERROR: KVM requested but /dev/kvm not accessible"
            exit 1
        fi
        cpu_opt="-cpu host"
        accel_opt="-accel kvm"
        echo "[INFO] KVM mode: -cpu host -accel kvm"
    else
        cpu_opt="-cpu cortex-a57"
        accel_opt="-accel tcg"
        echo "[INFO] TCG mode: -cpu cortex-a57 -accel tcg"
    fi

    if [ "$gic_version" = "auto" ]; then
        gic_version=3
    fi

    local machine_opt
    if [ "$gic_version" = "3" ]; then
        machine_opt="-machine virt,gic-version=3"
        echo "[INFO] GICv3"
    else
        machine_opt="-machine virt"
        echo "[INFO] GICv2"
    fi

    local log_file="/tmp/migvm_${use_kvm}_gicv${gic_version}.log"
    local qemu_opts="$machine_opt $cpu_opt -m 1024 $accel_opt"

    if [ -n "$VIRTIO_DEVICES" ]; then
        qemu_opts="$qemu_opts $VIRTIO_DEVICES"
        echo "[INFO] VirtIO devices: $VIRTIO_DEVICES"
    fi

    build

    echo ""
    echo "=== Launching QEMU AArch64 ==="
    echo "QEMU opts: $qemu_opts"
    echo "Serial output -> $log_file"
    echo ""

    rm -f "$log_file"

    timeout $TIMEOUT_SEC $QEMU_BIN \
        $qemu_opts \
        -monitor none \
        -display none \
        -serial file:"$log_file" \
        -kernel "$KERNEL" 2>/dev/null || true

    if [ -f "$log_file" ]; then
        echo "=== Serial Output ==="
        cat "$log_file"
        echo ""
        echo "=== End of Output ==="
    else
        echo "WARNING: No serial output captured"
    fi
}

cmd_interactive() {
    local use_kvm="${QEMU_KVM:-auto}"
    local gic_version="${QEMU_GIC:-3}"

    if [ "$use_kvm" = "auto" ]; then
        if detect_kvm; then use_kvm=yes; else use_kvm=no; fi
    fi

    local cpu_opt accel_opt
    if [ "$use_kvm" = "yes" ]; then
        cpu_opt="-cpu host"
        accel_opt="-accel kvm"
    else
        cpu_opt="-cpu cortex-a57"
        accel_opt="-accel tcg"
    fi

    if [ "$gic_version" = "auto" ]; then
        gic_version=3
    fi

    local machine_opt
    if [ "$gic_version" = "3" ]; then
        machine_opt="-machine virt,gic-version=3"
    else
        machine_opt="-machine virt"
    fi

    local qemu_opts="$machine_opt $cpu_opt -m 1024 $accel_opt"

    if [ -n "$VIRTIO_DEVICES" ]; then
        qemu_opts="$qemu_opts $VIRTIO_DEVICES"
    fi

    build

    echo ""
    echo "=== Launching QEMU AArch64 (interactive) ==="
    echo "QEMU opts: $qemu_opts"
    echo "Press Ctrl+A X to exit QEMU"
    echo ""

    $QEMU_BIN \
        $qemu_opts \
        -monitor none \
        -display none \
        -serial stdio \
        -kernel "$KERNEL"
}

cmd_debug() {
    local use_kvm="${QEMU_KVM:-auto}"
    local gic_version="${QEMU_GIC:-3}"

    if [ "$use_kvm" = "auto" ]; then
        if detect_kvm; then use_kvm=yes; else use_kvm=no; fi
    fi

    local cpu_opt accel_opt
    if [ "$use_kvm" = "yes" ]; then
        cpu_opt="-cpu host"
        accel_opt="-accel kvm"
    else
        cpu_opt="-cpu cortex-a57"
        accel_opt="-accel tcg"
    fi

    if [ "$gic_version" = "auto" ]; then
        gic_version=3
    fi

    local machine_opt
    if [ "$gic_version" = "3" ]; then
        machine_opt="-machine virt,gic-version=3"
    else
        machine_opt="-machine virt"
    fi

    local qemu_opts="$machine_opt $cpu_opt -m 1024 $accel_opt"

    if [ -n "$VIRTIO_DEVICES" ]; then
        qemu_opts="$qemu_opts $VIRTIO_DEVICES"
    fi

    build

    echo ""
    echo "=== Launching QEMU AArch64 with GDB stub (port 1234) ==="
    echo "QEMU opts: $qemu_opts"
    echo "Connect with: aarch64-linux-gnu-gdb $ELF"
    echo "Then: target remote :1234"
    echo ""

    $QEMU_BIN \
        $qemu_opts \
        -monitor none \
        -display none \
        -serial stdio \
        -kernel "$KERNEL" \
        -S \
        -gdb tcp::1234
}

cmd_clean() {
    echo "=== Cleaning build ==="
    (cd "$PAYLOAD_DIR" && cargo clean)
}

cmd_vsock() {
    local use_kvm="${QEMU_KVM:-auto}"
    local gic_version="${QEMU_GIC:-3}"
    local guest_cid="${QEMU_VSOCK_CID:-3}"

    if [ "$use_kvm" = "auto" ]; then
        if detect_kvm; then use_kvm=yes; else use_kvm=no; fi
    fi

    local cpu_opt accel_opt
    if [ "$use_kvm" = "yes" ]; then
        cpu_opt="-cpu host"
        accel_opt="-accel kvm"
        echo "[INFO] KVM mode"
    else
        cpu_opt="-cpu cortex-a57"
        accel_opt="-accel tcg"
        echo "[INFO] TCG mode"
    fi

    if [ "$gic_version" = "auto" ]; then
        gic_version=3
    fi

    local machine_opt
    if [ "$gic_version" = "3" ]; then
        machine_opt="-machine virt,gic-version=3"
    else
        machine_opt="-machine virt"
    fi

    local vsock_devs="-netdev user,id=net0"
    vsock_devs="$vsock_devs -device virtio-net-pci,netdev=net0,disable-legacy=on"
    vsock_devs="$vsock_devs -device vhost-vsock-pci,guest-cid=$guest_cid"

    local log_file="/tmp/migvm_${use_kvm}_gicv${gic_version}_vsock.log"

    build

    echo ""
    echo "=== Launching QEMU AArch64 (net + vsock, guest-cid=$guest_cid) ==="
    echo "Serial output -> $log_file"
    echo "Connect from host: socat - VSOCK-CONNECT:$guest_cid:4052"
    echo ""

    rm -f "$log_file"

    timeout $TIMEOUT_SEC $QEMU_BIN \
        $machine_opt \
        $cpu_opt \
        -m 1024 \
        $accel_opt \
        $vsock_devs \
        -monitor none \
        -display none \
        -serial file:"$log_file" \
        -kernel "$KERNEL" 2>/dev/null || true

    if [ -f "$log_file" ]; then
        echo "=== Serial Output ==="
        cat "$log_file"
        echo ""
        echo "=== End of Output ==="
    else
        echo "WARNING: No serial output captured"
    fi
}

cmd_virtcca() {
    local guest_cid="${QEMU_VSOCK_CID:-3}"

    export QEMU_FEATURES="virtcca"

    echo "[INFO] VirtCCA mode: -M virt,gic-version=host,accel=kvm,kvm-type=cvm -cpu host"
    echo "[INFO] TMM guest object: -object tmm-guest,id=tmm0"
    echo "[INFO] Features: $QEMU_FEATURES"

    local machine_opt="-M virt,usb=off,gic-version=host,accel=kvm,kvm-type=cvm -cpu host"
    local tmm_opt="-object tmm-guest,id=tmm0"

    local vsock_devs="-netdev user,id=net0"
    vsock_devs="$vsock_devs -device virtio-net-pci,netdev=net0,disable-legacy=on"
    vsock_devs="$vsock_devs -device vhost-vsock-pci,guest-cid=$guest_cid"

    local log_file="/tmp/migvm_virtcca_vsock.log"

    build

    echo ""
    echo "=== Launching QEMU AArch64 VirtCCA (net + vsock, guest-cid=$guest_cid) ==="
    echo "Serial output -> $log_file"
    echo "Connect from host: socat - VSOCK-CONNECT:$guest_cid:4052"
    echo ""

    rm -f "$log_file"

    timeout $TIMEOUT_SEC $QEMU_BIN \
        $machine_opt \
        $tmm_opt \
        -m 1024 \
        $vsock_devs \
        -monitor none \
        -display none \
        -serial file:"$log_file" \
        -kernel "$KERNEL" 2>/dev/null || true

    if [ -f "$log_file" ]; then
        echo "=== Serial Output ==="
        cat "$log_file"
        echo ""
        echo "=== End of Output ==="
    else
        echo "WARNING: No serial output captured"
    fi
}

case "${1:-run}" in
    verify)
        cmd_verify "${2:-both}"
        ;;
    build)
        build
        ;;
    run)
        cmd_run
        ;;
    interactive)
        cmd_interactive
        ;;
    debug)
        cmd_debug
        ;;
    clean)
        cmd_clean
        ;;
    vsock)
        cmd_vsock
        ;;
    virtcca)
        cmd_virtcca
        ;;
    *)
        echo "Usage: $0 {verify|build|run|interactive|debug|vsock|virtcca|clean}"
        echo ""
        echo "  verify [2|3|both]  - Run all mode combinations, output logs to /tmp"
        echo "  build              - Compile the aarch64 binary"
        echo "  run                - Build and run with net, capture serial to /tmp log"
        echo "  interactive        - Build and run with interactive console"
        echo "  debug              - Build and run with GDB stub on port 1234"
        echo "  vsock              - Build and run with net + vhost-vsock-pci"
        echo "  virtcca            - Build and run with net + vsock in VirtCCA guest mode"
        echo "                       (-M virt,kvm-type=cvm -object tmm-guest)"
        echo "  clean              - Clean build artifacts"
        echo ""
        echo "Environment variables:"
        echo "  QEMU_KVM=auto|yes|no       KVM acceleration (default: auto)"
        echo "  QEMU_GIC=auto|2|3          GIC version (default: auto -> 3)"
        echo "  QEMU_TIMEOUT=N             QEMU run timeout in seconds (default: 5)"
        echo "  QEMU_VSOCK_CID=N           Guest CID for vsock/virtcca mode (default: 3)"
        echo "  VIRTIO_DEVICES=\"...\"        Extra QEMU device args, e.g.:"
        echo "    VIRTIO_DEVICES=\"-device virtio-net-device -device vhost-vsock-device,guest-cid=3\""
        ;;
esac
