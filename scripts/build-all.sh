#!/usr/bin/env bash
# Build IronClaw Reborn and bundled first-party extensions.
#
# Run this before release or when bundled extension sources have changed.

set -euo pipefail

cd "$(dirname "$0")/.."

echo "Building bundled first-party extensions..."
./scripts/build-wasm-extensions.sh --first-party

echo ""
echo "Building ironclaw-reborn..."
cargo build --release \
    -p ironclaw_reborn_cli \
    --features webui-v2-beta,slack-v2-host-beta,libsql,postgres,inmemory-turn-state \
    --bin ironclaw-reborn

echo ""
echo "Done. Binary: target/release/ironclaw-reborn"
