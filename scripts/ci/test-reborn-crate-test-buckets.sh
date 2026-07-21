#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bucket_script="${script_dir}/reborn-crate-test-buckets.sh"

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

packages='[
  "ironclaw_telegram_extension",
  "ironclaw_common",
  "ironclaw_future_adapter"
]'

actual="$("${bucket_script}" "${packages}")"
expected='[
  {"name":"adapters-misc","packages":["ironclaw_telegram_extension","ironclaw_common","ironclaw_future_adapter"]}
]'

if ! jq -e --argjson expected "${expected}" '. == $expected' <<< "${actual}" >/dev/null; then
  fail "heavy packages were not isolated into the expected ordered buckets: ${actual}"
fi

if ! jq -e --argjson packages "${packages}" '
  [.[].packages[]] as $assigned
  | ($assigned | length) == ($packages | length)
    and ($assigned | unique | length) == ($assigned | length)
    and (($assigned | sort) == ($packages | sort))
' <<< "${actual}" >/dev/null; then
  fail "every input package must be assigned exactly once: ${actual}"
fi

if [ "$("${bucket_script}" '[]')" != '[]' ]; then
  fail "an empty package list must produce an empty matrix"
fi

echo "PASS Reborn crate bucket assignments"
