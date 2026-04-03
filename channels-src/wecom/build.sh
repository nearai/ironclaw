#!/usr/bin/env bash
# Build the WeCom channel WASM component

set -euo pipefail

cd "$(dirname "$0")"

echo "Building WeCom channel WASM component..."

cargo build --release --target wasm32-wasip2

WASM_PATH="target/wasm32-wasip2/release/wecom_channel.wasm"

if [ -f "$WASM_PATH" ]; then
    wasm-tools component new "$WASM_PATH" -o wecom.wasm 2>/dev/null || cp "$WASM_PATH" wecom.wasm
    wasm-tools strip wecom.wasm -o wecom.wasm

    echo "Built: wecom.wasm ($(du -h wecom.wasm | cut -f1))"
    echo ""
    echo "To install:"
    echo "  mkdir -p ~/.ironclaw/channels"
    echo "  cp wecom.wasm wecom.capabilities.json ~/.ironclaw/channels/"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
