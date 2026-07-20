#!/usr/bin/env bash
#
# build-test-tools.sh — build the test-tools/ WASM fixture bundles.
#
# Each test-tools/<tool>/ directory is a standalone uploadable extension
# bundle (manifest.toml + wasm/ + schemas/ + prompts/) used to exercise the
# WebUI v2 "Import Tool" flow (`POST /api/webchat/v2/extensions/import`)
# during live QA. See test-tools/README.md for the tool matrix.
#
# For each tool this script:
#   1. builds wasm-src/ with `cargo build --release --target wasm32-wasip2`
#      (wasip2 emits a WASI COMPONENT directly — the runtime loads tools via
#      `wasmtime::component::Component::new`, so a wasip1 core module fails at
#      dispatch with the redacted "the tool manifest is invalid")
#   2. verifies the artifact really is a component (layer bytes), not a core module
#   3. copies the artifact into <tool>/wasm/ (the path the manifest declares)
#   4. zips manifest.toml + wasm/ + schemas/ + prompts/ into test-tools/<tool>.zip
#
# The .zip files and wasm-src/target/ are git-ignored build artifacts.
#
# Usage: bash scripts/build-test-tools.sh [tool ...]
#        (no args = all tools)
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
tools_root="$repo_root/test-tools"

tools=("$@")
if [ ${#tools[@]} -eq 0 ]; then
    for dir in "$tools_root"/*/; do
        [ -f "$dir/manifest.toml" ] && tools+=("$(basename "$dir")")
    done
fi

rustup target list --installed | grep -q '^wasm32-wasip2$' \
    || { echo "missing target: run 'rustup target add wasm32-wasip2'" >&2; exit 1; }

# WASI component vs core module: bytes 4-7 after the `\0asm` magic are
# version+layer — `0d 00 01 00` marks a component, `01 00 00 00` a core module.
require_component() {
    local file="$1" label="$2"
    local header
    header="$(head -c 8 "$file" | od -An -tx1 | tr -d ' \n')"
    # `\0asm` magic + layer bytes `01 00` at offsets 6-7 mark a component
    # (version may move, the layer marker is the discriminator).
    case "$header" in
        0061736d????0100) ;;
        *) echo "$label: not a WASI component (header: $header) — the runtime requires a component" >&2
           exit 1 ;;
    esac
}

for tool in "${tools[@]}"; do
    tool_dir="$tools_root/$tool"
    [ -f "$tool_dir/manifest.toml" ] || { echo "no such tool: $tool" >&2; exit 1; }

    # 1. Build the WASM component (wasip2 componentizes via wasm-component-ld).
    (cd "$tool_dir/wasm-src" && cargo build --release --target wasm32-wasip2)

    # 2. Verify + copy the artifact to the manifest's [runtime].module path.
    crate_name="$(sed -n 's/^name = "\(.*\)"/\1/p' "$tool_dir/wasm-src/Cargo.toml" | head -1)"
    artifact="$tool_dir/wasm-src/target/wasm32-wasip2/release/${crate_name//-/_}.wasm"
    module_rel="$(sed -n 's/^module = "\(.*\)"/\1/p' "$tool_dir/manifest.toml" | head -1)"
    [ -f "$artifact" ] || { echo "$tool: build artifact not found: $artifact" >&2; exit 1; }
    [ -n "$module_rel" ] || { echo "$tool: manifest declares no [runtime].module" >&2; exit 1; }
    require_component "$artifact" "$tool"
    mkdir -p "$tool_dir/$(dirname "$module_rel")"
    cp "$artifact" "$tool_dir/$module_rel"

    # 3. Zip the uploadable bundle.
    rm -f "$tools_root/$tool.zip"
    (cd "$tool_dir" && zip -q -r "../$tool.zip" manifest.toml wasm schemas prompts -x "*.DS_Store")
    echo "$tool: built $module_rel (component) and $tool.zip"
done
