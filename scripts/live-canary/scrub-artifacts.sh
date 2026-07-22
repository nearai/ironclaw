#!/usr/bin/env bash
set -euo pipefail

# Scan live-canary artifacts before upload. This is intentionally conservative:
# public live lanes may upload sanitized logs, while private OAuth lanes should
# upload only summaries and can set STRICT_ARTIFACT_SCRUB=true.

ARTIFACT_DIR="${1:-${RUN_DIR:-artifacts/live-canary}}"
STRICT_ARTIFACT_SCRUB="${STRICT_ARTIFACT_SCRUB:-false}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUNDLED_SKILLS_ROOT="${LIVE_CANARY_BUNDLED_SKILLS_ROOT:-${REPO_ROOT}/skills}"
FIRST_PARTY_EXTENSIONS_ROOT="${LIVE_CANARY_FIRST_PARTY_EXTENSIONS_ROOT:-${REPO_ROOT}/crates/ironclaw_first_party_extensions/assets}"
NEARAI_MANIFEST_TEMPLATE="${REPO_ROOT}/scripts/live-canary/fixtures/nearai-runtime-manifest.toml"
BUNDLED_SKILL_MARKER=".ironclaw-reborn-bundled.json"
BUNDLED_SKILL_OWNER="ironclaw_reborn_composition_bundled_skill"

if [[ ! -d "${ARTIFACT_DIR}" ]]; then
  echo "Artifact directory does not exist: ${ARTIFACT_DIR}" >&2
  exit 2
fi

is_verified_bundled_skill() {
  local marker="$1"
  local skill_dir="$2"
  local skill_name
  skill_name="$(basename "${skill_dir}")"

  python3 - "${marker}" "${BUNDLED_SKILLS_ROOT}/${skill_name}" "${skill_dir}" \
    "${skill_name}" "${BUNDLED_SKILL_OWNER}" "${BUNDLED_SKILL_MARKER}" <<'PY'
import json
import os
import re
import stat
import sys
from pathlib import Path


marker_path = Path(sys.argv[1])
trusted_dir = Path(sys.argv[2])
staged_dir = Path(sys.argv[3])
skill_name = sys.argv[4]
expected_owner = sys.argv[5]
marker_name = sys.argv[6]


def regular_files(root: Path, *, omit_marker: bool) -> list[tuple[str, Path]]:
    if not root.is_dir() or root.is_symlink():
        raise ValueError("bundle root is not a real directory")
    files: list[tuple[str, Path]] = []
    for current, directories, filenames in os.walk(root, followlinks=False):
        current_path = Path(current)
        for directory in directories:
            if (current_path / directory).is_symlink():
                raise ValueError("bundle contains a symlinked directory")
        for filename in filenames:
            path = current_path / filename
            relative = path.relative_to(root).as_posix()
            if omit_marker and relative == marker_name:
                continue
            if not stat.S_ISREG(path.lstat().st_mode):
                raise ValueError("bundle contains a non-regular file")
            files.append((relative, path))
    return sorted(files)


def update_fnv64(value: int, data: bytes) -> int:
    for byte in data:
        value ^= byte
        value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return value


def trusted_content_hash(files: list[tuple[str, Path]]) -> str:
    value = update_fnv64(0xCBF29CE484222325, skill_name.encode())
    for relative, path in files:
        value = update_fnv64(value, relative.encode())
        value = update_fnv64(value, b"\0")
        with path.open("rb") as source:
            while chunk := source.read(1024 * 1024):
                value = update_fnv64(value, chunk)
        value = update_fnv64(value, b"\0")
    return f"{value:016x}"


def files_equal(left: Path, right: Path) -> bool:
    if left.stat().st_size != right.stat().st_size:
        return False
    with left.open("rb") as left_file, right.open("rb") as right_file:
        while True:
            left_chunk = left_file.read(1024 * 1024)
            right_chunk = right_file.read(1024 * 1024)
            if left_chunk != right_chunk:
                return False
            if not left_chunk:
                return True


try:
    if marker_path.stat().st_size > 4096:
        raise ValueError("bundle marker is too large")
    marker = json.loads(marker_path.read_text(encoding="utf-8"))
    if not isinstance(marker, dict):
        raise ValueError("bundle marker is not an object")
    content_hash = marker.get("content_hash")
    if (
        marker.get("owner") != expected_owner
        or type(marker.get("format")) is not int
        or marker.get("format") != 1
        or not isinstance(content_hash, str)
        or re.fullmatch(r"[0-9a-f]{16}", content_hash) is None
    ):
        raise ValueError("bundle marker is invalid")

    trusted_files = regular_files(trusted_dir, omit_marker=False)
    staged_files = regular_files(staged_dir, omit_marker=True)
    if any(relative == marker_name for relative, _ in trusted_files):
        raise ValueError("trusted source contains a runtime marker")
    if content_hash != trusted_content_hash(trusted_files):
        raise ValueError("bundle marker hash does not match trusted source")
    if [relative for relative, _ in trusted_files] != [relative for relative, _ in staged_files]:
        raise ValueError("staged bundle file set differs from trusted source")
    if not all(
        files_equal(trusted_path, staged_path)
        for (_, trusted_path), (_, staged_path) in zip(trusted_files, staged_files)
    ):
        raise ValueError("staged bundle content differs from trusted source")
except (OSError, UnicodeError, ValueError):
    sys.exit(1)
PY
}

is_verified_first_party_extension_manifest() {
  local manifest="$1"
  local extension_dir
  local extension_id
  local trusted_manifest
  extension_dir="$(dirname "${manifest}")"
  extension_id="$(basename "${extension_dir}")"
  trusted_manifest="${FIRST_PARTY_EXTENSIONS_ROOT}/${extension_id}/manifest.toml"

  if [[ -f "${trusted_manifest}" ]] && cmp -s -- "${trusted_manifest}" "${manifest}"; then
    return 0
  fi
  if [[ "${extension_id}" != "nearai" || ! -f "${NEARAI_MANIFEST_TEMPLATE}" ]]; then
    return 1
  fi
  sed -E \
    's#^server = "https://(cloud-api|private)\.near\.ai/mcp"$#server = "__LIVE_CANARY_NEARAI_MCP_SERVER__"#' \
    "${manifest}" | cmp -s -- "${NEARAI_MANIFEST_TEMPLATE}" -
}

# Reborn live QA copies each case's full home into the artifact staging tree.
# In strict mode, remove only managed system-skill installations whose marker,
# stable content hash, file set, and bytes all match the source-controlled
# bundle. Unverified and operator-owned skills remain in scope for scanning.
if [[ "${STRICT_ARTIFACT_SCRUB}" == "true" || "${STRICT_ARTIFACT_SCRUB}" == "1" ]]; then
  while IFS= read -r -d '' marker; do
    skill_dir="$(dirname "${marker}")"
    case "${skill_dir}" in
      "${ARTIFACT_DIR}"/*/reborn-home/*/local-dev/system/skills/*|\
      "${ARTIFACT_DIR}"/reborn-home/*/local-dev/system/skills/*)
        if is_verified_bundled_skill "${marker}" "${skill_dir}"; then
          rm -rf -- "${skill_dir}"
        fi
        ;;
    esac
  done < <(
    find "${ARTIFACT_DIR}" -type f \
      -path '*/reborn-home/*/local-dev/system/skills/*/.ironclaw-reborn-bundled.json' \
      -print0
  )

  # Installed first-party extension manifests are also source-controlled
  # package metadata. Their credential declarations contain secret-shaped
  # field names and OAuth response paths, but no credential values. Remove a
  # manifest only when every byte matches its trusted source. NEAR AI's
  # bootstrap rewrites and deterministically re-serializes the configured MCP
  # endpoint, so compare it to the pinned runtime template after normalizing
  # only the two repository-owned endpoints.
  while IFS= read -r -d '' manifest; do
    if is_verified_first_party_extension_manifest "${manifest}"; then
      rm -f -- "${manifest}"
    fi
  done < <(
    find "${ARTIFACT_DIR}" -type f \
      -path '*/reborn-home/*/local-dev/system/extensions/*/manifest.toml' \
      -print0
  )
fi

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
