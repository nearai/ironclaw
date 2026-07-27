#!/usr/bin/env bash
set -euo pipefail

echo "==> fmt check"
cargo fmt --all -- --check

run_cargo_ci() {
    env \
        -u NEARAI_API_KEY \
        -u NEARAI_BASE_URL \
        -u NEARAI_SESSION_TOKEN \
        -u NEARAI_PROVIDER_ID \
        -u NEARAI_MODEL \
        -u IRONCLAW_LLM_PROVIDER \
        -u IRONCLAW_LLM_MODEL \
        -u LLM_BACKEND \
        IRONCLAW_DISABLE_OS_KEYCHAIN="${IRONCLAW_DISABLE_OS_KEYCHAIN:-1}" \
        CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}" \
        CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}" \
        RUST_MIN_STACK="${RUST_MIN_STACK:-67108864}" \
        "$@"
}

echo "==> clippy (CI parity: all features, all warnings)"
run_cargo_ci cargo clippy --locked --all --tests --examples --all-features -- -D warnings

if [ "${IRONCLAW_PREPUSH_TEST:-1}" = "1" ]; then
    echo "==> tests (CI parity: workspace, all targets, all features; skip with IRONCLAW_PREPUSH_TEST=0)"
    run_cargo_ci cargo test --locked --workspace --all-targets --all-features --no-fail-fast
fi
