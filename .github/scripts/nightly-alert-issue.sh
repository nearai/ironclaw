#!/usr/bin/env bash
set -euo pipefail

# Create, update, or close a GitHub issue for a scheduled nightly workflow.
# Required env:
#   GH_TOKEN, REPO, ALERT_WORKFLOW_NAME, ALERT_ISSUE_TITLE, ALERT_RESULT
# Optional env:
#   ALERT_RUN_ID, ALERT_RUN_URL, ALERT_SHA, MAX_EXCERPT_LINES

require_env() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    echo "Required environment variable ${name} is not set" >&2
    exit 2
  fi
}

strip_ansi() {
  # Keep this sed expression POSIX-ish for the GitHub ubuntu runner.
  sed -E $'s/\x1B\[[0-9;?]*[ -/]*[@-~]//g'
}

truncate_lines() {
  local max_chars="$1"
  awk -v max_chars="$max_chars" '
    length($0) > max_chars { print substr($0, 1, max_chars) "... [line truncated]"; next }
    { print }
  '
}

truncate_file_chars() {
  local input_file="$1"
  local output_file="$2"
  local max_chars="$3"
  local label="${4:-content}"
  awk -v max_chars="$max_chars" -v label="$label" '
    BEGIN { used = 0 }
    {
      line = $0 "\n"
      line_len = length(line)
      if (used + line_len > max_chars) {
        remaining = max_chars - used
        if (remaining > 0) {
          printf "%s", substr(line, 1, remaining)
        }
        printf "\n... [%s truncated to %s characters]\n", label, max_chars
        exit
      }
      printf "%s", line
      used += line_len
    }
  ' "$input_file" > "$output_file"
}

extract_failure_excerpt() {
  local log_file="$1"
  local output_file="$2"
  local max_lines="${3:-${MAX_EXCERPT_LINES:-220}}"
  local max_line_chars="${MAX_EXCERPT_LINE_CHARS:-2000}"
  local max_excerpt_chars="${MAX_EXCERPT_CHARS:-50000}"
  local cleaned_log raw_excerpt line_limited_excerpt
  cleaned_log="$(mktemp)"
  raw_excerpt="$(mktemp)"
  line_limited_excerpt="$(mktemp)"

  if [[ ! -s "$log_file" ]]; then
    echo "No failed-job logs were available from GitHub Actions." > "$output_file"
    return 0
  fi

  strip_ansi < "$log_file" > "$cleaned_log"

  # Prefer high-signal lines from pytest, Rust/cargo, GitHub Actions, runner
  # infrastructure, and common timeout/disk failures. Keep the excerpt bounded so
  # repeated nightly failures do not create unreadably large issue comments.
  grep -nEi \
    '(^|[[:space:]])(FAILED|ERROR|FAILURES|failures:|test result: FAILED|panicked at|thread .+ panicked|No space left|timed out|timeout|Process completed with exit code|::error|Error:|Traceback|AssertionError|short test summary info|cargo (test|nextest|insta)|pytest|failed to|could not|cannot|killed|segmentation fault|signal:)' \
    "$cleaned_log" \
    | head -n "$max_lines" \
    > "$raw_excerpt" || true

  if [[ ! -s "$raw_excerpt" ]]; then
    {
      echo "No high-signal failure lines matched; showing the last 120 failed-log lines instead."
      echo
      tail -n 120 "$cleaned_log"
    } > "$raw_excerpt"
  fi

  truncate_lines "$max_line_chars" < "$raw_excerpt" > "$line_limited_excerpt"
  truncate_file_chars "$line_limited_excerpt" "$output_file" "$max_excerpt_chars" "excerpt"

  rm -f "$cleaned_log" "$raw_excerpt" "$line_limited_excerpt"
}

find_open_issue() {
  local title="$1"
  gh issue list \
    --repo "$REPO" \
    --state open \
    --search "${title} in:title" \
    --json number,title \
    --jq 'map(select(.title == env.ALERT_ISSUE_TITLE))[0].number // empty'
}

write_failure_body() {
  local body_file="$1"
  local jobs_file="$2"
  local excerpt_file="$3"
  local log_error_file="$4"

  {
    echo "❌ ${ALERT_WORKFLOW_NAME} scheduled run failed."
    echo
    echo "- Workflow: ${ALERT_WORKFLOW_NAME}"
    echo "- Result: ${ALERT_RESULT}"
    echo "- Run: ${ALERT_RUN_URL}"
    echo "- Commit: ${ALERT_SHA}"
    echo "- Attempt: ${GITHUB_RUN_ATTEMPT:-1}"
    echo "- Reported at: $(date -u +'%Y-%m-%d %H:%M:%S UTC')"
    echo
    echo "## Failed jobs"
    if [[ -s "$jobs_file" ]]; then
      cat "$jobs_file"
    else
      echo "No failed job metadata was available from the Actions API. Check the run link above."
    fi
    echo
    if [[ -s "$log_error_file" ]]; then
      echo "## Log retrieval notes"
      echo
      echo "The alert job could not retrieve full failed-job logs with \`gh run view --log-failed\`. The run link and failed job links above are still authoritative."
      echo
      echo '```text'
      head -n 40 "$log_error_file" | sed 's/```/` ` `/g'
      echo '```'
      echo
    fi
    echo "## Failure excerpt"
    echo
    echo '```text'
    sed 's/```/` ` `/g' "$excerpt_file"
    echo '```'
    echo
    echo "This issue is updated in place on repeated failures to avoid notification spam. If the next scheduled run passes, the nightly alert job will close this issue automatically."
  } > "$body_file"
}

write_recovery_body() {
  local body_file="$1"
  {
    echo "✅ ${ALERT_WORKFLOW_NAME} recovered on the latest scheduled run."
    echo
    echo "- Run: ${ALERT_RUN_URL}"
    echo "- Commit: ${ALERT_SHA}"
    echo "- Reported at: $(date -u +'%Y-%m-%d %H:%M:%S UTC')"
  } > "$body_file"
}

main() {
  require_env GH_TOKEN
  require_env REPO
  require_env ALERT_WORKFLOW_NAME
  require_env ALERT_ISSUE_TITLE
  require_env ALERT_RESULT

  export ALERT_ISSUE_TITLE

  ALERT_RUN_ID="${ALERT_RUN_ID:-${GITHUB_RUN_ID:-}}"
  ALERT_RUN_URL="${ALERT_RUN_URL:-${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-$REPO}/actions/runs/${ALERT_RUN_ID}}"
  ALERT_SHA="${ALERT_SHA:-${GITHUB_SHA:-unknown}}"

  if [[ -z "$ALERT_RUN_ID" ]]; then
    echo "ALERT_RUN_ID or GITHUB_RUN_ID is required" >&2
    exit 2
  fi

  local issue_number
  issue_number="$(find_open_issue "$ALERT_ISSUE_TITLE")"

  if [[ "$ALERT_RESULT" == "success" ]]; then
    if [[ -n "$issue_number" ]]; then
      local recovery_body
      recovery_body="$(mktemp)"
      write_recovery_body "$recovery_body"
      gh issue close "$issue_number" --repo "$REPO" --comment "$(cat "$recovery_body")"
      rm -f "$recovery_body"
    else
      echo "${ALERT_WORKFLOW_NAME} succeeded and no open alert issue exists."
    fi
    exit 0
  fi

  local tmp_dir log_file log_error failed_jobs excerpt body
  tmp_dir="$(mktemp -d)"
  log_file="${tmp_dir}/failed.log"
  log_error="${tmp_dir}/failed-log-error.txt"
  failed_jobs="${tmp_dir}/failed-jobs.md"
  excerpt="${tmp_dir}/excerpt.txt"
  body="${tmp_dir}/issue.md"

  if ! gh run view "$ALERT_RUN_ID" --repo "$REPO" --log-failed > "$log_file" 2> "$log_error"; then
    echo "Warning: failed to retrieve failed-job logs for run ${ALERT_RUN_ID}" >&2
  fi

  gh api "repos/${REPO}/actions/runs/${ALERT_RUN_ID}/jobs?per_page=100" \
    --jq '.jobs[] | select(.status == "completed" and .conclusion != "success" and .conclusion != "skipped") | "- \(.name) (`\(.conclusion // "unknown")`): \(.html_url)"' \
    > "$failed_jobs" || true

  extract_failure_excerpt "$log_file" "$excerpt" "${MAX_EXCERPT_LINES:-220}"
  write_failure_body "$body" "$failed_jobs" "$excerpt" "$log_error"
  truncate_file_chars "$body" "${body}.bounded" "${MAX_ISSUE_BODY_CHARS:-60000}" "issue body"
  mv "${body}.bounded" "$body"

  if [[ -n "$issue_number" ]]; then
    gh issue edit "$issue_number" --repo "$REPO" --body-file "$body"
    echo "Updated existing nightly alert issue #${issue_number}."
  else
    gh issue create --repo "$REPO" --title "$ALERT_ISSUE_TITLE" --body-file "$body"
  fi

  rm -rf "$tmp_dir"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
