#!/usr/bin/env bash
# Classify a PR by size, risk, and contributor tier.
# Called by the pr-label-classify workflow.
#
# Inputs (env vars):
#   PR_NUMBER  — pull request number
#   REPO       — owner/repo (e.g. "user/ironclaw")
#
# Tunables (env vars, mainly for tests):
#   GH_RETRY_ATTEMPTS — max tries per gh call before giving up (default 4)
#   GH_RETRY_SLEEP    — base backoff seconds between tries (default 3)
#
# Requires: gh CLI, jq
#
# Resilience: labeling is an advisory convenience, not a merge gate. A
# transient GitHub API hiccup returns an HTML error page, which makes
# `gh --jq` abort with `invalid character '<' looking for beginning of value`;
# under `set -e` that used to kill the whole classify job and block the PR.
# Every gh call now goes through `gh_retry` (retry with backoff), and a
# classifier that still can't fetch after retries only warns — the script
# exits 0 regardless so labeling failures never block a merge.

set -euo pipefail

# ─── retry wrapper ──────────────────────────────────────────────────────────

GH_RETRY_ATTEMPTS="${GH_RETRY_ATTEMPTS:-4}"
GH_RETRY_SLEEP="${GH_RETRY_SLEEP:-3}"

# Run a command, retrying with linear backoff on any non-zero exit. On success
# its stdout is forwarded verbatim (stderr passes through untouched so the
# underlying gh error — e.g. the `invalid character '<'` HTML-parse failure —
# stays visible in the logs). Returns the last exit code if all tries fail.
gh_retry() {
  local attempt=1 rc=0 out
  while :; do
    # Capture rc in the `else` — a bare `if cmd; then …; fi` resets $? to 0
    # after `fi`, which would make a give-up look like success.
    if out=$("$@"); then
      printf '%s' "$out"
      return 0
    else
      rc=$?
    fi
    if (( attempt >= GH_RETRY_ATTEMPTS )); then
      echo "gh_retry: '$*' failed after ${attempt} attempt(s) (rc=${rc})" >&2
      return "$rc"
    fi
    echo "gh_retry: attempt ${attempt}/${GH_RETRY_ATTEMPTS} for '$*' failed (rc=${rc}); retrying in $(( GH_RETRY_SLEEP * attempt ))s" >&2
    sleep "$(( GH_RETRY_SLEEP * attempt ))"
    attempt=$(( attempt + 1 ))
  done
}

# ─── helpers ────────────────────────────────────────────────────────────────

# Remove all labels in a dimension except the desired one.
# Usage: set_exclusive_label "size" "size: M"
set_exclusive_label() {
  local prefix="$1" desired="$2"

  # Fetch current labels on the PR. Check explicitly — a classifier is called
  # on the left of `||` in main(), which suppresses `set -e` for its whole
  # body, so a failing command substitution would otherwise be swallowed.
  local current
  if ! current=$(gh_retry gh pr view "$PR_NUMBER" --repo "$REPO" --json labels --jq '.labels[].name'); then
    return 1
  fi

  # Remove any existing label with the same prefix
  while IFS= read -r label; do
    [[ -z "$label" ]] && continue
    if [[ "$label" == "${prefix}:"* && "$label" != "$desired" ]]; then
      gh pr edit "$PR_NUMBER" --repo "$REPO" --remove-label "$label" 2>/dev/null || true
    fi
  done <<< "$current"

  # Add the desired label (best-effort: retry transient failures, but a
  # persistent failure must not abort the classifier).
  gh_retry gh pr edit "$PR_NUMBER" --repo "$REPO" --add-label "$desired" >/dev/null || true
}

# ─── size ───────────────────────────────────────────────────────────────────

classify_size() {
  # Sum changed lines across non-doc files
  local total
  if ! total=$(gh_retry gh api "repos/${REPO}/pulls/${PR_NUMBER}/files" \
    --paginate --jq '
      [.[]
        | select(.filename | test("\\.(md|txt|rst|adoc)$") | not)
        | select(.filename | test("^tests/|_test\\.rs$|_tests\\.rs$|/tests/|\\.test\\.[jt]sx?$|\\.spec\\.[jt]sx?$") | not)
        | .changes]
      | add // 0
    '); then
    return 1
  fi

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
  if ! current=$(gh_retry gh pr view "$PR_NUMBER" --repo "$REPO" --json labels --jq '.labels[].name'); then
    return 1
  fi
  if echo "$current" | grep -qx "risk: manual"; then
    echo "Risk: skipped (manual override)"
    return 0
  fi

  # Fetch changed file paths
  local files
  if ! files=$(gh_retry gh api "repos/${REPO}/pulls/${PR_NUMBER}/files" \
    --paginate --jq '.[].filename'); then
    return 1
  fi

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
  if ! author=$(gh_retry gh api "repos/${REPO}/pulls/${PR_NUMBER}" --jq '.user.login'); then
    return 1
  fi
  local count
  if [ -z "$author" ] || [ "$author" = "null" ]; then
    echo "Contributor: unable to resolve PR author; defaulting to new"
    count=0
  else
    # Count merged PRs by this author in this repo
    if ! count=$(gh_retry gh api --method GET "search/issues" \
      -f q="repo:${REPO} type:pr is:merged author:${author}" \
      --jq '.total_count') || ! [[ "$count" =~ ^[0-9]+$ ]]; then
      echo "Contributor: unable to query merged PR count for ${author}; defaulting to new"
      count=0
    fi
  fi

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

main() {
  echo "Classifying PR #${PR_NUMBER} in ${REPO}..."

  # Each classifier is best-effort. A transient API failure (even after
  # retries) only warns — labeling must never block a merge. The classifiers
  # check their own fetches explicitly (see set_exclusive_label) rather than
  # leaning on `set -e`, which is suppressed on the left of `||`.
  local failed=0
  classify_size        || { echo "::warning::PR classification: size step failed (transient API error?)"; failed=1; }
  classify_risk        || { echo "::warning::PR classification: risk step failed (transient API error?)"; failed=1; }
  classify_contributor || { echo "::warning::PR classification: contributor step failed (transient API error?)"; failed=1; }

  if (( failed )); then
    echo "One or more classifiers failed; labels may be incomplete. Not blocking the PR."
  fi
  echo "Done."
  return 0
}

# Only run when executed directly — sourcing (e.g. from tests) just loads the
# functions without requiring PR_NUMBER/REPO or hitting the API.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  PR_NUMBER="${PR_NUMBER:?PR_NUMBER is required}"
  REPO="${REPO:?REPO is required}"
  main
fi
