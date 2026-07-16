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

case "${LATENCY_BACKENDS:-libsql,postgres}" in
  libsql,postgres|postgres,libsql) ;;
  *)
    if [[ "${LATENCY_ALLOW_DIAGNOSTIC_BACKENDS:-}" != "1" ]]; then
      echo "VOID: constraint violation"
      exit 1
    fi
    ;;
esac

if ! command -v rg >/dev/null 2>&1; then
  echo "VOID: rg not found, cannot verify constraints"
  exit 1
fi

if rg -n "LATENCY_|latency|benchmark|bench" crates src -g '*.rs' 2>/dev/null \
  | rg -q "sleep|tokio::time::sleep|std::thread::sleep|mock readiness|fast path|fast-path"; then
  echo "VOID: constraint violation"
  exit 1
fi
