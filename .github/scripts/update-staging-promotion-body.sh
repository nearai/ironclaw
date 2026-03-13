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

gh pr view "${PR_NUMBER}" --repo "${REPO}" --json body,baseRefName,headRefName > "${TMP_DIR}/pr.json"
jq -r '.body // ""' < "${TMP_DIR}/pr.json" > "${TMP_DIR}/body.md"
BASE="$(jq -r '.baseRefName' < "${TMP_DIR}/pr.json")"
HEAD="$(jq -r '.headRefName' < "${TMP_DIR}/pr.json")"
RANGE="origin/${BASE}..origin/${HEAD}"

git fetch origin "${BASE}" "${HEAD}"

COMMIT_LIST="$(git log --oneline --no-merges --reverse "${RANGE}" 2>/dev/null || echo "")"
if [ -n "${COMMIT_LIST}" ]; then
  COMMIT_COUNT="$(printf '%s\n' "${COMMIT_LIST}" | wc -l | tr -d ' ')"
  if [ "${COMMIT_COUNT}" -gt "${MAX_COMMITS}" ]; then
    COMMIT_MD="$(printf '%s\n' "${COMMIT_LIST}" | head -n "${MAX_COMMITS}" | sed 's/^/- /')"
    COMMIT_MD+=$'\n'"- ... and $((COMMIT_COUNT - MAX_COMMITS)) more (see compare view)"
  else
    COMMIT_MD="$(printf '%s\n' "${COMMIT_LIST}" | sed 's/^/- /')"
  fi
else
  COMMIT_COUNT=0
  COMMIT_MD="- (no non-merge commits in range)"
fi

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

if grep -qF "${SECTION_START}" "${TMP_DIR}/body.md" && grep -qF "${SECTION_END}" "${TMP_DIR}/body.md"; then
  awk -v start="${SECTION_START}" -v end="${SECTION_END}" -v replacement_file="${TMP_DIR}/section.md" '
    BEGIN {
      while ((getline line < replacement_file) > 0) {
        replacement = replacement line ORS
      }
      in_block = 0
    }
    $0 == start {
      printf "%s", replacement
      in_block = 1
      next
    }
    $0 == end {
      in_block = 0
      next
    }
    !in_block {
      print
    }
  ' "${TMP_DIR}/body.md" > "${TMP_DIR}/new-body.md"
else
  cp "${TMP_DIR}/body.md" "${TMP_DIR}/new-body.md"
  if [ -s "${TMP_DIR}/new-body.md" ]; then
    printf '\n\n' >> "${TMP_DIR}/new-body.md"
  fi
  cat "${TMP_DIR}/section.md" >> "${TMP_DIR}/new-body.md"
fi

if [ "${DRY_RUN}" = "true" ]; then
  echo "Dry run enabled. Computed PR body for #${PR_NUMBER}:"
  cat "${TMP_DIR}/new-body.md"
else
  gh pr edit "${PR_NUMBER}" --repo "${REPO}" --body-file "${TMP_DIR}/new-body.md"
fi
