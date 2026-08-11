#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
workflow="${repo_root}/.github/workflows/main-ci-slack-alerts.yml"
code_style_workflow="${repo_root}/.github/workflows/code_style.yml"

assert_contains() {
  local expected="$1"
  if ! grep -Fq -- "$expected" "$workflow"; then
    echo "Expected main-ci-slack-alerts.yml to contain: ${expected}" >&2
    exit 1
  fi
}

assert_contains "- gh-readonly-queue/main/**"
assert_contains "contains(fromJSON('[\"push\",\"merge_group\"]'), github.event.workflow_run.event)"
assert_contains "checks: read"
assert_contains "pull-requests: read"
assert_contains 'LIVE_CANARY_SLACK_WEBHOOK_URL: ${{ secrets.SLACK_WEBHOOK_URL }}'
assert_contains 'if [[ "$HEAD_BRANCH" =~ ^gh-readonly-queue/main/pr-([0-9]+)- ]]; then'
assert_contains '"repos/${GITHUB_REPOSITORY}/pulls/${pr_number}"'
assert_contains '*Failed jobs / steps:*'
assert_contains '*Failure annotations (when available):*'

if ! grep -Fq -- '.github/workflows/(code_style|main-ci-slack-alerts)\.yml$' "$code_style_workflow"; then
  echo "Expected code_style.yml to run CI alert workflow contract tests when the alert changes" >&2
  exit 1
fi

echo "main-ci-slack-alerts workflow contract passed"
