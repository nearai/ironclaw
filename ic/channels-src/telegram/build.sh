#!/usr/bin/env bash
# Build the Telegram channel WASM component
#
# Prerequisites:
#   - Rust with wasm32-wasip2 target: rustup target add wasm32-wasip2
#   - wasm-tools for component creation: cargo install wasm-tools
#
# Output:
#   - telegram.wasm - WASM component ready for deployment
#   - telegram.capabilities.json - Capabilities file (copy alongside .wasm)

set -euo pipefail

cd "$(dirname "$0")"

echo "Building Telegram channel WASM component..."

# Build the WASM module
cargo build --release --target wasm32-wasip2

# Convert to component model (if not already a component)
# wasm-tools component new is idempotent on components
WASM_PATH="target/wasm32-wasip2/release/telegram_channel.wasm"

if [ -f "$WASM_PATH" ]; then
    # Create component if needed
    wasm-tools component new "$WASM_PATH" -o telegram.wasm 2>/dev/null || cp "$WASM_PATH" telegram.wasm

    # Optimize the component
    wasm-tools strip telegram.wasm -o telegram.wasm

    echo "Built: telegram.wasm ($(du -h telegram.wasm | cut -f1))"
    echo ""
    echo "To install:"
    echo "  mkdir -p ~/.ironclaw/channels"
    echo "  cp telegram.wasm telegram.capabilities.json ~/.ironclaw/channels/"
    echo ""
    echo "Then add your bot token to secrets:"
    echo "  # Set TELEGRAM_BOT_TOKEN in your environment or secrets store"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
