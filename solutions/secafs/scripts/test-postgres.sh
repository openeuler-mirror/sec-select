#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
POSTGRES_USER="secafs"
POSTGRES_PASS="secafs"
POSTGRES_DB="${1:-secafs_test}"
POSTGRES_URL="postgres://${POSTGRES_USER}:${POSTGRES_PASS}@127.0.0.1:5432/${POSTGRES_DB}"

print_db_state() {
  local label="$1"
  echo "==> DB state: ${label}"
  su - postgres -c "psql -d ${POSTGRES_DB} -v ON_ERROR_STOP=1 <<'SQL'
\\echo 'tables:'
\\dt
\\echo ''
\\echo 'row counts:'
SELECT 'fs_inode' AS table, 0 AS count WHERE to_regclass('public.fs_inode') IS NULL;
SELECT format('SELECT ''fs_inode'' AS table, COUNT(*) AS count FROM %I', 'fs_inode')
  WHERE to_regclass('public.fs_inode') IS NOT NULL \\gexec
SELECT 'fs_dentry' AS table, 0 AS count WHERE to_regclass('public.fs_dentry') IS NULL;
SELECT format('SELECT ''fs_dentry'' AS table, COUNT(*) AS count FROM %I', 'fs_dentry')
  WHERE to_regclass('public.fs_dentry') IS NOT NULL \\gexec
SELECT 'fs_data' AS table, 0 AS count WHERE to_regclass('public.fs_data') IS NULL;
SELECT format('SELECT ''fs_data'' AS table, COUNT(*) AS count FROM %I', 'fs_data')
  WHERE to_regclass('public.fs_data') IS NOT NULL \\gexec
SELECT 'fs_symlink' AS table, 0 AS count WHERE to_regclass('public.fs_symlink') IS NULL;
SELECT format('SELECT ''fs_symlink'' AS table, COUNT(*) AS count FROM %I', 'fs_symlink')
  WHERE to_regclass('public.fs_symlink') IS NOT NULL \\gexec
SELECT 'fs_config' AS table, 0 AS count WHERE to_regclass('public.fs_config') IS NULL;
SELECT format('SELECT ''fs_config'' AS table, COUNT(*) AS count FROM %I', 'fs_config')
  WHERE to_regclass('public.fs_config') IS NOT NULL \\gexec
SELECT 'kv_store' AS table, 0 AS count WHERE to_regclass('public.kv_store') IS NULL;
SELECT format('SELECT ''kv_store'' AS table, COUNT(*) AS count FROM %I', 'kv_store')
  WHERE to_regclass('public.kv_store') IS NOT NULL \\gexec
SELECT 'tool_calls' AS table, 0 AS count WHERE to_regclass('public.tool_calls') IS NULL;
SELECT format('SELECT ''tool_calls'' AS table, COUNT(*) AS count FROM %I', 'tool_calls')
  WHERE to_regclass('public.tool_calls') IS NOT NULL \\gexec
\\echo ''
\\echo 'sample kv_store:'
SELECT format('SELECT key, value FROM %I ORDER BY key LIMIT 3', 'kv_store')
  WHERE to_regclass('public.kv_store') IS NOT NULL \\gexec
\\echo ''
\\echo 'sample tool_calls:'
SELECT format('SELECT id, name, status, started_at, completed_at FROM %I ORDER BY id DESC LIMIT 3', 'tool_calls')
  WHERE to_regclass('public.tool_calls') IS NOT NULL \\gexec
SQL"
}

# 仅在缺少依赖时安装
REQUIRED_CMDS="psql python3 pip3 cargo curl pkg-config"
MISSING=""
for cmd in $REQUIRED_CMDS; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    MISSING="$MISSING $cmd"
  fi
done
if [ -n "${MISSING}" ]; then
  echo "==> Installing missing deps:${MISSING}"
  [ "${RUN_APT_UPDATE:-0}" = "1" ] && apt-get update -y
  apt-get install -y postgresql postgresql-contrib python3-pip python3-venv cargo curl pkg-config libssl-dev libunwind-dev
else
  echo "==> System deps OK, skipping apt install"
fi

echo "==> Starting Postgres"
service postgresql start

echo "==> Creating Postgres user/database (db=${POSTGRES_DB})"
su - postgres -c "psql -v ON_ERROR_STOP=1 <<'SQL'
DO \$\$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '${POSTGRES_USER}') THEN
    CREATE ROLE ${POSTGRES_USER} LOGIN PASSWORD '${POSTGRES_PASS}' CREATEDB;
  ELSE
    ALTER ROLE ${POSTGRES_USER} CREATEDB;
  END IF;
END
\$\$;
SQL"

DB_EXISTS="$(su - postgres -c "psql -tAc \"SELECT 1 FROM pg_database WHERE datname='${POSTGRES_DB}'\"")"
if [ -n "${DB_EXISTS}" ]; then
  echo "==> Dropping existing database ${POSTGRES_DB}"
  su - postgres -c "psql -v ON_ERROR_STOP=1 -d postgres -c \"SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname='${POSTGRES_DB}' AND pid <> pg_backend_pid();\""
  su - postgres -c "dropdb ${POSTGRES_DB}"
fi
su - postgres -c "createdb -O ${POSTGRES_USER} ${POSTGRES_DB}"
print_db_state "after create"

# Python SDK：venv 已存在且 secafs_sdk 可导入则跳过
VENV_DIR="${ROOT_DIR}/.venv-secafs"
if [ -f "${VENV_DIR}/bin/python" ] && "${VENV_DIR}/bin/python" -c "import secafs_sdk" 2>/dev/null; then
  echo "==> Python SDK venv OK, skipping pip install"
else
  echo "==> Installing Python SDK dependencies"
  python3 -m venv "${VENV_DIR}"
  "${VENV_DIR}/bin/pip" install --upgrade pip -q
  "${VENV_DIR}/bin/pip" install -e "${ROOT_DIR}/sdk/python" -q
fi

echo "==> Running Python SDK Postgres smoke test"
POSTGRES_URL="${POSTGRES_URL}" "${VENV_DIR}/bin/python" - <<'PY'
import asyncio
import os
from secafs_sdk import SecAFS, SecAFSOptions

async def main():
    agent = await SecAFS.open(SecAFSOptions(postgres_url=os.environ["POSTGRES_URL"]))
    await agent.kv.set("hello", {"value": "world"})
    value = await agent.kv.get("hello")
    assert value == {"value": "world"}
    await agent.fs.write_file("/notes/hello.txt", "hello postgres")
    content = await agent.fs.read_file("/notes/hello.txt")
    assert content == "hello postgres"
    call_id = await agent.tools.start("test", {"ok": True})
    await agent.tools.success(call_id, {"done": True})
    await agent.close()

asyncio.run(main())
print("Python SDK test OK")
PY
print_db_state "after python sdk"

# Rust：仅在没有 rustup 时安装
if ! command -v rustup >/dev/null 2>&1; then
  echo "==> Installing rustup"
  curl https://sh.rustup.rs -sSf | sh -s -- -y
fi
if [ -f "${HOME}/.cargo/env" ]; then
  # shellcheck disable=SC1090
  source "${HOME}/.cargo/env"
fi

echo "==> Running CLI Postgres smoke test"
cd "${ROOT_DIR}"
cargo run --manifest-path cli/Cargo.toml --no-default-features -- fs "${POSTGRES_URL}" write /cli/hello.txt "hello from cli"
print_db_state "after cli write"
cargo run --manifest-path cli/Cargo.toml --no-default-features -- fs "${POSTGRES_URL}" cat /cli/hello.txt
print_db_state "after cli cat"
cargo run --manifest-path cli/Cargo.toml --no-default-features -- fs "${POSTGRES_URL}" ls /
print_db_state "after cli ls"

echo "All Postgres tests completed."
exit 0
