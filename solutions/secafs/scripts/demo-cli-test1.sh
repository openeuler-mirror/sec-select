#!/usr/bin/env bash
# 使用本地 PostgreSQL 的 test1 数据库演示 CLI 功能。
# 连接信息：默认 postgres://secafs:secafs@127.0.0.1:5432/test1
# 可通过环境变量覆盖： export SECAFS_POSTGRES_URL='postgres://user:pass@host:5432/test1'
# 注意：若已设置 SECAFS_POSTGRES_URL 为无密码的 URL（如 postgres://localhost/test1），
#       migrate 会报 invalid configuration，请改为完整 URL 或 unset SECAFS_POSTGRES_URL。

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
POSTGRES_URL="${SECAFS_POSTGRES_URL:-postgres://secafs:secafs@127.0.0.1:5432/test1}"
CLI="${ROOT_DIR}/cli/target/debug/secafs"

# 若未构建则构建（无 sandbox，避免 reverie 编译问题）
if [ ! -x "${CLI}" ]; then
  echo "==> 构建 CLI (no-default-features)..."
  cargo build --manifest-path "${ROOT_DIR}/cli/Cargo.toml" --no-default-features
fi

echo "==> 使用数据库: ${POSTGRES_URL}"
echo ""

# 1) 若 test1 尚未初始化，可先 init（会创建 schema）
echo "==> 1) 检查/迁移 schema: secafs migrate"
"${CLI}" migrate "${POSTGRES_URL}"
echo ""

# 2) 列出根目录（可能在某些环境下会阻塞，用 timeout 避免脚本卡死）
echo "==> 2) 列出根目录: secafs fs <URL> ls /"
timeout 12 "${CLI}" fs "${POSTGRES_URL}" ls / 2>&1 || echo "(若超时：可改用平台或 Python SDK 操作 test1)"
echo ""

# 3) 写入文件
echo "==> 3) 写入文件: secafs fs <URL> write /demo/hello.txt '...'"
timeout 12 "${CLI}" fs "${POSTGRES_URL}" write /demo/hello.txt "Hello from CLI demo at $(date -Isec)" 2>&1 || echo "(若超时：可改用 Python SDK)"
echo ""

# 4) 读取文件
echo "==> 4) 读取文件: secafs fs <URL> cat /demo/hello.txt"
timeout 12 "${CLI}" fs "${POSTGRES_URL}" cat /demo/hello.txt 2>&1 || true
echo ""

# 5) 再次 ls 查看 /demo
echo "==> 5) 再次列出根目录"
timeout 12 "${CLI}" fs "${POSTGRES_URL}" ls / 2>&1 || true
echo ""

# 6) 时间线（若有 tool_calls 数据）
echo "==> 6) 时间线: secafs timeline <URL>"
timeout 12 "${CLI}" timeline "${POSTGRES_URL}" --limit 5 2>&1 || true
echo ""

echo "==> 演示结束。migrate 已确认可用；若 fs/timeline 超时，可改用平台 Web 界面或 Python SDK 操作同一 test1 数据库。"
