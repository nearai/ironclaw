#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 3 ]]; then
  echo "usage: preview_state.sh <pr> [repo] [preview_url]" >&2
  exit 2
fi

pr="${1%/}"
repo="${2:-nearai/ironclaw}"
preview_url="${3:-}"

# Accept a PR number or a PR URL. The preview hostname fallback needs the
# numeric PR number, so extract it before building any hostname.
pr_num=""
if [[ "$pr" =~ ^[0-9]+$ ]]; then
  pr_num="$pr"
elif command -v gh >/dev/null 2>&1; then
  pr_num="$(gh pr view "$pr" --repo "$repo" --json number --jq .number 2>/dev/null || true)"
fi

head_sha="$(gh pr view "$pr" --repo "$repo" --json headRefOid --jq .headRefOid)"

# Match the Railway check by name or description through the structured
# --json interface instead of parsing the human-readable check table.
railway_check="$(
  gh pr checks "$pr" --repo "$repo" --json name,state,description 2>/dev/null \
    --jq '.[] |
      select(
        (.name | ascii_downcase | contains("railway")) or
        (.description // "" | ascii_downcase | contains("railway"))
      ) |
      "\(.state)\t\(.description)"' \
    | head -1 \
    || true
)"

railway_state="missing"
railway_description=""
if [[ -n "$railway_check" ]]; then
  IFS=$'\t' read -r railway_state railway_description <<<"$railway_check"
fi

if [[ -z "$preview_url" && "$repo" == "nearai/ironclaw" && "$pr_num" =~ ^[0-9]+$ ]]; then
  preview_url="https://ironclaw-ironclaw-pr-${pr_num}.up.railway.app"
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
