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

# Which runner executes the workspace tests.
#
# `cargo test` runs each test binary strictly sequentially — it parallelises
# *within* a binary but never across them. This workspace builds well over 400
# test binaries, the large majority finishing in under a second, so the gate
# spends most of its wall clock starting and tearing down processes one at a
# time. `cargo nextest` runs them in a single parallel pool instead.
#
# Coverage is equivalent: `--all-targets` expands to
# `--lib --bins --tests --benches --examples` and so has never included
# doctests, which nextest also does not run. This swap therefore changes
# scheduling only, not what is executed.
#
# nextest stays OPTIONAL. `tests/fixtures/llm_traces/README.md` documents that
# local development must keep working without it, so an absent binary falls
# back rather than failing. `.config/nextest.toml` already carries the
# repository's profiles and per-test slow-timeouts; CI's insta gate uses them.
#
#   auto (default) — nextest when installed, otherwise cargo test
#   nextest        — require nextest, fail if missing
#   cargo          — force the sequential runner
select_test_runner() {
    case "${IRONCLAW_GATE_TEST_RUNNER:-auto}" in
        cargo) echo "cargo" ;;
        nextest)
            if command -v cargo-nextest >/dev/null 2>&1; then
                echo "nextest"
            else
                echo "IRONCLAW_GATE_TEST_RUNNER=nextest requires cargo-nextest" >&2
                return 1
            fi
            ;;
        auto)
            if command -v cargo-nextest >/dev/null 2>&1; then
                echo "nextest"
            else
                echo "cargo"
            fi
            ;;
        *)
            echo "unknown IRONCLAW_GATE_TEST_RUNNER: ${IRONCLAW_GATE_TEST_RUNNER}" >&2
            return 1
            ;;
    esac
}

if [ "${IRONCLAW_PREPUSH_TEST:-1}" = "1" ]; then
    runner="$(select_test_runner)"
    if [ "$runner" = "nextest" ]; then
        echo "==> tests (nextest: workspace, all targets, all features; skip with IRONCLAW_PREPUSH_TEST=0)"
        run_cargo_ci cargo nextest run --locked --workspace --all-targets --all-features --no-fail-fast
    else
        echo "==> tests (cargo test: workspace, all targets, all features; skip with IRONCLAW_PREPUSH_TEST=0)"
        echo "    install cargo-nextest for a parallel run: https://nexte.st/docs/installation/"
        run_cargo_ci cargo test --locked --workspace --all-targets --all-features --no-fail-fast
    fi
fi
