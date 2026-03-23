#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/monitor-prs.sh [--repo owner/name] [--author login]

Shows open PRs for the author with:
- review decision
- latest review summary
- failing or pending checks

Defaults:
- repo: current gitHub repo from `gh repo view`
- author: currently authenticated GitHub user from `gh api user`
EOF
}

repo=""
author=""

while [ $# -gt 0 ]; do
    case "$1" in
        --repo)
            repo="${2:-}"
            shift 2
            ;;
        --author)
            author="${2:-}"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

if ! command -v gh >/dev/null 2>&1; then
    echo "gh CLI is required" >&2
    exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
    echo "jq is required" >&2
    exit 1
fi

if [ -z "$repo" ]; then
    repo="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
fi

if [ -z "$author" ]; then
    author="$(gh api user -q .login)"
fi

json_fields="number,title,url,headRefName,reviewDecision,latestReviews,statusCheckRollup"
prs="$(gh pr list --repo "$repo" --author "$author" --state open --limit 100 --json "$json_fields")"

count="$(printf '%s' "$prs" | jq 'length')"
echo "Open PRs for $author in $repo: $count"
echo

if [ "$count" -eq 0 ]; then
    exit 0
fi

printf '%s' "$prs" | jq -r '
  def check_name:
    .name // .context // .workflowName // "unknown-check";

  def failing_checks:
    [.statusCheckRollup[]?
      | select(.status == "COMPLETED" and (.conclusion // .state // "") != "SUCCESS")
      | {
          name: check_name,
          workflow: (.workflowName // ""),
          url: (.detailsUrl // "")
        }];

  def pending_checks:
    [.statusCheckRollup[]?
      | select(.status != "COMPLETED")
      | {
          name: check_name,
          workflow: (.workflowName // ""),
          url: (.detailsUrl // "")
        }];

  .[]
  | . as $pr
  | failing_checks as $failing
  | pending_checks as $pending
  | [
      ("#" + (.number | tostring) + " " + .title),
      ("  Branch: " + .headRefName),
      ("  URL: " + .url),
      ("  Review: " + (.reviewDecision // "UNKNOWN")),
      (
        if (.latestReviews | length) > 0 then
          "  Latest review: "
          + .latestReviews[0].state
          + " by "
          + .latestReviews[0].author.login
          + " at "
          + .latestReviews[0].submittedAt
        else
          "  Latest review: none"
        end
      ),
      ("  Checks: " + ($failing | length | tostring) + " failing, "
        + ($pending | length | tostring) + " pending"),
      (
        if ($failing | length) > 0 then
          ($failing[] | "    FAIL: " + .name
            + (if .workflow != "" then " [" + .workflow + "]" else "" end)
            + (if .url != "" then " -> " + .url else "" end))
        else
          "    FAIL: none"
        end
      ),
      (
        if ($pending | length) > 0 then
          ($pending[] | "    PENDING: " + .name
            + (if .workflow != "" then " [" + .workflow + "]" else "" end)
            + (if .url != "" then " -> " + .url else "" end))
        else
          "    PENDING: none"
        end
      )
    ]
  | .[]
  , ""
'
