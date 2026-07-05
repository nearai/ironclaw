#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

if [[ "${LATENCY_POSTGRES_POOL_SIZES:-1,2}" != "1,2" ]]; then
  if [[ "${LATENCY_ALLOW_DIAGNOSTIC_POOL_SIZES:-}" != "1" ]]; then
    echo "VOID: constraint violation"
    exit 1
  fi
fi

if rg -n "LATENCY_|latency|benchmark|bench" crates src \
  -g '*.rs' >/tmp/ironclaw-latency-lint.$$ 2>/dev/null; then
  if rg -n "sleep|tokio::time::sleep|std::thread::sleep|mock readiness|fast path|fast-path" \
    /tmp/ironclaw-latency-lint.$$ >/dev/null 2>&1; then
    rm -f /tmp/ironclaw-latency-lint.$$
    echo "VOID: constraint violation"
    exit 1
  fi
fi
rm -f /tmp/ironclaw-latency-lint.$$
