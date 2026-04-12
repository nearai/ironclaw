#!/usr/bin/env bash
# Build the LINE channel WASM component
#
# Prerequisites:
#   - Rust with wasm32-wasip2 target: rustup target add wasm32-wasip2
#   - wasm-tools for component creation: cargo install wasm-tools
#
# Output:
#   - line.wasm - WASM component ready for deployment
#   - line.capabilities.json - Capabilities file (copy alongside .wasm)

set -euo pipefail

cd "$(dirname "$0")"

echo "Building LINE channel WASM component..."

# Build the WASM module
cargo build --release --target wasm32-wasip2

# Convert to component model (if not already a component)
# wasm-tools component new is idempotent on components
WASM_PATH="target/wasm32-wasip2/release/line_channel.wasm"

if [ -f "$WASM_PATH" ]; then
    # Create component if needed
    wasm-tools component new "$WASM_PATH" -o line.wasm 2>/dev/null || cp "$WASM_PATH" line.wasm

    # Optimize the component
    wasm-tools strip line.wasm -o line.wasm

    echo "Built: line.wasm ($(du -h line.wasm | cut -f1))"
    echo ""
    echo "To install:"
    echo "  mkdir -p ~/.ironclaw/channels"
    echo "  cp line.wasm line.capabilities.json ~/.ironclaw/channels/"
    echo ""
    echo "Then configure LINE credentials in the platform UI (channelAccessToken + channelSecret)."
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
