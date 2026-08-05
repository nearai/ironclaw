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
#
# Every crate carries a real Cargo.toml because the gate now resolves both its
# numerator and its denominator through the crate inventory
# (scripts/ci/lib/crate_tree.py). The padding crates exist only to clear that
# module's fail-closed floor and deliberately have NO src/ tree, so they
# contribute 0 LOC and every share assertion below stays exact. Padding the
# fixture rather than exempting it from the floor is the point: these tests
# drive the same discovery path production uses, so there is no weaker mode for
# the gate to silently degrade into.
# ---------------------------------------------------------------------------
# The discovery floor is `crate_tree.py`'s own MIN_CRATE_DIRECTORIES, read from
# the module rather than copied: a literal here goes stale the moment the floor
# moves, and the fixture then fails with an error pointing at the fixture
# instead of at the change that raised the floor.
read_crate_floor() {
    python3 -c 'import importlib.util, sys
spec = importlib.util.spec_from_file_location("crate_tree", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
print(module.MIN_CRATE_DIRECTORIES)' "$1"
}
CRATE_FLOOR_HEADROOM=4
CRATE_FLOOR_PADDING=$(( $(read_crate_floor "${repo_root}/scripts/ci/lib/crate_tree.py") + CRATE_FLOOR_HEADROOM ))

write_manifest() {  # dir crate_name
    mkdir -p "$1"
    printf '[package]\nname = "%s"\nversion = "0.0.0"\nedition = "2021"\n' "$2" > "$1/Cargo.toml"
}

pad_inventory() {  # crates_dir
    local i name
    for i in $(seq 1 "${CRATE_FLOOR_PADDING}"); do
        name="$(printf 'ironclaw_pad%02d' "${i}")"
        write_manifest "$1/${name}" "${name}"
    done
}

make_fixture() {
    local dir="$1" comp_lines="$2" other_lines="$3"
    rm -rf "${dir}"
    mkdir -p "${dir}/ironclaw_reborn_composition/src" "${dir}/other_crate/src"
    write_manifest "${dir}/ironclaw_reborn_composition" ironclaw_reborn_composition
    write_manifest "${dir}/other_crate" other_crate
    pad_inventory "${dir}"
    # `|| true`: `yes | head` makes `yes` exit with SIGPIPE (141), which under
    # `set -e`+`pipefail` would abort the harness. The file is fully written.
    { yes 'let _ = 1;' | head -n "${comp_lines}";  } > "${dir}/ironclaw_reborn_composition/src/lib.rs" || true
    { yes 'let _ = 2;' | head -n "${other_lines}"; } > "${dir}/other_crate/src/lib.rs" || true
}

# 3000 comp / (3000+7000) = 30.00% = 3000 bp
make_fixture "${tmp}/crates" 3000 7000

budget() {  # enforce ceiling_bp tolerance_bp [arc_ceiling=0] [arc_tol=0] [loc_ceiling] [loc_nudge_slack]
    # arc_ceiling defaults to 0: mass-focused fixtures have no Arc<dyn>, so a
    # 0 ceiling neither breaches nor emits a dispatch nudge, isolating the mass
    # metric under test. Dispatch cases pass an explicit ceiling.
    #
    # loc_ceiling / loc_nudge_slack default ABSURDLY HIGH for the same reason:
    # the absolute-mass metric (#7151) must not breach or nudge in the cases
    # that are isolating another metric. The L cases below drive it explicitly.
    cat > "${tmp}/budget.toml" <<EOF
[gate]
enforce = $1
ceiling_bp = $2
tolerance_bp = $3
observed_bp = $2
arc_dyn_ceiling = ${4:-0}
arc_dyn_tolerance = ${5:-0}
arc_dyn_observed = ${4:-0}
loc_ceiling = ${6:-1000000}
loc_tolerance = 0
loc_nudge_slack = ${7:-1000000}
loc_observed = ${6:-1000000}
loc_observed_date = "2026-08-04"
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

# ---------------------------------------------------------------------------
# T. Tree-shape independence (WS10 / #6963).
#
# The gate used to key its numerator to the literal
# crates/ironclaw_reborn_composition/src path and its denominator to
# crates/*/src. Under the target-architecture family move both stop matching.
# The denominator failure is loud (the den_loc guard); the NUMERATOR failure is
# silent — a partial move leaves the denominator healthy, so the gate reported
# "0.00% (0 bp) ... OK" and exited 0, a ratchet passing while measuring nothing.
# These cases pin both directions.
# ---------------------------------------------------------------------------

# Discovery-mode runner: no COMPOSITION_SRC override, so the gate must find the
# composition crate BY NAME through the inventory, wherever it sits.
run_discovered() {  # crates_dir
    CRATES_ROOT="$1" \
    BUDGET_FILE="${tmp}/budget.toml" \
    capture bash "${gate}"
}

# T1: flat tree, numerator DISCOVERED (not overridden) -> same 30.00% share.
make_fixture "${tmp}/crates" 3000 7000
budget true 3000 30; run_discovered "${tmp}/crates"
assert_rc       "T1 flat discovery exits 0"        0 "${CAP_RC}"
assert_contains "T1 flat discovery finds share"    "${CAP_OUT}" "30.00% (3000 bp)"

# T2: POSITIVE — every crate nested one level under a family directory. Both the
#     numerator (by name) and the denominator must still resolve, with the share
#     byte-identical to the flat case. This is the case that is dark today.
rm -rf "${tmp}/nested"
mkdir -p "${tmp}/nested/crates/substrates"
make_fixture "${tmp}/flatsrc" 3000 7000
mv "${tmp}/flatsrc"/* "${tmp}/nested/crates/substrates/"
budget true 3000 30; run_discovered "${tmp}/nested/crates"
assert_rc       "T2 nested tree exits 0"           0 "${CAP_RC}"
assert_contains "T2 nested share matches flat"     "${CAP_OUT}" "30.00% (3000 bp)"
assert_contains "T2 nested denominator is real"    "${CAP_OUT}" "3000 LOC of 10000"

# T3: POSITIVE — partial move (composition nested, everything else flat). The
#     shape that silently reported 0.00% before: the denominator stays healthy
#     so the old den_loc guard never fired.
rm -rf "${tmp}/partial"
make_fixture "${tmp}/partial/crates" 3000 7000
mkdir -p "${tmp}/partial/crates/app"
mv "${tmp}/partial/crates/ironclaw_reborn_composition" "${tmp}/partial/crates/app/"
budget true 3000 30; run_discovered "${tmp}/partial/crates"
assert_rc       "T3 partial move exits 0"          0 "${CAP_RC}"
assert_contains "T3 partial move keeps measuring"  "${CAP_OUT}" "30.00% (3000 bp)"

# T4: NEGATIVE — the composition crate is absent (renamed). Must be a loud
#     repoint, not a 0.00% pass.
rm -rf "${tmp}/renamed"
make_fixture "${tmp}/renamed/crates" 3000 7000
mv "${tmp}/renamed/crates/ironclaw_reborn_composition" "${tmp}/renamed/crates/ironclaw_composition"
budget true 3000 30; run_discovered "${tmp}/renamed/crates"
assert_rc       "T4 renamed crate exits 1"         1 "${CAP_RC}"
assert_contains "T4 renamed crate names the crate" "${CAP_OUT}" "expected exactly one crate directory named 'ironclaw_reborn_composition'"

# T5: NEGATIVE — an inventory below the discovery floor must refuse rather than
#     measure a truncated tree.
rm -rf "${tmp}/thin"
mkdir -p "${tmp}/thin/crates/ironclaw_reborn_composition/src"
write_manifest "${tmp}/thin/crates/ironclaw_reborn_composition" ironclaw_reborn_composition
printf 'let _ = 1;\n' > "${tmp}/thin/crates/ironclaw_reborn_composition/src/lib.rs"
budget true 3000 30; run_discovered "${tmp}/thin/crates"
assert_rc       "T5 thin inventory exits 1"        1 "${CAP_RC}"
assert_contains "T5 thin inventory refuses"        "${CAP_OUT}" "crate discovery failed"

# T6: NEGATIVE — a zero-LOC numerator is an error even when it is reached
#     through an explicit COMPOSITION_SRC override. This is the backstop for the
#     silent 0.00% pass.
make_fixture "${tmp}/crates" 3000 7000
mkdir -p "${tmp}/empty_src"
budget true 3000 30
COMPOSITION_SRC="${tmp}/empty_src" \
CRATES_ROOT="${tmp}/crates" \
BUDGET_FILE="${tmp}/budget.toml" \
capture bash "${gate}"
assert_rc       "T6 zero numerator exits 1"        1 "${CAP_RC}"
assert_contains "T6 zero numerator refuses"        "${CAP_OUT}" "composition LOC is 0"

# T7: NEGATIVE — CRATES_ROOT that is not a `crates` directory must be refused
#     rather than silently discovering the wrong tree.
budget true 3000 30
COMPOSITION_SRC="${tmp}/crates/ironclaw_reborn_composition/src" \
CRATES_ROOT="${tmp}" \
BUDGET_FILE="${tmp}/budget.toml" \
capture bash "${gate}"
assert_rc       "T7 bad CRATES_ROOT exits 1"       1 "${CAP_RC}"
assert_contains "T7 bad CRATES_ROOT explains"      "${CAP_OUT}" "must be a directory named 'crates'"

make_fixture "${tmp}/crates" 3000 7000  # restore clean fixture

# ---------------------------------------------------------------------------
# L. Absolute mass (production LOC) — the metric with no denominator (#7151).
#
# The share metric cannot see composition growing while the rest of the
# workspace grows faster; every case here holds the share FIXED at 3000 bp
# (well inside its ceiling) so only the absolute bound can decide the outcome.
# ---------------------------------------------------------------------------
make_fixture "${tmp}/crates" 3000 7000

# L1: 3000 LOC against a 3000 ceiling -> inclusive pass, and the line is shown.
budget true 3000 30 0 0 3000 0; run_gate
assert_rc       "L1 at absolute ceiling exits 0"  0 "${CAP_RC}"
assert_contains "L1 reports absolute mass"        "${CAP_OUT}" "[abs] composition src  : 3000 LOC"
assert_contains "L1 reports OK"                   "${CAP_OUT}" "OK: composition within mass + dispatch budget"

# L2: THE DEFECT THIS METRIC EXISTS FOR. Composition grows by 619 LOC (the real
#     2026-08-02..04 inflow) while the workspace grows faster, so the SHARE
#     IMPROVES — 30.00% -> 26.57%, further inside its ceiling than before — and
#     only the absolute bound objects.
make_fixture "${tmp}/crates" 3619 10000
budget true 3000 30 0 0 3000 0; run_gate
assert_rc       "L2 absolute breach exits 1"       1 "${CAP_RC}"
assert_contains "L2 reports ABSOLUTE MASS EXCEEDED" "${CAP_OUT}" "ABSOLUTE MASS EXCEEDED"
assert_contains "L2 names the overage"             "${CAP_OUT}" "3619 production LOC, 619 over"
assert_contains "L2 share metric IMPROVED"         "${CAP_OUT}" "26.57% (2657 bp)"
assert_not_contains "L2 share did NOT fire"        "${CAP_OUT}" "MASS EXCEEDED: composition is"

# L3: same breach, DRY-RUN -> exit 0 with the marker.
budget false 3000 30 0 0 3000 0; run_gate
assert_rc       "L3 absolute breach dry-run exits 0" 0 "${CAP_RC}"
assert_contains "L3 would-fail marker"               "${CAP_OUT}" "[dry-run, would FAIL] ABSOLUTE MASS EXCEEDED"

# L4: tolerance absorbs an in-flight PR: 3619 vs ceiling 3000 + tol 619.
cat > "${tmp}/budget.toml" <<'EOF'
[gate]
enforce = true
ceiling_bp = 3000
tolerance_bp = 30
observed_bp = 3000
arc_dyn_ceiling = 0
arc_dyn_tolerance = 0
arc_dyn_observed = 0
loc_ceiling = 3000
loc_tolerance = 619
loc_nudge_slack = 1000000
loc_observed = 3000
loc_observed_date = "2026-08-04"
observed_date = "2026-07-16"
EOF
run_gate
assert_rc       "L4 within tolerance exits 0"      0 "${CAP_RC}"
assert_contains "L4 reports OK"                    "${CAP_OUT}" "OK: composition within mass + dispatch budget"

# L5: down-ratchet nudge after a wave evicts behavior (ceiling 4000, obs 3619).
make_fixture "${tmp}/crates" 3619 10000
budget true 3000 30 0 0 4000 200; run_gate
assert_rc       "L5 under ceiling exits 0"         0 "${CAP_RC}"
assert_contains "L5 emits re-ratchet nudge"        "${CAP_OUT}" "lower loc_ceiling to lock it in"

# L6: no nudge when the slack is inside loc_nudge_slack (381 < 400).
budget true 3000 30 0 0 4000 400; run_gate
assert_rc          "L6 small slack exits 0"        0 "${CAP_RC}"
assert_not_contains "L6 no absolute nudge"         "${CAP_OUT}" "lower loc_ceiling to lock it in"

# L7: SCHEMA — the absolute keys are REQUIRED. Deleting them must be a loud
#     schema error, not a silently disarmed binding metric.
cat > "${tmp}/budget.toml" <<'EOF'
[gate]
enforce = true
ceiling_bp = 3000
tolerance_bp = 30
arc_dyn_ceiling = 0
arc_dyn_tolerance = 0
EOF
run_gate
assert_rc       "L7 missing loc_ceiling exits 1"   1 "${CAP_RC}"
assert_contains "L7 reports loc schema error"      "${CAP_OUT}" "loc_ceiling must be an integer"

# L8: SCHEMA — loc_ceiling = 0 is a disarmed gate, not a bound.
cat > "${tmp}/budget.toml" <<'EOF'
[gate]
enforce = true
ceiling_bp = 3000
tolerance_bp = 30
arc_dyn_ceiling = 0
arc_dyn_tolerance = 0
loc_ceiling = 0
loc_tolerance = 0
loc_nudge_slack = 100
EOF
run_gate
assert_rc       "L8 zero loc_ceiling exits 1"      1 "${CAP_RC}"
assert_contains "L8 refuses a zero ceiling"        "${CAP_OUT}" "loc_ceiling must be greater than 0"

# L9: --print reports the absolute count.
make_fixture "${tmp}/crates" 3000 7000
budget true 3000 30 0 0 3000 0
COMPOSITION_SRC="${tmp}/crates/ironclaw_reborn_composition/src" \
CRATES_ROOT="${tmp}/crates" \
BUDGET_FILE="${tmp}/budget.toml" \
capture bash "${gate}" --print
assert_rc       "L9 --print exits 0"               0 "${CAP_RC}"
assert_contains "L9 --print shows absolute LOC"    "${CAP_OUT}" "composition absolute: 3000 LOC"

# L10: test-only FILES are excluded from the absolute metric too (same
#      numerator as the share metric — one definition, two bounds).
printf 'let _ = 9;\n%.0s' $(seq 1 5000) > "${tmp}/crates/ironclaw_reborn_composition/src/tests.rs"
budget true 3000 30 0 0 3000 0; run_gate
assert_rc       "L10 test file excluded exits 0"   0 "${CAP_RC}"
assert_contains "L10 absolute ignores tests.rs"    "${CAP_OUT}" "[abs] composition src  : 3000 LOC"
make_fixture "${tmp}/crates" 3000 7000  # restore clean fixture

# C10: guard against committing a red gate — the REAL repo budget file must pass
#      against the REAL tree right now.
capture bash "${gate}"
assert_rc       "C10 real tree within committed budget" 0 "${CAP_RC}"

# C11: the committed budget's absolute ceiling must actually BIND — a ceiling
#      more than loc_nudge_slack above the live count is the "17.4pp of slack"
#      failure that made the share metric inert, reproduced on the new metric.
capture bash "${gate}"
assert_not_contains "C11 committed loc_ceiling is not slack" "${CAP_OUT}" "lower loc_ceiling to lock it in"

echo ""
echo "composition-budget gate tests: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
