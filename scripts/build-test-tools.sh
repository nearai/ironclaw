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
#   1. builds wasm-src/ with `cargo build --release --target wasm32-wasip1`
#   2. copies the artifact into <tool>/wasm/ (the path the manifest declares)
#   3. zips manifest.toml + wasm/ + schemas/ + prompts/ into test-tools/<tool>.zip
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

rustup target list --installed | grep -q '^wasm32-wasip1$' \
    || { echo "missing target: run 'rustup target add wasm32-wasip1'" >&2; exit 1; }

for tool in "${tools[@]}"; do
    tool_dir="$tools_root/$tool"
    [ -f "$tool_dir/manifest.toml" ] || { echo "no such tool: $tool" >&2; exit 1; }

    # 1. Build the WASM module.
    (cd "$tool_dir/wasm-src" && cargo build --release --target wasm32-wasip1)

    # 2. Copy the artifact to the path the manifest's [runtime].module declares.
    crate_name="$(sed -n 's/^name = "\(.*\)"/\1/p' "$tool_dir/wasm-src/Cargo.toml" | head -1)"
    artifact="$tool_dir/wasm-src/target/wasm32-wasip1/release/${crate_name//-/_}.wasm"
    module_rel="$(sed -n 's/^module = "\(.*\)"/\1/p' "$tool_dir/manifest.toml" | head -1)"
    [ -f "$artifact" ] || { echo "$tool: build artifact not found: $artifact" >&2; exit 1; }
    [ -n "$module_rel" ] || { echo "$tool: manifest declares no [runtime].module" >&2; exit 1; }
    mkdir -p "$tool_dir/$(dirname "$module_rel")"
    cp "$artifact" "$tool_dir/$module_rel"

    # 3. Zip the uploadable bundle.
    rm -f "$tools_root/$tool.zip"
    (cd "$tool_dir" && zip -q -r "../$tool.zip" manifest.toml wasm schemas prompts -x "*.DS_Store")
    echo "$tool: built $module_rel and $tool.zip"
done
