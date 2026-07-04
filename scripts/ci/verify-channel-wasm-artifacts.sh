#!/usr/bin/env bash
set -euo pipefail
shopt -s nullglob

cd "$(dirname "$0")/../.."

channel="${1:-all}"
if [ "${channel}" = "all" ]; then
  manifests=(registry/channels/*.json)
else
  manifests=("registry/channels/${channel}.json")
fi

if [ "${#manifests[@]}" -eq 0 ]; then
  echo "No channel manifests found for '${channel}'" >&2
  exit 1
fi

for manifest in "${manifests[@]}"; do
  if [ ! -f "${manifest}" ]; then
    echo "Channel manifest not found: ${manifest}" >&2
    exit 1
  fi

  hidden="$(jq -r '.hidden // false' "${manifest}")"
  if [ "${hidden}" = "true" ]; then
    continue
  fi

  source_dir="$(jq -r '.source.dir' "${manifest}")"
  crate_name="$(jq -r '.source.crate_name' "${manifest}")"
  artifact_name="${crate_name//-/_}.wasm"
  artifact="${source_dir}/target/wasm32-wasip2/release/${artifact_name}"

  if [ ! -s "${artifact}" ]; then
    echo "Expected channel WASM artifact is missing or empty: ${artifact}" >&2
    exit 1
  fi

  ls -lh "${artifact}"
done
