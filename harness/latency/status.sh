#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

echo "latency_profile=${LATENCY_PROFILE:-unset}"
echo "latency_postgres_pool_sizes=${LATENCY_POSTGRES_POOL_SIZES:-1,2}"
echo "latency_backends=${LATENCY_BACKENDS:-libsql,postgres}"
echo "postgres_url_present=$([[ -n "${IRONCLAW_REBORN_POSTGRES_URL:-}" ]] && echo yes || echo no)"
echo "database_url_present=$([[ -n "${DATABASE_URL:-}" ]] && echo yes || echo no)"
echo "git_head=$(git rev-parse --short HEAD)"
echo "worktree_dirty=$([[ -n "$(git status --short)" ]] && echo yes || echo no)"
echo "tracked_changes=$(git status --short | wc -l | tr -d ' ')"
if command -v pg_isready >/dev/null 2>&1; then
  pg_isready -h localhost -p 5432 -d ironclaw_latency || true
fi
