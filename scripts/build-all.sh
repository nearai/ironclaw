#!/usr/bin/env bash
# Build the canonical IronClaw CLI and bundled first-party extensions.

set -euo pipefail

cd "$(dirname "$0")/.."

echo "Building bundled first-party extensions..."
./scripts/build-wasm-extensions.sh --first-party

echo ""
echo "Building ironclaw..."
cargo build --release \
    -p ironclaw_reborn_cli \
    --features webui-v2-beta,slack-v2-host-beta,libsql,postgres,inmemory-turn-state \
    --bin ironclaw

echo ""
echo "Done. Binary: target/release/ironclaw"
