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

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
gate="${repo_root}/scripts/ci/check-composition-budget.sh"

PASS=0
FAIL=0
CAP_OUT=""
CAP_RC=0

# Record output+exit without tripping errexit when the gate exits non-zero.
capture() { CAP_RC=0; CAP_OUT="$("$@" 2>&1)" || CAP_RC=$?; }

assert_rc() {
    local name="$1" want="$2" got="$3"
    if [ "${got}" -eq "${want}" ]; then PASS=$((PASS+1));
    else FAIL=$((FAIL+1)); echo "FAIL: ${name} — expected exit ${want}, got ${got}"; echo "----"; echo "${CAP_OUT}"; echo "----"; fi
}

# Pure-bash substring match — no pipes, so immune to SIGPIPE under pipefail.
assert_contains() {
    local name="$1" hay="$2" needle="$3"
    if [[ "${hay}" == *"${needle}"* ]]; then PASS=$((PASS+1));
    else FAIL=$((FAIL+1)); echo "FAIL: ${name} — output missing: ${needle}"; echo "----"; echo "${hay}"; echo "----"; fi
}

assert_not_contains() {
    local name="$1" hay="$2" needle="$3"
    if [[ "${hay}" == *"${needle}"* ]]; then FAIL=$((FAIL+1)); echo "FAIL: ${name} — output should NOT contain: ${needle}";
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
    # `|| true`: `yes | head` makes `yes` exit with SIGPIPE (141), which under
    # `set -e`+`pipefail` would abort the harness. The file is fully written.
    { yes 'let _ = 1;' | head -n "${comp_lines}";  } > "${dir}/ironclaw_reborn_composition/src/lib.rs" || true
    { yes 'let _ = 2;' | head -n "${other_lines}"; } > "${dir}/other_crate/src/lib.rs" || true
}

# 3000 comp / (3000+7000) = 30.00% = 3000 bp
make_fixture "${tmp}/crates" 3000 7000

budget() {  # enforce ceiling_bp tolerance_bp [arc_ceiling=0] [arc_tol=0]
    # arc_ceiling defaults to 0: mass-focused fixtures have no Arc<dyn>, so a
    # 0 ceiling neither breaches nor emits a dispatch nudge, isolating the mass
    # metric under test. Dispatch cases pass an explicit ceiling.
    cat > "${tmp}/budget.toml" <<EOF
[gate]
enforce = $1
ceiling_bp = $2
tolerance_bp = $3
observed_bp = $2
arc_dyn_ceiling = ${4:-0}
arc_dyn_tolerance = ${5:-0}
arc_dyn_observed = ${4:-0}
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
assert_contains "C1 reports OK"            "${CAP_OUT}" "OK: composition within mass + dispatch budget"
assert_contains "C1 shows observed share"  "${CAP_OUT}" "30.00% (3000 bp)"

# C2: observed 3000bp, ceiling 2900 + tol 30 -> effective 2930, BREACH, enforcing.
budget true 2900 30; run_gate
assert_rc       "C2 breach (enforce) exits 1"  1 "${CAP_RC}"
assert_contains "C2 reports MASS EXCEEDED"    "${CAP_OUT}" "MASS EXCEEDED"
assert_contains "C2 names carve-out guidance"  "${CAP_OUT}" "ironclaw-reborn-architecture-review"

# C3: same breach but DRY-RUN -> exit 0, prefixed marker, no hard fail.
budget false 2900 30; run_gate
assert_rc       "C3 breach (dry-run) exits 0"  0 "${CAP_RC}"
assert_contains "C3 marks would-fail"          "${CAP_OUT}" "[dry-run, would FAIL]"
assert_contains "C3 banner shows DRY-RUN"      "${CAP_OUT}" "DRY-RUN"

# C4: observed 3000bp exactly at effective ceiling (ceiling 2970 + tol 30 = 3000) -> inclusive pass.
budget true 2970 30; run_gate
assert_rc       "C4 boundary inclusive exits 0" 0 "${CAP_RC}"
assert_contains "C4 reports OK"                 "${CAP_OUT}" "OK: composition within mass + dispatch budget"

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

# C8b: MISSING key (not just bad value) must reach schema validation, not crash
# under set -e+pipefail (regression for the toml_get grep-fail abort).
cat > "${tmp}/budget.toml" <<'EOF'
[gate]
enforce = true
tolerance_bp = 30
EOF
run_gate
assert_rc       "C8b missing ceiling_bp exits 1"   1 "${CAP_RC}"
assert_contains "C8b reports schema error not crash" "${CAP_OUT}" "ceiling_bp must be an integer"

# C8c: test-only FILES are excluded from the metric. Add a big tests.rs to the
# composition fixture; the observed share must stay 30.00% (3000 bp), unchanged.
make_fixture "${tmp}/crates" 3000 7000
printf 'let _ = 9;\n%.0s' $(seq 1 5000) > "${tmp}/crates/ironclaw_reborn_composition/src/tests.rs"
budget true 3000 30; run_gate
assert_rc       "C8c test file excluded exits 0"   0 "${CAP_RC}"
assert_contains "C8c share ignores tests.rs"       "${CAP_OUT}" "30.00% (3000 bp)"
make_fixture "${tmp}/crates" 3000 7000  # restore clean fixture for later cases

# C9: --print never fails and reports the share.
budget true 100 0; run_gate  # ceiling absurdly low, but --print ignores it
COMPOSITION_SRC="${tmp}/crates/ironclaw_reborn_composition/src" \
CRATES_ROOT="${tmp}/crates" \
BUDGET_FILE="${tmp}/budget.toml" \
capture bash "${gate}" --print
assert_rc       "C9 --print exits 0"      0 "${CAP_RC}"
assert_contains "C9 --print shows share"  "${CAP_OUT}" "composition share: 30.00%"

# ---------------------------------------------------------------------------
# D. Dispatch (Arc<dyn>) sub-metric.
# ---------------------------------------------------------------------------
make_fixture "${tmp}/crates" 3000 7000
comp_src="${tmp}/crates/ironclaw_reborn_composition/src"
# 10 Arc<dyn> sites in a production file.
printf 'let x: Arc<dyn Foo> = y;\n%.0s' $(seq 1 10) > "${comp_src}/dispatch.rs"

# D1: arc_dyn 10, ceiling 10 + tol 0 -> within budget.
budget true 3000 30 10 0; run_gate
assert_rc       "D1 dispatch within exits 0"     0 "${CAP_RC}"
assert_contains "D1 shows dispatch count"        "${CAP_OUT}" "Arc<dyn> (excl slack/extension_host): 10"

# D2: ceiling 5 + tol 0 -> dispatch breach, enforcing.
budget true 3000 30 5 0; run_gate
assert_rc       "D2 dispatch breach exits 1"     1 "${CAP_RC}"
assert_contains "D2 reports DISPATCH EXCEEDED"   "${CAP_OUT}" "DISPATCH EXCEEDED"

# D3: same dispatch breach but DRY-RUN -> exit 0.
budget false 3000 30 5 0; run_gate
assert_rc       "D3 dispatch dry-run exits 0"    0 "${CAP_RC}"
assert_contains "D3 dispatch would-fail marker"  "${CAP_OUT}" "[dry-run, would FAIL]"

# D4: Arc<dyn> in slack/ and extension_host/ is NOT counted (separate workstream).
mkdir -p "${comp_src}/slack" "${comp_src}/extension_host"
printf 'Arc<dyn Bar>\n%.0s' $(seq 1 50) > "${comp_src}/slack/x.rs"
printf 'Arc<dyn Baz>\n%.0s' $(seq 1 50) > "${comp_src}/extension_host/y.rs"
# high mass ceiling: the slack/ext files add to the MASS count (which does not
# exclude them) — this case only asserts the DISPATCH exclusion.
budget true 4000 30 10 0; run_gate
assert_rc       "D4 slack/ext dispatch excluded exits 0" 0 "${CAP_RC}"
assert_contains "D4 count stays 10"              "${CAP_OUT}" "Arc<dyn> (excl slack/extension_host): 10"

# D5: missing arc_dyn_ceiling reaches schema validation (not a crash).
cat > "${tmp}/budget.toml" <<'EOF'
[gate]
enforce = true
ceiling_bp = 3000
tolerance_bp = 30
EOF
run_gate
assert_rc       "D5 missing arc_dyn_ceiling exits 1"  1 "${CAP_RC}"
assert_contains "D5 reports arc schema error"         "${CAP_OUT}" "arc_dyn_ceiling must be an integer"

rm -rf "${comp_src}/dispatch.rs" "${comp_src}/slack" "${comp_src}/extension_host"

# C10: guard against committing a red gate — the REAL repo budget file must pass
#      against the REAL tree right now.
capture bash "${gate}"
assert_rc       "C10 real tree within committed budget" 0 "${CAP_RC}"

echo ""
echo "composition-budget gate tests: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
