#!/usr/bin/env bash
# Build the Feishu channel WASM component
#
# Prerequisites:
#   - Rust with wasm32-wasip2 target: rustup target add wasm32-wasip2
#   - wasm-tools for component creation: cargo install wasm-tools
#
# Output:
#   - feishu.wasm - WASM component ready for deployment
#   - feishu.capabilities.json - Capabilities file (copy alongside .wasm)

set -euo pipefail

cd "$(dirname "$0")"

echo "Building Feishu channel WASM component..."

# Build the WASM module
cargo build --release --target wasm32-wasip2

# Convert to component model (if not already a component)
# wasm-tools component new is idempotent on components
WASM_PATH="target/wasm32-wasip2/release/feishu_channel.wasm"

if [ -f "$WASM_PATH" ]; then
    # Create component if needed
    wasm-tools component new "$WASM_PATH" -o feishu.wasm 2>/dev/null || cp "$WASM_PATH" feishu.wasm

    # Optimize the component
    wasm-tools strip feishu.wasm -o feishu.wasm

    echo "Built: feishu.wasm ($(du -h feishu.wasm | cut -f1))"
    echo ""
    echo "To install:"
    echo "  mkdir -p ~/.ironclaw/channels"
    echo "  cp feishu.wasm feishu.capabilities.json ~/.ironclaw/channels/"
    echo ""
    echo "Then add your Feishu app credentials to secrets:"
    echo "  # Set FEISHU_APP_ID and FEISHU_APP_SECRET in your environment or secrets store"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
