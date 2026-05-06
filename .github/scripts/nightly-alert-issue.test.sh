#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=nightly-alert-issue.sh
source "${SCRIPT_DIR}/nightly-alert-issue.sh"

assert_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$file"; then
    echo "Expected ${file} to contain: ${needle}" >&2
    echo "--- ${file}" >&2
    cat "$file" >&2
    exit 1
  fi
}

workdir="$(mktemp -d)"
trap 'rm -rf "${workdir}"' EXIT

cat > "${workdir}/failed.log" <<'LOG'
Build step noise
E2E (features)	Run E2E tests (features)	FAILED tests/e2e/scenarios/test_tool_approval.py::test_approval_flow - AssertionError: expected approval dialog
Rust tests	Run Tests	thread 'runtime::manager::tests::stop_thread_works' panicked at crates/ironclaw_engine/src/runtime/manager.rs:123:5
Coverage	Generate coverage	Error: No space left on device
Runner	Complete job	Process completed with exit code 101.
LOG

extract_failure_excerpt "${workdir}/failed.log" "${workdir}/excerpt.txt" 20
assert_contains "${workdir}/excerpt.txt" "FAILED tests/e2e/scenarios/test_tool_approval.py::test_approval_flow"
assert_contains "${workdir}/excerpt.txt" "panicked at crates/ironclaw_engine/src/runtime/manager.rs"
assert_contains "${workdir}/excerpt.txt" "No space left on device"
assert_contains "${workdir}/excerpt.txt" "Process completed with exit code 101"

cat > "${workdir}/quiet.log" <<'LOG'
ordinary line one
ordinary line two
LOG

extract_failure_excerpt "${workdir}/quiet.log" "${workdir}/fallback.txt" 20
assert_contains "${workdir}/fallback.txt" "No high-signal failure lines matched"
assert_contains "${workdir}/fallback.txt" "ordinary line two"

python3 - <<'PY' "${workdir}/long.log"
from pathlib import Path
import sys
Path(sys.argv[1]).write_text('ERROR: ' + ('x' * 70000) + '\n')
PY
MAX_EXCERPT_LINE_CHARS=120 MAX_EXCERPT_CHARS=300 extract_failure_excerpt "${workdir}/long.log" "${workdir}/long-excerpt.txt" 20
assert_contains "${workdir}/long-excerpt.txt" "[line truncated]"
long_excerpt_bytes="$(wc -c < "${workdir}/long-excerpt.txt" | tr -d ' ')"
if [[ "${long_excerpt_bytes}" -gt 360 ]]; then
  echo "Expected long excerpt to stay bounded, got ${long_excerpt_bytes} bytes" >&2
  cat "${workdir}/long-excerpt.txt" >&2
  exit 1
fi

cat > "${workdir}/many.log" <<'LOG'
ERROR: first failure line with enough content to keep
ERROR: second failure line with enough content to keep
ERROR: third failure line with enough content to keep
ERROR: fourth failure line with enough content to keep
ERROR: fifth failure line with enough content to keep
LOG
MAX_EXCERPT_LINE_CHARS=1000 MAX_EXCERPT_CHARS=120 extract_failure_excerpt "${workdir}/many.log" "${workdir}/many-excerpt.txt" 20
assert_contains "${workdir}/many-excerpt.txt" "[excerpt truncated to 120 characters]"
many_excerpt_bytes="$(wc -c < "${workdir}/many-excerpt.txt" | tr -d ' ')"
if [[ "${many_excerpt_bytes}" -gt 180 ]]; then
  echo "Expected many-line excerpt to stay bounded, got ${many_excerpt_bytes} bytes" >&2
  cat "${workdir}/many-excerpt.txt" >&2
  exit 1
fi

cat > "${workdir}/jobs.md" <<'JOBS'
- E2E (features) (`failure`): https://github.example/jobs/1
JOBS
cat > "${workdir}/log-error.txt" <<'ERR'
HTTP 404: logs are not ready yet
ERR

ALERT_WORKFLOW_NAME="Nightly E2E" \
ALERT_RESULT="failure" \
ALERT_RUN_URL="https://github.example/runs/1" \
ALERT_SHA="abc123" \
write_failure_body "${workdir}/body.md" "${workdir}/jobs.md" "${workdir}/excerpt.txt" "${workdir}/log-error.txt"
assert_contains "${workdir}/body.md" "Log retrieval notes"
assert_contains "${workdir}/body.md" "HTTP 404: logs are not ready yet"
assert_contains "${workdir}/body.md" "updated in place on repeated failures"
truncate_file_chars "${workdir}/body.md" "${workdir}/body-bounded.md" 240 "issue body"
assert_contains "${workdir}/body-bounded.md" "[issue body truncated to 240 characters]"

mkdir -p "${workdir}/bin"
cat > "${workdir}/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "${FAKE_GH_CALLS}"
case "${1:-} ${2:-}" in
  "issue list")
    if [[ "${FAKE_EXISTING_ISSUE:-}" == "1" ]]; then
      echo "42"
    fi
    ;;
  "run view")
    echo "FAILED tests/e2e/scenarios/test_chat.py::test_chat - AssertionError"
    ;;
  "api repos/example/repo/actions/runs/99/jobs?per_page=100")
    echo "- E2E (core) (\`failure\`): https://github.example/jobs/1"
    ;;
  "issue edit"|"issue create"|"issue close")
    ;;
  *)
    echo "unexpected gh call: $*" >&2
    exit 1
    ;;
esac
GH
chmod +x "${workdir}/bin/gh"

run_alert_script() {
  local result="$1"
  PATH="${workdir}/bin:${PATH}" \
  FAKE_GH_CALLS="${workdir}/gh-calls.txt" \
  FAKE_EXISTING_ISSUE="${FAKE_EXISTING_ISSUE:-}" \
  GH_TOKEN="token" \
  REPO="example/repo" \
  GITHUB_RUN_ID="99" \
  GITHUB_RUN_ATTEMPT="1" \
  GITHUB_SERVER_URL="https://github.example" \
  GITHUB_REPOSITORY="example/repo" \
  GITHUB_SHA="abc123" \
  ALERT_WORKFLOW_NAME="Nightly E2E" \
  ALERT_ISSUE_TITLE="Nightly E2E failed" \
  ALERT_RESULT="$result" \
  "${SCRIPT_DIR}/nightly-alert-issue.sh"
}

: > "${workdir}/gh-calls.txt"
FAKE_EXISTING_ISSUE="1" run_alert_script "failure"
assert_contains "${workdir}/gh-calls.txt" "issue edit 42"
if grep -Fq "issue comment" "${workdir}/gh-calls.txt"; then
  echo "Repeated failures must update the issue body, not add comment spam" >&2
  cat "${workdir}/gh-calls.txt" >&2
  exit 1
fi

: > "${workdir}/gh-calls.txt"
FAKE_EXISTING_ISSUE="1" run_alert_script "success"
assert_contains "${workdir}/gh-calls.txt" "issue close 42"
if grep -Fq "issue comment" "${workdir}/gh-calls.txt"; then
  echo "Recovery should close with one close comment, not a separate comment" >&2
  cat "${workdir}/gh-calls.txt" >&2
  exit 1
fi

echo "nightly-alert-issue tests passed"
