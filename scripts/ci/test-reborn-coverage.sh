#!/usr/bin/env bash
#
# Regression tests for the Reborn coverage CI helpers:
#   - reborn-coverage-merge-lcov.sh     (per-lane lcov merge + crate filter)
#   - reborn-coverage-summary.sh        (report + --zero-crates modes)
#   - reborn-coverage-comment.sh        (sticky PR comment upsert via `gh api`)
#   - reborn-coverage-int-tier-tests.sh (int-tier suite discovery)
#   - reborn-coverage-ratchet.sh        (coverage-floor ratchet gate)
#
# Mirrors test-classify-test-scope.sh: self-contained, locates the
# scripts-under-test relative to this file's own directory, builds its own
# fixtures in a mktemp dir, and reports PASS/FAIL per case. Unlike that
# precedent (which exits on the first failure), this suite runs every case
# and prints a final summary, exiting non-zero only if something failed —
# with five scripts and 50 cases (M/A/B/C/D/R sections), seeing the full
# picture in one run beats stopping at the first mismatch.
#
# reborn-coverage-summary.sh and reborn-coverage-ratchet.sh share one lcov-
# parsing + exemption-filtering + by-crate-aggregation implementation
# (scripts/ci/lib/reborn_coverage_lcov.py) — the M/A/B/C sections below are
# this module's regression proof (exercised transitively through both
# consuming scripts), not a separate lib-level test file.
#
# The R section (reborn-coverage-ratchet.sh cases) lives in the sibling
# test-reborn-coverage-ratchet-cases.sh, `source`d near the end of this file —
# split out to keep this file under 1000 lines once a fifth script's cases
# joined the suite. That file is not standalone; it shares this script's
# helpers, fixtures, and PASS_COUNT/FAIL_COUNT counters.
#
# reborn-coverage-summary.sh and reborn-coverage-comment.sh consume a merged,
# crate-filtered lcov tracefile (scripts/ci/reborn-coverage-merge-lcov.sh) plus
# the exemptions manifest (tests/integration/coverage-exemptions.toml schema),
# not a cargo-llvm-cov JSON export — the coverage-report job in
# .github/workflows/reborn-tests.yml downloads 5 per-lane lcov artifacts,
# merges+filters them, then renders. Fixtures below build lcov tracefiles and
# exemptions TOML by hand rather than shelling out to cargo-llvm-cov.
#
# reborn-coverage-comment.sh shells out to `gh api`. It is exercised here
# against a fake `gh` (a fixture script placed first on PATH) that emulates
# `gh api --paginate <path> --jq '<filter>'` by running the given jq filter
# over a canned comments JSON array, and records the verb/path/body of any
# mutating call (-X POST / -X PATCH) to a log file this suite inspects.
#
# reborn-coverage-int-tier-tests.sh derives its repo root from its own path
# (`$(dirname BASH_SOURCE)/../..`) and `cd`s there, so it cannot simply be
# pointed at a fixture tree via an argument. Each case copies the real
# script into a temp tree's scripts/ci/ and builds a tests/integration/
# subtree next to it, so the copy's own repo-root resolution lands on the
# temp tree.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
merge_sh="${script_dir}/reborn-coverage-merge-lcov.sh"
summary_sh="${script_dir}/reborn-coverage-summary.sh"
comment_sh="${script_dir}/reborn-coverage-comment.sh"
int_tier_sh="${script_dir}/reborn-coverage-int-tier-tests.sh"
ratchet_sh="${script_dir}/reborn-coverage-ratchet.sh"

tmp_root="$(mktemp -d)"
trap 'rm -rf "${tmp_root}"' EXIT

fixtures_dir="${tmp_root}/fixtures"
mkdir -p "${fixtures_dir}"

# Empty-but-valid exemptions manifest reused by cases that don't care about
# exemption behavior specifically.
empty_exemptions="${fixtures_dir}/empty-exemptions.toml"
cat > "${empty_exemptions}" <<'TOML'
# No entries.
TOML

PASS_COUNT=0
FAIL_COUNT=0

report_pass() {
  PASS_COUNT=$((PASS_COUNT + 1))
  printf 'PASS %s\n' "$1"
}

report_fail() {
  FAIL_COUNT=$((FAIL_COUNT + 1))
  printf 'FAIL %s\n' "$1" >&2
}

assert_eq() {
  local name="$1" expected="$2" actual="$3"
  if [ "${actual}" = "${expected}" ]; then
    report_pass "${name}"
  else
    report_fail "${name}"
    printf 'Expected:\n%s\n' "${expected}" >&2
    printf 'Actual:\n%s\n' "${actual}" >&2
  fi
}

assert_exit_code() {
  local name="$1" expected="$2" actual="$3"
  if [ "${actual}" -eq "${expected}" ]; then
    report_pass "${name}"
  else
    report_fail "${name}"
    printf 'Expected exit code %s, got %s\n' "${expected}" "${actual}" >&2
  fi
}

assert_contains() {
  local name="$1" haystack="$2" needle="$3"
  if [[ "${haystack}" == *"${needle}"* ]]; then
    report_pass "${name}"
  else
    report_fail "${name}"
    printf 'Expected to contain:\n%s\n' "${needle}" >&2
    printf 'Actual:\n%s\n' "${haystack}" >&2
  fi
}

assert_not_contains() {
  local name="$1" haystack="$2" needle="$3"
  if [[ "${haystack}" != *"${needle}"* ]]; then
    report_pass "${name}"
  else
    report_fail "${name}"
    printf 'Expected NOT to contain:\n%s\n' "${needle}" >&2
    printf 'Actual:\n%s\n' "${haystack}" >&2
  fi
}

# Asserts needle_a's first occurrence is on an earlier line than needle_b's
# first occurrence within haystack (both must be present).
assert_line_before() {
  local name="$1" haystack="$2" needle_a="$3" needle_b="$4"
  local line_a line_b
  # `|| true`: a missing needle makes grep exit 1, which under `set -o pipefail`
  # would abort the whole suite on assignment instead of falling through to the
  # empty-check below and reporting a normal FAIL (the harness runs every case).
  line_a="$(printf '%s\n' "${haystack}" | grep -n -F -- "${needle_a}" | head -n1 | cut -d: -f1 || true)"
  line_b="$(printf '%s\n' "${haystack}" | grep -n -F -- "${needle_b}" | head -n1 | cut -d: -f1 || true)"
  if [ -n "${line_a}" ] && [ -n "${line_b}" ] && [ "${line_a}" -lt "${line_b}" ]; then
    report_pass "${name}"
  else
    report_fail "${name}"
    printf 'line_a=%s line_b=%s\n' "${line_a:-<missing>}" "${line_b:-<missing>}" >&2
  fi
}

# Runs "$@", capturing stdout/stderr/exit code into CAP_OUT/CAP_ERR/CAP_RC
# without tripping this script's own `set -e` on a non-zero exit.
CAP_OUT=""
CAP_ERR=""
CAP_RC=0
capture() {
  local err_file out rc
  err_file="$(mktemp "${tmp_root}/capture.XXXXXX")"
  set +e
  out="$("$@" 2>"${err_file}")"
  rc=$?
  set -e
  CAP_OUT="${out}"
  CAP_ERR="$(cat "${err_file}")"
  CAP_RC="${rc}"
  rm -f "${err_file}"
}

# ---------------------------------------------------------------------------
# M. reborn-coverage-merge-lcov.sh
# ---------------------------------------------------------------------------

cat > "${fixtures_dir}/m1_part0.lcov" <<'EOF'
SF:/work/ironclaw/crates/ironclaw_reborn/src/runtime.rs
DA:1,1
DA:2,0
DA:3,1
LF:3
LH:2
end_of_record
SF:/work/ironclaw/src/main.rs
DA:1,1
LF:1
LH:1
end_of_record
EOF

cat > "${fixtures_dir}/m1_part1.lcov" <<'EOF'
SF:/work/ironclaw/crates/ironclaw_reborn/src/runtime.rs
DA:1,0
DA:2,1
DA:3,0
LF:3
LH:1
end_of_record
SF:/work/ironclaw/crates/ironclaw_product_workflow/src/lib.rs
DA:1,1
DA:2,1
LF:2
LH:2
end_of_record
EOF

capture "${merge_sh}" "${tmp_root}/m1_merged.lcov" "${fixtures_dir}/m1_part0.lcov" "${fixtures_dir}/m1_part1.lcov"
assert_exit_code "M1: merge exits 0" 0 "${CAP_RC}"

m1_merged_body="$(cat "${tmp_root}/m1_merged.lcov")"
assert_contains "M1: merged output keeps crates/ironclaw_reborn" "${m1_merged_body}" "SF:/work/ironclaw/crates/ironclaw_reborn/src/runtime.rs"
assert_not_contains "M1: merged output drops non-crates/ src/main.rs" "${m1_merged_body}" "src/main.rs"
assert_contains "M1: merged output keeps crates/ironclaw_product_workflow" "${m1_merged_body}" "ironclaw_product_workflow"
assert_contains "M1: per-line DA counts are SUMMED across lanes (line1: 1+0=1)" "${m1_merged_body}" "DA:1,1"
assert_contains "M1: per-line DA counts are SUMMED across lanes (line2: 0+1=1)" "${m1_merged_body}" "DA:2,1"
assert_contains "M1: per-line DA counts are SUMMED across lanes (line3: 1+0=1)" "${m1_merged_body}" "DA:3,1"
assert_contains "M1: LH recomputed from merged counts, not trusted from either lane (all 3 lines now covered)" \
  "${m1_merged_body}" "$(printf 'LF:3\nLH:3')"

# M2: missing input -> non-zero exit, no output file written over a bad arg.
capture "${merge_sh}" "${tmp_root}/m2_merged.lcov" "${fixtures_dir}/does_not_exist.lcov"
assert_exit_code "M2: merge exits non-zero for missing input" 1 "${CAP_RC}"
assert_contains "M2: merge reports missing input file" "${CAP_ERR}" "input lcov file not found"

# M3: single input with zero matching (crates/ironclaw_*) files -> empty output, exit 0.
cat > "${fixtures_dir}/m3_no_match.lcov" <<'EOF'
SF:/work/ironclaw/src/other.rs
DA:1,1
LF:1
LH:1
end_of_record
EOF
capture "${merge_sh}" "${tmp_root}/m3_merged.lcov" "${fixtures_dir}/m3_no_match.lcov"
assert_exit_code "M3: merge exits 0 when nothing matches the crate filter" 0 "${CAP_RC}"
assert_eq "M3: merge writes an empty tracefile when nothing matches" "" "$(cat "${tmp_root}/m3_merged.lcov")"

# ---------------------------------------------------------------------------
# A. reborn-coverage-summary.sh (default report mode)
# ---------------------------------------------------------------------------

cat > "${fixtures_dir}/a1_mixed.lcov" <<'EOF'
SF:/work/ironclaw/crates/ironclaw_reborn/src/runtime.rs
DA:1,1
LF:100
LH:80
end_of_record
SF:/work/ironclaw/crates/ironclaw_product_workflow/src/lib.rs
LF:50
LH:50
end_of_record
EOF

capture "${summary_sh}" "${fixtures_dir}/a1_mixed.lcov" "${empty_exemptions}"
assert_exit_code "A1: summary exits 0 for a mixed-crate fixture" 0 "${CAP_RC}"
assert_contains "A1: aggregate matches hand-computed 86.67% (130/150)" "${CAP_OUT}" \
  '**Line coverage (Reborn crates): 86.67%** — 130 / 150 lines'
assert_contains "A1: table includes ironclaw_reborn row" "${CAP_OUT}" "| \`ironclaw_reborn\` | 80% | 80 / 100 |"
assert_contains "A1: table includes ironclaw_product_workflow row" "${CAP_OUT}" \
  "| \`ironclaw_product_workflow\` | 100% | 50 / 50 |"

# A2: no data at all -> exit 0, "no data" message.
: > "${fixtures_dir}/a2_empty.lcov"
capture "${summary_sh}" "${fixtures_dir}/a2_empty.lcov" "${empty_exemptions}"
assert_exit_code "A2: summary exits 0 for an empty lcov file" 0 "${CAP_RC}"
assert_contains "A2: prints no-data message when the lcov file is empty" "${CAP_OUT}" \
  "No Reborn crate coverage data found"

# A3: missing lcov file -> non-zero exit + not-found error on stderr.
capture "${summary_sh}" "${fixtures_dir}/does_not_exist.lcov" "${empty_exemptions}"
assert_exit_code "A3: summary exits non-zero for missing coverage lcov" 1 "${CAP_RC}"
assert_contains "A3: summary reports missing coverage lcov" "${CAP_ERR}" "coverage lcov file not found"

# A4: missing exemptions manifest -> non-zero exit + not-found error.
capture "${summary_sh}" "${fixtures_dir}/a1_mixed.lcov" "${fixtures_dir}/does_not_exist_exemptions.toml"
assert_exit_code "A4: summary exits non-zero for missing exemptions manifest" 1 "${CAP_RC}"
assert_contains "A4: summary reports missing exemptions manifest" "${CAP_ERR}" "coverage exemptions manifest not found"

# A5: zero-covered-crate fixture, sorted lowest-covered-first.
cat > "${fixtures_dir}/a5_zero_sorted.lcov" <<'EOF'
SF:/work/ironclaw/crates/ironclaw_reborn_zero/src/a.rs
LF:10
LH:0
end_of_record
SF:/work/ironclaw/crates/ironclaw_reborn_half/src/a.rs
LF:10
LH:5
end_of_record
EOF
capture "${summary_sh}" "${fixtures_dir}/a5_zero_sorted.lcov" "${empty_exemptions}"
assert_exit_code "A5: zero-covered-crate fixture summary exits 0" 0 "${CAP_RC}"
assert_contains "A5: zero-covered crate row shows 0%" "${CAP_OUT}" "| \`ironclaw_reborn_zero\` | 0% | 0 / 10 |"
assert_line_before "A5: zero-covered crate sorted to top (lowest-covered first)" "${CAP_OUT}" \
  "\`ironclaw_reborn_zero\`" "\`ironclaw_reborn_half\`"

# A6: the crate filter now covers ALL crates/ironclaw_* (all workspace
# crates the int-tier suites link — a superset of the historical Reborn-only
# allowlist), but files outside crates/ entirely (or under a different
# top-level crates-like dir) are still excluded from the aggregate.
cat > "${fixtures_dir}/a6_boundary.lcov" <<'EOF'
SF:/work/ironclaw/crates/ironclaw_engine/src/a.rs
LF:10
LH:5
end_of_record
SF:/work/ironclaw/src/main.rs
LF:999
LH:0
end_of_record
EOF
capture "${summary_sh}" "${fixtures_dir}/a6_boundary.lcov" "${empty_exemptions}"
assert_exit_code "A6: boundary fixture summary exits 0" 0 "${CAP_RC}"
assert_contains "A6: any crates/ironclaw_* crate is included (not just the old Reborn allowlist)" "${CAP_OUT}" \
  "| \`ironclaw_engine\` | 50% | 5 / 10 |"
assert_not_contains "A6: non-crates/ file excluded from the table" "${CAP_OUT}" "main.rs"
assert_contains "A6: aggregate drops the non-crates file's 999 lines (5/10, not 5/1009)" "${CAP_OUT}" \
  '**Line coverage (Reborn crates): 50%** — 5 / 10 lines'

# A7: exemptions manifest excludes a file from the accounting entirely and
# lists it in the report's own Exemptions section.
cat > "${fixtures_dir}/a7_exemptions.toml" <<'TOML'
[[exemption]]
module = "crates/ironclaw_engine/src/a.rs"
reason = "generated code, not exercisable by int-tier tests"
issue = "https://github.com/nearai/ironclaw/issues/1"
TOML

capture "${summary_sh}" "${fixtures_dir}/a6_boundary.lcov" "${fixtures_dir}/a7_exemptions.toml"
assert_exit_code "A7: summary with an exemption exits 0" 0 "${CAP_RC}"
# The exempted crate's file path still legitimately appears in the report's
# own Exemptions section below, so assert against the per-crate table ROW
# specifically (not "the whole output"), matching the A6/A5 row-shaped checks.
assert_not_contains "A7: exempted crate dropped from the per-crate table" "${CAP_OUT}" "| \`ironclaw_engine\` |"
assert_contains "A7: exempted file listed in its own Exemptions section" "${CAP_OUT}" \
  "\`crates/ironclaw_engine/src/a.rs\`"
assert_contains "A7: exemption reason rendered" "${CAP_OUT}" "generated code, not exercisable by int-tier tests"
assert_contains "A7: exemption issue link rendered" "${CAP_OUT}" "https://github.com/nearai/ironclaw/issues/1"
assert_contains "A7: aggregate becomes 'no data' once the only crate is fully exempted" "${CAP_OUT}" \
  "No Reborn crate coverage data found"

# A8: malformed exemption (missing reason/issue) -> summary refuses to render.
cat > "${fixtures_dir}/a8_malformed_exemptions.toml" <<'TOML'
[[exemption]]
module = "crates/ironclaw_engine/src/a.rs"
reason = "missing issue link"
TOML
capture "${summary_sh}" "${fixtures_dir}/a6_boundary.lcov" "${fixtures_dir}/a8_malformed_exemptions.toml"
assert_exit_code "A8: malformed exemption (no issue) exits non-zero" 1 "${CAP_RC}"
assert_contains "A8: malformed exemption reports the missing issue" "${CAP_ERR}" "missing 'issue'"

# A9: a non-repo-relative exemption module path (missing the "crates/" prefix)
# would otherwise match every same-named file across all crates via the
# endswith("/" + m) check in the summary's own accounting — reject it at
# parse time instead of silently over-exempting.
cat > "${fixtures_dir}/a9_bare_module_exemptions.toml" <<'TOML'
[[exemption]]
module = "a.rs"
reason = "bare filename, not repo-relative"
issue = "https://github.com/nearai/ironclaw/issues/1"
TOML
capture "${summary_sh}" "${fixtures_dir}/a6_boundary.lcov" "${fixtures_dir}/a9_bare_module_exemptions.toml"
assert_exit_code "A9: non-crates/-prefixed exemption module exits non-zero" 1 "${CAP_RC}"
assert_contains "A9: non-crates/-prefixed exemption module reports the validation error" "${CAP_ERR}" \
  "must be repo-relative and start with 'crates/'"

# A10: whole-crate `crate =` exemption form drops the entire crate from the
# table and is rendered in the Exemptions section with a "crate: X" label
# (never `entry["module"]` — that key doesn't exist on this entry shape, the
# structural regression this case pins).
cat > "${fixtures_dir}/a10_two_crates.lcov" <<'EOF'
SF:/work/ironclaw/crates/ironclaw_embeddings/src/a.rs
LF:10
LH:0
end_of_record
SF:/work/ironclaw/crates/ironclaw_reborn/src/a.rs
LF:10
LH:5
end_of_record
EOF
cat > "${fixtures_dir}/a10_crate_exemption.toml" <<'TOML'
[[exemption]]
crate = "ironclaw_embeddings"
reason = "v1-only: consumed exclusively by the root ironclaw package"
issue = "https://github.com/nearai/ironclaw/issues/1"
TOML
capture "${summary_sh}" "${fixtures_dir}/a10_two_crates.lcov" "${fixtures_dir}/a10_crate_exemption.toml"
assert_exit_code "A10: whole-crate exemption exits 0 (no KeyError on missing 'module')" 0 "${CAP_RC}"
assert_not_contains "A10: exempted crate dropped from the per-crate table" "${CAP_OUT}" "| \`ironclaw_embeddings\` |"
assert_contains "A10: exempted crate kept out of the table, other crate still reported" "${CAP_OUT}" \
  "| \`ironclaw_reborn\` | 50% | 5 / 10 |"
assert_contains "A10: whole-crate exemption listed under its 'crate: X' label" "${CAP_OUT}" "\`crate: ironclaw_embeddings\`"
assert_contains "A10: aggregate excludes the exempted crate's lines (5/10, not 5/20)" "${CAP_OUT}" \
  '**Line coverage (Reborn crates): 50%** — 5 / 10 lines'

# A11: mixed manifest (one per-file `module` entry + one whole-crate `crate`
# entry) renders both, sorted together by their shared `label` field.
cat > "${fixtures_dir}/a11_mixed_forms.toml" <<'TOML'
[[exemption]]
module = "crates/ironclaw_reborn/src/a.rs"
reason = "per-file exemption"
issue = "https://github.com/nearai/ironclaw/issues/2"

[[exemption]]
crate = "ironclaw_embeddings"
reason = "whole-crate exemption"
issue = "https://github.com/nearai/ironclaw/issues/1"
TOML
capture "${summary_sh}" "${fixtures_dir}/a10_two_crates.lcov" "${fixtures_dir}/a11_mixed_forms.toml"
assert_exit_code "A11: mixed module+crate manifest exits 0" 0 "${CAP_RC}"
assert_contains "A11: per-file form still rendered by its module path" "${CAP_OUT}" "\`crates/ironclaw_reborn/src/a.rs\`"
assert_contains "A11: whole-crate form still rendered by its 'crate: X' label" "${CAP_OUT}" "\`crate: ironclaw_embeddings\`"
assert_contains "A11: both exemptions fully drain the table (no data left)" "${CAP_OUT}" \
  "No Reborn crate coverage data found"

# A12: malformed entry with BOTH 'module' and 'crate' set -> exactly-one-of
# validation rejects it instead of silently preferring one key.
cat > "${fixtures_dir}/a12_both_keys.toml" <<'TOML'
[[exemption]]
module = "crates/ironclaw_reborn/src/a.rs"
crate = "ironclaw_embeddings"
reason = "ambiguous"
issue = "https://github.com/nearai/ironclaw/issues/1"
TOML
capture "${summary_sh}" "${fixtures_dir}/a10_two_crates.lcov" "${fixtures_dir}/a12_both_keys.toml"
assert_exit_code "A12: exemption with both 'module' and 'crate' exits non-zero" 1 "${CAP_RC}"
assert_contains "A12: reports the exactly-one-of violation (both present)" "${CAP_ERR}" "both present"

# A13: malformed entry with NEITHER 'module' nor 'crate' set.
cat > "${fixtures_dir}/a13_neither_key.toml" <<'TOML'
[[exemption]]
reason = "no scope given"
issue = "https://github.com/nearai/ironclaw/issues/1"
TOML
capture "${summary_sh}" "${fixtures_dir}/a10_two_crates.lcov" "${fixtures_dir}/a13_neither_key.toml"
assert_exit_code "A13: exemption with neither 'module' nor 'crate' exits non-zero" 1 "${CAP_RC}"
assert_contains "A13: reports the exactly-one-of violation (neither present)" "${CAP_ERR}" "neither present"

# ---------------------------------------------------------------------------
# B. reborn-coverage-summary.sh --zero-crates
# ---------------------------------------------------------------------------

cat > "${fixtures_dir}/b1_mixed_zero.lcov" <<'EOF'
SF:/work/ironclaw/crates/ironclaw_reborn_zero_a/src/a.rs
LF:5
LH:0
end_of_record
SF:/work/ironclaw/crates/ironclaw_reborn_zero_b/src/a.rs
LF:3
LH:0
end_of_record
SF:/work/ironclaw/crates/ironclaw_reborn_partial/src/a.rs
LF:10
LH:4
end_of_record
SF:/work/ironclaw/crates/ironclaw_reborn_full/src/a.rs
LF:10
LH:10
end_of_record
EOF

capture "${summary_sh}" --zero-crates "${fixtures_dir}/b1_mixed_zero.lcov" "${empty_exemptions}"
assert_exit_code "B1: --zero-crates exits 0" 0 "${CAP_RC}"
assert_eq "B1: --zero-crates emits exactly the 2 zero-covered crate names" \
  "$(printf 'ironclaw_reborn_zero_a\nironclaw_reborn_zero_b')" "${CAP_OUT}"

cat > "${fixtures_dir}/b2_all_covered.lcov" <<'EOF'
SF:/work/ironclaw/crates/ironclaw_reborn_full/src/a.rs
LF:10
LH:10
end_of_record
EOF
capture "${summary_sh}" --zero-crates "${fixtures_dir}/b2_all_covered.lcov" "${empty_exemptions}"
assert_exit_code "B2: all-covered fixture --zero-crates exits 0" 0 "${CAP_RC}"
assert_eq "B2: all-covered fixture --zero-crates emits nothing" "" "${CAP_OUT}"

capture "${summary_sh}" --zero-crates "${fixtures_dir}/a2_empty.lcov" "${empty_exemptions}"
assert_exit_code "B3: empty lcov --zero-crates exits 0" 0 "${CAP_RC}"
assert_eq "B3: empty lcov --zero-crates emits nothing" "" "${CAP_OUT}"

# B4: a whole-crate `crate =` exemption drops its zero-covered crate from
# --zero-crates too, not just from the report-mode table (A10 only covers
# report mode; reuses that fixture — ironclaw_embeddings is 0/10, exempted).
capture "${summary_sh}" --zero-crates "${fixtures_dir}/a10_two_crates.lcov" "${fixtures_dir}/a10_crate_exemption.toml"
assert_exit_code "B4: --zero-crates with a whole-crate exemption exits 0" 0 "${CAP_RC}"
assert_eq "B4: exempted zero-covered crate is excluded from --zero-crates output" "" "${CAP_OUT}"

# ---------------------------------------------------------------------------
# C. reborn-coverage-comment.sh (sticky PR comment upsert via a fake `gh`)
# ---------------------------------------------------------------------------

gh_bin_dir="${tmp_root}/bin"
mkdir -p "${gh_bin_dir}"

# Emulates `gh api [--paginate] <path> [--jq <filter>]` (read path: runs the
# jq filter over the canned comments JSON, exercising env.STICKY_MARKER) and
# `gh api -X POST|PATCH <path> -f body=<value>` (mutation path: records
# verb + path + body to FAKE_GH_LOG instead of calling the network).
cat > "${gh_bin_dir}/gh" <<'GHEOF'
#!/usr/bin/env bash
set -euo pipefail

if [ "${1:-}" != "api" ]; then
  echo "fake gh: unsupported command: $*" >&2
  exit 1
fi
shift

: "${FAKE_GH_COMMENTS_JSON:?FAKE_GH_COMMENTS_JSON must be set}"
: "${FAKE_GH_LOG:?FAKE_GH_LOG must be set}"

method="GET"
req_path=""
jq_filter=""
fields=()

while [ "$#" -gt 0 ]; do
  case "$1" in
    --paginate)
      shift
      ;;
    -X)
      method="$2"
      shift 2
      ;;
    --jq)
      jq_filter="$2"
      shift 2
      ;;
    -f)
      fields+=("$2")
      shift 2
      ;;
    *)
      req_path="$1"
      shift
      ;;
  esac
done

if [ "${method}" = "GET" ]; then
  if [ -n "${jq_filter}" ]; then
    jq -r "${jq_filter}" "${FAKE_GH_COMMENTS_JSON}"
  else
    cat "${FAKE_GH_COMMENTS_JSON}"
  fi
  exit 0
fi

body_value=""
for f in "${fields[@]}"; do
  case "${f}" in
    body=*)
      body_value="${f#body=}"
      ;;
  esac
done

{
  printf 'VERB=%s\n' "${method}"
  printf 'API_PATH=%s\n' "${req_path}"
  printf 'BODY_START\n'
  printf '%s' "${body_value}"
  printf '\nBODY_END\n'
} > "${FAKE_GH_LOG}"

echo '{}'
GHEOF
chmod +x "${gh_bin_dir}/gh"

cat > "${fixtures_dir}/c_basic_coverage.lcov" <<'EOF'
SF:/work/ironclaw/crates/ironclaw_reborn/src/a.rs
LF:10
LH:8
end_of_record
EOF

cat > "${fixtures_dir}/c_zero_coverage.lcov" <<'EOF'
SF:/work/ironclaw/crates/ironclaw_reborn_zero/src/a.rs
LF:5
LH:0
end_of_record
EOF

# Permissive floor (dry-run, floor 0.0) so C-section cases exercise the
# comment script's OWN behavior (marker/upsert/callout), not ratchet gating —
# ratchet-specific behavior is covered by the R section below.
cat > "${fixtures_dir}/c_permissive_floor.toml" <<'TOML'
[global]
enforce = false
floor_percent = 0.0
TOML

cat > "${fixtures_dir}/c1_comments_empty.json" <<'JSON'
[]
JSON

# The sticky comment (id 99) is deliberately NOT first in the list, pinning
# that the lookup filters by marker rather than assuming position.
cat > "${fixtures_dir}/c2_comments_with_sticky.json" <<'JSON'
[
  { "id": 1, "body": "Just a regular comment, unrelated to coverage." },
  { "id": 99, "body": "<!-- reborn-coverage-sticky -->\n\nstale summary body" }
]
JSON

gh_repo="acme/ironclaw-test"
gh_pr="42"

# C1: no existing sticky comment -> POST a new one.
c1_log="${tmp_root}/c1-gh.log"
capture env \
  GH_TOKEN="fake-token" \
  GITHUB_REPOSITORY="${gh_repo}" \
  PR_NUMBER="${gh_pr}" \
  PATH="${gh_bin_dir}:${PATH}" \
  FAKE_GH_COMMENTS_JSON="${fixtures_dir}/c1_comments_empty.json" \
  FAKE_GH_LOG="${c1_log}" \
  "${comment_sh}" "${fixtures_dir}/c_basic_coverage.lcov" "${empty_exemptions}" "${fixtures_dir}/c_permissive_floor.toml"
assert_exit_code "C1: comment script exits 0 (no existing sticky)" 0 "${CAP_RC}"

if [ -f "${c1_log}" ]; then
  c1_body="$(sed -n '/^BODY_START$/,/^BODY_END$/p' "${c1_log}" | sed '1d;$d')"
  assert_contains "C1: no existing sticky issues a POST" "$(sed -n '1p' "${c1_log}")" "VERB=POST"
  assert_contains "C1: POST targets the PR comments collection" "$(sed -n '2p' "${c1_log}")" \
    "API_PATH=repos/${gh_repo}/issues/${gh_pr}/comments"
  assert_contains "C1: POST body starts with the sticky marker" "${c1_body}" "<!-- reborn-coverage-sticky -->"
  assert_contains "C1: POST body contains the Line coverage line" "${c1_body}" "Line coverage"
else
  report_fail "C1: fake gh did not record a mutation"
fi

# C2: existing sticky present (not first in the list) -> PATCH it, not a new POST.
c2_log="${tmp_root}/c2-gh.log"
capture env \
  GH_TOKEN="fake-token" \
  GITHUB_REPOSITORY="${gh_repo}" \
  PR_NUMBER="${gh_pr}" \
  PATH="${gh_bin_dir}:${PATH}" \
  FAKE_GH_COMMENTS_JSON="${fixtures_dir}/c2_comments_with_sticky.json" \
  FAKE_GH_LOG="${c2_log}" \
  "${comment_sh}" "${fixtures_dir}/c_basic_coverage.lcov" "${empty_exemptions}" "${fixtures_dir}/c_permissive_floor.toml"
assert_exit_code "C2: comment script exits 0 (existing sticky)" 0 "${CAP_RC}"

if [ -f "${c2_log}" ]; then
  assert_contains "C2: existing sticky (non-first in list) triggers a PATCH" "$(sed -n '1p' "${c2_log}")" "VERB=PATCH"
  assert_contains "C2: PATCH targets the matched comment id (99), not a POST" "$(sed -n '2p' "${c2_log}")" \
    "API_PATH=repos/${gh_repo}/issues/comments/99"
else
  report_fail "C2: fake gh did not record a mutation"
fi

# C3: zero-covered crates present -> callout line prepended before the header.
c3_log="${tmp_root}/c3-gh.log"
capture env \
  GH_TOKEN="fake-token" \
  GITHUB_REPOSITORY="${gh_repo}" \
  PR_NUMBER="${gh_pr}" \
  PATH="${gh_bin_dir}:${PATH}" \
  FAKE_GH_COMMENTS_JSON="${fixtures_dir}/c1_comments_empty.json" \
  FAKE_GH_LOG="${c3_log}" \
  "${comment_sh}" "${fixtures_dir}/c_zero_coverage.lcov" "${empty_exemptions}" "${fixtures_dir}/c_permissive_floor.toml"
assert_exit_code "C3: comment script exits 0 (zero-covered crate present)" 0 "${CAP_RC}"

if [ -f "${c3_log}" ]; then
  c3_body="$(sed -n '/^BODY_START$/,/^BODY_END$/p' "${c3_log}" | sed '1d;$d')"
  assert_contains "C3: body contains the 0-coverage callout" "${c3_body}" \
    "⚠️ 1 Reborn crate(s) have 0 int-tier coverage"
  assert_line_before "C3: callout is prepended before the coverage header" "${c3_body}" \
    "⚠️ 1 Reborn crate(s) have 0 int-tier coverage" "## Reborn integration-tier coverage"
else
  report_fail "C3: fake gh did not record a mutation"
fi

# C4: GH_TOKEN unset -> fast-fail before any `gh` call. `env -u` (not a subshell
# export/unset) keeps this the same shape as C1-C3 and shellcheck-clean.
capture env -u GH_TOKEN \
  GITHUB_REPOSITORY="${gh_repo}" \
  PR_NUMBER="${gh_pr}" \
  "${comment_sh}" "${fixtures_dir}/c_basic_coverage.lcov" "${empty_exemptions}" "${fixtures_dir}/c_permissive_floor.toml"
assert_exit_code "C4: GH_TOKEN unset exits non-zero" 1 "${CAP_RC}"
assert_contains "C4: GH_TOKEN unset reports the missing-var guard" "${CAP_ERR}" "GH_TOKEN must be set"

# C5: PR_NUMBER unset -> guard fires before GH_TOKEN is even checked.
capture env -u PR_NUMBER \
  GH_TOKEN="fake-token" \
  GITHUB_REPOSITORY="${gh_repo}" \
  "${comment_sh}" "${fixtures_dir}/c_basic_coverage.lcov" "${empty_exemptions}" "${fixtures_dir}/c_permissive_floor.toml"
assert_exit_code "C5: PR_NUMBER unset exits non-zero" 1 "${CAP_RC}"
assert_contains "C5: PR_NUMBER unset reports the missing-var guard" "${CAP_ERR}" "PR_NUMBER must be set"

# C6: GITHUB_REPOSITORY unset -> the first guard, fires immediately.
capture env -u GITHUB_REPOSITORY \
  GH_TOKEN="fake-token" \
  PR_NUMBER="${gh_pr}" \
  "${comment_sh}" "${fixtures_dir}/c_basic_coverage.lcov" "${empty_exemptions}" "${fixtures_dir}/c_permissive_floor.toml"
assert_exit_code "C6: GITHUB_REPOSITORY unset exits non-zero" 1 "${CAP_RC}"
assert_contains "C6: GITHUB_REPOSITORY unset reports the missing-var guard" "${CAP_ERR}" "GITHUB_REPOSITORY must be set"

# C7: missing coverage lcov -> the "if [ ! -f ]" guard at the top of
# comment.sh fires before GH_TOKEN/GITHUB_REPOSITORY/PR_NUMBER are even
# consulted, so no `gh` call — and therefore no mutation — is ever recorded.
c7_log="${tmp_root}/c7-gh.log"
capture env \
  GH_TOKEN="fake-token" \
  GITHUB_REPOSITORY="${gh_repo}" \
  PR_NUMBER="${gh_pr}" \
  PATH="${gh_bin_dir}:${PATH}" \
  FAKE_GH_COMMENTS_JSON="${fixtures_dir}/c1_comments_empty.json" \
  FAKE_GH_LOG="${c7_log}" \
  "${comment_sh}" "${fixtures_dir}/does_not_exist.lcov" "${empty_exemptions}" "${fixtures_dir}/c_permissive_floor.toml"
assert_exit_code "C7: comment script exits non-zero for missing coverage lcov" 1 "${CAP_RC}"
assert_contains "C7: comment script reports missing coverage lcov" "${CAP_ERR}" "coverage lcov file not found"
if [ -f "${c7_log}" ]; then
  report_fail "C7: fake gh did not record a mutation (guard fires before gh use)"
else
  report_pass "C7: fake gh did not record a mutation (guard fires before gh use)"
fi

# C8: missing exemptions manifest -> same "fires before gh use" guard shape.
c8_log="${tmp_root}/c8-gh.log"
capture env \
  GH_TOKEN="fake-token" \
  GITHUB_REPOSITORY="${gh_repo}" \
  PR_NUMBER="${gh_pr}" \
  PATH="${gh_bin_dir}:${PATH}" \
  FAKE_GH_COMMENTS_JSON="${fixtures_dir}/c1_comments_empty.json" \
  FAKE_GH_LOG="${c8_log}" \
  "${comment_sh}" "${fixtures_dir}/c_basic_coverage.lcov" "${fixtures_dir}/does_not_exist_exemptions.toml" "${fixtures_dir}/c_permissive_floor.toml"
assert_exit_code "C8: comment script exits non-zero for missing exemptions manifest" 1 "${CAP_RC}"
assert_contains "C8: comment script reports missing exemptions manifest" "${CAP_ERR}" "coverage exemptions manifest not found"
if [ -f "${c8_log}" ]; then
  report_fail "C8: fake gh did not record a mutation (guard fires before gh use)"
else
  report_pass "C8: fake gh did not record a mutation (guard fires before gh use)"
fi

# C9: a zero-covered crate excluded by a whole-crate exemption must not surface
# in the sticky comment's 0-coverage callout (reuses the A10/B4 fixture pair —
# ironclaw_embeddings is 0/10 but exempted, ironclaw_reborn is 5/10 not zero).
c9_log="${tmp_root}/c9-gh.log"
capture env \
  GH_TOKEN="fake-token" \
  GITHUB_REPOSITORY="${gh_repo}" \
  PR_NUMBER="${gh_pr}" \
  PATH="${gh_bin_dir}:${PATH}" \
  FAKE_GH_COMMENTS_JSON="${fixtures_dir}/c1_comments_empty.json" \
  FAKE_GH_LOG="${c9_log}" \
  "${comment_sh}" "${fixtures_dir}/a10_two_crates.lcov" "${fixtures_dir}/a10_crate_exemption.toml" "${fixtures_dir}/c_permissive_floor.toml"
assert_exit_code "C9: comment script exits 0 (zero-covered crate whole-crate-exempted)" 0 "${CAP_RC}"
if [ -f "${c9_log}" ]; then
  c9_body="$(sed -n '/^BODY_START$/,/^BODY_END$/p' "${c9_log}" | sed '1d;$d')"
  assert_not_contains "C9: sticky comment omits the 0-coverage callout for the exempted crate" \
    "${c9_body}" "0 int-tier coverage"
else
  report_fail "C9: fake gh did not record a mutation"
fi

# C10: the ratchet section is rendered at the very top of the comment body —
# after the (hidden, first-line) marker, but before the 0%-crate callout and
# the coverage header (decision: simplest placement given
# reborn-coverage-summary.sh has no exposed seam to splice "before the
# per-crate table" specifically — see reborn-coverage-comment.sh's own
# header comment).
c10_log="${tmp_root}/c10-gh.log"
capture env \
  GH_TOKEN="fake-token" \
  GITHUB_REPOSITORY="${gh_repo}" \
  PR_NUMBER="${gh_pr}" \
  PATH="${gh_bin_dir}:${PATH}" \
  FAKE_GH_COMMENTS_JSON="${fixtures_dir}/c1_comments_empty.json" \
  FAKE_GH_LOG="${c10_log}" \
  "${comment_sh}" "${fixtures_dir}/c_zero_coverage.lcov" "${empty_exemptions}" "${fixtures_dir}/c_permissive_floor.toml"
assert_exit_code "C10: comment script exits 0 (ratchet section present)" 0 "${CAP_RC}"
if [ -f "${c10_log}" ]; then
  c10_body="$(sed -n '/^BODY_START$/,/^BODY_END$/p' "${c10_log}" | sed '1d;$d')"
  assert_contains "C10: body contains the ratchet section heading" "${c10_body}" "### Coverage ratchet"
  assert_contains "C10: ratchet section carries the unconditional mode banner" "${c10_body}" "Ratchet mode: DRY-RUN"
  assert_line_before "C10: ratchet section precedes the 0%-crate callout" "${c10_body}" \
    "### Coverage ratchet" "⚠️ 1 Reborn crate(s) have 0 int-tier coverage"
  assert_line_before "C10: ratchet section precedes the coverage header" "${c10_body}" \
    "### Coverage ratchet" "## Reborn integration-tier coverage"
else
  report_fail "C10: fake gh did not record a mutation"
fi

# C11: missing floor manifest -> same "fires before gh use" guard shape as
# C7/C8 (file-existence guards run before GH_TOKEN/GITHUB_REPOSITORY/PR_NUMBER
# are consulted, and before any `gh` call).
c11_log="${tmp_root}/c11-gh.log"
capture env \
  GH_TOKEN="fake-token" \
  GITHUB_REPOSITORY="${gh_repo}" \
  PR_NUMBER="${gh_pr}" \
  PATH="${gh_bin_dir}:${PATH}" \
  FAKE_GH_COMMENTS_JSON="${fixtures_dir}/c1_comments_empty.json" \
  FAKE_GH_LOG="${c11_log}" \
  "${comment_sh}" "${fixtures_dir}/c_basic_coverage.lcov" "${empty_exemptions}" "${fixtures_dir}/does_not_exist_floor.toml"
assert_exit_code "C11: comment script exits non-zero for missing floor manifest" 1 "${CAP_RC}"
assert_contains "C11: comment script reports missing floor manifest" "${CAP_ERR}" "coverage floor manifest not found"
if [ -f "${c11_log}" ]; then
  report_fail "C11: fake gh did not record a mutation (guard fires before gh use)"
else
  report_pass "C11: fake gh did not record a mutation (guard fires before gh use)"
fi

# C12: floor manifest exists (passes the C11 preflight guard) but is
# malformed — missing [global] — so reborn-coverage-ratchet.sh itself exits
# 1. The comment script's `|| true` around that call must still render the
# ratchet's stderr into the sticky comment body and exit 0 (visibility-only,
# never gates). Pins PR #5718 user comment.
cat > "${fixtures_dir}/c12_malformed_floor.toml" <<'TOML'
[[crate]]
name = "ironclaw_reborn"
floor_percent = 90.0
TOML
c12_log="${tmp_root}/c12-gh.log"
capture env \
  GH_TOKEN="fake-token" \
  GITHUB_REPOSITORY="${gh_repo}" \
  PR_NUMBER="${gh_pr}" \
  PATH="${gh_bin_dir}:${PATH}" \
  FAKE_GH_COMMENTS_JSON="${fixtures_dir}/c1_comments_empty.json" \
  FAKE_GH_LOG="${c12_log}" \
  "${comment_sh}" "${fixtures_dir}/c_basic_coverage.lcov" "${empty_exemptions}" "${fixtures_dir}/c12_malformed_floor.toml"
assert_exit_code "C12: comment script still exits 0 for a malformed (but present) floor manifest" 0 "${CAP_RC}"
if [ -f "${c12_log}" ]; then
  c12_body="$(sed -n '/^BODY_START$/,/^BODY_END$/p' "${c12_log}" | sed '1d;$d')"
  assert_contains "C12: rendered comment carries the ratchet schema error" "${c12_body}" \
    "missing required [global] section"
else
  report_fail "C12: fake gh did not record a mutation (comment must still render/upsert)"
fi

# ---------------------------------------------------------------------------
# D. reborn-coverage-int-tier-tests.sh (int-tier suite discovery)
# ---------------------------------------------------------------------------
#
# The script derives its repo root from its own path and `cd`s there, so
# each case copies it into a fresh temp tree's scripts/ci/ and builds a
# tests/integration/ subtree alongside it, then invokes the copy. It also
# filters candidates against a `[[test]] name = "..."` entry in Cargo.toml
# (see that script's header comment), so every case seeds a fake Cargo.toml
# with one `[[test]]` block per fixture suite the case constructs — mirrors
# the real repo root always having a `[[test]]` entry per suite.

setup_int_tier_case() {
  local case_dir="$1"
  shift
  mkdir -p "${case_dir}/scripts/ci" "${case_dir}/tests/integration"
  cp "${int_tier_sh}" "${case_dir}/scripts/ci/reborn-coverage-int-tier-tests.sh"
  chmod +x "${case_dir}/scripts/ci/reborn-coverage-int-tier-tests.sh"
  : > "${case_dir}/Cargo.toml"
  local candidate
  for candidate in "$@"; do
    cat >>"${case_dir}/Cargo.toml" <<EOF
[[test]]
name = "${candidate}"
path = "tests/integration/${candidate}.rs"

EOF
  done
}

# D1: empty tests/integration/ -> non-zero exit + discovery error.
d1="${tmp_root}/d1"
setup_int_tier_case "${d1}"
capture "${d1}/scripts/ci/reborn-coverage-int-tier-tests.sh"
assert_exit_code "D1: empty tests/integration/ exits non-zero" 1 "${CAP_RC}"
assert_contains "D1: empty tests/integration/ prints the discovery error" "${CAP_ERR}" \
  "No Reborn integration-tier test binaries discovered"

# D2: one tests/integration/foo.rs -> --test / reborn_integration_foo.
d2="${tmp_root}/d2"
setup_int_tier_case "${d2}" reborn_integration_foo
: > "${d2}/tests/integration/foo.rs"
capture "${d2}/scripts/ci/reborn-coverage-int-tier-tests.sh"
assert_exit_code "D2: single flat integration file exits 0" 0 "${CAP_RC}"
assert_eq "D2: single flat integration file emits its --test pair" \
  "$(printf -- '--test\nreborn_integration_foo')" "${CAP_OUT}"

# D3: one tests/integration/group_bar/main.rs -> --test / reborn_group_bar.
d3="${tmp_root}/d3"
setup_int_tier_case "${d3}" reborn_group_bar
mkdir -p "${d3}/tests/integration/group_bar"
: > "${d3}/tests/integration/group_bar/main.rs"
capture "${d3}/scripts/ci/reborn-coverage-int-tier-tests.sh"
assert_exit_code "D3: single group dir exits 0" 0 "${CAP_RC}"
assert_eq "D3: single group dir emits its --test pair, dir->name rewrite applied" \
  "$(printf -- '--test\nreborn_group_bar')" "${CAP_OUT}"

# D3b: a half-scaffolded group dir (no main.rs yet) is skipped, not errored.
d3b="${tmp_root}/d3b"
setup_int_tier_case "${d3b}" reborn_group_bar
mkdir -p "${d3b}/tests/integration/group_bar" "${d3b}/tests/integration/group_incomplete"
: > "${d3b}/tests/integration/group_bar/main.rs"
capture "${d3b}/scripts/ci/reborn-coverage-int-tier-tests.sh"
assert_exit_code "D3b: half-scaffolded group dir does not error the whole discovery" 0 "${CAP_RC}"
assert_eq "D3b: half-scaffolded group dir (no main.rs) is skipped" \
  "$(printf -- '--test\nreborn_group_bar')" "${CAP_OUT}"

# D4: multiple files + dirs, created out of alphabetical order -> sorted,
# deduped output. Group dirs ('g') sort before integration files ('i').
d4="${tmp_root}/d4"
setup_int_tier_case "${d4}" reborn_group_beta reborn_group_omega reborn_integration_alpha reborn_integration_zeta
: > "${d4}/tests/integration/zeta.rs"
: > "${d4}/tests/integration/alpha.rs"
mkdir -p "${d4}/tests/integration/group_omega" "${d4}/tests/integration/group_beta"
: > "${d4}/tests/integration/group_omega/main.rs"
: > "${d4}/tests/integration/group_beta/main.rs"
capture "${d4}/scripts/ci/reborn-coverage-int-tier-tests.sh"
assert_exit_code "D4: multiple suites exits 0" 0 "${CAP_RC}"
assert_eq "D4: multiple suites sorted+deduped in expected order" \
  "$(printf -- '--test\nreborn_group_beta\n--test\nreborn_group_omega\n--test\nreborn_integration_alpha\n--test\nreborn_integration_zeta')" \
  "${CAP_OUT}"

# D5: a `support/` subdirectory alongside the flat files must not be
# mistaken for a suite (no main.rs, and doesn't match the `group_*` name
# pattern either) — mirrors the real tests/integration/support/ harness tree.
d5="${tmp_root}/d5"
setup_int_tier_case "${d5}" reborn_integration_only
: > "${d5}/tests/integration/only.rs"
mkdir -p "${d5}/tests/integration/support"
: > "${d5}/tests/integration/support/mod.rs"
capture "${d5}/scripts/ci/reborn-coverage-int-tier-tests.sh"
assert_exit_code "D5: support/ dir alongside flat suites exits 0" 0 "${CAP_RC}"
assert_eq "D5: support/ dir is not discovered as a suite" \
  "$(printf -- '--test\nreborn_integration_only')" "${CAP_OUT}"

# ---------------------------------------------------------------------------
# R. reborn-coverage-ratchet.sh (coverage-floor ratchet gate)
# ---------------------------------------------------------------------------
#
# Split into test-reborn-coverage-ratchet-cases.sh (sourced below, sharing
# this script's helpers/fixtures/counters) to keep this file under 1000
# lines as a fifth script's worth of cases joined the suite.

# shellcheck source=./test-reborn-coverage-ratchet-cases.sh
source "${script_dir}/test-reborn-coverage-ratchet-cases.sh"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

printf '\n%s of %s cases passed\n' "${PASS_COUNT}" "$((PASS_COUNT + FAIL_COUNT))"
if [ "${FAIL_COUNT}" -gt 0 ]; then
  printf '%s case(s) FAILED\n' "${FAIL_COUNT}" >&2
  exit 1
fi
exit 0
