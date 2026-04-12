#!/usr/bin/env bash
set -euo pipefail

target_dir="${1:-artifacts/live-canary}"
strict="${STRICT_ARTIFACT_SCRUB:-0}"
matches_file="${target_dir%/}/scrub-matches.txt"

mkdir -p "$(dirname "${matches_file}")"

if [[ ! -d "${target_dir}" ]]; then
  echo "No artifact directory at ${target_dir}" > "${matches_file}"
  exit 0
fi

pattern='gh[pousr]_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,}|ya29\.[A-Za-z0-9._-]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|sk-ant-[A-Za-z0-9_-]{10,}'

if rg -a -n --hidden --glob '!*.png' --glob '!*.jpg' --glob '!*.jpeg' --glob '!*.webp' --glob '!*.zip' --glob '!*.wasm' -e "${pattern}" "${target_dir}" > "${matches_file}"; then
  echo "Potential secret-like strings found in ${target_dir}" >&2
  if [[ "${strict}" == "1" || "${strict}" == "true" ]]; then
    exit 1
  fi
else
  echo "No secret-like strings detected in ${target_dir}" > "${matches_file}"
fi
