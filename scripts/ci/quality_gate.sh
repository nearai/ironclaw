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

# See scripts/ci/lib/select-test-runner.sh for the runner-selection
# rationale and contract (shared with every CI runner script).
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/select-test-runner.sh"

if [ "${IRONCLAW_PREPUSH_TEST:-1}" = "1" ]; then
    runner="$(select_test_runner optional)"
    if [ "$runner" = "nextest" ]; then
        echo "==> tests (nextest: workspace, all targets, all features; skip with IRONCLAW_PREPUSH_TEST=0)"
        run_cargo_ci cargo nextest run --locked --workspace --all-targets --all-features --no-fail-fast
    else
        echo "==> tests (cargo test: workspace, all targets, all features; skip with IRONCLAW_PREPUSH_TEST=0)"
        echo "    install cargo-nextest for a parallel run: https://nexte.st/docs/installation/"
        run_cargo_ci cargo test --locked --workspace --all-targets --all-features --no-fail-fast
    fi
fi
