#!/usr/bin/env bash
# Regression tests for the quality gate's test-runner selection.
#
# The gate must keep working on a machine without cargo-nextest
# (`tests/fixtures/llm_traces/README.md` documents that local development does
# not require it), must use nextest when it is available, and must honour an
# explicit override. Every cargo invocation here is stubbed, so this runs in
# milliseconds and compiles nothing.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GATE="$SCRIPT_DIR/quality_gate.sh"
failures=0

# Run the gate with a stubbed `cargo` (and optionally a stubbed
# `cargo-nextest`) on PATH, echoing each invocation to a log.
run_gate() {
    local with_nextest="$1"
    shift
    local sandbox
    sandbox="$(mktemp -d)"
    cat >"$sandbox/cargo" <<'STUB'
#!/usr/bin/env bash
echo "cargo $*" >>"$GATE_TEST_LOG"
STUB
    chmod +x "$sandbox/cargo"
    if [ "$with_nextest" = "with-nextest" ]; then
        cat >"$sandbox/cargo-nextest" <<'STUB'
#!/usr/bin/env bash
echo "cargo-nextest $*" >>"$GATE_TEST_LOG"
STUB
        chmod +x "$sandbox/cargo-nextest"
    fi
    # PATH is REPLACED, not prepended: a real cargo-nextest installed in the
    # developer's ~/.cargo/bin would otherwise satisfy `command -v` and make
    # the "nextest absent" case silently test the wrong branch.
    local status
    set +e
    env \
        GATE_TEST_LOG="$sandbox/log" \
        PATH="$sandbox:/usr/bin:/bin" \
        IRONCLAW_GATE_TEST_RUNNER=auto \
        IRONCLAW_PREPUSH_TEST=1 \
        "$@" bash "$GATE" >/dev/null 2>&1
    status=$?
    set -e
    cat "$sandbox/log" 2>/dev/null || true
    rm -rf "$sandbox"
    return "$status"
}

capture_gate() {
    set +e
    gate_log="$(run_gate "$@")"
    gate_status=$?
    set -e
}

expect_contains() {
    local label="$1" haystack="$2" needle="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        echo "ok   — $label"
    else
        echo "FAIL — $label"
        echo "       expected to find: $needle"
        echo "       in: $haystack"
        failures=$((failures + 1))
    fi
}

expect_not_contains() {
    local label="$1" haystack="$2" needle="$3"
    if [[ "$haystack" != *"$needle"* ]]; then
        echo "ok   — $label"
    else
        echo "FAIL — $label"
        echo "       expected NOT to find: $needle"
        failures=$((failures + 1))
    fi
}

expect_status() {
    local label="$1" actual="$2" expected="$3"
    if [ "$actual" -eq "$expected" ]; then
        echo "ok   — $label"
    else
        echo "FAIL — $label"
        echo "       expected status: $expected"
        echo "       actual status: $actual"
        failures=$((failures + 1))
    fi
}

expect_failure() {
    local label="$1" actual="$2"
    if [ "$actual" -ne 0 ]; then
        echo "ok   — $label"
    else
        echo "FAIL — $label"
        echo "       expected a non-zero status"
        failures=$((failures + 1))
    fi
}

capture_gate with-nextest
expect_status "nextest present: gate succeeds" "$gate_status" 0
expect_contains "nextest present: runs the parallel runner" "$gate_log" "cargo nextest run"
expect_not_contains "nextest present: does not also run cargo test" "$gate_log" "cargo test"

capture_gate without-nextest
expect_status "nextest absent: gate succeeds" "$gate_status" 0
expect_contains "nextest absent: falls back to cargo test" "$gate_log" "cargo test --locked --workspace"
expect_not_contains "nextest absent: does not invoke nextest" "$gate_log" "nextest run"
expect_contains "nextest absent: keeps --all-targets --all-features" "$gate_log" "--all-targets --all-features"

capture_gate with-nextest IRONCLAW_GATE_TEST_RUNNER=cargo
expect_status "override=cargo: gate succeeds" "$gate_status" 0
expect_contains "override=cargo: forces the sequential runner" "$gate_log" "cargo test --locked --workspace"
expect_not_contains "override=cargo: skips nextest even when installed" "$gate_log" "nextest run"

capture_gate with-nextest IRONCLAW_GATE_TEST_RUNNER=nextest
expect_status "override=nextest: gate succeeds" "$gate_status" 0
expect_contains "override=nextest: runs the parallel runner" "$gate_log" "cargo nextest run"
expect_not_contains "override=nextest: does not also run cargo test" "$gate_log" "cargo test"

capture_gate without-nextest IRONCLAW_PREPUSH_TEST=0
expect_status "IRONCLAW_PREPUSH_TEST=0: gate succeeds" "$gate_status" 0
expect_not_contains "IRONCLAW_PREPUSH_TEST=0: skips cargo test" "$gate_log" "cargo test"
expect_not_contains "IRONCLAW_PREPUSH_TEST=0: skips nextest" "$gate_log" "cargo nextest run"
expect_contains "IRONCLAW_PREPUSH_TEST=0: still runs clippy" "$gate_log" "clippy"

capture_gate without-nextest IRONCLAW_GATE_TEST_RUNNER=nextest
expect_failure "override=nextest: fails when cargo-nextest is absent" "$gate_status"

capture_gate with-nextest IRONCLAW_GATE_TEST_RUNNER=invalid
expect_failure "invalid runner: gate fails" "$gate_status"

# Coverage parity is the reason this swap is safe: `--all-targets` never
# included doctests, so neither runner loses them.
capture_gate with-nextest
expect_status "coverage parity: gate succeeds" "$gate_status" 0
expect_contains "nextest run keeps --all-targets --all-features" "$gate_log" "--all-targets --all-features"

if [ "$failures" -ne 0 ]; then
    echo "$failures check(s) failed"
    exit 1
fi
echo "all quality-gate runner checks passed"
