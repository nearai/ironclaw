#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$ROOT_DIR/.." && pwd)"
IMAGE_NAME="ironclaw-rust-tests"

DEFAULT_CMD=(
  cargo test --test anp_identity_integration --no-default-features --features libsql
)

if [ "$#" -gt 0 ]; then
  CMD=("$@")
else
  CMD=("${DEFAULT_CMD[@]}")
fi

docker pull rust:1.92-slim-bookworm >/dev/null || true
docker build -t "$IMAGE_NAME" -f "$ROOT_DIR/Dockerfile.rust-tests" "$ROOT_DIR" >/dev/null

docker run --rm \
  -v "$REPO_DIR:/workspace" \
  -v ironclaw-rust-tests-cargo:/cargo \
  -v ironclaw-rust-tests-target:/target \
  -w /workspace \
  -e CARGO_HOME=/cargo \
  -e CARGO_TARGET_DIR=/target \
  -e CARGO_BUILD_JOBS=1 \
  -e CARGO_INCREMENTAL=0 \
  -e RUSTFLAGS="-Cdebuginfo=0 -Clink-arg=-Wl,--no-keep-memory -Clink-arg=-Wl,--reduce-memory-overheads" \
  "$IMAGE_NAME" \
  "${CMD[@]}"
