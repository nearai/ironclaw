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

# --- Direct unit tests of the shared lib (scripts/ci/lib/select-test-runner.sh) ---
# Exercises the "require-in-ci" policy the quality_gate.sh end-to-end
# cases above never touch (quality_gate.sh always calls "optional").
LIB="$SCRIPT_DIR/lib/select-test-runner.sh"

run_lib() {
    local with_nextest="$1" policy="$2"
    shift 2
    local sandbox
    sandbox="$(mktemp -d)"
    cat >"$sandbox/cargo-nextest" <<'STUB'
#!/usr/bin/env bash
exit 0
STUB
    if [ "$with_nextest" = "with-nextest" ]; then
        chmod +x "$sandbox/cargo-nextest"
    else
        rm -f "$sandbox/cargo-nextest"
    fi
    # PATH is REPLACED, and CI is force-unset before any trailing "$@"
    # NAME=value overrides are applied -- `env -u CI ... CI=true ...`
    # unsets first, then a later CI=true argument re-adds it, so callers
    # opt IN to CI=true by passing it as a trailing arg rather than via a
    # `VAR=val function` prefix (which a shell function does not reliably
    # forward into a subsequently-`env`-invoked subprocess's environment).
    # The status is captured via an `if`, not a bare `set +e`/`set -e`
    # toggle: `set -e` is a global shell option, not function-scoped, so
    # flipping it back on inside this function would leak out and fight
    # whatever errexit state the *caller* had established around its own
    # call to run_lib.
    local status status_code
    if status="$(
        env -u CI PATH="$sandbox:/usr/bin:/bin" "$@" \
            bash -c "source '$LIB' && select_test_runner '$policy'"
    )"; then
        status_code=0
    else
        status_code=$?
    fi
    rm -rf "$sandbox"
    echo "$status"
    return "$status_code"
}

lib_out="$(run_lib with-nextest require-in-ci)"
expect_contains "require-in-ci, nextest present: picks nextest" "$lib_out" "nextest"

# CI unset explicitly (env -u CI): GitHub Actions runners export CI=true
# ambiently, so a case meaning "local reproduction, nextest absent" must
# unset it or it silently tests the wrong branch.
lib_out="$(run_lib without-nextest require-in-ci)"
expect_contains "require-in-ci, nextest absent, CI unset: falls back to cargo" "$lib_out" "cargo"

set +e
run_lib without-nextest require-in-ci CI=true >/dev/null
require_ci_status=$?
set -e
expect_failure "require-in-ci, nextest absent, CI=true: hard fails" "$require_ci_status"

lib_out="$(run_lib with-nextest optional CI=true)"
expect_contains "optional policy ignores CI=true when nextest is present" "$lib_out" "nextest"

# --- reborn-coverage-lane-run.sh's group-mode carve-out (Task 5, T2 plan) ---
# Decision 3: `group` mode must force the sequential cargo-test runner
# regardless of what select_test_runner would otherwise pick, so a
# nextest install in this job never silently pulls reborn_group_* suites
# into the parallel pool. Exercised end-to-end (stubbed cargo/nextest,
# stubbed suite discovery) rather than grepped, since the carve-out is
# inline control flow, not a separately-sourceable function.
run_lane_group_carveout() {
    local sandbox
    sandbox="$(mktemp -d)"
    cat >"$sandbox/cargo" <<'STUB'
#!/usr/bin/env bash
echo "cargo $*" >>"$GATE_TEST_LOG"
STUB
    chmod +x "$sandbox/cargo"
    cat >"$sandbox/cargo-nextest" <<'STUB'
#!/usr/bin/env bash
echo "cargo-nextest $*" >>"$GATE_TEST_LOG"
STUB
    chmod +x "$sandbox/cargo-nextest"
    mkdir -p "$sandbox/scripts/ci/lib"
    cp "$SCRIPT_DIR/reborn-coverage-lane-run.sh" "$sandbox/scripts/ci/reborn-coverage-lane-run.sh"
    cp "$SCRIPT_DIR/lib/select-test-runner.sh" "$sandbox/scripts/ci/lib/select-test-runner.sh"
    cat >"$sandbox/scripts/ci/reborn-coverage-int-tier-tests.sh" <<'STUB'
#!/usr/bin/env bash
printf -- '--test\nreborn_group_fixture\n'
STUB
    chmod +x "$sandbox/scripts/ci/reborn-coverage-int-tier-tests.sh"
    local status
    set +e
    status="$(
        env \
            GATE_TEST_LOG="$sandbox/log" \
            PATH="$sandbox:/usr/bin:/bin" \
            REBORN_COV_COLLECT=false \
            REBORN_COV_LANE_MODE=group \
            "$sandbox/scripts/ci/reborn-coverage-lane-run.sh" "$sandbox/unused.lcov" 2>&1
    )"
    set -e
    rm -rf "$sandbox"
    echo "$status"
}

lane_out="$(run_lane_group_carveout)"
expect_contains "group mode: forces cargo even with nextest present" "$lane_out" "cargo test -p ironclaw_integration_tests"
expect_not_contains "group mode: never invokes nextest" "$lane_out" "nextest run"

if [ "$failures" -ne 0 ]; then
    echo "$failures check(s) failed"
    exit 1
fi
echo "all quality-gate runner checks passed"
