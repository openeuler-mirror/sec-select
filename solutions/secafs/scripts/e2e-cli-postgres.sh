#!/usr/bin/env bash
# SecAFS CLI 端到端验证：使用 PostgreSQL 运行 init、fs ls、mount 等命令
# 用法: ./scripts/e2e-cli-postgres.sh [POSTGRES_URL]
# 示例: ./scripts/e2e-cli-postgres.sh postgres://secafs:secafs@localhost:5432/test
#
# 要求：在可访问 PostgreSQL 的环境下运行（本机或已端口转发的远程库）。
# SDK 默认带 connect_timeout=10，连接失败会在约 10 秒内报错。

set -e
POSTGRES_URL="${1:-postgres://secafs:secafs@localhost:5432/test}"
CLI="${CLI:-./target/debug/secafs}"
MOUNT_POINT="${MOUNT_POINT:-/tmp/secafs_e2e_mnt}"

if ! [[ "$POSTGRES_URL" =~ ^postgres(ql)?:// ]]; then
  echo "Usage: $0 [POSTGRES_URL]"
  echo "Example: $0 postgres://secafs:secafs@localhost:5432/test"
  exit 1
fi

cd "$(dirname "$0")/.."
CLI_DIR="$(pwd)/cli"
if [[ ! -x "$CLI_DIR/target/debug/secafs" && ! -x "$CLI_DIR/target/release/secafs" ]]; then
  echo "Building CLI (no-default-features)..."
  (cd "$CLI_DIR" && cargo build --no-default-features)
fi
if [[ -x "$CLI_DIR/target/release/secafs" ]]; then
  CLI="$CLI_DIR/target/release/secafs"
else
  CLI="$CLI_DIR/target/debug/secafs"
fi

echo "=== SecAFS CLI E2E (PostgreSQL) ==="
echo "POSTGRES_URL=$POSTGRES_URL"
echo "CLI=$CLI"
echo ""

echo "--- 1. secafs --help ---"
"$CLI" --help
echo ""

echo "--- 2. secafs init ---"
"$CLI" init "$POSTGRES_URL"
echo ""

echo "--- 3. secafs fs ls / ---"
"$CLI" fs "$POSTGRES_URL" ls /
echo ""

echo "--- 4. secafs fs write + cat ---"
"$CLI" fs "$POSTGRES_URL" write /e2e_hello.txt "Hello from CLI E2E"
"$CLI" fs "$POSTGRES_URL" cat /e2e_hello.txt
echo ""

echo "--- 5. secafs fs ls / (after write) ---"
"$CLI" fs "$POSTGRES_URL" ls /
echo ""

if [[ "$(uname)" == "Linux" ]] && command -v fusermount3 &>/dev/null; then
  echo "--- 6. secafs mount (3s 后卸载) ---"
  mkdir -p "$MOUNT_POINT"
  "$CLI" mount "$POSTGRES_URL" "$MOUNT_POINT" -f &
  MPID=$!
  sleep 3
  fusermount3 -u "$MOUNT_POINT" 2>/dev/null || fusermount -u "$MOUNT_POINT" 2>/dev/null || true
  kill $MPID 2>/dev/null || true
  rmdir "$MOUNT_POINT" 2>/dev/null || true
  echo "mount test done"
else
  echo "--- 6. mount skip (非 Linux 或无 fusermount) ---"
fi

echo ""
echo "=== E2E 完成 ==="
