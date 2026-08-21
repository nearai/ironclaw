#!/usr/bin/env bash
# Self-test for scripts/preflight-gates.sh modes (--ci, --queue-shape, default).
# Every gate command is stubbed inside a throwaway git repo, so this runs in
# milliseconds and compiles nothing. PATH is REPLACED, not prepended, so a real
# cargo/cargo-nextest on the developer's machine cannot leak into the sandbox
# (same stance as scripts/ci/test-quality-gate-runner.sh).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
failures=0

make_sandbox() {
    sandbox="$(mktemp -d)"
    (
        cd "$sandbox"
        git init -q -b main .
        mkdir -p scripts/ci bin
        cp "$REPO_ROOT/scripts/preflight-gates.sh" scripts/preflight-gates.sh
        # Stub every script gate preflight names.
        for stub in scripts/ci/check-composition-budget.sh \
                    scripts/ci/check-include-str-paths.sh \
                    scripts/ci/check-hermetic-env.sh; do
            printf '#!/usr/bin/env bash\necho "ran ${0##*/}" >>"$PREFLIGHT_TEST_LOG"\n' >"$stub"
            chmod +x "$stub"
        done
        for stub in scripts/check_no_panics.py scripts/ci/check-target-tree.py \
                    scripts/ci/check-guidance.py scripts/ci/docs_publication_boundary.py \
                    scripts/ci/reborn_pr_test_plan.py; do
            mkdir -p "$(dirname "$stub")"
            printf 'import os,sys\nopen(os.environ["PREFLIGHT_TEST_LOG"],"a").write("ran %%s %%s\\n" %% (os.path.basename(sys.argv[0]), " ".join(sys.argv[1:])))\nif os.path.basename(sys.argv[0]) == "reborn_pr_test_plan.py":\n    print("{\\"crate_buckets\\":[{\\"name\\":\\"reborn-core\\",\\"packages\\":[\\"a\\"]}],\\"root_partitions\\":[0,1,2,3],\\"integration_lanes\\":[0,1,2,3,\\"groups\\"],\\"run_group_tests\\":true,\\"run_qa_replay\\":true,\\"run_sandbox_docker\\":true}")\n' >"$stub"
        done
        # Stub cargo: logs argv; exits per PREFLIGHT_FAIL_MATCH.
        cat >bin/cargo <<'STUB'
#!/usr/bin/env bash
echo "cargo $*" >>"$PREFLIGHT_TEST_LOG"
if [ -n "${PREFLIGHT_FAIL_MATCH:-}" ] && [[ "$*" == *"$PREFLIGHT_FAIL_MATCH"* ]]; then
    exit 1
fi
STUB
        chmod +x bin/cargo
        # python3 must stay real (the planner stub is a real python file), but
        # git/bash/env come from /usr/bin:/bin.
        git add -A >/dev/null 2>&1 || true
    )
    echo "$sandbox"
}

run_preflight() {
    local sandbox="$1"; shift
    set +e
    # BUG FIXED (gate-audit revision, fix 4): "${extra_env[@]:-}" passes ONE
    # empty-string argument to `env` whenever extra_env is a declared-but-empty
    # array (bash's :- operator treats a zero-element array as "unset or null"
    # and substitutes the default, here nothing, AS one word) — env then tries
    # to exec "" as the command and the real invocation never runs at all
    # (rc=127, "env: '': No such file or directory" — reproduced live against
    # this exact shape before landing the fix). ${arr[@]+"${arr[@]}"} emits
    # ZERO words when the array is empty and the array's own elements,
    # correctly quoted, when it is not — verified both cases live.
    output="$(cd "$sandbox" && \
        env PREFLIGHT_TEST_LOG="$sandbox/log" \
            PATH="$sandbox/bin:/usr/bin:/bin" \
            ${extra_env[@]+"${extra_env[@]}"} \
            bash scripts/preflight-gates.sh "$@" 2>&1)"
    status=$?
    set -e
}

expect() { # label, haystack, needle
    if ! grep -qF -- "$3" <<<"$2"; then
        echo "FAIL: $1 — expected to find: $3" >&2
        failures=$((failures + 1))
    fi
}
expect_absent() {
    if grep -qF -- "$3" <<<"$2"; then
        echo "FAIL: $1 — expected NOT to find: $3" >&2
        failures=$((failures + 1))
    fi
}

# 1. Default mode: no GitHub annotations, gates run, exit 0 when all green,
#    and (unrelated to --ci) Layer 2's nextest-absent fallback still fires —
#    real behavior default mode must keep, now asserted here rather than
#    inside a --ci case (Layer 2 no longer runs under --ci at all).
sandbox="$(make_sandbox)"; extra_env=()
run_preflight "$sandbox"
[ "$status" -eq 0 ] || { echo "FAIL: default green run exited $status" >&2; failures=$((failures+1)); }
expect "default runs fmt" "$(cat "$sandbox/log")" "cargo fmt --all -- --check"
expect_absent "default has no ::group::" "$output" "::group::"
expect "default Layer 2 falls back to cargo test (nextest absent)" "$(cat "$sandbox/log")" \
    "cargo test -p ironclaw_architecture_tests --no-fail-fast"

# 2. --ci: ::group:: per gate, ::error + REPRO on failure, later gates still
#    run, Layer 2 AND Layer 3 are both skipped with a printed reason
#    (fix 3 — Layer 1 only), exit 1 on any failure.
sandbox="$(make_sandbox)"
extra_env=(PREFLIGHT_FAIL_MATCH="fmt")
run_preflight "$sandbox" --ci
[ "$status" -eq 1 ] || { echo "FAIL: --ci with failing fmt exited $status" >&2; failures=$((failures+1)); }
expect "--ci groups output" "$output" "::group::fmt check"
expect "--ci endgroup" "$output" "::endgroup::"
expect "--ci error annotation" "$output" "::error title=preflight gate failed::fmt check"
expect "--ci prints the exact repro command" "$output" "REPRO: cargo fmt --all -- --check"
expect "--ci keeps going after a failure" "$(cat "$sandbox/log")" "ran check-guidance.py"
expect_absent "--ci does NOT run the architecture suite" "$(cat "$sandbox/log")" "ironclaw_architecture_tests"
expect "--ci skips Layer 2 loudly with a reason" "$output" "architecture suite: skipped under --ci"
expect "--ci skips charters loudly" "$output" "charter tests: skipped under --ci"
expect "--ci verdict lists the failure" "$output" "1 gate(s) FAILED"

# 3. --ci all green exits 0 and prints the OK verdict.
sandbox="$(make_sandbox)"; extra_env=()
run_preflight "$sandbox" --ci
[ "$status" -eq 0 ] || { echo "FAIL: --ci green run exited $status" >&2; failures=$((failures+1)); }
expect "--ci OK verdict" "$output" "preflight-gates: OK"

# 4. Unknown flag fails loudly (no silent no-op modes) — this is the case
#    that catches "the script currently ignores argv entirely" for real:
#    verified live against the unmodified script that `--nonsense` today runs
#    the full default gate list rather than rejecting the flag.
sandbox="$(make_sandbox)"; extra_env=()
run_preflight "$sandbox" --nonsense
[ "$status" -eq 2 ] || { echo "FAIL: unknown flag exited $status, want 2" >&2; failures=$((failures+1)); }
expect "unknown flag names itself" "$output" "unknown option: --nonsense"

# 5. --queue-shape runs exactly the three queue clippy shapes + the planner
#    full-plan listing, and NONE of the default gates.
sandbox="$(make_sandbox)"; extra_env=()
run_preflight "$sandbox" --queue-shape
[ "$status" -eq 0 ] || { echo "FAIL: --queue-shape green run exited $status" >&2; failures=$((failures+1)); }
log="$(cat "$sandbox/log")"
expect "queue clippy shape 1: all-targets all-features" "$log" \
    "cargo clippy --locked --all --tests --examples --all-features -- -D warnings"
expect "queue clippy shape 2: all-targets default-features" "$log" \
    "cargo clippy --locked --all --tests --examples -- -D warnings"
expect "queue clippy shape 3: production-targets default-features" "$log" \
    "cargo clippy --locked --all --lib --bins -- -D warnings"
expect "planner full-plan listing invoked" "$log" \
    "reborn_pr_test_plan.py --event merge_group"
expect "bucket list rendered" "$output" "reborn-core"
expect_absent "queue-shape skips default gates" "$log" "cargo fmt --all -- --check"

# 6. --queue-shape collects all three clippy failures in one lap.
sandbox="$(make_sandbox)"
extra_env=(PREFLIGHT_FAIL_MATCH="clippy")
run_preflight "$sandbox" --queue-shape
[ "$status" -eq 1 ] || { echo "FAIL: --queue-shape with failing clippy exited $status" >&2; failures=$((failures+1)); }
expect "all three shapes reported" "$output" "3 gate(s) FAILED"

if [ "$failures" -gt 0 ]; then
    echo "test-preflight-gates: $failures assertion(s) failed" >&2
    exit 1
fi
echo "test-preflight-gates: OK"
