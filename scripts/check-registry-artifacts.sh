#!/usr/bin/env bash
set -euo pipefail

if ! command -v curl >/dev/null 2>&1; then
  echo "ERROR: curl is required" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "ERROR: jq is required" >&2
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

checksums_url="${CHECKSUMS_URL:-https://github.com/nearai/ironclaw/releases/latest/download/checksums.txt}"
checksums_file="$(mktemp)"
trap 'rm -f "$checksums_file"' EXIT

echo "Fetching checksums from: $checksums_url"
curl -fsSL "$checksums_url" -o "$checksums_file"

declare -A release_sha_by_file
while IFS= read -r line; do
  [[ -z "$line" ]] && continue

  sha="${line%% *}"
  file="${line#* }"
  file="${file## }"

  if [[ "$file" == *-wasm32-wasip2.tar.gz ]]; then
    release_sha_by_file["$file"]="$sha"
  fi
done < "$checksums_file"

if [[ "${#release_sha_by_file[@]}" -eq 0 ]]; then
  echo "ERROR: No wasm32-wasip2 artifacts found in checksums file" >&2
  exit 1
fi

failures=0
checked=0

check_manifest() {
  local manifest="$1"

  local base_with_ext
  base_with_ext="$(basename -- "$manifest")"
  local base="${base_with_ext%.json}"

  local version
  version="$(jq -r '.version // empty' "$manifest")"
  if [[ -z "$version" || "$version" == "null" ]]; then
    return
  fi

  local artifact_file
  artifact_file="${base}-${version}-wasm32-wasip2.tar.gz"

  local expected_sha
  expected_sha="${release_sha_by_file[$artifact_file]-}"

  # Only enforce for release-published artifacts present in checksums.txt.
  if [[ -z "$expected_sha" ]]; then
    return
  fi

  checked=$((checked + 1))

  local actual_url
  actual_url="$(jq -r '.artifacts["wasm32-wasip2"].url // empty' "$manifest")"

  local actual_sha
  actual_sha="$(jq -r '.artifacts["wasm32-wasip2"].sha256 // empty' "$manifest")"

  local expected_url
  expected_url="https://github.com/nearai/ironclaw/releases/latest/download/${artifact_file}"

  local ok=true

  if [[ -z "$actual_sha" || "$actual_sha" == "null" ]]; then
    echo "FAIL $manifest: sha256 is missing or null"
    ok=false
  elif [[ "$actual_sha" != "$expected_sha" ]]; then
    echo "FAIL $manifest: sha256 mismatch (manifest=$actual_sha release=$expected_sha)"
    ok=false
  fi

  if [[ -z "$actual_url" || "$actual_url" == "null" ]]; then
    echo "FAIL $manifest: url is missing or null"
    ok=false
  else
    if [[ "$actual_url" != "$expected_url" ]]; then
      echo "FAIL $manifest: url mismatch"
      echo "  expected: $expected_url"
      echo "  actual:   $actual_url"
      ok=false
    fi

    http_code="$(curl -L --silent --show-error --output /dev/null --write-out '%{http_code}' "$actual_url" || true)"
    if [[ "$http_code" != "200" ]]; then
      echo "FAIL $manifest: url is not reachable (HTTP $http_code)"
      ok=false
    fi
  fi

  if [[ "$ok" == true ]]; then
    echo "OK   $manifest"
  else
    failures=$((failures + 1))
  fi
}

for manifest in registry/tools/*.json registry/channels/*.json; do
  if [[ -f "$manifest" ]]; then
    check_manifest "$manifest"
  fi
done

if [[ "$checked" -eq 0 ]]; then
  echo "ERROR: No release-published registry manifests were checked" >&2
  exit 1
fi

if [[ "$failures" -gt 0 ]]; then
  echo
  echo "Registry artifact validation failed: $failures manifest(s) with errors" >&2
  exit 1
fi

echo
echo "Registry artifact validation passed ($checked manifests checked)."
