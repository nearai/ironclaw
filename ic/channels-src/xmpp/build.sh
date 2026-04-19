#!/usr/bin/env bash
# Build the XMPP channel WASM component.

set -euo pipefail

cd "$(dirname "$0")"

echo "Building XMPP channel WASM component..."

cargo build --release --target wasm32-wasip2

WASM_PATH="target/wasm32-wasip2/release/xmpp_channel.wasm"
WASM_TOOLS_BIN="${WASM_TOOLS_BIN:-$(command -v wasm-tools || true)}"

if [ -z "$WASM_TOOLS_BIN" ] && [ -x "$HOME/.cargo/bin/wasm-tools" ]; then
    WASM_TOOLS_BIN="$HOME/.cargo/bin/wasm-tools"
fi

if [ -z "$WASM_TOOLS_BIN" ]; then
    echo "Error: wasm-tools not found. Install it with:"
    echo "  cargo install wasm-tools"
    echo "Or set WASM_TOOLS_BIN=/path/to/wasm-tools"
    exit 1
fi

if [ -f "$WASM_PATH" ]; then
    "$WASM_TOOLS_BIN" component new "$WASM_PATH" -o xmpp.wasm 2>/dev/null || cp "$WASM_PATH" xmpp.wasm
    "$WASM_TOOLS_BIN" strip xmpp.wasm -o xmpp.wasm
    echo "Built: xmpp.wasm ($(du -h xmpp.wasm | cut -f1))"
    echo "Copy xmpp.wasm and xmpp.capabilities.json to ~/.ironclaw/channels/"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
