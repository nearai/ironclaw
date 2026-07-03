#!/usr/bin/env bash
# Build all WASM tools and channels from source.
#
# Verifies that every tool/channel in the registry compiles against the
# current WIT definitions. Used by CI and can be run locally.
#
# Prerequisites:
#   rustup target add wasm32-wasip2
#   rustup target add wasm32-wasip1
#   cargo install cargo-component --locked
#
# Usage:
#   ./scripts/build-wasm-extensions.sh           # build all
#   ./scripts/build-wasm-extensions.sh --tools    # tools only
#   ./scripts/build-wasm-extensions.sh --channels # channels only
#   ./scripts/build-wasm-extensions.sh --first-party # first-party extensions only

set -euo pipefail
shopt -s nullglob

cd "$(dirname "$0")/.."

# cargo-component may be installed under ~/.cargo while Homebrew rustc wins PATH on
# developer machines. Force rustup rustc when available so installed WASI targets are
# visible to component builds.
RUSTUP_RUSTC=""
RUSTUP_TOOLCHAIN_NAME=""
if command -v rustup >/dev/null 2>&1; then
    RUSTUP_RUSTC=$(rustup which rustc 2>/dev/null || true)
    RUSTUP_TOOLCHAIN_NAME=$(rustup show active-toolchain 2>/dev/null | awk '{print $1}' || true)
    if [ -z "${RUSTC:-}" ] && [ -n "$RUSTUP_RUSTC" ]; then
        export RUSTC="$RUSTUP_RUSTC"
    fi
fi

cargo_component_build() {
    if [ -n "$RUSTUP_TOOLCHAIN_NAME" ]; then
        RUSTC="${RUSTC:-$RUSTUP_RUSTC}" rustup run "$RUSTUP_TOOLCHAIN_NAME" cargo component build "$@"
    else
        cargo component build "$@"
    fi
}

BUILD_TOOLS=true
BUILD_CHANNELS=true
BUILD_FIRST_PARTY=true
FAILED=()

if [[ "${1:-}" == "--tools" ]]; then
    BUILD_CHANNELS=false
    BUILD_FIRST_PARTY=false
elif [[ "${1:-}" == "--channels" ]]; then
    BUILD_TOOLS=false
    BUILD_FIRST_PARTY=false
elif [[ "${1:-}" == "--first-party" ]]; then
    BUILD_TOOLS=false
    BUILD_CHANNELS=false
fi

fail_build() {
    local name="$1"
    local reason="$2"

    echo "  FAIL $name ($reason)"
    FAILED+=("$name")
}

build_manifest_set() {
    local label="$1"
    local builder="$2"
    shift 2

    echo "Building $label..."
    if [ "$#" -eq 0 ]; then
        fail_build "$label" "no manifests found"
        return 0
    fi

    local manifest
    for manifest in "$@"; do
        "$builder" "$manifest"
    done
}

build_extension() {
    local manifest_path="$1"
    local hidden
    local source_dir
    local crate_name

    if ! hidden=$(jq -r '.hidden // false' "$manifest_path"); then
        fail_build "$(basename "$manifest_path" .json)" "could not read hidden flag"
        return 0
    fi
    if [ "$hidden" = "true" ]; then
        echo "  SKIP $(basename "$manifest_path" .json) (hidden registry entry)"
        return 0
    fi

    if ! source_dir=$(jq -r '.source.dir' "$manifest_path"); then
        fail_build "$(basename "$manifest_path" .json)" "could not read source dir"
        return 0
    fi
    if ! crate_name=$(jq -r '.source.crate_name' "$manifest_path"); then
        fail_build "$(basename "$manifest_path" .json)" "could not read crate name"
        return 0
    fi
    local name
    name=$(basename "$manifest_path" .json)

    if [ ! -d "$source_dir" ]; then
        echo "  SKIP $name (source dir $source_dir not found)"
        return 0
    fi

    echo "  BUILD $name ($crate_name) from $source_dir"
    if ! cargo_component_build --release --target wasm32-wasip2 --manifest-path "$source_dir/Cargo.toml" 2>&1; then
        fail_build "$name" "cargo component build failed"
        return 0
    fi
    echo "  OK   $name"
}

build_first_party_extension() {
    local manifest_path="$1"
    local extension_root
    local source_dir
    local module_path
    local runtime_kind
    local target_root
    local artifact
    local candidate
    local name

    extension_root=$(dirname "$manifest_path")
    source_dir="$extension_root/wasm-src"
    name="first-party/$(basename "$extension_root")"
    if ! runtime_kind=$(awk -F'"' '/^[[:space:]]*kind[[:space:]]*=/{ print $2; exit }' "$manifest_path"); then
        fail_build "$name" "could not read manifest"
        return 0
    fi
    if [ "$runtime_kind" != "wasm" ]; then
        echo "  SKIP $name (runtime kind $runtime_kind is host-native)"
        return 0
    fi
    if ! module_path=$(awk -F'"' '/^[[:space:]]*module[[:space:]]*=/{ print $2; exit }' "$manifest_path"); then
        fail_build "$name" "could not read manifest"
        return 0
    fi

    if [ ! -d "$source_dir" ]; then
        fail_build "$name" "source dir $source_dir not found"
        return 0
    fi
    if [ -z "$module_path" ]; then
        fail_build "$name" "manifest missing runtime module"
        return 0
    fi

    echo "  BUILD $name from $source_dir"
    if ! cargo_component_build --release --target wasm32-wasip2 --manifest-path "$source_dir/Cargo.toml" 2>&1; then
        fail_build "$name" "cargo component build failed"
        return 0
    fi
    target_root="${CARGO_TARGET_DIR:-$source_dir/target}"
    artifact=""
    for target_triple in wasm32-wasip2 wasm32-wasip1 wasm32-wasi; do
        candidate="$target_root/$target_triple/release/$(basename "$module_path")"
        if [ -f "$candidate" ]; then
            artifact="$candidate"
            break
        fi
    done
    if [ ! -f "$artifact" ]; then
        fail_build "$name" "expected artifact $(basename "$module_path") not found under $target_root"
        return 0
    fi

    if ! mkdir -p "$(dirname "$extension_root/$module_path")"; then
        fail_build "$name" "could not create output directory"
        return 0
    fi
    if ! cp "$artifact" "$extension_root/$module_path"; then
        fail_build "$name" "could not copy artifact"
        return 0
    fi
    echo "  OK   $name"
}

if $BUILD_TOOLS; then
    tool_manifests=(registry/tools/*.json)
    build_manifest_set "WASM tools" build_extension "${tool_manifests[@]}"
fi

if $BUILD_FIRST_PARTY; then
    first_party_manifests=(crates/ironclaw_first_party_extensions/assets/*/manifest.toml)
    build_manifest_set "first-party WASM extensions" build_first_party_extension "${first_party_manifests[@]}"
fi

if $BUILD_CHANNELS; then
    channel_manifests=(registry/channels/*.json)
    build_manifest_set "WASM channels" build_extension "${channel_manifests[@]}"
fi

echo ""
if [ ${#FAILED[@]} -gt 0 ]; then
    echo "FAILED: ${FAILED[*]}"
    exit 1
else
    echo "All WASM extensions built successfully."
fi
