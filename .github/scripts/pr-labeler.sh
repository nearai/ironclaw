#!/usr/bin/env bash
# Classify a PR by size, risk, and contributor tier.
# Called by the pr-label-classify workflow.
#
# Inputs (env vars):
#   PR_NUMBER  — pull request number
#   REPO       — owner/repo (e.g. "user/ironclaw")
#
# Requires: gh CLI, jq

set -euo pipefail

PR_NUMBER="${PR_NUMBER:?PR_NUMBER is required}"
REPO="${REPO:?REPO is required}"

# ─── helpers ────────────────────────────────────────────────────────────────

# Remove all labels in a dimension except the desired one.
# Usage: set_exclusive_label "size" "size: M"
set_exclusive_label() {
  local prefix="$1" desired="$2"

  # Fetch current labels on the PR
  local current
  current=$(gh pr view "$PR_NUMBER" --repo "$REPO" --json labels --jq '.labels[].name')

  # Remove any existing label with the same prefix
  while IFS= read -r label; do
    [[ -z "$label" ]] && continue
    if [[ "$label" == "${prefix}:"* && "$label" != "$desired" ]]; then
      gh pr edit "$PR_NUMBER" --repo "$REPO" --remove-label "$label" 2>/dev/null || true
    fi
  done <<< "$current"

  # Add the desired label
  gh pr edit "$PR_NUMBER" --repo "$REPO" --add-label "$desired"
}

# ─── size ───────────────────────────────────────────────────────────────────

classify_size() {
  # Sum changed lines across non-doc files
  local total
  total=$(gh api "repos/${REPO}/pulls/${PR_NUMBER}/files" \
    --paginate --jq '
      [.[] | select(.filename | test("\\.(md|txt|rst|adoc)$") | not) | .changes]
      | add // 0
    ')

  local label
  if   (( total < 10 ));  then label="size: XS"
  elif (( total < 50 ));  then label="size: S"
  elif (( total < 200 )); then label="size: M"
  elif (( total < 500 )); then label="size: L"
  else                         label="size: XL"
  fi

  echo "Size: ${total} changed lines -> ${label}"
  set_exclusive_label "size" "$label"
}

# ─── risk ───────────────────────────────────────────────────────────────────

classify_risk() {
  # If "risk: manual" is present, skip — it's a sticky override
  local current
  current=$(gh pr view "$PR_NUMBER" --repo "$REPO" --json labels --jq '.labels[].name')
  if echo "$current" | grep -qx "risk: manual"; then
    echo "Risk: skipped (manual override)"
    return
  fi

  # Fetch changed file paths
  local files
  files=$(gh api "repos/${REPO}/pulls/${PR_NUMBER}/files" \
    --paginate --jq '.[].filename')

  local risk="low"

  while IFS= read -r file; do
    [[ -z "$file" ]] && continue

    case "$file" in
      # High risk: safety, secrets, auth, crypto, setup, orchestrator auth
      src/safety/*|src/secrets/*|src/llm/session.rs|src/orchestrator/auth.rs|\
      src/channels/web/auth.rs|src/setup/*)
        risk="high"
        break  # can't go higher
        ;;

      # Medium risk: agent core, config, database, worker, tools, channels
      src/agent/*|src/config.rs|src/settings.rs|src/db/*|src/worker/*|\
      src/tools/*|src/channels/*|src/orchestrator/*|src/context/*|\
      src/hooks/*|src/sandbox/*|src/extensions/*|Cargo.toml|\
      .github/workflows/*)
        # Only upgrade, never downgrade
        [[ "$risk" != "high" ]] && risk="medium"
        ;;

      # Low risk: docs, tests, estimation, evaluation, history, etc.
      *)
        ;;
    esac
  done <<< "$files"

  echo "Risk: ${risk}"
  set_exclusive_label "risk" "risk: ${risk}"
}

# ─── contributor tier ───────────────────────────────────────────────────────

classify_contributor() {
  # Get PR author
  local author
  author=$(gh pr view "$PR_NUMBER" --repo "$REPO" --json author --jq '.author.login')

  # Count merged PRs by this author in this repo
  local count
  count=$(gh api "/search/issues" \
    -f q="repo:${REPO} is:pr is:merged author:${author}" \
    --jq '.total_count')

  local label
  if   (( count == 0 )); then label="contributor: new"
  elif (( count < 6 ));  then label="contributor: regular"
  elif (( count < 20 )); then label="contributor: experienced"
  else                        label="contributor: core"
  fi

  echo "Contributor: ${author} has ${count} merged PRs -> ${label}"
  set_exclusive_label "contributor" "$label"
}

# ─── main ───────────────────────────────────────────────────────────────────

echo "Classifying PR #${PR_NUMBER} in ${REPO}..."
classify_size
classify_risk
classify_contributor
echo "Done."
