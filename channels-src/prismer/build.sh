#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

echo "Building Prismer channel WASM component..."

cargo build --release --target wasm32-wasip2

WASM_PATH="target/wasm32-wasip2/release/prismer_channel.wasm"

if [ -f "$WASM_PATH" ]; then
    wasm-tools component new "$WASM_PATH" -o prismer.wasm 2>/dev/null || cp "$WASM_PATH" prismer.wasm
    wasm-tools strip prismer.wasm -o prismer.wasm
    echo "Built: prismer.wasm ($(du -h prismer.wasm | cut -f1))"
    echo ""
    echo "To install:"
    echo "  mkdir -p ~/.ironclaw/channels"
    echo "  cp prismer.wasm prismer.capabilities.json ~/.ironclaw/channels/"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
