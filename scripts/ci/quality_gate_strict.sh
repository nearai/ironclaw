#!/usr/bin/env bash
set -euo pipefail

# Ensure we are running from the repository root
cd "$(git rev-parse --show-toplevel)"

echo "==> fmt check"
cargo fmt --all -- --check

echo "==> clippy (all warnings)"
cargo clippy --locked --all --benches --tests --examples --all-features -- -D warnings

echo "==> cargo deny"
if ! command -v cargo-deny &>/dev/null; then
    echo "ERROR: cargo-deny not installed (install with: cargo install cargo-deny)"
    exit 1
fi
cargo deny check

echo "==> tests"
cargo test --locked
