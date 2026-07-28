#!/usr/bin/env bash
#
# Regression tests for pr-labeler.sh — the PR size/risk/contributor classifier.
#
# Standalone: bash .github/scripts/test-pr-labeler.sh
# Also run in CI (.github/workflows/code_style.yml, "Static-check self-tests")
# whenever the labeler or this test changes — guardrails are code
# (.claude/rules/review-discipline.md: "Checks and hooks need regression tests
# ... and must run when their own files change").
#
# Pins the #6167-class flake: a transient GitHub API error returns an HTML
# page, `gh --jq` aborts with `invalid character '<'`, and under `set -e` the
# whole classify job used to fail and block the PR over labels-only work.
# Covers (1) gh_retry's retry/backoff and give-up behavior, and (2) the
# end-to-end guarantee that a persistently failing API still exits 0.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
labeler="${script_dir}/pr-labeler.sh"

PASS=0
FAIL=0

assert_rc() {
    local name="$1" want="$2" got="$3"
    if [ "${got}" -eq "${want}" ]; then PASS=$((PASS+1));
    else FAIL=$((FAIL+1)); echo "FAIL: ${name} — expected exit ${want}, got ${got}"; fi
}

assert_rc_nonzero() {
    local name="$1" got="$2"
    if [ "${got}" -ne 0 ]; then PASS=$((PASS+1));
    else FAIL=$((FAIL+1)); echo "FAIL: ${name} — expected non-zero exit, got 0"; fi
}

# Pure-bash substring match — no pipes, so immune to SIGPIPE under pipefail.
assert_contains() {
    local name="$1" hay="$2" needle="$3"
    if [[ "${hay}" == *"${needle}"* ]]; then PASS=$((PASS+1));
    else FAIL=$((FAIL+1)); echo "FAIL: ${name} — output missing: ${needle}"; echo "----"; echo "${hay}"; echo "----"; fi
}

assert_not_contains() {
    local name="$1" hay="$2" needle="$3"
    if [[ "${hay}" != *"${needle}"* ]]; then PASS=$((PASS+1));
    else FAIL=$((FAIL+1)); echo "FAIL: ${name} — output should NOT contain: ${needle}"; echo "----"; echo "${hay}"; echo "----"; fi
}

assert_eq() {
    local name="$1" want="$2" got="$3"
    if [ "${got}" = "${want}" ]; then PASS=$((PASS+1));
    else FAIL=$((FAIL+1)); echo "FAIL: ${name} — expected '${want}', got '${got}'"; fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

# ---------------------------------------------------------------------------
# Unit tests: gh_retry (source the labeler so main() does not run).
# ---------------------------------------------------------------------------
# shellcheck source=/dev/null
source "${labeler}"

# A command that fails (exit 1) until it has been called more than $2 times,
# tracking the count in file $1. On success it prints a recognizable value.
flaky="${tmp}/flaky.sh"
cat > "${flaky}" <<'EOF'
#!/usr/bin/env bash
count_file="$1"; fail_until="$2"
n=$(cat "${count_file}" 2>/dev/null || echo 0); n=$((n+1)); echo "${n}" > "${count_file}"
if (( n <= fail_until )); then echo "boom ${n}" >&2; exit 1; fi
echo "value-after-${n}"
EOF
chmod +x "${flaky}"

# 1. Immediate success — value forwarded, rc 0, exactly one call.
cf="${tmp}/c1"; : > "${cf}"
rc=0; out=$(GH_RETRY_SLEEP=0 GH_RETRY_ATTEMPTS=4 gh_retry "${flaky}" "${cf}" 0) || rc=$?
assert_rc       "gh_retry immediate success rc" 0 "${rc}"
assert_contains "gh_retry immediate success value" "${out}" "value-after-1"
assert_eq       "gh_retry immediate success call count" "1" "$(cat "${cf}")"

# 2. Transient failures then success — retries until it works.
cf="${tmp}/c2"; : > "${cf}"
rc=0; out=$(GH_RETRY_SLEEP=0 GH_RETRY_ATTEMPTS=4 gh_retry "${flaky}" "${cf}" 2) || rc=$?
assert_rc       "gh_retry transient rc" 0 "${rc}"
assert_contains "gh_retry transient value" "${out}" "value-after-3"
assert_eq       "gh_retry transient call count" "3" "$(cat "${cf}")"

# 3. Persistent failure — gives up after GH_RETRY_ATTEMPTS and returns non-zero.
cf="${tmp}/c3"; : > "${cf}"
rc=0; out=$(GH_RETRY_SLEEP=0 GH_RETRY_ATTEMPTS=3 gh_retry "${flaky}" "${cf}" 99) || rc=$?
assert_rc_nonzero "gh_retry give-up rc" "${rc}"
assert_eq         "gh_retry give-up call count" "3" "$(cat "${cf}")"

# ---------------------------------------------------------------------------
# End-to-end tests: run the labeler as a subprocess with a fake `gh` on PATH.
# ---------------------------------------------------------------------------
fakebin="${tmp}/bin"
mkdir -p "${fakebin}"
cat > "${fakebin}/gh" <<'EOF'
#!/usr/bin/env bash
# Fake gh for pr-labeler tests. Env knobs:
#   FAKE_GH_FAIL_UNTIL / FAKE_GH_COUNT_FILE : fail (HTML-ish, exit 1) for the
#     first N total invocations, simulating a transient API outage.
#   FAKE_GH_CHANGES  : total changed lines (size endpoint)
#   FAKE_GH_FILES    : newline-separated changed paths (risk endpoint)
#   FAKE_GH_AUTHOR   : PR author login
#   FAKE_GH_MERGED   : merged-PR count
#   FAKE_GH_EDIT_LOG : append add/remove-label actions here
args="$*"

if [[ -n "${FAKE_GH_FAIL_UNTIL:-}" ]]; then
  n=$(cat "${FAKE_GH_COUNT_FILE}" 2>/dev/null || echo 0); n=$((n+1)); echo "${n}" > "${FAKE_GH_COUNT_FILE}"
  if (( n <= FAKE_GH_FAIL_UNTIL )); then
    echo "invalid character '<' looking for beginning of value" >&2
    exit 1
  fi
fi

case "${args}" in
  *"pr view"*"--json labels"*)
    cat "${FAKE_GH_LABELS_FILE:-/dev/null}" 2>/dev/null || true
    ;;
  *"pulls/"*"/files"*)
    if [[ "${args}" == *".changes"* ]]; then echo "${FAKE_GH_CHANGES:-0}";
    else printf '%s\n' "${FAKE_GH_FILES:-}"; fi
    ;;
  *"search/issues"*)
    echo "${FAKE_GH_MERGED:-0}"
    ;;
  *"pr edit"*"--add-label"*)
    echo "add: ${args##*--add-label }" >> "${FAKE_GH_EDIT_LOG:-/dev/null}"
    echo "https://github.com/o/r/pull/1"
    ;;
  *"pr edit"*"--remove-label"*)
    echo "remove: ${args##*--remove-label }" >> "${FAKE_GH_EDIT_LOG:-/dev/null}"
    ;;
  *"api "*"pulls/"*)
    echo "${FAKE_GH_AUTHOR:-octocat}"
    ;;
  *)
    echo "fake gh: unhandled args: ${args}" >&2; exit 2
    ;;
esac
EOF
chmod +x "${fakebin}/gh"

# 4. THE regression: a persistently failing API must NOT block the PR — the
#    script warns and still exits 0.
rc=0
out=$(PATH="${fakebin}:${PATH}" GH_RETRY_SLEEP=0 GH_RETRY_ATTEMPTS=2 \
      PR_NUMBER=1 REPO=o/r \
      FAKE_GH_FAIL_UNTIL=9999 FAKE_GH_COUNT_FILE="${tmp}/e2e_fail_count" \
      bash "${labeler}" 2>&1) || rc=$?
assert_rc       "non-fatal: exits 0 on total API outage" 0 "${rc}"
assert_contains "non-fatal: warns on size step" "${out}" "size step failed"
assert_contains "non-fatal: does not block" "${out}" "Not blocking the PR"
assert_contains "non-fatal: reaches Done" "${out}" "Done."

# 5. Happy path: classification still works and labels get applied.
editlog="${tmp}/editlog"; : > "${editlog}"
rc=0
out=$(PATH="${fakebin}:${PATH}" GH_RETRY_SLEEP=0 GH_RETRY_ATTEMPTS=2 \
      PR_NUMBER=1 REPO=o/r \
      FAKE_GH_CHANGES=1089 FAKE_GH_FILES="src/agent/mod.rs" \
      FAKE_GH_AUTHOR=ilblackdragon FAKE_GH_MERGED=369 \
      FAKE_GH_EDIT_LOG="${editlog}" \
      bash "${labeler}" 2>&1) || rc=$?
assert_rc       "happy path: exits 0" 0 "${rc}"
assert_contains "happy path: size XL"       "${out}" "size: XL"
assert_contains "happy path: risk medium"   "${out}" "Risk: medium"
assert_contains "happy path: contributor core" "${out}" "contributor: core"
assert_contains "happy path: size label applied"        "$(cat "${editlog}")" "add: size: XL"
assert_contains "happy path: contributor label applied" "$(cat "${editlog}")" "add: contributor: core"

# 6. Transient-then-recover end-to-end: fails the first few calls, then the
#    retries carry it through to a full, correct classification (exit 0).
editlog2="${tmp}/editlog2"; : > "${editlog2}"
rc=0
out=$(PATH="${fakebin}:${PATH}" GH_RETRY_SLEEP=0 GH_RETRY_ATTEMPTS=5 \
      PR_NUMBER=1 REPO=o/r \
      FAKE_GH_FAIL_UNTIL=2 FAKE_GH_COUNT_FILE="${tmp}/e2e_recover_count" \
      FAKE_GH_CHANGES=1089 FAKE_GH_FILES="src/agent/mod.rs" \
      FAKE_GH_AUTHOR=ilblackdragon FAKE_GH_MERGED=369 \
      FAKE_GH_EDIT_LOG="${editlog2}" \
      bash "${labeler}" 2>&1) || rc=$?
assert_rc           "transient recover: exits 0" 0 "${rc}"
assert_contains     "transient recover: size XL"  "${out}" "size: XL"
assert_contains     "transient recover: contributor core" "${out}" "contributor: core"
assert_not_contains "transient recover: no step-failed warning" "${out}" "step failed"

# ---------------------------------------------------------------------------
echo ""
echo "pr-labeler tests: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
