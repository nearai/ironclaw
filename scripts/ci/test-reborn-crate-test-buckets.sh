#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bucket_script="${script_dir}/reborn-crate-test-buckets.sh"

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

packages='[
  "ironclaw_architecture",
  "ironclaw_common",
  "ironclaw_extension_host",
  "ironclaw_extension_manager",
  "ironclaw_extensions",
  "ironclaw_future_adapter",
  "ironclaw_host_ingress",
  "ironclaw_libsql_runtime",
  "ironclaw_operator",
  "ironclaw_reborn_traces",
  "ironclaw_slack_extension",
  "ironclaw_telegram_extension",
  "ironclaw_telegram_v2_adapter",
  "ironclaw_triggers"
]'

actual="$("${bucket_script}" "${packages}")"
expected='[
  {"name":"channel-adapters","packages":["ironclaw_host_ingress","ironclaw_slack_extension","ironclaw_telegram_extension","ironclaw_telegram_v2_adapter"]},
  {"name":"extension-operator","packages":["ironclaw_extension_host","ironclaw_extension_manager","ironclaw_extensions","ironclaw_operator"]},
  {"name":"architecture-misc","packages":["ironclaw_architecture","ironclaw_common","ironclaw_future_adapter","ironclaw_libsql_runtime","ironclaw_reborn_traces","ironclaw_triggers"]}
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
