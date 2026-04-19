#!/usr/bin/env bash
# Build IronClaw and all bundled channels.
#
# Run this before release or when channel sources have changed.
# The main binary bundles telegram.wasm via include_bytes!; it must exist.

set -euo pipefail

cd "$(dirname "$0")/.."

echo "Building bundled channels..."
if [ -d "channels-src/telegram" ]; then
    ./channels-src/telegram/build.sh
fi

echo ""
echo "Building IronClaw..."
cargo build --release

echo ""
echo "Done. Binary: target/release/ironclaw"
