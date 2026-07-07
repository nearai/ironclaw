#!/usr/bin/env bash
#
# Upsert a single sticky PR comment carrying the Reborn integration-tier
# coverage summary, so the %/hole-list lives in the PR conversation instead of
# being buried in the Actions job summary.
#
# This SCRIPT is VISIBILITY ONLY — it never gates. The workflow step that runs
# it is `continue-on-error: true`, so any failure here (notably: fork PRs get a
# read-only GITHUB_TOKEN and the comment API 403s) must never red the check.
# Enforcement lives entirely in reborn-coverage-ratchet.sh's own exit code,
# invoked as its own separate (non-continue-on-error) workflow step — this
# script only renders that same script's report-mode output into the comment.
#
# The comment body reuses reborn-coverage-summary.sh (the %, table and hole list
# are computed there, once — it is the single owner of the crate aggregation)
# and prepends, after the hidden marker line (which must stay the literal
# first line — the upsert lookup below matches on it):
#   1. the coverage ratchet section (reborn-coverage-ratchet.sh's own report
#      output, verbatim) at the very top of the visible body — highest signal
#      first, and simpler than splicing into reborn-coverage-summary.sh's
#      output (which today is one opaque rendered string, no exposed seam to
#      insert into "before the per-crate table" specifically).
#   2. a breadth callout counting Reborn crates with instrumented-but-uncovered
#      lines, also sourced from reborn-coverage-summary.sh (--zero-crates). The
#      callout is informational too — the "target: 0" is the roadmap goal, not
#      a check that can fail.
#
# Usage: reborn-coverage-comment.sh <lcov-path> <exemptions-toml-path> <floor-toml-path>
#
# Requires env: GH_TOKEN (for gh), GITHUB_REPOSITORY, PR_NUMBER.

set -euo pipefail

lcov_path="${1:?usage: reborn-coverage-comment.sh <lcov-path> <exemptions-toml-path> <floor-toml-path>}"
exemptions_path="${2:?usage: reborn-coverage-comment.sh <lcov-path> <exemptions-toml-path> <floor-toml-path>}"
floor_path="${3:?usage: reborn-coverage-comment.sh <lcov-path> <exemptions-toml-path> <floor-toml-path>}"

if [ ! -f "${lcov_path}" ]; then
  echo "coverage lcov file not found: ${lcov_path}" >&2
  exit 1
fi

if [ ! -f "${exemptions_path}" ]; then
  echo "coverage exemptions manifest not found: ${exemptions_path}" >&2
  exit 1
fi

if [ ! -f "${floor_path}" ]; then
  echo "coverage floor manifest not found: ${floor_path}" >&2
  exit 1
fi

: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY must be set}"
: "${PR_NUMBER:?PR_NUMBER must be set}"
: "${GH_TOKEN:?GH_TOKEN must be set — gh api needs it (the workflow passes github.token)}"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
summary_sh="${script_dir}/reborn-coverage-summary.sh"
ratchet_sh="${script_dir}/reborn-coverage-ratchet.sh"

marker='<!-- reborn-coverage-sticky -->'

# Reuse the canonical summary renderer for the body and the breadth holes — no
# duplicated %/table/aggregation logic lives here.
summary_body="$("${summary_sh}" "${lcov_path}" "${exemptions_path}")"
mapfile -t zero_crates < <("${summary_sh}" --zero-crates "${lcov_path}" "${exemptions_path}")

# Reuse the ratchet script's own report output verbatim — this comment is a
# visibility mirror of it, never a second computation. `|| true`: the ratchet
# script's exit code (1 on an enforced violation or a schema error) must never
# propagate here and abort a `continue-on-error: true` step's job before it
# even reaches the `gh api` call — the job summary step already surfaces the
# real exit code for CI purposes.
ratchet_output="$("${ratchet_sh}" "${lcov_path}" "${exemptions_path}" "${floor_path}" 2>&1 || true)"
# shellcheck disable=SC2016 # single-quoted: the backtick fence is literal markdown, not command substitution.
ratchet_section="$(printf '### Coverage ratchet\n\n```\n%s\n```' "${ratchet_output}")"

callout=""
if [ "${#zero_crates[@]}" -gt 0 ]; then
  # Crate names are constrained to [a-z0-9_]+, so a plain ", " join is safe.
  crate_list="$(printf '%s, ' "${zero_crates[@]}")"
  crate_list="${crate_list%, }"
  callout="⚠️ ${#zero_crates[@]} Reborn crate(s) have 0 int-tier coverage (target: 0) — ${crate_list}"
fi

if [ -n "${callout}" ]; then
  body="$(printf '%s\n\n%s\n\n%s\n\n%s' "${marker}" "${ratchet_section}" "${callout}" "${summary_body}")"
else
  body="$(printf '%s\n\n%s\n\n%s' "${marker}" "${ratchet_section}" "${summary_body}")"
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
