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

failures=()

run_gate() {
    local label="$1"
    shift
    echo "==> ${label}"
    if ! "$@"; then
        failures+=("${label}")
        echo "    FAILED: ${label}"
    fi
}

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
if command -v cargo-nextest >/dev/null 2>&1; then
    run_gate "architecture suite (nextest)" \
        cargo nextest run -p ironclaw_architecture_tests --no-fail-fast
else
    run_gate "architecture suite (cargo; install cargo-nextest to parallelize)" \
        cargo test -p ironclaw_architecture_tests --no-fail-fast
fi

# --- Layer 3: module-charter tests for crates the diff touches -------------
# Charter maps live inside their crates, so only changed crates pay the
# compile. Diff base mirrors the pre-push hook's default (origin/main).
# Discovery fails CLOSED: when the base is missing or the diff plumbing
# errors, the fallback widens to all five charters — a broken setup may cost
# compile time, never a silent skip.
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
