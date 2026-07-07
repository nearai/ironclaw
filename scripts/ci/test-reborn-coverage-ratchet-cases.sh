#!/usr/bin/env bash
#
# R-section cases for reborn-coverage-ratchet.sh (coverage-floor ratchet
# gate), split out of test-reborn-coverage.sh to keep that file under 1000
# lines as a fifth script's worth of cases joined the suite.
#
# NOT standalone: this file assumes it is `source`d from
# test-reborn-coverage.sh, reusing that script's shell state verbatim —
# `set -euo pipefail`, `fixtures_dir`/`ratchet_sh`/`empty_exemptions`, the
# assert_*/capture helpers, and the PASS_COUNT/FAIL_COUNT counters they
# update. Running it directly does nothing useful.
#
# shellcheck disable=SC2154 # fixtures_dir/ratchet_sh/empty_exemptions are
# assigned by the sourcing parent (test-reborn-coverage.sh); shellcheck can't
# see that when this file is linted on its own (only when the parent lints it
# with `-x`, which resolves cleanly).

# ---------------------------------------------------------------------------
# R. reborn-coverage-ratchet.sh (coverage-floor ratchet gate)
# ---------------------------------------------------------------------------
#
# reborn-coverage-ratchet.sh <lcov-path> <exemptions-toml> <floor-toml>. All
# fixtures below reuse the merge_sh/summary_sh conventions above: hand-built
# lcov tracefiles, no cargo-llvm-cov shellout.

cat > "${fixtures_dir}/r_composition.lcov" <<'EOF'
SF:/work/ironclaw/crates/ironclaw_reborn_composition/src/a.rs
LF:1000
LH:400
end_of_record
EOF

# R1: global below its effective floor (enforce=true) -> exit 1, RATCHET FAIL
# names "global" and shows the observed/floor numbers.
cat > "${fixtures_dir}/r1_floor.toml" <<'TOML'
[global]
enforce = true
floor_percent = 50.0
tolerance_percent = 0.5
captured_total_lines = 1000
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r1_floor.toml"
assert_exit_code "R1: global below floor (enforce=true) exits 1" 1 "${CAP_RC}"
assert_contains "R1: reports RATCHET FAIL for global" "${CAP_OUT}" "RATCHET FAIL: global"
assert_contains "R1: shows observed global numbers" "${CAP_OUT}" "observed: 40% (400 / 1000 lines)"
assert_contains "R1: shows floor and effective floor" "${CAP_OUT}" "floor:    50% (tolerance 0.5pp -> effective floor 49.5%)"

# R2: global exactly at the effective floor (boundary inclusive) -> exit 0.
# floor_percent 40.5, tolerance 0.5 -> effective floor 40.0, which equals the
# fixture's observed 400/1000 = 40.0% exactly.
cat > "${fixtures_dir}/r2_floor.toml" <<'TOML'
[global]
enforce = true
floor_percent = 40.5
tolerance_percent = 0.5
captured_total_lines = 1000
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r2_floor.toml"
assert_exit_code "R2: global exactly at effective floor exits 0 (boundary inclusive)" 0 "${CAP_RC}"
assert_contains "R2: reports RATCHET PASS for global" "${CAP_OUT}" "RATCHET PASS: global"

# R3: crate below its own floor_percent -> exit 1, names the crate.
cat > "${fixtures_dir}/r3_floor.toml" <<'TOML'
[global]
enforce = true
floor_percent = 0.0

[[crate]]
name = "ironclaw_reborn_composition"
floor_percent = 90.0
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r3_floor.toml"
assert_exit_code "R3: crate below floor_percent exits 1" 1 "${CAP_RC}"
assert_contains "R3: names the failing crate" "${CAP_OUT}" "RATCHET FAIL: ironclaw_reborn_composition"

# R4: crate holds floor_percent (40% >= 39.5% effective) but drops below
# floor_covered_lines (400 < 430 effective) -> exit 1, proving the two
# checks are ANDed (both must hold), not OR'd.
cat > "${fixtures_dir}/r4_floor.toml" <<'TOML'
[global]
enforce = true
floor_percent = 0.0

[[crate]]
name = "ironclaw_reborn_composition"
floor_percent = 40.0
floor_covered_lines = 450
tolerance_percent = 0.5
tolerance_lines = 20
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r4_floor.toml"
assert_exit_code "R4: crate holds percent floor but fails lines floor exits 1 (ANDed, not ORed)" 1 "${CAP_RC}"
assert_contains "R4: names the failing crate" "${CAP_OUT}" "RATCHET FAIL: ironclaw_reborn_composition"
assert_contains "R4: percent line present and would itself pass" "${CAP_OUT}" \
  "floor:    40% (tolerance 0.5pp -> effective floor 39.5%)"
assert_contains "R4: lines floor line shows the violated effective floor" "${CAP_OUT}" \
  "floor_covered_lines: 450 (tolerance 20 lines -> effective floor 430)"

# R5: crate configured with only floor_covered_lines (no percent) -> gates
# correctly on lines alone (no "floor:" percent line rendered at all).
cat > "${fixtures_dir}/r5_floor.toml" <<'TOML'
[global]
enforce = true
floor_percent = 0.0

[[crate]]
name = "ironclaw_reborn_composition"
floor_covered_lines = 500
tolerance_lines = 20
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r5_floor.toml"
assert_exit_code "R5: lines-only crate floor exits 1 (400 < 480 effective)" 1 "${CAP_RC}"
assert_contains "R5: lines floor line rendered" "${CAP_OUT}" \
  "floor_covered_lines: 500 (tolerance 20 lines -> effective floor 480)"
# Scope the "no percent floor line" check to just this crate's own block (the
# [global] entry above it legitimately prints its own "floor:" line) — slice
# from the crate's header down to the next blank line.
r5_crate_block="$(printf '%s\n' "${CAP_OUT}" | sed -n '/^RATCHET FAIL: ironclaw_reborn_composition$/,/^$/p')"
assert_not_contains "R5: no percent floor line rendered for a lines-only entry" "${r5_crate_block}" "  floor:    "

# R6: floor entry missing both floor_percent and floor_covered_lines -> exit
# 1, schema error (structural bug, independent of enforce).
cat > "${fixtures_dir}/r6_floor.toml" <<'TOML'
[global]
enforce = false
floor_percent = 0.0

[[crate]]
name = "ironclaw_reborn_composition"
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r6_floor.toml"
assert_exit_code "R6: crate entry missing both floor forms exits 1" 1 "${CAP_RC}"
assert_contains "R6: reports the missing-both-fields schema error" "${CAP_ERR}" \
  "missing both 'floor_percent' and 'floor_covered_lines'"

# R7: floor entry for a crate that is ALSO whole-crate-exempted in
# coverage-exemptions.toml -> exit 1, conflict error.
cat > "${fixtures_dir}/r7_exemptions.toml" <<'TOML'
[[exemption]]
crate = "ironclaw_reborn_composition"
reason = "test-only whole-crate exemption"
issue = "https://github.com/nearai/ironclaw/issues/1"
TOML
cat > "${fixtures_dir}/r7_floor.toml" <<'TOML'
[global]
enforce = false
floor_percent = 0.0

[[crate]]
name = "ironclaw_reborn_composition"
floor_percent = 10.0
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${fixtures_dir}/r7_exemptions.toml" "${fixtures_dir}/r7_floor.toml"
assert_exit_code "R7: floored crate also whole-crate-exempted exits 1" 1 "${CAP_RC}"
assert_contains "R7: reports the floor/exemption conflict" "${CAP_ERR}" "also whole-crate-exempted"

# R8: duplicate [[crate]] entries for the same crate name -> exit 1.
cat > "${fixtures_dir}/r8_floor.toml" <<'TOML'
[global]
enforce = false
floor_percent = 0.0

[[crate]]
name = "ironclaw_reborn_composition"
floor_percent = 10.0

[[crate]]
name = "ironclaw_reborn_composition"
floor_percent = 20.0
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r8_floor.toml"
assert_exit_code "R8: duplicate [[crate]] entries exits 1" 1 "${CAP_RC}"
assert_contains "R8: reports the duplicate-entry error" "${CAP_ERR}" "duplicate [[crate]] entry"

# R9: enforce=false with a failing crate -> exit 0, but stdout still shows
# the RATCHET FAIL-shaped diagnostic prefixed "[dry-run, would FAIL]" — dry
# run never masks the signal, only the exit code.
cat > "${fixtures_dir}/r9_floor.toml" <<'TOML'
[global]
enforce = false
floor_percent = 0.0

[[crate]]
name = "ironclaw_reborn_composition"
floor_percent = 90.0
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r9_floor.toml"
assert_exit_code "R9: enforce=false with a failing crate still exits 0" 0 "${CAP_RC}"
assert_contains "R9: dry-run diagnostic keeps the RATCHET FAIL shape" "${CAP_OUT}" "RATCHET FAIL"
assert_contains "R9: dry-run diagnostic is prefixed [dry-run, would FAIL]" "${CAP_OUT}" "[dry-run, would FAIL]"

# R10: missing floor-toml file -> exit 1, same not-found phrasing convention
# as the other three scripts.
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/does_not_exist_floor.toml"
assert_exit_code "R10: missing floor manifest exits 1" 1 "${CAP_RC}"
assert_contains "R10: reports missing floor manifest" "${CAP_ERR}" "coverage floor manifest not found"

# R11: captured_total_lines diverges >5% from the current total -> the
# denominator note appears even when the crate otherwise PASSES
# (informational, never itself gates).
cat > "${fixtures_dir}/r11_floor.toml" <<'TOML'
[global]
enforce = true
floor_percent = 0.0

[[crate]]
name = "ironclaw_reborn_composition"
floor_percent = 10.0
captured_total_lines = 800
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r11_floor.toml"
assert_exit_code "R11: crate with diverged denominator still PASSES (percent floor holds)" 0 "${CAP_RC}"
assert_contains "R11: crate reported as PASS" "${CAP_OUT}" "RATCHET PASS: ironclaw_reborn_composition"
assert_contains "R11: denominator note flags the >5% material change" "${CAP_OUT}" \
  "denominator: 1000 lines now vs 800 at floor capture (+200 lines, +25%) — material change (>5%)"

# R12: crate named in the floor file produces zero lines in the merged lcov
# at all (e.g. renamed/removed) -> no crash (divide-by-zero guarded),
# treated as 0% / 0 covered, fails since its floor is > 0.
cat > "${fixtures_dir}/r12_floor.toml" <<'TOML'
[global]
enforce = true
floor_percent = 0.0

[[crate]]
name = "ironclaw_nonexistent_crate"
floor_percent = 10.0
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r12_floor.toml"
assert_exit_code "R12: crate absent from the merged lcov exits 1 (guarded, not a crash)" 1 "${CAP_RC}"
assert_contains "R12: treated as 0% / 0 covered" "${CAP_OUT}" "observed: 0% (0 / 0 lines)"

# R13: enforce=false, everything passes (no violations at all) -> exit 0 AND
# stdout still contains the unconditional "Ratchet mode: DRY-RUN" banner —
# the dry-run reminder is structurally guaranteed, not dependent on a
# violation existing to attach a "[dry-run, would FAIL]" prefix to.
cat > "${fixtures_dir}/r13_floor.toml" <<'TOML'
[global]
enforce = false
floor_percent = 0.0
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r13_floor.toml"
assert_exit_code "R13: enforce=false with no violations exits 0" 0 "${CAP_RC}"
assert_contains "R13: unconditional DRY-RUN banner present even with no violations" "${CAP_OUT}" "Ratchet mode: DRY-RUN"

# R14: [global].enforce is a quoted string ("false") instead of a native TOML
# bool -> must be rejected, not coerced (bool("false") is truthy in Python,
# which would silently flip a dry-run typo into enforcing). Pins PR #5718
# coderabbit comment.
cat > "${fixtures_dir}/r14_floor.toml" <<'TOML'
[global]
enforce = "false"
floor_percent = 0.0
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r14_floor.toml"
assert_exit_code "R14: enforce as a quoted string is rejected, exits 1" 1 "${CAP_RC}"
assert_contains "R14: reports the enforce-must-be-boolean schema error" "${CAP_ERR}" \
  "[global].enforce must be a boolean"

# R15: the first per-crate floor is written as a single [crate] table (dict)
# instead of [[crate]] (array-of-tables) -> must be rejected, not silently
# replaced with an empty list (which would skip the per-crate gate entirely
# even at enforce=true). Pins PR #5718 codex (P2) comment.
cat > "${fixtures_dir}/r15_floor.toml" <<'TOML'
[global]
enforce = true
floor_percent = 0.0

[crate]
name = "ironclaw_reborn_composition"
floor_percent = 90.0
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r15_floor.toml"
assert_exit_code "R15: a [crate] table instead of [[crate]] is rejected, exits 1" 1 "${CAP_RC}"
assert_contains "R15: reports the [[crate]] array-of-tables schema error" "${CAP_ERR}" "[[crate]]"

# R16: floor TOML has no [global] section at all -> exit 1, schema error
# (existing branch had no regression case). Pins PR #5718 user comment.
cat > "${fixtures_dir}/r16_floor.toml" <<'TOML'
[[crate]]
name = "ironclaw_reborn_composition"
floor_percent = 90.0
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r16_floor.toml"
assert_exit_code "R16: floor TOML missing [global] entirely exits 1" 1 "${CAP_RC}"
assert_contains "R16: reports the missing-[global]-section schema error" "${CAP_ERR}" \
  "missing required [global] section"

# R17: [global] present but missing required 'floor_percent' -> exit 1,
# schema error (existing branch had no regression case). Pins PR #5718 user
# comment.
cat > "${fixtures_dir}/r17_floor.toml" <<'TOML'
[global]
enforce = false
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r17_floor.toml"
assert_exit_code "R17: [global] without floor_percent exits 1" 1 "${CAP_RC}"
assert_contains "R17: reports the missing-floor_percent schema error" "${CAP_ERR}" \
  "[global] missing required 'floor_percent'"

# R18: [[crate]] entry has floor_percent but no 'name' -> exit 1, schema
# error (existing branch had no regression case). Pins PR #5718 user comment.
cat > "${fixtures_dir}/r18_floor.toml" <<'TOML'
[global]
enforce = false
floor_percent = 0.0

[[crate]]
floor_percent = 90.0
TOML
capture "${ratchet_sh}" "${fixtures_dir}/r_composition.lcov" "${empty_exemptions}" "${fixtures_dir}/r18_floor.toml"
assert_exit_code "R18: [[crate]] entry missing 'name' exits 1" 1 "${CAP_RC}"
assert_contains "R18: reports the missing-'name' schema error" "${CAP_ERR}" \
  "missing required 'name'"
