#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

echo "Building WeChat channel WASM component..."

cargo build --release --target wasm32-wasip2

WASM_PATH="target/wasm32-wasip2/release/wechat_channel.wasm"

if [ -f "$WASM_PATH" ]; then
    if command -v wasm-tools >/dev/null 2>&1; then
        wasm-tools component new "$WASM_PATH" -o wechat.wasm 2>/dev/null || cp "$WASM_PATH" wechat.wasm
        wasm-tools strip wechat.wasm -o wechat.wasm
    else
        cp "$WASM_PATH" wechat.wasm
        echo "wasm-tools not found; copied raw wasm output without component conversion/strip"
    fi

    echo "Built: wechat.wasm ($(du -h wechat.wasm | cut -f1))"
    echo ""
    echo "To install:"
    echo "  mkdir -p ~/.ironclaw/channels"
    echo "  cp wechat.wasm wechat.capabilities.json ~/.ironclaw/channels/"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
