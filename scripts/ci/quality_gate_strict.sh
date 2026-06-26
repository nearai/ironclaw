#!/usr/bin/env bash
set -euo pipefail

# Ensure we are running from the repository root
cd "$(git rev-parse --show-toplevel)"

require_command() {
    local command_name="$1"
    local install_hint="$2"
    if ! command -v "$command_name" &>/dev/null; then
        echo "ERROR: $command_name not installed ($install_hint)" >&2
        exit 1
    fi
}

echo "==> fmt check"
cargo fmt --all -- --check

echo "==> WebUI bundle toolchain"
require_command node "install Node.js 22+"
require_command npm "install npm with Node.js"
node --version
npm --version

echo "==> clippy (all warnings)"
cargo clippy --locked --all --benches --tests --examples --all-features -- -D warnings

echo "==> cargo deny"
require_command cargo-deny "install with: cargo install cargo-deny"
cargo deny check

echo "==> tests"
cargo test --locked
