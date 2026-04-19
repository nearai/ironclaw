#!/usr/bin/env bash
# Build the xmpp-bridge helper binary.

set -euo pipefail

cd "$(dirname "$0")"

echo "Building xmpp-bridge..."
cargo build --release
echo "Built: target/release/xmpp-bridge"
