#!/usr/bin/env bash
set -euo pipefail

: "${PR_NUMBER:?PR_NUMBER is required}"
: "${REPO:?REPO is required}"

MAX_COMMITS="${MAX_COMMITS:-50}"
DRY_RUN="${DRY_RUN:-false}"
SECTION_START="<!-- staging-ci-current:start -->"
SECTION_END="<!-- staging-ci-current:end -->"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

# shellcheck source=.github/scripts/pr-body-utils.sh
source "$(dirname "$0")/pr-body-utils.sh"

gh pr view "${PR_NUMBER}" --repo "${REPO}" --json body,baseRefName,headRefName > "${TMP_DIR}/pr.json"
jq -r '.body // ""' < "${TMP_DIR}/pr.json" > "${TMP_DIR}/body.md"
BASE="$(jq -r '.baseRefName' < "${TMP_DIR}/pr.json")"
HEAD="$(jq -r '.headRefName' < "${TMP_DIR}/pr.json")"
RANGE="origin/${BASE}..origin/${HEAD}"

git fetch origin "${BASE}" "${HEAD}"

load_commit_summary "${RANGE}" "${MAX_COMMITS}"

{
  echo "${SECTION_START}"
  echo "### Current commits in this promotion (${COMMIT_COUNT})"
  echo
  echo "**Current base:** \`${BASE}\`"
  echo "**Current head:** \`${HEAD}\`"
  echo "**Current range:** \`${RANGE}\`"
  echo
  echo "${COMMIT_MD}"
  echo
  echo "*Auto-updated by staging promotion metadata workflow*"
  echo "${SECTION_END}"
} > "${TMP_DIR}/section.md"

replace_marked_section \
  "${TMP_DIR}/body.md" \
  "${TMP_DIR}/section.md" \
  "${SECTION_START}" \
  "${SECTION_END}" \
  "${TMP_DIR}/new-body.md"

if [ "${DRY_RUN}" = "true" ]; then
  echo "Dry run enabled. Computed PR body for #${PR_NUMBER}:"
  cat "${TMP_DIR}/new-body.md"
else
  gh pr edit "${PR_NUMBER}" --repo "${REPO}" --body-file "${TMP_DIR}/new-body.md"
fi
