#!/usr/bin/env bash
# 停止 SecAFS 开发数据库（PostgreSQL 和/或 OpenGauss）
set -euo pipefail
cd "$(dirname "$0")/.."

BACKEND="${1:-all}"

case "$BACKEND" in
  opengauss|og)
    docker compose -f docker-compose.dev.yml --profile opengauss down
    ;;
  postgres|pg)
    docker compose -f docker-compose.dev.yml --profile postgres down
    ;;
  all|*)
    docker compose -f docker-compose.dev.yml --profile postgres --profile opengauss down
    ;;
esac
