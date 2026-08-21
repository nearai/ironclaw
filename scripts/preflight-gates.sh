#!/usr/bin/env bash
# PROPOSED (gate audit 2026-08, docs/internal/gate-audit-2026-08.md §4): the
# cheap deterministic pre-push gauntlet. NOT wired into any hook or workflow —
# run it by hand before pushing:
#
#   bash scripts/preflight-gates.sh
#
# Scope: exactly the gate classes that produced PR #7157's six red CI runs —
# the script gates (composition budget, panic baseline, target tree, guidance,
# include_str!, docs boundary), the architecture suite (contracts size
# ceilings and every other reborn_* gate), and the module-charter tests of
# crates your diff touches. It deliberately runs NO ordinary unit/integration
# tests — `.githooks/pre-push` (quality_gate.sh) remains the full CI-parity
# gate; this script is the five-minute version that catches the
# deterministic-gate class.
#
# Measured on a warm target dir (2026-08-07, M-series mac): script layer ~10s,
# architecture suite ~4.2min (cargo) — nextest is used when installed and cuts
# the suite further by running the 37 binaries in parallel. Charter tests are
# compile-dominated (~80s for ironclaw_webui warm, near-zero when unchanged).
# Cold-target first runs pay the usual build cost on top.
#
# Every gate runs even after an earlier one fails (the point is ONE round-trip
# reporting EVERYTHING — CI's bucket shape stops at the first failing binary);
# the script exits non-zero if any gate failed.

set -uo pipefail

# Deliberately no `-e`: the gate-running section must keep going after a
# failing gate (one round-trip, full report). The setup below therefore
# checks its own plumbing explicitly — a setup failure must never let the
# script report OK over work it didn't do.
REPO_ROOT="$(git rev-parse --show-toplevel)" || {
    echo "preflight-gates: not inside a git repository" >&2
    exit 2
}
cd "${REPO_ROOT}" || exit 2

ci_mode=0
queue_shape=0
for arg in "$@"; do
    case "${arg}" in
        --ci) ci_mode=1 ;;
        --queue-shape) queue_shape=1 ;;
        *)
            echo "preflight-gates: unknown option: ${arg}" >&2
            echo "usage: preflight-gates.sh [--ci] [--queue-shape]" >&2
            exit 2
            ;;
    esac
done

failures=()

run_gate() {
    local label="$1"
    shift
    if [ "${ci_mode}" -eq 1 ]; then
        echo "::group::${label}"
    else
        echo "==> ${label}"
    fi
    if ! "$@"; then
        failures+=("${label}")
        echo "    FAILED: ${label}"
        # REPRO invariant (Global Constraints): this is literally "$@", the
        # array run_gate was handed — it cannot drift from what actually ran.
        echo "    REPRO: $(printf '%q ' "$@")"
        if [ "${ci_mode}" -eq 1 ]; then
            echo "::error title=preflight gate failed::${label}"
        fi
    fi
    if [ "${ci_mode}" -eq 1 ]; then
        echo "::endgroup::"
    fi
}

# Mirrors scripts/ci/quality_gate.sh:7-22's env scrub exactly (two copies —
# below the rule-of-three threshold this repo's own discipline uses to decide
# when duplication earns an extraction; if a third caller appears, factor
# both into scripts/ci/lib/run-cargo-ci-env.sh).
run_cargo_ci_scrubbed() {
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

queue_plan_listing() {
    python3 - <<'PY'
import json
import subprocess
import sys

result = subprocess.run(
    ["python3", "scripts/ci/reborn_pr_test_plan.py", "--event", "merge_group"],
    capture_output=True,
    text=True,
)
if result.returncode != 0:
    sys.stderr.write(result.stderr)
    sys.exit(1)
plan = json.loads(result.stdout)
print("merge-queue full plan (what the queue will run that this PR did not):")
for bucket in plan["crate_buckets"]:
    print(f"  crate bucket {bucket['name']}: {len(bucket['packages'])} packages")
print(f"  root partitions: {plan['root_partitions']}")
print(f"  integration lanes: {plan['integration_lanes']}")
for key in ("run_group_tests", "run_qa_replay", "run_sandbox_docker"):
    print(f"  {key}: {plan[key]}")
PY
}

if [ "${queue_shape}" -eq 1 ]; then
    # The three shapes the merge queue runs that PR CI skips (#7119 class;
    # code_style.yml's queue/push branch, matrix flavors all-features/default,
    # verified live 2026-08-21 — see Evidence). --locked + the scrubbed env
    # match quality_gate.sh's CI-parity clippy exactly so a dirty local
    # Cargo.lock or ambient LLM-backend env can't false-green this tool.
    #
    # Honesty about cost: the all-features shape alone measures ~5.8 min on
    # the warm 2-core CI runner; a cold local run of all three can take well
    # over 20 minutes and the --all-features shape needs Node 22 + pnpm for
    # the WebUI bundle build.
    run_gate "clippy (queue shape: all targets, all features)" \
        run_cargo_ci_scrubbed cargo clippy --locked --all --tests --examples --all-features -- -D warnings
    run_gate "clippy (queue shape: all targets, default features)" \
        run_cargo_ci_scrubbed cargo clippy --locked --all --tests --examples -- -D warnings
    run_gate "clippy (queue shape: production targets, default features)" \
        run_cargo_ci_scrubbed cargo clippy --locked --all --lib --bins -- -D warnings
    run_gate "planner full-plan listing (merge_group)" queue_plan_listing
    echo
    if [ "${#failures[@]}" -eq 0 ]; then
        echo "preflight-gates --queue-shape: OK — queue-only shapes green"
        exit 0
    fi
    echo "preflight-gates --queue-shape: ${#failures[@]} gate(s) FAILED:"
    for f in "${failures[@]}"; do
        echo "  - ${f}"
    done
    exit 1
fi

# --- Layer 1: script gates (seconds each) ----------------------------------
run_gate "fmt check" cargo fmt --all -- --check
run_gate "composition mass budget" bash scripts/ci/check-composition-budget.sh
run_gate "panic baseline (full closure scan)" \
    python3 scripts/check_no_panics.py --reborn-baseline
run_gate "target tree vs documented tree" python3 scripts/ci/check-target-tree.py
run_gate "guidance path references" python3 scripts/ci/check-guidance.py
run_gate "include_str! + Docker COPY coverage" bash scripts/ci/check-include-str-paths.sh
run_gate "hermetic env mutation (delta)" bash scripts/ci/check-hermetic-env.sh
run_gate "docs publication boundary" python3 scripts/ci/docs_publication_boundary.py

# --- Layer 2: the architecture gate suite ----------------------------------
if [ "${ci_mode}" -eq 1 ]; then
    echo "==> architecture suite: skipped under --ci — the crate-bucket lane" \
         "(reborn-tests.yml, 'Test Reborn crate bucket') already runs" \
         "ironclaw_architecture_tests with --all-targets whenever it is in the" \
         "affected closure; compiling it again here would be a second cold" \
         "build of the same ~40 binaries in fast-checks' one cache-less lane" \
         "on every PR, including guidance-only PRs that touch no crate at all." \
         "Known residual gap (accepted; see the PR body): a guidance-only PR" \
         "never runs this suite anywhere even though it pins guidance files —" \
         "routed to T3's planner track as a follow-up, not fixed here."
elif command -v cargo-nextest >/dev/null 2>&1; then
    run_gate "architecture suite (nextest)" \
        cargo nextest run -p ironclaw_architecture_tests --no-fail-fast
else
    run_gate "architecture suite (cargo; install cargo-nextest to parallelize)" \
        cargo test -p ironclaw_architecture_tests --no-fail-fast
fi

# --- Layer 3: module-charter tests for crates the diff touches -------------
if [ "${ci_mode}" -eq 1 ]; then
    echo "==> charter tests: skipped under --ci — they are --test targets of their" \
         "crates and the crate-bucket lane runs them with --all-targets when the" \
         "crate is in the affected closure (reborn-tests.yml, 'Run crate tests')"
else
    # Charter maps live inside their crates, so only changed crates pay the
    # compile. Diff base mirrors the pre-push hook's default (origin/main).
    # Discovery fails CLOSED: when the base is missing or the diff plumbing
    # errors, the fallback widens to all five charters — a broken setup may
    # cost compile time, never a silent skip.
    all_charter_crates="crates/product/ironclaw_webui crates/product/ironclaw_assistant \
             crates/domains/ironclaw_llm crates/domains/ironclaw_auth \
             crates/lanes/ironclaw_mcp"
    base_ref="${IRONCLAW_PREFLIGHT_BASE:-origin/main}"
    if git rev-parse --verify --quiet "${base_ref}^{commit}" >/dev/null; then
        if merge_base="$(git merge-base HEAD "${base_ref}")" \
            && changed="$(git diff --name-only "${merge_base}...HEAD")"; then
            :
        else
            echo "==> charter tests: cannot diff against ${base_ref}; running all five"
            changed="${all_charter_crates}"
        fi
    else
        echo "==> charter tests: base ${base_ref} not found; running all five"
        changed="${all_charter_crates}"
    fi

    charter() {
        local crate_dir="$1" package="$2" test_name="$3"
        shift 3
        if grep -q "${crate_dir}" <<<"${changed}"; then
            run_gate "charter: ${package}" \
                env "$@" cargo test -p "${package}" --test "${test_name}"
        fi
    }

    charter "crates/product/ironclaw_webui" ironclaw_webui handlers_module_charter \
        SKIP_FRONTEND_BUILD=1
    charter "crates/product/ironclaw_assistant" ironclaw_assistant \
        reborn_services_module_charter
    charter "crates/domains/ironclaw_llm" ironclaw_llm module_charter
    charter "crates/domains/ironclaw_auth" ironclaw_auth module_charter
    charter "crates/lanes/ironclaw_mcp" ironclaw_mcp module_charter
fi

# --- Verdict ----------------------------------------------------------------
echo
if [ "${#failures[@]}" -eq 0 ]; then
    echo "preflight-gates: OK — every deterministic gate green"
    exit 0
fi
echo "preflight-gates: ${#failures[@]} gate(s) FAILED:"
for f in "${failures[@]}"; do
    echo "  - ${f}"
done
exit 1
