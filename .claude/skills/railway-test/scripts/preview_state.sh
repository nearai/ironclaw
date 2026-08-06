#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 3 ]]; then
  echo "usage: preview_state.sh <pr> [repo] [preview_url]" >&2
  exit 2
fi

pr="$1"
repo="${2:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"
preview_url="${3:-}"

head_sha="$(gh pr view "$pr" --repo "$repo" --json headRefOid --jq .headRefOid)"
check_line="$(
  gh pr checks "$pr" --repo "$repo" 2>/dev/null \
    | awk -F '\t' '
        tolower($1) ~ /railway/ ||
        $1 ~ /^ironclaw-ci-preview/ ||
        tolower($5) ~ /railway/ {
          print
          exit
        }
      ' \
    || true
)"

railway_state="missing"
railway_description=""
if [[ -n "$check_line" ]]; then
  IFS=$'\t' read -r _ railway_state _ _ railway_description <<<"$check_line"
fi

if [[ -z "$preview_url" && "$repo" == "nearai/ironclaw" ]]; then
  preview_url="https://ironclaw-ironclaw-pr-${pr}.up.railway.app"
fi
if [[ -z "$preview_url" && "$railway_description" =~ ([A-Za-z0-9.-]+\.up\.railway\.app) ]]; then
  preview_url="https://${BASH_REMATCH[1]}"
fi

asset=""
if [[ -n "$preview_url" ]]; then
  asset="$(
    curl -fsSL --max-time 15 "$preview_url/" 2>/dev/null \
      | rg -o 'assets/app-[A-Za-z0-9_-]+\.js' \
      | head -1 \
      || true
  )"
fi

printf 'pr=%s\n' "$pr"
printf 'repo=%s\n' "$repo"
printf 'head_sha=%s\n' "$head_sha"
printf 'preview_url=%s\n' "${preview_url:-unresolved}"
printf 'railway_state=%s\n' "$railway_state"
printf 'railway_description=%s\n' "$railway_description"
printf 'asset=%s\n' "${asset:-unavailable}"
