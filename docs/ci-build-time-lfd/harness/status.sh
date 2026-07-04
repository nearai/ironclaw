#!/usr/bin/env bash
set -euo pipefail

repo="${REPO:-nearai/ironclaw}"
branch="$(git rev-parse --abbrev-ref HEAD)"
workflows="${WORKFLOWS:-reborn-tests.yml,reborn-e2e.yml,reborn-coverage.yml,test.yml,live-canary.yml}"

echo "# CI build-time LFD status"
echo
echo "branch: $branch"
echo "repo: $repo"
echo

if command -v gh >/dev/null 2>&1; then
  if pr_json="$(gh pr view --repo "$repo" --json number,url,headRefName,state 2>/dev/null)"; then
    echo "PR: $(jq -r '.url + \" (\" + .state + \")\"' <<< "$pr_json")"
  else
    echo "PR: none for current branch"
  fi
else
  echo "PR: gh not installed"
fi

echo
echo "Recent workflow runs:"
while IFS= read -r workflow; do
  [ -n "$workflow" ] || continue
  echo
  echo "## $workflow"
  gh run list --repo "$repo" --workflow "$workflow" --limit 3 \
    --json databaseId,branch,status,conclusion,createdAt,updatedAt,url \
    --jq '.[] | [.databaseId, .branch, .status, (.conclusion // ""), .createdAt, .updatedAt, .url] | @tsv'
done < <(printf '%s' "$workflows" | tr ',' '\n')
