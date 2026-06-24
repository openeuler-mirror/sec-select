#!/usr/bin/env bash
# 启动 SecAFS 开发数据库（默认 OpenGauss；PostgreSQL 可选）
# 用法:
#   ./scripts/dev-up.sh              # 默认 OpenGauss
#   ./scripts/dev-up.sh postgres     # 启动 PostgreSQL
set -euo pipefail
cd "$(dirname "$0")/.."

BACKEND="${1:-opengauss}"

case "$BACKEND" in
  postgres|pg)
    docker compose -f docker-compose.dev.yml --profile postgres up -d
    echo "Postgres at postgres://secafs:secafs@localhost:5433/secafs"
    ;;
  opengauss|og|*)
    docker compose -f docker-compose.dev.yml --profile opengauss up -d
    echo "OpenGauss at opengauss://secafs:Secafs!123@localhost:5433/secafs"
    echo "  (also works with postgres://secafs:Secafs!123@localhost:5433/secafs)"
    ;;
esac
