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
tmp_matches="$(mktemp "${RUNNER_TEMP:-/tmp}/live-canary-scrub-matches.XXXXXX")"
tmp_files="$(mktemp "${RUNNER_TEMP:-/tmp}/live-canary-scrub-files.XXXXXX")"
trap 'rm -f "${tmp_matches}" "${tmp_files}"' EXIT

redact_matches() {
  sed -E \
    -e 's/(bearer[[:space:]]+)[^[:space:]]+/\1<REDACTED>/Ig' \
    -e 's/gh[pousr]_[A-Za-z0-9_]{20,}/<REDACTED_GITHUB_TOKEN>/g' \
    -e 's/github_pat_[A-Za-z0-9_]{20,}/<REDACTED_GITHUB_PAT>/g' \
    -e 's/ya29\.[A-Za-z0-9._-]{20,}/<REDACTED_GOOGLE_TOKEN>/g' \
    -e 's/xox[baprs]-[A-Za-z0-9-]{10,}/<REDACTED_SLACK_TOKEN>/g' \
    -e 's/sk-ant-[A-Za-z0-9_-]{10,}/<REDACTED_ANTHROPIC_KEY>/g' \
    -e 's/(api[_-]?key[[:space:]]*[:=][[:space:]]*)[^[:space:]]+/\1<REDACTED>/Ig' \
    -e 's/(access[_-]?token[[:space:]]*[:=][[:space:]]*)[^[:space:]]+/\1<REDACTED>/Ig' \
    -e 's/(refresh[_-]?token[[:space:]]*[:=][[:space:]]*)[^[:space:]]+/\1<REDACTED>/Ig' \
    -e 's/(secret[[:space:]]*[:=][[:space:]]*)[^[:space:]]+/\1<REDACTED>/Ig'
}

: > "${tmp_matches}"
: > "${tmp_files}"

while IFS= read -r -d '' file; do
  if [[ "${file}" == "${matches_file}" ]]; then
    continue
  fi
  case "${file}" in
    *.png|*.jpg|*.jpeg|*.gif|*.webp|*.sqlite|*.db|*.wasm|*.zip) continue ;;
  esac
  for pattern in "${patterns[@]}"; do
    if grep -qIEi "${pattern}" "${file}" 2>/dev/null; then
      printf '%s\n' "${file}" >> "${tmp_files}"
      grep -nHIEi "${pattern}" "${file}" 2>/dev/null | redact_matches >> "${tmp_matches}" || true
    fi
  done
done < <(find "${ARTIFACT_DIR}" -type f -print0)

if [[ -s "${tmp_matches}" ]]; then
  sort -u "${tmp_matches}" > "${matches_file}"
  echo "Potential secret material found in live canary artifacts:"
  head -200 "${matches_file}"
  if [[ "${STRICT_ARTIFACT_SCRUB}" == "true" || "${STRICT_ARTIFACT_SCRUB}" == "1" ]]; then
    sort -u "${tmp_files}" | while IFS= read -r matched_file; do
      if [[ -n "${matched_file}" && "${matched_file}" != "${matches_file}" ]]; then
        rm -f -- "${matched_file}"
      fi
    done
    exit 1
  fi
  echo "Continuing because STRICT_ARTIFACT_SCRUB is not true."
else
  : > "${matches_file}"
  echo "No obvious secret material found in ${ARTIFACT_DIR}."
fi
