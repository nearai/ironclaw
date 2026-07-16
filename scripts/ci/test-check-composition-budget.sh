#!/usr/bin/env bash
#
# Regression tests for check-composition-budget.sh (the composition mass ratchet).
#
# Standalone: bash scripts/ci/test-check-composition-budget.sh
# Also run in CI (.github/workflows/code_style.yml) whenever the gate, its
# budget file, or this test changes — guardrails are code (.claude/rules/
# review-discipline.md: "Checks and hooks need regression tests ... and must run
# when their own files change").
#
# Each case builds a throwaway fixture tree with known LOC and a fixture budget
# file, points the gate at them via COMPOSITION_SRC / CRATES_ROOT / BUDGET_FILE,
# and asserts the exit code + key output lines.

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
gate="${repo_root}/scripts/ci/check-composition-budget.sh"

PASS=0
FAIL=0
CAP_OUT=""
CAP_RC=0

capture() { CAP_OUT="$("$@" 2>&1)"; CAP_RC=$?; }

assert_rc() {
    local name="$1" want="$2" got="$3"
    if [ "${got}" -eq "${want}" ]; then PASS=$((PASS+1));
    else FAIL=$((FAIL+1)); echo "FAIL: ${name} — expected exit ${want}, got ${got}"; echo "----"; echo "${CAP_OUT}"; echo "----"; fi
}

assert_contains() {
    local name="$1" hay="$2" needle="$3"
    if printf '%s' "${hay}" | grep -qF -- "${needle}"; then PASS=$((PASS+1));
    else FAIL=$((FAIL+1)); echo "FAIL: ${name} — output missing: ${needle}"; echo "----"; echo "${hay}"; echo "----"; fi
}

assert_not_contains() {
    local name="$1" hay="$2" needle="$3"
    if printf '%s' "${hay}" | grep -qF -- "${needle}"; then FAIL=$((FAIL+1)); echo "FAIL: ${name} — output should NOT contain: ${needle}";
    else PASS=$((PASS+1)); fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

# ---------------------------------------------------------------------------
# Fixture builder: a crates root with composition + one other crate, sized to
# an exact share. comp_lines / (comp_lines+other_lines) is the observed share.
# ---------------------------------------------------------------------------
make_fixture() {
    local dir="$1" comp_lines="$2" other_lines="$3"
    rm -rf "${dir}"
    mkdir -p "${dir}/ironclaw_reborn_composition/src" "${dir}/other_crate/src"
    yes 'let _ = 1;' | head -n "${comp_lines}"  > "${dir}/ironclaw_reborn_composition/src/lib.rs"
    yes 'let _ = 2;' | head -n "${other_lines}" > "${dir}/other_crate/src/lib.rs"
}

# 3000 comp / (3000+7000) = 30.00% = 3000 bp
make_fixture "${tmp}/crates" 3000 7000

budget() {  # write a budget file: enforce ceiling tolerance
    cat > "${tmp}/budget.toml" <<EOF
[gate]
enforce = $1
ceiling_bp = $2
tolerance_bp = $3
observed_bp = $2
observed_date = "2026-07-16"
EOF
}

run_gate() {
    COMPOSITION_SRC="${tmp}/crates/ironclaw_reborn_composition/src" \
    CRATES_ROOT="${tmp}/crates" \
    BUDGET_FILE="${tmp}/budget.toml" \
    capture bash "${gate}"
}

# C1: observed 3000bp, ceiling 3000 + tol 30 -> effective 3030, within budget.
budget true 3000 30; run_gate
assert_rc       "C1 within budget exits 0" 0 "${CAP_RC}"
assert_contains "C1 reports OK"            "${CAP_OUT}" "OK: composition share within budget"
assert_contains "C1 shows observed share"  "${CAP_OUT}" "30.00% (3000 bp)"

# C2: observed 3000bp, ceiling 2900 + tol 30 -> effective 2930, BREACH, enforcing.
budget true 2900 30; run_gate
assert_rc       "C2 breach (enforce) exits 1"  1 "${CAP_RC}"
assert_contains "C2 reports BUDGET EXCEEDED"   "${CAP_OUT}" "BUDGET EXCEEDED"
assert_contains "C2 names carve-out guidance"  "${CAP_OUT}" "ironclaw-reborn-architecture-review"

# C3: same breach but DRY-RUN -> exit 0, prefixed marker, no hard fail.
budget false 2900 30; run_gate
assert_rc       "C3 breach (dry-run) exits 0"  0 "${CAP_RC}"
assert_contains "C3 marks would-fail"          "${CAP_OUT}" "[dry-run, would FAIL]"
assert_contains "C3 banner shows DRY-RUN"      "${CAP_OUT}" "DRY-RUN"

# C4: observed 3000bp exactly at effective ceiling (ceiling 2970 + tol 30 = 3000) -> inclusive pass.
budget true 2970 30; run_gate
assert_rc       "C4 boundary inclusive exits 0" 0 "${CAP_RC}"
assert_contains "C4 reports OK"                 "${CAP_OUT}" "OK: composition share within budget"

# C5: down-ratchet nudge when observed is >1pp under the ceiling (ceiling 3200, obs 3000 -> 2pp slack).
budget true 3200 30; run_gate
assert_rc       "C5 well-under exits 0"   0 "${CAP_RC}"
assert_contains "C5 emits down-ratchet nudge" "${CAP_OUT}" "NUDGE:"

# C6: no nudge when slack is small (ceiling 3050 -> 0.5pp slack, under the 1pp threshold).
budget true 3050 30; run_gate
assert_rc          "C6 small-slack exits 0" 0 "${CAP_RC}"
assert_not_contains "C6 no nudge"            "${CAP_OUT}" "NUDGE:"

# C7: schema error — non-integer ceiling — always exits 1, even dry-run.
cat > "${tmp}/budget.toml" <<'EOF'
[gate]
enforce = false
ceiling_bp = twenty
tolerance_bp = 30
EOF
run_gate
assert_rc       "C7 bad ceiling exits 1"  1 "${CAP_RC}"
assert_contains "C7 reports schema error" "${CAP_OUT}" "ceiling_bp must be an integer"

# C8: schema error — bad enforce value.
cat > "${tmp}/budget.toml" <<'EOF'
[gate]
enforce = maybe
ceiling_bp = 3000
tolerance_bp = 30
EOF
run_gate
assert_rc       "C8 bad enforce exits 1"  1 "${CAP_RC}"
assert_contains "C8 reports enforce error" "${CAP_OUT}" "enforce must be true or false"

# C9: --print never fails and reports the share.
budget true 100 0; run_gate  # ceiling absurdly low, but --print ignores it
COMPOSITION_SRC="${tmp}/crates/ironclaw_reborn_composition/src" \
CRATES_ROOT="${tmp}/crates" \
BUDGET_FILE="${tmp}/budget.toml" \
capture bash "${gate}" --print
assert_rc       "C9 --print exits 0"      0 "${CAP_RC}"
assert_contains "C9 --print shows share"  "${CAP_OUT}" "composition share: 30.00%"

# C10: guard against committing a red gate — the REAL repo budget file must pass
#      against the REAL tree right now.
capture bash "${gate}"
assert_rc       "C10 real tree within committed budget" 0 "${CAP_RC}"

echo ""
echo "composition-budget gate tests: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
