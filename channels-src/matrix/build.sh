#!/usr/bin/env bash
# Build the Matrix channel WASM component.

set -euo pipefail

cd "$(dirname "$0")"

echo "Building Matrix channel WASM component..."

cargo build --release --target wasm32-wasip2

WASM_PATH="target/wasm32-wasip2/release/matrix_channel.wasm"

if [ -f "$WASM_PATH" ]; then
    wasm-tools component new "$WASM_PATH" -o matrix.wasm 2>/dev/null || cp "$WASM_PATH" matrix.wasm
    wasm-tools strip matrix.wasm -o matrix.wasm

    echo "Built: matrix.wasm ($(du -h matrix.wasm | cut -f1))"
    echo "Copy matrix.wasm and matrix.capabilities.json to ~/.ironclaw/channels/"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
