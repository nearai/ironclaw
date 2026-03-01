#!/usr/bin/env bash
# build-atlas-ironclaw.sh
# Builds and installs Telegram channel and Atlas tools to ~/.ironclaw/

set -e

REPO_ROOT="/root/Projects/atlas/ironclaw"
OUT_CHANNELS="$HOME/.ironclaw/channels"
OUT_TOOLS="$HOME/.ironclaw/tools"

mkdir -p "$OUT_CHANNELS" "$OUT_TOOLS"

# 1. Build Telegram Channel
echo "--- Building Telegram Channel ---"
cd "$REPO_ROOT/channels-src/telegram"
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/telegram_channel.wasm "$OUT_CHANNELS/telegram.wasm"
cp telegram.capabilities.json "$OUT_CHANNELS/"

# 2. Build Atlas Tools
for tool in atlas-classify atlas-brain atlas-notion; do
    echo "--- Building Tool: $tool ---"
    cd "$REPO_ROOT/tools-src/$tool"
    cargo build --release --target wasm32-wasip2
    crate_name=$(echo "$tool" | sed 's/-/_/g')
    cp "target/wasm32-wasip2/release/${crate_name}_tool.wasm" "$OUT_TOOLS/${tool}.wasm"
    cp "${tool}-tool.capabilities.json" "$OUT_TOOLS/"
done

# 3. Build Main IronClaw Binary
echo "--- Building Main IronClaw Binary ---"
cd "$REPO_ROOT"
cargo build --release

echo "--- Build & Install COMPLETE ---"
ls -la "$OUT_CHANNELS"
ls -la "$OUT_TOOLS"
