#!/usr/bin/env bash
# Thin wrapper around cargo-insta for the replay snapshot gate.
#
# Subcommands:
#   review  — interactive review + accept/reject of pending snapshots
#   accept  — accept all pending snapshots without prompting
#   test    — run the replay test set and fail on pending snapshots
#   record  — record a fresh fixture from a live agent session
#
# Usage:
#   scripts/replay-snap.sh review
#   scripts/replay-snap.sh accept
#   scripts/replay-snap.sh test
#   scripts/replay-snap.sh record <fixture_name> [<model_name>]

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

FIXTURE_DIR="tests/fixtures/llm_traces"

ensure_insta() {
  if ! command -v cargo-insta >/dev/null 2>&1; then
    echo "cargo-insta not installed. Installing..."
    cargo install cargo-insta --locked
  fi
}

case "${1:-}" in
  review)
    ensure_insta
    cargo insta review
    ;;
  accept)
    ensure_insta
    cargo insta accept --all
    ;;
  test)
    ensure_insta
    cargo insta test \
      --check \
      --no-default-features \
      --features "libsql,replay" \
      --test e2e_engine_v2 \
      --test e2e_recorded_trace \
      --test e2e_live \
      --test e2e_bug_bash_snapshots
    ;;
  record)
    name="${2:-}"
    if [ -z "$name" ]; then
      echo "usage: $0 record <fixture_name> [<model_name>]" >&2
      exit 2
    fi
    model="${3:-recording-$name}"
    out="$FIXTURE_DIR/$name.json"
    mkdir -p "$(dirname "$out")"
    echo "Recording $out (model_name: $model)"
    echo "Interact with the agent, then quit to flush the trace."
    IRONCLAW_RECORD_TRACE=1 \
    IRONCLAW_TRACE_OUTPUT="$out" \
    IRONCLAW_TRACE_MODEL_NAME="$model" \
      cargo run
    ;;
  *)
    echo "usage: $0 {review|accept|test|record}"
    exit 2
    ;;
esac
