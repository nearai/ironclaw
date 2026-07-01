#!/usr/bin/env bash
#
# Regression tests for the three Reborn coverage CI helpers:
#   - reborn-coverage-summary.sh        (report + --zero-crates modes)
#   - reborn-coverage-comment.sh        (sticky PR comment upsert via `gh api`)
#   - reborn-coverage-int-tier-tests.sh (int-tier suite discovery)
#
# Mirrors test-classify-test-scope.sh: self-contained, locates the
# scripts-under-test relative to this file's own directory, builds its own
# fixtures in a mktemp dir, and reports PASS/FAIL per case. Unlike that
# precedent (which exits on the first failure), this suite runs every case
# and prints a final summary, exiting non-zero only if something failed —
# with three scripts and ~20 cases, seeing the full picture in one run beats
# stopping at the first mismatch.
#
# reborn-coverage-comment.sh shells out to `gh api`. It is exercised here
# against a fake `gh` (a fixture script placed first on PATH) that emulates
# `gh api --paginate <path> --jq '<filter>'` by running the given jq filter
# over a canned comments JSON array, and records the verb/path/body of any
# mutating call (-X POST / -X PATCH) to a log file this suite inspects.
#
# reborn-coverage-int-tier-tests.sh derives its repo root from its own path
# (`$(dirname BASH_SOURCE)/../..`) and `cd`s there, so it cannot simply be
# pointed at a fixture tree via an argument. Each case here copies the real
# script into a temp tree's scripts/ci/ and builds a tests/ subtree next to
# it, so the copy's own repo-root resolution lands on the temp tree.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
summary_sh="${script_dir}/reborn-coverage-summary.sh"
comment_sh="${script_dir}/reborn-coverage-comment.sh"
int_tier_sh="${script_dir}/reborn-coverage-int-tier-tests.sh"

tmp_root="$(mktemp -d)"
trap 'rm -rf "${tmp_root}"' EXIT

fixtures_dir="${tmp_root}/fixtures"
mkdir -p "${fixtures_dir}"

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
# A. reborn-coverage-summary.sh (default report mode)
# ---------------------------------------------------------------------------

cat > "${fixtures_dir}/a1_mixed.json" <<'JSON'
{
  "data": [
    {
      "files": [
        { "filename": "/work/ironclaw/crates/ironclaw_reborn/src/runtime.rs", "summary": { "lines": { "covered": 80, "count": 100 } } },
        { "filename": "/work/ironclaw/crates/ironclaw_product_workflow/src/lib.rs", "summary": { "lines": { "covered": 50, "count": 50 } } },
        { "filename": "/work/ironclaw/crates/ironclaw_engine/src/lib.rs", "summary": { "lines": { "covered": 10, "count": 10 } } }
      ]
    }
  ]
}
JSON

capture "${summary_sh}" "${fixtures_dir}/a1_mixed.json"
assert_exit_code "A1: summary exits 0 for mixed Reborn/non-Reborn fixture" 0 "${CAP_RC}"
assert_not_contains "A1: non-Reborn crate excluded from output" "${CAP_OUT}" "ironclaw_engine"
assert_contains "A1: aggregate matches hand-computed 86.67% (130/150)" "${CAP_OUT}" \
  '**Line coverage (Reborn crates): 86.67%** — 130 / 150 lines'
assert_contains "A1: table includes ironclaw_reborn row" "${CAP_OUT}" "| \`ironclaw_reborn\` | 80% | 80 / 100 |"
assert_contains "A1: table includes ironclaw_product_workflow row" "${CAP_OUT}" \
  "| \`ironclaw_product_workflow\` | 100% | 50 / 50 |"

cat > "${fixtures_dir}/a2_non_reborn_only.json" <<'JSON'
{
  "data": [
    { "files": [ { "filename": "/work/ironclaw/crates/ironclaw_engine/src/lib.rs", "summary": { "lines": { "covered": 10, "count": 10 } } } ] }
  ]
}
JSON

capture "${summary_sh}" "${fixtures_dir}/a2_non_reborn_only.json"
assert_exit_code "A2: summary exits 0 when only non-Reborn crates present" 0 "${CAP_RC}"
assert_contains "A2: prints no-data message when no Reborn files match" "${CAP_OUT}" \
  "No Reborn crate coverage data found"

printf '{"data":[]}' > "${fixtures_dir}/a3_empty_data.json"
printf '{}' > "${fixtures_dir}/a3_absent_data.json"

capture "${summary_sh}" "${fixtures_dir}/a3_empty_data.json"
assert_exit_code 'A3: {"data":[]} exits 0' 0 "${CAP_RC}"
assert_contains 'A3: {"data":[]} prints no-data message' "${CAP_OUT}" "No Reborn crate coverage data found"

capture "${summary_sh}" "${fixtures_dir}/a3_absent_data.json"
assert_exit_code 'A3: {} exits 0' 0 "${CAP_RC}"
assert_contains 'A3: {} prints no-data message' "${CAP_OUT}" "No Reborn crate coverage data found"

cat > "${fixtures_dir}/a4_multi_dataset.json" <<'JSON'
{
  "data": [
    { "files": [ { "filename": "/work/ironclaw/crates/ironclaw_reborn/src/a.rs", "summary": { "lines": { "covered": 10, "count": 20 } } } ] },
    { "files": [ { "filename": "/work/ironclaw/crates/ironclaw_reborn_cli/src/main.rs", "summary": { "lines": { "covered": 5, "count": 5 } } } ] }
  ]
}
JSON

capture "${summary_sh}" "${fixtures_dir}/a4_multi_dataset.json"
assert_exit_code "A4: multi-dataset summary exits 0" 0 "${CAP_RC}"
assert_contains "A4: aggregate counts files from BOTH data[] entries (15/25)" "${CAP_OUT}" \
  '**Line coverage (Reborn crates): 60%** — 15 / 25 lines'
assert_contains "A4: includes crate from data[0]" "${CAP_OUT}" "\`ironclaw_reborn\`"
assert_contains "A4: includes crate from data[1]" "${CAP_OUT}" "\`ironclaw_reborn_cli\`"

cat > "${fixtures_dir}/a5_zero_sorted.json" <<'JSON'
{
  "data": [
    {
      "files": [
        { "filename": "/work/ironclaw/crates/ironclaw_reborn_zero/src/a.rs", "summary": { "lines": { "covered": 0, "count": 10 } } },
        { "filename": "/work/ironclaw/crates/ironclaw_reborn_half/src/a.rs", "summary": { "lines": { "covered": 5, "count": 10 } } }
      ]
    }
  ]
}
JSON

capture "${summary_sh}" "${fixtures_dir}/a5_zero_sorted.json"
assert_exit_code "A5: zero-covered-crate fixture summary exits 0" 0 "${CAP_RC}"
assert_contains "A5: zero-covered crate row shows 0%" "${CAP_OUT}" "| \`ironclaw_reborn_zero\` | 0% | 0 / 10 |"
assert_line_before "A5: zero-covered crate sorted to top (lowest-covered first)" "${CAP_OUT}" \
  "\`ironclaw_reborn_zero\`" "\`ironclaw_reborn_half\`"

# A6: allowlist boundary — two exact-match single crates, one family-prefix
# crate, and a lookalike (ironclaw_architecture_extra) that must be dropped:
# it is not one of the four exact-match crates and does not start with a
# reborn/product/webui_v2 family prefix.
cat > "${fixtures_dir}/a6_allowlist_boundary.json" <<'JSON'
{
  "data": [
    {
      "files": [
        { "filename": "/work/ironclaw/crates/ironclaw_architecture/src/a.rs", "summary": { "lines": { "covered": 5, "count": 10 } } },
        { "filename": "/work/ironclaw/crates/ironclaw_slack_v2_adapter/src/a.rs", "summary": { "lines": { "covered": 3, "count": 10 } } },
        { "filename": "/work/ironclaw/crates/ironclaw_reborn_config/src/a.rs", "summary": { "lines": { "covered": 8, "count": 10 } } },
        { "filename": "/work/ironclaw/crates/ironclaw_architecture_extra/src/a.rs", "summary": { "lines": { "covered": 0, "count": 999 } } }
      ]
    }
  ]
}
JSON

capture "${summary_sh}" "${fixtures_dir}/a6_allowlist_boundary.json"
assert_exit_code "A6: allowlist boundary fixture summary exits 0" 0 "${CAP_RC}"
assert_contains "A6: table includes exact-match ironclaw_architecture row" "${CAP_OUT}" \
  "| \`ironclaw_architecture\` | 50% | 5 / 10 |"
assert_contains "A6: table includes exact-match ironclaw_slack_v2_adapter row" "${CAP_OUT}" \
  "| \`ironclaw_slack_v2_adapter\` | 30% | 3 / 10 |"
assert_contains "A6: table includes family-prefix ironclaw_reborn_config row" "${CAP_OUT}" \
  "| \`ironclaw_reborn_config\` | 80% | 8 / 10 |"
assert_not_contains "A6: lookalike ironclaw_architecture_extra excluded from table" "${CAP_OUT}" \
  "ironclaw_architecture_extra"
assert_contains "A6: aggregate drops lookalike's 999 lines (16/30, not 16/1029)" "${CAP_OUT}" \
  '**Line coverage (Reborn crates): 53.33%** — 16 / 30 lines'

# A7: missing coverage JSON -> non-zero exit + not-found error on stderr.
capture "${summary_sh}" "${fixtures_dir}/does_not_exist.json"
assert_exit_code "A7: summary exits non-zero for missing coverage JSON" 1 "${CAP_RC}"
assert_contains "A7: summary reports missing coverage JSON" "${CAP_ERR}" "coverage JSON not found"

# ---------------------------------------------------------------------------
# B. reborn-coverage-summary.sh --zero-crates
# ---------------------------------------------------------------------------

cat > "${fixtures_dir}/b1_mixed_zero.json" <<'JSON'
{
  "data": [
    {
      "files": [
        { "filename": "/work/ironclaw/crates/ironclaw_reborn_zero_a/src/a.rs", "summary": { "lines": { "covered": 0, "count": 5 } } },
        { "filename": "/work/ironclaw/crates/ironclaw_reborn_zero_b/src/a.rs", "summary": { "lines": { "covered": 0, "count": 3 } } },
        { "filename": "/work/ironclaw/crates/ironclaw_reborn_partial/src/a.rs", "summary": { "lines": { "covered": 4, "count": 10 } } },
        { "filename": "/work/ironclaw/crates/ironclaw_reborn_full/src/a.rs", "summary": { "lines": { "covered": 10, "count": 10 } } }
      ]
    }
  ]
}
JSON

capture "${summary_sh}" --zero-crates "${fixtures_dir}/b1_mixed_zero.json"
assert_exit_code "B1: --zero-crates exits 0" 0 "${CAP_RC}"
assert_eq "B1: --zero-crates emits exactly the 2 zero-covered crate names" \
  "$(printf 'ironclaw_reborn_zero_a\nironclaw_reborn_zero_b')" "${CAP_OUT}"

cat > "${fixtures_dir}/b2_all_covered.json" <<'JSON'
{
  "data": [
    { "files": [ { "filename": "/work/ironclaw/crates/ironclaw_reborn_full/src/a.rs", "summary": { "lines": { "covered": 10, "count": 10 } } } ] }
  ]
}
JSON

capture "${summary_sh}" --zero-crates "${fixtures_dir}/b2_all_covered.json"
assert_exit_code "B2: all-covered fixture --zero-crates exits 0" 0 "${CAP_RC}"
assert_eq "B2: all-covered fixture --zero-crates emits nothing" "" "${CAP_OUT}"

capture "${summary_sh}" --zero-crates "${fixtures_dir}/a3_empty_data.json"
assert_exit_code "B3: empty data --zero-crates exits 0" 0 "${CAP_RC}"
assert_eq "B3: empty data --zero-crates emits nothing" "" "${CAP_OUT}"

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

cat > "${fixtures_dir}/c_basic_coverage.json" <<'JSON'
{
  "data": [
    { "files": [ { "filename": "/work/ironclaw/crates/ironclaw_reborn/src/a.rs", "summary": { "lines": { "covered": 8, "count": 10 } } } ] }
  ]
}
JSON

cat > "${fixtures_dir}/c_zero_coverage.json" <<'JSON'
{
  "data": [
    { "files": [ { "filename": "/work/ironclaw/crates/ironclaw_reborn_zero/src/a.rs", "summary": { "lines": { "covered": 0, "count": 5 } } } ] }
  ]
}
JSON

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
  "${comment_sh}" "${fixtures_dir}/c_basic_coverage.json"
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
  "${comment_sh}" "${fixtures_dir}/c_basic_coverage.json"
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
  "${comment_sh}" "${fixtures_dir}/c_zero_coverage.json"
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
  "${comment_sh}" "${fixtures_dir}/c_basic_coverage.json"
assert_exit_code "C4: GH_TOKEN unset exits non-zero" 1 "${CAP_RC}"
assert_contains "C4: GH_TOKEN unset reports the missing-var guard" "${CAP_ERR}" "GH_TOKEN must be set"

# C5: PR_NUMBER unset -> guard fires before GH_TOKEN is even checked.
capture env -u PR_NUMBER \
  GH_TOKEN="fake-token" \
  GITHUB_REPOSITORY="${gh_repo}" \
  "${comment_sh}" "${fixtures_dir}/c_basic_coverage.json"
assert_exit_code "C5: PR_NUMBER unset exits non-zero" 1 "${CAP_RC}"
assert_contains "C5: PR_NUMBER unset reports the missing-var guard" "${CAP_ERR}" "PR_NUMBER must be set"

# C6: GITHUB_REPOSITORY unset -> the first guard, fires immediately.
capture env -u GITHUB_REPOSITORY \
  GH_TOKEN="fake-token" \
  PR_NUMBER="${gh_pr}" \
  "${comment_sh}" "${fixtures_dir}/c_basic_coverage.json"
assert_exit_code "C6: GITHUB_REPOSITORY unset exits non-zero" 1 "${CAP_RC}"
assert_contains "C6: GITHUB_REPOSITORY unset reports the missing-var guard" "${CAP_ERR}" "GITHUB_REPOSITORY must be set"

# C7: missing coverage JSON -> the "if [ ! -f ]" guard at the top of
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
  "${comment_sh}" "${fixtures_dir}/does_not_exist.json"
assert_exit_code "C7: comment script exits non-zero for missing coverage JSON" 1 "${CAP_RC}"
assert_contains "C7: comment script reports missing coverage JSON" "${CAP_ERR}" "coverage JSON not found"
if [ -f "${c7_log}" ]; then
  report_fail "C7: fake gh did not record a mutation (guard fires before gh use)"
else
  report_pass "C7: fake gh did not record a mutation (guard fires before gh use)"
fi

# C8: existing-but-malformed coverage JSON aborts before any gh mutation.
#
# Distinct path from C7: this fixture file exists, so it passes comment.sh's
# "[ -f ]" guard and proceeds to call reborn-coverage-summary.sh to render
# the body — and that render fails on jq's parse error under `set -e`,
# before any POST/PATCH is ever issued. Pins the "render-before-mutate"
# ordering.
cat > "${fixtures_dir}/c8_malformed.json" <<'JSON'
{ this is not valid json
JSON

c8_log="${tmp_root}/c8-gh.log"
capture env \
  GH_TOKEN="fake-token" \
  GITHUB_REPOSITORY="${gh_repo}" \
  PR_NUMBER="${gh_pr}" \
  PATH="${gh_bin_dir}:${PATH}" \
  FAKE_GH_COMMENTS_JSON="${fixtures_dir}/c1_comments_empty.json" \
  FAKE_GH_LOG="${c8_log}" \
  "${comment_sh}" "${fixtures_dir}/c8_malformed.json"
# Non-zero, not an exact code: jq's parse-error exit differs across versions.
if [ "${CAP_RC}" -ne 0 ]; then
  report_pass "C8: comment script exits non-zero on malformed coverage JSON"
else
  report_fail "C8: comment script exits non-zero on malformed coverage JSON (got 0)"
fi
if [ -f "${c8_log}" ]; then
  report_fail "C8: fake gh did not record a mutation (render fails before gh use)"
else
  report_pass "C8: fake gh did not record a mutation (render fails before gh use)"
fi

# ---------------------------------------------------------------------------
# D. reborn-coverage-int-tier-tests.sh (int-tier suite discovery)
# ---------------------------------------------------------------------------
#
# The script derives its repo root from its own path and `cd`s there, so
# each case copies it into a fresh temp tree's scripts/ci/ and builds a
# tests/ subtree alongside it, then invokes the copy.

setup_int_tier_case() {
  local case_dir="$1"
  mkdir -p "${case_dir}/scripts/ci" "${case_dir}/tests"
  cp "${int_tier_sh}" "${case_dir}/scripts/ci/reborn-coverage-int-tier-tests.sh"
  chmod +x "${case_dir}/scripts/ci/reborn-coverage-int-tier-tests.sh"
}

# D1: empty tests/ -> non-zero exit + discovery error.
d1="${tmp_root}/d1"
setup_int_tier_case "${d1}"
capture "${d1}/scripts/ci/reborn-coverage-int-tier-tests.sh"
assert_exit_code "D1: empty tests/ exits non-zero" 1 "${CAP_RC}"
assert_contains "D1: empty tests/ prints the discovery error" "${CAP_ERR}" \
  "No Reborn integration-tier test binaries discovered"

# D2: one tests/reborn_integration_foo.rs -> --test / reborn_integration_foo.
d2="${tmp_root}/d2"
setup_int_tier_case "${d2}"
: > "${d2}/tests/reborn_integration_foo.rs"
capture "${d2}/scripts/ci/reborn-coverage-int-tier-tests.sh"
assert_exit_code "D2: single integration file exits 0" 0 "${CAP_RC}"
assert_eq "D2: single integration file emits its --test pair" \
  "$(printf -- '--test\nreborn_integration_foo')" "${CAP_OUT}"

# D3: one tests/reborn_group_bar/ -> --test / reborn_group_bar.
d3="${tmp_root}/d3"
setup_int_tier_case "${d3}"
mkdir -p "${d3}/tests/reborn_group_bar"
capture "${d3}/scripts/ci/reborn-coverage-int-tier-tests.sh"
assert_exit_code "D3: single group dir exits 0" 0 "${CAP_RC}"
assert_eq "D3: single group dir emits its --test pair" \
  "$(printf -- '--test\nreborn_group_bar')" "${CAP_OUT}"

# D4: multiple files + dirs, created out of alphabetical order -> sorted,
# deduped output. Group dirs ('g') sort before integration files ('i').
d4="${tmp_root}/d4"
setup_int_tier_case "${d4}"
: > "${d4}/tests/reborn_integration_zeta.rs"
: > "${d4}/tests/reborn_integration_alpha.rs"
mkdir -p "${d4}/tests/reborn_group_omega" "${d4}/tests/reborn_group_beta"
capture "${d4}/scripts/ci/reborn-coverage-int-tier-tests.sh"
assert_exit_code "D4: multiple suites exits 0" 0 "${CAP_RC}"
assert_eq "D4: multiple suites sorted+deduped in expected order" \
  "$(printf -- '--test\nreborn_group_beta\n--test\nreborn_group_omega\n--test\nreborn_integration_alpha\n--test\nreborn_integration_zeta')" \
  "${CAP_OUT}"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

printf '\n%s of %s cases passed\n' "${PASS_COUNT}" "$((PASS_COUNT + FAIL_COUNT))"
if [ "${FAIL_COUNT}" -gt 0 ]; then
  printf '%s case(s) FAILED\n' "${FAIL_COUNT}" >&2
  exit 1
fi
exit 0
