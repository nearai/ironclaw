#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TOOL_NAME="gotify"

echo "==> Building ${TOOL_NAME} WASM tool for IronClaw..."

if ! rustup target list --installed | grep -q wasm32-wasip2; then
    echo "==> Installing wasm32-wasip2 target..."
    rustup target add wasm32-wasip2
fi

if ! command -v wasm-tools &>/dev/null; then
    echo "==> Installing wasm-tools..."
    cargo install wasm-tools
fi

cargo build --release --target wasm32-wasip2 \
    --manifest-path "${SCRIPT_DIR}/Cargo.toml"

RAW_WASM="${SCRIPT_DIR}/target/wasm32-wasip2/release/gotify_tool.wasm"

if [ ! -f "$RAW_WASM" ]; then
    echo "ERROR: WASM not found at ${RAW_WASM}"
    exit 1
fi

OUTPUT="${SCRIPT_DIR}/${TOOL_NAME}.wasm"

if wasm-tools component new "$RAW_WASM" -o "$OUTPUT" 2>/dev/null; then
    echo "==> Component: ${OUTPUT}"
else
    echo "==> Copying raw module..."
    cp "$RAW_WASM" "$OUTPUT"
fi

SIZE=$(du -h "$OUTPUT" | cut -f1)
echo "==> Done! ${OUTPUT} (${SIZE})"
echo ""
echo "Install:"
echo "  cp ${TOOL_NAME}.wasm ${TOOL_NAME}.capabilities.json ~/.ironclaw/tools/"
