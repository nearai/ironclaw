#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MODE="${1:---full}"

case "$MODE" in
  --dev)
    export LATENCY_WARMUP="${LATENCY_WARMUP:-5}"
    export LATENCY_SAMPLES="${LATENCY_SAMPLES:-40}"
    export LATENCY_CONCURRENCY="${LATENCY_CONCURRENCY:-1,4}"
    export LATENCY_PROFILE="${LATENCY_PROFILE:-dev}"
    ;;
  --holdout)
    export LATENCY_WARMUP="${LATENCY_WARMUP:-30}"
    export LATENCY_SAMPLES="${LATENCY_SAMPLES:-300}"
    export LATENCY_CONCURRENCY="${LATENCY_CONCURRENCY:-1,4,16}"
    export LATENCY_PROFILE="${LATENCY_PROFILE:-holdout}"
    ;;
  --full)
    export LATENCY_WARMUP="${LATENCY_WARMUP:-30}"
    export LATENCY_SAMPLES="${LATENCY_SAMPLES:-300}"
    export LATENCY_CONCURRENCY="${LATENCY_CONCURRENCY:-1,4,16}"
    export LATENCY_PROFILE="${LATENCY_PROFILE:-full-dev}"
    ;;
  *)
    echo "usage: $0 [--dev|--full|--holdout]" >&2
    exit 2
    ;;
esac

export IRONCLAW_REBORN_POSTGRES_URL="${IRONCLAW_REBORN_POSTGRES_URL:-postgres://postgres:postgres@localhost:5432/ironclaw_latency}"
export LATENCY_POSTGRES_POOL_SIZES="${LATENCY_POSTGRES_POOL_SIZES:-1,2}"

cd "$ROOT"
"$ROOT/harness/latency/lint.sh"
cargo run --quiet --manifest-path harness/latency/runner/Cargo.toml
