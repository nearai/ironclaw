#!/usr/bin/env bash
set -euo pipefail

# Scan live-canary artifacts before upload. This is intentionally conservative:
# public live lanes may upload sanitized logs, while private OAuth lanes should
# upload only summaries and can set STRICT_ARTIFACT_SCRUB=true.

ARTIFACT_DIR="${1:-${RUN_DIR:-artifacts/live-canary}}"
STRICT_ARTIFACT_SCRUB="${STRICT_ARTIFACT_SCRUB:-false}"

if [[ ! -d "${ARTIFACT_DIR}" ]]; then
  echo "Artifact directory does not exist: ${ARTIFACT_DIR}" >&2
  exit 2
fi

patterns=(
  'bearer[[:space:]]+[A-Za-z0-9._~+/=-]+'
  'api[_-]?key[[:space:]]*[:=][[:space:]]*[^[:space:]]+'
  'access[_-]?token[[:space:]]*[:=][[:space:]]*[^[:space:]]+'
  'refresh[_-]?token[[:space:]]*[:=][[:space:]]*[^[:space:]]+'
  'secret[[:space:]]*[:=][[:space:]]*[^[:space:]]+'
  'gh[pousr]_[A-Za-z0-9_]{20,}'
  'github_pat_[A-Za-z0-9_]{20,}'
  'ya29\.[A-Za-z0-9._-]{20,}'
  'xox[baprs]-[A-Za-z0-9-]{10,}'
  'sk-ant-[A-Za-z0-9_-]{10,}'
)

matches_file="${ARTIFACT_DIR}/scrub-matches.txt"
: > "${matches_file}"

while IFS= read -r -d '' file; do
  if [[ "${file}" == "${matches_file}" ]]; then
    continue
  fi
  case "${file}" in
    *.png|*.jpg|*.jpeg|*.gif|*.webp|*.sqlite|*.db|*.wasm|*.zip) continue ;;
  esac
  for pattern in "${patterns[@]}"; do
    if grep -nIEi "${pattern}" "${file}" >> "${matches_file}" 2>/dev/null; then
      true
    fi
  done
done < <(find "${ARTIFACT_DIR}" -type f -print0)

if [[ -s "${matches_file}" ]]; then
  echo "Potential secret material found in live canary artifacts:"
  sed -E \
    -e 's/(bearer[[:space:]]+)[^[:space:]]+/\1<REDACTED>/Ig' \
    -e 's/(token[[:space:]]*[:=][[:space:]]*)[^[:space:]]+/\1<REDACTED>/Ig' \
    -e 's/(key[[:space:]]*[:=][[:space:]]*)[^[:space:]]+/\1<REDACTED>/Ig' \
    -e 's/(secret[[:space:]]*[:=][[:space:]]*)[^[:space:]]+/\1<REDACTED>/Ig' \
    "${matches_file}" | head -200
  if [[ "${STRICT_ARTIFACT_SCRUB}" == "true" || "${STRICT_ARTIFACT_SCRUB}" == "1" ]]; then
    exit 1
  fi
  echo "Continuing because STRICT_ARTIFACT_SCRUB is not true."
else
  echo "No obvious secret material found in ${ARTIFACT_DIR}."
fi
