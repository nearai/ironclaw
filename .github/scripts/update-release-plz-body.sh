#!/usr/bin/env bash
set -euo pipefail

: "${PR_NUMBER:?PR_NUMBER is required}"
: "${REPO:?REPO is required}"

MAIN_BRANCH="${MAIN_BRANCH:-main}"
DRY_RUN="${DRY_RUN:-false}"
SECTION_START="<!-- staging-promotion-release-summary:start -->"
SECTION_END="<!-- staging-promotion-release-summary:end -->"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

gh pr view "${PR_NUMBER}" --repo "${REPO}" --json body > "${TMP_DIR}/pr.json"
jq -r '.body // ""' < "${TMP_DIR}/pr.json" > "${TMP_DIR}/body.md"

git fetch origin "${MAIN_BRANCH}" --tags

LAST_TAG="$(git describe --tags --abbrev=0 "origin/${MAIN_BRANCH}" 2>/dev/null || true)"
if [ -n "${LAST_TAG}" ]; then
  RANGE="${LAST_TAG}..origin/${MAIN_BRANCH}"
  HEADER="## Staging promotion batches since ${LAST_TAG}"
  EMPTY_MESSAGE="_No structured staging promotion merges found since ${LAST_TAG}._"
else
  RANGE="origin/${MAIN_BRANCH}"
  HEADER="## Staging promotion batches on ${MAIN_BRANCH}"
  EMPTY_MESSAGE="_No structured staging promotion merges found on ${MAIN_BRANCH}._"
fi

{
  echo "${SECTION_START}"
  echo "${HEADER}"
  echo
} > "${TMP_DIR}/section.md"

FOUND_SUMMARY=false
while IFS= read -r sha; do
  [ -n "${sha}" ] || continue
  BODY="$(git show -s --format=%b "${sha}")"
  if ! printf '%s\n' "${BODY}" | grep -q '^staging-promotion-summary-v1$'; then
    continue
  fi

  FOUND_SUMMARY=true
  SUBJECT="$(git show -s --format=%s "${sha}")"
  PR_REF="$(printf '%s\n' "${BODY}" | sed -n 's/^promotion-pr: //p' | head -n 1)"
  COMMIT_COUNT="$(printf '%s\n' "${BODY}" | sed -n 's/^current-commit-count: //p' | head -n 1)"
  CURRENT_RANGE="$(printf '%s\n' "${BODY}" | sed -n 's/^current-range: //p' | head -n 1)"
  COMMIT_BLOCK="$(printf '%s\n' "${BODY}" | awk 'capture { print } /^Current commits in this promotion \([0-9]+\):$/ { capture = 1 }')"

  {
    echo "### ${SUBJECT}"
    echo
    if [ -n "${PR_REF}" ]; then
      echo "**Promotion PR:** ${PR_REF}"
    fi
    if [ -n "${COMMIT_COUNT}" ]; then
      echo "**Commit count:** ${COMMIT_COUNT}"
    fi
    if [ -n "${CURRENT_RANGE}" ]; then
      echo "**Range:** \`${CURRENT_RANGE}\`"
    fi
    echo
    if [ -n "${COMMIT_BLOCK}" ]; then
      echo "${COMMIT_BLOCK}"
    else
      echo "- (no commit summary found)"
    fi
    echo
  } >> "${TMP_DIR}/section.md"
done < <(git log --merges --reverse --format='%H' "${RANGE}")

if [ "${FOUND_SUMMARY}" = false ]; then
  {
    echo "${EMPTY_MESSAGE}"
    echo
  } >> "${TMP_DIR}/section.md"
fi

{
  echo "*Auto-updated from structured staging promotion merge bodies on ${MAIN_BRANCH}.*"
  echo "${SECTION_END}"
} >> "${TMP_DIR}/section.md"

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
