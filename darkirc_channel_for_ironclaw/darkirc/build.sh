#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
#CHANNEL_DIR="$SCRIPT_DIR/channel"
CHANNEL_DIR="$SCRIPT_DIR"

INSTALL_DIR="${IRONCLAW_HOME:-$HOME/.ironclaw}/channels"

echo "=== Building DarkIRC WASM channel ==="

# The wit_bindgen macro expects ../../wit/channel.wit relative to the crate.
# This means the channel dir should be at channels-src/darkirc/channel/
# inside the ironclaw repo, OR we symlink the WIT directory.
if [ ! -f "$CHANNEL_DIR/../../wit/channel.wit" ]; then
    # Try to find ironclaw repo and symlink wit/
    IRONCLAW_REPO="${IRONCLAW_REPO:-}"

    if [ -z "$IRONCLAW_REPO" ]; then
        # Common locations
        for candidate in \
            "$HOME/ironclaw" \
            "$HOME/src/ironclaw" \
            "$HOME/projects/ironclaw" \
            "$HOME/code/ironclaw" \
            "$HOME/git/ironclaw"; do
            if [ -f "$candidate/wit/channel.wit" ]; then
                IRONCLAW_REPO="$candidate"
                break
            fi
        done
    fi

    if [ -n "$IRONCLAW_REPO" ] && [ -f "$IRONCLAW_REPO/wit/channel.wit" ]; then
        echo "Found ironclaw repo at $IRONCLAW_REPO"
        echo "Creating wit symlink..."
        mkdir -p "$CHANNEL_DIR/.."
        ln -sfn "$IRONCLAW_REPO/wit" "$CHANNEL_DIR/../wit"
    else
        echo ""
        echo "ERROR: Cannot find ironclaw's wit/channel.wit"
        echo ""
        echo "The WASM channel needs the WIT interface definition."
        echo "Either:"
        echo "  1) Place this project at channels-src/darkirc/ inside your ironclaw checkout"
        echo "  2) Set IRONCLAW_REPO=/path/to/ironclaw and re-run"
        echo "  3) Symlink: ln -s /path/to/ironclaw/wit $CHANNEL_DIR/../wit"
        echo ""
        exit 1
    fi
fi

# Ensure wasm32-wasip2 target
if ! rustup target list --installed | grep -q wasm32-wasip2; then
    echo "Adding wasm32-wasip2 target..."
    rustup target add wasm32-wasip2
fi

# Build
cd "$CHANNEL_DIR"
cargo build --target wasm32-wasip2 --release

WASM_FILE="$CHANNEL_DIR/target/wasm32-wasip2/release/darkirc_channel.wasm"

if [ ! -f "$WASM_FILE" ]; then
    echo "ERROR: WASM output not found at $WASM_FILE"
    exit 1
fi

echo "Built: $WASM_FILE ($(du -h "$WASM_FILE" | cut -f1))"

# Install
echo ""
echo "=== Installing to $INSTALL_DIR ==="
mkdir -p "$INSTALL_DIR"
cp "$WASM_FILE" "$INSTALL_DIR/darkirc.wasm"
cp "$CHANNEL_DIR/darkirc.capabilities.json" "$INSTALL_DIR/"

echo "Installed:"
echo "  $INSTALL_DIR/darkirc.wasm"
echo "  $INSTALL_DIR/darkirc.capabilities.json"
echo ""
echo "Done. Restart ironclaw to activate the DarkIRC channel."
echo "Make sure darkirc_adapter.py is running before starting ironclaw."
