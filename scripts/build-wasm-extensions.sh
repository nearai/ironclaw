#!/usr/bin/env bash
# Build all WASM tools and channels from source.
#
# Verifies that every tool/channel in the registry compiles against the
# current WIT definitions. Used by CI and can be run locally.
#
# Prerequisites:
#   rustup target add wasm32-wasip2
#   cargo install cargo-component --locked
#
# Usage:
#   ./scripts/build-wasm-extensions.sh           # build all
#   ./scripts/build-wasm-extensions.sh --tools    # tools only
#   ./scripts/build-wasm-extensions.sh --channels # channels only

set -euo pipefail

cd "$(dirname "$0")/.."

BUILD_TOOLS=true
BUILD_CHANNELS=true
FAILED=()

if [[ "${1:-}" == "--tools" ]]; then
    BUILD_CHANNELS=false
elif [[ "${1:-}" == "--channels" ]]; then
    BUILD_TOOLS=false
fi

build_extension() {
    local manifest_path="$1"
    local source_dir
    local crate_name

    source_dir=$(jq -r '.source.dir' "$manifest_path")
    crate_name=$(jq -r '.source.crate_name' "$manifest_path")
    local name
    name=$(basename "$manifest_path" .json)

    if [ ! -d "$source_dir" ]; then
        echo "  SKIP $name (source dir $source_dir not found)"
        return 0
    fi

    echo "  BUILD $name ($crate_name) from $source_dir"
    if ! cargo component build --release --target wasm32-wasip2 --manifest-path "$source_dir/Cargo.toml" 2>&1; then
        echo "  FAIL $name"
        FAILED+=("$name")
        return 1
    fi
    echo "  OK   $name"
}

if $BUILD_TOOLS; then
    echo "Building WASM tools..."
    for manifest in registry/tools/*.json; do
        build_extension "$manifest" || true
    done
fi

if $BUILD_CHANNELS; then
    echo "Building WASM channels..."
    for manifest in registry/channels/*.json; do
        build_extension "$manifest" || true
    done
fi

echo ""
if [ ${#FAILED[@]} -gt 0 ]; then
    echo "FAILED: ${FAILED[*]}"
    exit 1
else
    echo "All WASM extensions built successfully."
fi
