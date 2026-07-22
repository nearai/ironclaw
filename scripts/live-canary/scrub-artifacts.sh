#!/usr/bin/env bash
set -euo pipefail

# Scan live-canary artifacts before upload. This is intentionally conservative:
# public live lanes may upload sanitized logs, while private OAuth lanes should
# upload only summaries and can set STRICT_ARTIFACT_SCRUB=true.

ARTIFACT_DIR="${1:-${RUN_DIR:-artifacts/live-canary}}"
STRICT_ARTIFACT_SCRUB="${STRICT_ARTIFACT_SCRUB:-false}"
BUNDLED_SKILL_MARKER=".ironclaw-reborn-bundled.json"
BUNDLED_SKILL_OWNER="ironclaw_reborn_composition_bundled_skill"

if [[ ! -d "${ARTIFACT_DIR}" ]]; then
  echo "Artifact directory does not exist: ${ARTIFACT_DIR}" >&2
  exit 2
fi

# Reborn live QA copies each case's full home into the artifact staging tree.
# System skills carrying this marker are byte-for-byte runtime installations of
# source-controlled bundles, so retaining them adds no run-specific evidence.
# Some bundles intentionally contain dummy credential examples, which strict
# scanning must not mistake for leaked live material. Remove only directories
# with Ironclaw's managed-bundle marker; unmanaged/operator system skills remain
# in the artifact tree and are scanned normally.
while IFS= read -r -d '' marker; do
  if ! grep -qE "\"owner\"[[:space:]]*:[[:space:]]*\"${BUNDLED_SKILL_OWNER}\"" "${marker}"; then
    continue
  fi
  skill_dir="$(dirname "${marker}")"
  case "${skill_dir}" in
    "${ARTIFACT_DIR}"/*/reborn-home/*/local-dev/system/skills/*|\
    "${ARTIFACT_DIR}"/reborn-home/*/local-dev/system/skills/*)
      rm -rf -- "${skill_dir}"
      ;;
  esac
done < <(
  find "${ARTIFACT_DIR}" -type f \
    -path '*/reborn-home/*/local-dev/system/skills/*/.ironclaw-reborn-bundled.json' \
    -print0
)

patterns=(
  'bearer[[:space:]]+[A-Za-z0-9._~+/=-]+'
  'api[_-]?key[[:space:]]*[:=][[:space:]]*[^[:space:]]+'
  'access[_-]?token[[:space:]]*[:=][[:space:]]*[^[:space:]]+'
  'refresh[_-]?token[[:space:]]*[:=][[:space:]]*[^[:space:]]+'
  'secret[[:space:]]*[:=][[:space:]]*[^[:space:]]+'
  'password[[:space:]]*[:=][[:space:]]*[^[:space:]]+'
  # JSON-quoted token shapes — the seeded/browser auth lanes emit results.json
  # files containing full OAuth responses, which use `"access_token": "…"` /
  # `"refresh_token": "…"` form. The `token:` / `token=` patterns above do
  # not match those, so redaction would silently miss them.
  '"(access|refresh|id|bearer)_token"[[:space:]]*:[[:space:]]*"[^"]+"'
  '"(api[_-]?key|client[_-]?secret|password)"[[:space:]]*:[[:space:]]*"[^"]+"'
  'gh[pousr]_[A-Za-z0-9_]{20,}'
  'github_pat_[A-Za-z0-9_]{20,}'
  'ya29\.[A-Za-z0-9._-]{20,}'
  'xox[baprs]-[A-Za-z0-9-]{10,}'
  'sk-[A-Za-z0-9_-]{20,}'
  'sk-ant-[A-Za-z0-9_-]{10,}'
  'A[KS]IA[0-9A-Z]{16}'
)

matches_file="${ARTIFACT_DIR}/scrub-matches.txt"
tmp_matches="$(mktemp "${RUNNER_TEMP:-/tmp}/live-canary-scrub-matches.XXXXXX")"
tmp_files="$(mktemp "${RUNNER_TEMP:-/tmp}/live-canary-scrub-files.XXXXXX")"
trap 'rm -f "${tmp_matches}" "${tmp_files}"' EXIT

redact_matches() {
  sed -E \
    -e 's/(bearer[[:space:]]+)[^[:space:]\",}]+/\1<REDACTED>/Ig' \
    -e 's/gh[pousr]_[A-Za-z0-9_]{20,}/<REDACTED_GITHUB_TOKEN>/g' \
    -e 's/github_pat_[A-Za-z0-9_]{20,}/<REDACTED_GITHUB_PAT>/g' \
    -e 's/ya29\.[A-Za-z0-9._-]{20,}/<REDACTED_GOOGLE_TOKEN>/g' \
    -e 's/xox[baprs]-[A-Za-z0-9-]{10,}/<REDACTED_SLACK_TOKEN>/g' \
    -e 's/sk-ant-[A-Za-z0-9_-]{10,}/<REDACTED_ANTHROPIC_KEY>/g' \
    -e 's/sk-[A-Za-z0-9_-]{20,}/<REDACTED_OPENAI_KEY>/g' \
    -e 's/A[KS]IA[0-9A-Z]{16}/<REDACTED_AWS_ACCESS_KEY>/g' \
    -e 's/(api[_-]?key[[:space:]]*[:=][[:space:]]*)[^[:space:]\",}]+/\1<REDACTED>/Ig' \
    -e 's/(access[_-]?token[[:space:]]*[:=][[:space:]]*)[^[:space:]\",}]+/\1<REDACTED>/Ig' \
    -e 's/(refresh[_-]?token[[:space:]]*[:=][[:space:]]*)[^[:space:]\",}]+/\1<REDACTED>/Ig' \
    -e 's/(secret[[:space:]]*[:=][[:space:]]*)[^[:space:]\",}]+/\1<REDACTED>/Ig' \
    -e 's/(password[[:space:]]*[:=][[:space:]]*)[^[:space:]\",}]+/\1<REDACTED>/Ig' \
    -e 's/("(access|refresh|id|bearer)_token"[[:space:]]*:[[:space:]]*)"[^"]+"/\1"<REDACTED>"/Ig' \
    -e 's/("(api[_-]?key|client[_-]?secret|password)"[[:space:]]*:[[:space:]]*)"[^"]+"/\1"<REDACTED>"/Ig'
}

is_llm_trace_artifact() {
  local file="$1"
  case "${file}" in
    llm-traces/*.json|*/llm-traces/*.json)
      return 0
      ;;
  esac
  return 1
}

is_redactable_artifact() {
  local file="$1"
  local base
  base="$(basename "${file}")"
  # Per-case LLM trace recordings (reborn-webui-v2-live-qa `llm-traces/*.json`)
  # are captured LLM I/O harvested for fixture curation. Matched by their
  # `llm-traces/` directory (the basename is an arbitrary case name) so that
  # under strict scrub any token-shaped material is redacted in place rather
  # than the whole trace being deleted. Token-shape regexes only — no content
  # normalization or identifier scrubbing beyond the shared patterns above.
  if is_llm_trace_artifact "${file}"; then
    return 0
  fi
  case "${base}" in
    *.log|*.jsonl|summary.md|env-summary.txt|trace-fixture-status.txt|auth-canary-junit.xml|results.json|case-manifest.json|preflight.json|preflight.*.json|browser-summary.json|browser-events.jsonl)
      return 0
      ;;
  esac
  return 1
}

validate_redacted_artifact() {
  local original_file="$1"
  local redacted_file="$2"
  if is_llm_trace_artifact "${original_file}"; then
    python3 -m json.tool "${redacted_file}" >/dev/null
  fi
}

contains_unredacted_secret_patterns() {
  local file="$1"
  local pattern
  local matches
  for pattern in "${patterns[@]}"; do
    matches="$(grep -nHIEi "${pattern}" "${file}" 2>/dev/null || true)"
    if [[ -n "${matches}" ]] && grep -qv '<REDACTED' <<< "${matches}"; then
      return 0
    fi
  done
  return 1
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
    unsafe_found=0
    redacted_found=0
    while IFS= read -r matched_file; do
      if [[ -n "${matched_file}" && "${matched_file}" != "${matches_file}" ]]; then
        if is_redactable_artifact "${matched_file}"; then
          redacted_tmp="$(mktemp "${RUNNER_TEMP:-/tmp}/live-canary-redacted.XXXXXX")" || redacted_tmp=""
          if [[ -n "${redacted_tmp}" ]] \
            && redact_matches < "${matched_file}" > "${redacted_tmp}" \
            && ! contains_unredacted_secret_patterns "${redacted_tmp}" \
            && validate_redacted_artifact "${matched_file}" "${redacted_tmp}"; then
            mv "${redacted_tmp}" "${matched_file}"
            redacted_found=1
          else
            if [[ -n "${redacted_tmp}" ]]; then
              rm -f -- "${redacted_tmp}"
            fi
            rm -f -- "${matched_file}"
            unsafe_found=1
          fi
        else
          rm -f -- "${matched_file}"
          unsafe_found=1
        fi
      fi
    done < <(sort -u "${tmp_files}")
    if [[ "${unsafe_found}" == "1" ]]; then
      exit 1
    fi
    if [[ "${redacted_found}" == "1" ]]; then
      echo "Strict scrub redacted diagnostic artifacts in place."
    fi
    exit 0
  fi
  echo "Continuing because STRICT_ARTIFACT_SCRUB is not true."
else
  : > "${matches_file}"
  echo "No obvious secret material found in ${ARTIFACT_DIR}."
fi
