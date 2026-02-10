#!/usr/bin/env bash
# Build the Slack channel WASM component
#
# Prerequisites:
#   - Rust with wasm32-wasip2 target: rustup target add wasm32-wasip2
#   - wasm-tools for component creation: cargo install wasm-tools
#
# Output:
#   - slack.wasm - WASM component ready for deployment
#   - slack.capabilities.json - Capabilities file (copy alongside .wasm)

set -euo pipefail

cd "$(dirname "$0")"

echo "Building Slack channel WASM component..."

# Build the WASM module
cargo build --release --target wasm32-wasip2

# Convert to component model (if not already a component)
# wasm-tools component new is idempotent on components
WASM_PATH="target/wasm32-wasip2/release/slack_channel.wasm"

if [ -f "$WASM_PATH" ]; then
    # Create component if needed
    wasm-tools component new "$WASM_PATH" -o slack.wasm 2>/dev/null || cp "$WASM_PATH" slack.wasm

    # Optimize the component
    wasm-tools strip slack.wasm -o slack.wasm

    echo "Built: slack.wasm ($(du -h slack.wasm | cut -f1))"
    echo "Copy slack.wasm and slack.capabilities.json to ~/.ironclaw/channels/"
else
    echo "Error: WASM output not found at $WASM_PATH"
    exit 1
fi
