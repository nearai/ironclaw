#!/usr/bin/env bash
# Build the Discord channel WASM component
#
# Prerequisites:
#   - Rust with wasm32-wasip2 target: rustup target add wasm32-wasip2
#   - wasm-tools for component creation: cargo install wasm-tools
#
# Output:
#   - discord.wasm - WASM component ready for deployment
#   - discord.capabilities.json - Capabilities file (copy alongside .wasm)

set -euo pipefail

cd "$(dirname "$0")"

if ! command -v wasm-tools &> /dev/null; then
    echo "Error: wasm-tools not found. Install with: cargo install wasm-tools"
    exit 1
fi

echo "Building Discord channel WASM component..."

# Build the WASM module
cargo build --release --target wasm32-wasip2

# Convert to component model (if not already a component)
# wasm-tools component new is idempotent on components
WASM_PATH="target/wasm32-wasip2/release/discord_channel.wasm"

if [ -f "$WASM_PATH" ]; then
    # Create component if needed
    wasm-tools component new "$WASM_PATH" -o discord.wasm 2>/dev/null || cp "$WASM_PATH" discord.wasm

    # Optimize the component
    wasm-tools strip discord.wasm -o discord.wasm

    echo "Built: discord.wasm ($(du -h discord.wasm | cut -f1))"
    echo ""
    echo "To install:"
    echo "  mkdir -p ~/.ironclaw/channels"
    echo "  cp discord.wasm discord.capabilities.json ~/.ironclaw/channels/"
    echo ""
    echo "Then add your bot token to secrets:"
    echo "  # Set discord_bot_token and discord_public_key in your environment or secrets store"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
