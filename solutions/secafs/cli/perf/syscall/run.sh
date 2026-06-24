#!/bin/bash
#
# Benchmark syscall performance across different scenarios:
#   1. Native filesystem
#   2. SecAFS (file in base layer)
#   3. SecAFS (file copied up to delta layer)
#

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CLI_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
TEST_FILE="$SCRIPT_DIR/hello.txt"
ITERATIONS="${1:-100000}"
SECAFS="$CLI_DIR/target/release/secafs"

# Build benchmarks if needed
make -C "$SCRIPT_DIR" -s

# Check secafs binary
if [ ! -x "$SECAFS" ]; then
    echo "Error: secafs binary not found at $SECAFS"
    echo "Run: cargo build --release"
    exit 1
fi

# Extract metrics from benchmark output
extract_latency() {
    grep "Avg latency:" | awk '{print $3}'
}

extract_throughput() {
    grep "Throughput:" | awk '{print $2}'
}

# Run a benchmark for all three scenarios
run_benchmark() {
    local name="$1"
    local benchmark="$2"

    echo "=============================================="
    echo "$name"
    echo "=============================================="
    echo "Iterations: $ITERATIONS"
    echo ""

    # Test 1: Native filesystem
    echo "[1/3] Native filesystem..."
    NATIVE_OUTPUT=$("$benchmark" "$TEST_FILE" "$ITERATIONS" 2>&1)
    NATIVE_LATENCY=$(echo "$NATIVE_OUTPUT" | extract_latency)
    NATIVE_THROUGHPUT=$(echo "$NATIVE_OUTPUT" | extract_throughput)

    # Test 2: SecAFS (file in base layer)
    echo "[2/3] SecAFS (base layer)..."
    SECAFS_BASE_OUTPUT=$("$SECAFS" run "$benchmark" "$TEST_FILE" "$ITERATIONS" 2>&1)
    SECAFS_BASE_LATENCY=$(echo "$SECAFS_BASE_OUTPUT" | extract_latency)
    SECAFS_BASE_THROUGHPUT=$(echo "$SECAFS_BASE_OUTPUT" | extract_throughput)

    # Test 3: SecAFS (file copied up to delta)
    echo "[3/3] SecAFS (delta layer)..."
    SECAFS_DELTA_OUTPUT=$("$SECAFS" run sh -c "
        touch '$TEST_FILE'
        '$benchmark' '$TEST_FILE' '$ITERATIONS'
    " 2>&1)
    SECAFS_DELTA_LATENCY=$(echo "$SECAFS_DELTA_OUTPUT" | extract_latency)
    SECAFS_DELTA_THROUGHPUT=$(echo "$SECAFS_DELTA_OUTPUT" | extract_throughput)

    # Calculate overhead percentages
    SECAFS_BASE_OVERHEAD=$(echo "scale=1; (($SECAFS_BASE_LATENCY / $NATIVE_LATENCY) - 1) * 100" | bc)
    SECAFS_DELTA_OVERHEAD=$(echo "scale=1; (($SECAFS_DELTA_LATENCY / $NATIVE_LATENCY) - 1) * 100" | bc)

    # Results
    echo ""
    echo "Results:"
    echo "----------------------------------------------"
    printf "%-25s %10s %12s %10s\n" "Scenario" "Latency" "Throughput" "Overhead"
    printf "%-25s %10s %12s %10s\n" "--------" "-------" "----------" "--------"
    printf "%-25s %8s ns %10s/s %10s\n" "Native" "$NATIVE_LATENCY" "$NATIVE_THROUGHPUT" "-"
    printf "%-25s %8s ns %10s/s %8s %%\n" "SecAFS (base)" "$SECAFS_BASE_LATENCY" "$SECAFS_BASE_THROUGHPUT" "$SECAFS_BASE_OVERHEAD"
    printf "%-25s %8s ns %10s/s %8s %%\n" "SecAFS (delta)" "$SECAFS_DELTA_LATENCY" "$SECAFS_DELTA_THROUGHPUT" "$SECAFS_DELTA_OVERHEAD"
    echo "----------------------------------------------"
    echo ""
}

# Run all benchmarks
run_benchmark "open()+close() Micro-Benchmark" "$SCRIPT_DIR/perf-open-close"
run_benchmark "statx() Micro-Benchmark" "$SCRIPT_DIR/perf-statx"
