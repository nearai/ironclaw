#!/usr/bin/env bash
#
# Upsert a single sticky PR comment carrying the Reborn integration-tier
# coverage summary, so the %/hole-list lives in the PR conversation instead of
# being buried in the Actions job summary.
#
# This is VISIBILITY ONLY — it never gates. The workflow step that runs it is
# `continue-on-error: true`, so any failure here (notably: fork PRs get a
# read-only GITHUB_TOKEN and the comment API 403s) must never red the check.
#
# The comment body reuses reborn-coverage-summary.sh (the %, table and hole list
# are computed there, once — it is the single owner of the crate aggregation)
# and prepends:
#   1. a hidden marker line so re-runs edit the same comment in place, and
#   2. a breadth callout counting Reborn crates with instrumented-but-uncovered
#      lines, also sourced from that script (--zero-crates). The callout is
#      informational too — the "target: 0" is the roadmap goal, not a check that
#      can fail.
#
# Usage: reborn-coverage-comment.sh <llvm-cov-json-export>
#
# Requires env: GH_TOKEN (for gh), GITHUB_REPOSITORY, PR_NUMBER.

set -euo pipefail

json_path="${1:?usage: reborn-coverage-comment.sh <llvm-cov-json-export>}"

if [ ! -f "${json_path}" ]; then
  echo "coverage JSON not found: ${json_path}" >&2
  exit 1
fi

: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY must be set}"
: "${PR_NUMBER:?PR_NUMBER must be set}"
: "${GH_TOKEN:?GH_TOKEN must be set — gh api needs it (the workflow passes github.token)}"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
summary_sh="${script_dir}/reborn-coverage-summary.sh"

marker='<!-- reborn-coverage-sticky -->'

# Reuse the canonical summary renderer for the body and the breadth holes — no
# duplicated %/table/aggregation jq lives here.
summary_body="$("${summary_sh}" "${json_path}")"
mapfile -t zero_crates < <("${summary_sh}" --zero-crates "${json_path}")

callout=""
if [ "${#zero_crates[@]}" -gt 0 ]; then
  # Crate names are constrained to [a-z0-9_]+, so a plain ", " join is safe.
  crate_list="$(printf '%s, ' "${zero_crates[@]}")"
  crate_list="${crate_list%, }"
  callout="⚠️ ${#zero_crates[@]} Reborn crate(s) have 0 int-tier coverage (target: 0) — ${crate_list}"
fi

if [ -n "${callout}" ]; then
  body="$(printf '%s\n\n%s\n\n%s' "${marker}" "${callout}" "${summary_body}")"
else
  body="$(printf '%s\n\n%s' "${marker}" "${summary_body}")"
fi

# Upsert: find an existing sticky comment (marker is the first body line) and
# PATCH it; otherwise POST a new one. Pure `gh api`, no extra action dependency.
#
# --paginate: the comment thread can exceed one page (30) on a busy PR, and the
# sticky may have aged off page 1 — without it the lookup misses and we POST a
# duplicate every run. The marker is passed via the environment (env.STICKY_*)
# rather than interpolated into the jq program. Capture the ids first, then take
# the first: piping `gh --paginate` straight into `head` would SIGPIPE gh mid-
# stream and, under `pipefail`, fail the pipeline.
existing_ids="$(STICKY_MARKER="${marker}" gh api --paginate \
  "repos/${GITHUB_REPOSITORY}/issues/${PR_NUMBER}/comments" \
  --jq '.[] | select(.body | startswith(env.STICKY_MARKER)) | .id')"
existing="$(printf '%s\n' "${existing_ids}" | head -n1)"

if [ -n "${existing}" ]; then
  gh api -X PATCH "repos/${GITHUB_REPOSITORY}/issues/comments/${existing}" -f body="${body}"
else
  gh api -X POST "repos/${GITHUB_REPOSITORY}/issues/${PR_NUMBER}/comments" -f body="${body}"
fi
