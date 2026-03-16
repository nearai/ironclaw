#!/usr/bin/env bash
set -euo pipefail

echo "==> fmt check"
cargo fmt --all -- --check

echo "==> clippy (correctness)"
cargo clippy --locked --all-targets -- -D clippy::correctness

if [ "${IRONCLAW_PREPUSH_TEST:-1}" = "1" ]; then
    echo "==> tests (skip with IRONCLAW_PREPUSH_TEST=0)"
    cargo test --locked --lib
fi
