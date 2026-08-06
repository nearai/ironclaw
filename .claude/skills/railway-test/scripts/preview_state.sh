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

# Query the commit-scoped statuses (and check-runs as a fallback) so the
# Railway result is bound to the tested head SHA. Query failures are captured
# separately: an empty successful result means "no Railway status for this
# head", a failed query is reported as query_failed — never as `missing` — so
# the skill does not waste polling budget without diagnostics.
railway_query="ok"
railway_line=""
if status_line="$(
  gh api "repos/$repo/commits/$head_sha/status" 2>/dev/null \
    --jq '.statuses[] |
      select(
        (.context | ascii_downcase | contains("railway")) or
        (.context | ascii_downcase | contains("ci-preview")) or
        ((.description // "") | ascii_downcase | contains("railway"))
      ) |
      "\(.state)\t\(.description // "")\t\(.target_url // "")"' \
    | head -1
)"; then
  railway_line="$status_line"
else
  railway_query="failed"
fi
if [[ -z "$railway_line" && "$railway_query" == "ok" ]]; then
  # Some providers post check-runs instead of statuses; still commit-scoped.
  if check_line="$(
    gh api --paginate "repos/$repo/commits/$head_sha/check-runs" 2>/dev/null \
      --jq '.check_runs[] |
        select(
          (.name | ascii_downcase | contains("railway")) or
          (.name | ascii_downcase | startswith("ironclaw-ci-preview")) or
          ((.app.name // "") | ascii_downcase | contains("railway"))
        ) |
        "\(.status):\(.conclusion // "unknown")\t\(.name)\t\(.details_url // "")"' \
      | head -1
  )"; then
    railway_line="$check_line"
  else
    railway_query="failed"
  fi
fi

railway_state="missing"
railway_details=""
if [[ "$railway_query" == "ok" && -n "$railway_line" ]]; then
  IFS=$'\t' read -r railway_state railway_details _ <<<"$railway_line"
elif [[ "$railway_query" == "failed" ]]; then
  railway_state="query_failed"
fi

if [[ -z "$preview_url" && "$repo" == "nearai/ironclaw" && "$pr_num" =~ ^[0-9]+$ ]]; then
  preview_url="https://ironclaw-ironclaw-pr-${pr_num}.up.railway.app"
fi
if [[ -z "$preview_url" && "$railway_details" =~ ([A-Za-z0-9.-]+\.up\.railway\.app) ]]; then
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
printf 'railway_details=%s\n' "$railway_details"
printf 'asset=%s\n' "${asset:-unavailable}"
