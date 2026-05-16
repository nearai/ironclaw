#!/usr/bin/env bash
# Standalone build script for the optional WeChat-voice SILK decoder helper.
#
# This crate is intentionally excluded from the IronClaw workspace so the
# main `cargo build` does not require libclang. It is built separately:
#
#     ./crates/ironclaw_silk_decoder/build.sh
#
# After building, the binary lands in `target/release/ironclaw-silk-decoder`
# (relative to this crate). Install it next to your `ironclaw` binary, on
# `$PATH`, or point the `IRONCLAW_SILK_DECODER` environment variable at it.

set -euo pipefail

cd "$(dirname "$0")"

if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo not found on PATH" >&2
    exit 1
fi

echo "Building ironclaw-silk-decoder (requires libclang + a C toolchain)..."
cargo build --release

OUT_BIN="target/release/ironclaw-silk-decoder"

if [ ! -f "$OUT_BIN" ]; then
    echo "Error: build did not produce $OUT_BIN" >&2
    exit 1
fi

echo ""
echo "Built: $OUT_BIN ($(du -h "$OUT_BIN" | cut -f1))"
echo ""
echo "To install (one of the following):"
echo "  cp $OUT_BIN \"\$(dirname \"\$(command -v ironclaw)\")/\"   # sibling install"
echo "  cp $OUT_BIN /usr/local/bin/                                  # system PATH"
echo "  export IRONCLAW_SILK_DECODER=\"\$(pwd)/$OUT_BIN\"          # explicit path"
