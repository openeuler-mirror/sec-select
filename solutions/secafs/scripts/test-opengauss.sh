#!/usr/bin/env bash
# SecAFS OpenGauss 集成测试
# 启动 OpenGauss 容器，运行基础操作验证，然后清理。
#
# 用法: ./scripts/test-opengauss.sh
set -euo pipefail
cd "$(dirname "$0")/.."

OG_URL="postgres://secafs:Secafs!123@127.0.0.1:5433/postgres"
PROFILE="opengauss"
CONTAINER_NAME="secafs-opengauss-1"

# Helper: 在 OpenGauss 容器内以管理员身份（omm）执行 SQL
og_admin_sql() {
  local db="$1"
  local sql="$2"
  docker exec "$CONTAINER_NAME" bash -c \
    "export LD_LIBRARY_PATH=/usr/local/opengauss/lib && su - omm -c \"/usr/local/opengauss/bin/gsql -d $db -p 5432 -c \\\"$sql\\\"\"" \
    > /dev/null 2>&1
}

echo "=== 1. 启动 OpenGauss 开发容器 ==="
docker compose -f docker-compose.dev.yml --profile "$PROFILE" up -d

echo "  等待 OpenGauss 就绪..."
for i in $(seq 1 90); do
  if python3 -c "
import asyncio, asyncpg
async def t():
    c = await asyncpg.connect('$OG_URL', ssl=False)
    await c.close()
asyncio.run(t())
" > /dev/null 2>&1; then
    echo "  OpenGauss 已就绪（${i}s）"
    break
  fi
  if [ "$i" -eq 90 ]; then
    echo "  错误：OpenGauss 未能在 90s 内就绪" >&2
    docker compose -f docker-compose.dev.yml --profile "$PROFILE" logs
    docker compose -f docker-compose.dev.yml --profile "$PROFILE" down
    exit 1
  fi
  sleep 1
done

# 确保 secafs 用户在默认 postgres 库中有权限
og_admin_sql postgres "GRANT ALL ON SCHEMA public TO secafs" || true
og_admin_sql postgres "ALTER ROLE secafs CREATEDB" || true

PASS=0
FAIL=0

run_test() {
  local desc="$1"
  shift
  echo -n "  测试: $desc ... "
  if "$@" > /dev/null 2>&1; then
    echo "PASS"
    PASS=$((PASS + 1))
  else
    echo "FAIL"
    FAIL=$((FAIL + 1))
  fi
}

echo ""
echo "=== 2. Python SDK 测试 ==="
python3 -c "
import asyncio, sys
sys.path.insert(0, 'sdk/python')
from secafs_sdk import SecAFS, SecAFSOptions

async def test():
    # 测试 opengauss:// URL scheme 自动检测
    opts = SecAFSOptions(postgres_url='opengauss://secafs:Secafs!123@127.0.0.1:5433/postgres')
    assert opts.backend == 'opengauss', f'Expected opengauss, got {opts.backend}'

    # 创建测试数据库
    import asyncpg
    admin = await asyncpg.connect('$OG_URL', ssl=False)
    try:
        await admin.execute('DROP DATABASE IF EXISTS secafs_og_test')
        await admin.execute('CREATE DATABASE secafs_og_test OWNER secafs')
    finally:
        await admin.close()

    # 在新库中授予 schema 权限（通过 docker exec 以 omm 管理员身份执行）
    import subprocess
    subprocess.run([
        'docker', 'exec', '$CONTAINER_NAME', 'bash', '-c',
        'export LD_LIBRARY_PATH=/usr/local/opengauss/lib && '
        'su - omm -c \"/usr/local/opengauss/bin/gsql -d secafs_og_test -p 5432 '
        '-c \\\"GRANT ALL ON SCHEMA public TO secafs;\\\"\"'
    ], check=True, capture_output=True)

    # 连接并执行基本操作（使用 opengauss:// 触发 SQL 兼容层）
    test_url = 'opengauss://secafs:Secafs!123@127.0.0.1:5433/secafs_og_test'
    agent = await SecAFS.open(SecAFSOptions(postgres_url=test_url))
    await agent.fs.write_file('/test_og.txt', 'Hello from OpenGauss!')
    content = await agent.fs.read_file('/test_og.txt')
    assert content == 'Hello from OpenGauss!', f'Unexpected content: {content}'
    await agent.fs.unlink('/test_og.txt')

    # KV 操作
    await agent.kv.set('og_key', {'engine': 'opengauss'})
    val = await agent.kv.get('og_key')
    assert val == {'engine': 'opengauss'}, f'Unexpected KV value: {val}'

    await agent.close()

    # 清理测试数据库
    admin2 = await asyncpg.connect('$OG_URL', ssl=False)
    await admin2.execute('DROP DATABASE IF EXISTS secafs_og_test')
    await admin2.close()
    print('Python SDK: All tests passed')

asyncio.run(test())
" && run_test "Python SDK 基础操作" true || run_test "Python SDK 基础操作" false

echo ""
echo "=== 3. 测试 URL 归一化 ==="
python3 -c "
import sys
sys.path.insert(0, 'sdk/python')
from secafs_sdk.db import normalize_db_url

# opengauss:// 应归一化为 postgres://
assert normalize_db_url('opengauss://u:p@h:5432/db') == 'postgres://u:p@h:5432/db'
# postgres:// 应保持不变
assert normalize_db_url('postgres://u:p@h:5432/db') == 'postgres://u:p@h:5432/db'
print('URL normalization: All tests passed')
" && run_test "URL 归一化" true || run_test "URL 归一化" false

echo ""
echo "=== 4. 后端检测测试 ==="
python3 -c "
import asyncio, sys
sys.path.insert(0, 'sdk/python')
from secafs_sdk.db import connect_postgres, detect_backend

async def test():
    conn = await connect_postgres('$OG_URL')
    backend = await detect_backend(conn)
    print(f'Detected backend: {backend}')
    await conn.close()
    assert backend == 'opengauss', f'Expected opengauss, got {backend}'

asyncio.run(test())
" && run_test "后端检测" true || run_test "后端检测" false

echo ""
echo "=== 5. 清理 ==="
docker compose -f docker-compose.dev.yml --profile "$PROFILE" down

echo ""
echo "==============================="
echo "  结果: ${PASS} passed, ${FAIL} failed"
echo "==============================="

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
