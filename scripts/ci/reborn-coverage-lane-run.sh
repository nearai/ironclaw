#!/usr/bin/env bash
# Run inventory-selected IronClaw integration lanes. Pull requests and merge
# groups use one bounded nextest command; main keeps one instrumented llvm-cov
# command per lane so its LCOV artifact topology remains unchanged.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
output_lcov="${1:-}"
collect_coverage="${REBORN_COV_COLLECT:-true}"
lanes_json="${REBORN_COV_LANES_JSON:?REBORN_COV_LANES_JSON must select at least one lane}"
test_timeout="${REBORN_COV_LANE_TEST_TIMEOUT:-45m}"

if [[ "${collect_coverage}" != "true" && "${collect_coverage}" != "false" ]]; then
  echo "REBORN_COV_COLLECT must be 'true' or 'false'; got '${collect_coverage}'" >&2
  exit 1
fi
if [[ "${collect_coverage}" == "true" && -z "${output_lcov}" ]]; then
  echo "coverage requires an output LCOV path" >&2
  exit 1
fi

inventory_json="$(
  python3 "${script_dir}/lib/integration_test_inventory.py" --json "${PWD}"
)"
partition_count="$(jq -r '.partition_count' <<<"${inventory_json}")"

if ! jq -e --argjson count "${partition_count}" '
  type == "array" and length > 0 and
  all(.[]; . == "groups" or (type == "number" and floor == . and . >= 0 and . < $count)) and
  length == (unique | length)
' <<<"${lanes_json}" >/dev/null; then
  echo "REBORN_COV_LANES_JSON must contain unique lane indices in [0, ${partition_count}) and/or \"groups\"; got ${lanes_json}" >&2
  exit 1
fi
if [[ "${collect_coverage}" == "true" ]] &&
   ! jq -e 'length == 1' <<<"${lanes_json}" >/dev/null; then
  echo "coverage batches must contain exactly one lane; got ${lanes_json}" >&2
  exit 1
fi

run_groups="$(jq -r 'index("groups") != null' <<<"${lanes_json}")"
if [[ "${run_groups}" == "true" ]]; then
  python3 "${script_dir}/lib/integration_test_inventory.py" \
    --validate-group-topology "${PWD}"
  export RUST_MIN_STACK=67108864
fi

mapfile -t selected_names < <(
  jq -r --argjson lanes "${lanes_json}" '
    .tests[] | select(.lane as $lane | $lanes | index($lane)) | .name
  ' <<<"${inventory_json}"
)
if [[ "${#selected_names[@]}" -eq 0 ]]; then
  echo "No IronClaw integration-tier suites assigned to batch ${lanes_json}; passing by design"
  if [[ "${collect_coverage}" == "true" ]]; then
    : >"${output_lcov}"
  fi
  exit 0
fi

test_args=()
for test_name in "${selected_names[@]}"; do
  test_args+=(--test "${test_name}")
done

if [[ "${collect_coverage}" == "true" ]]; then
  echo "::group::cargo llvm-cov --workspace test ${test_args[*]}"
  timeout --signal=INT --kill-after=30s "${test_timeout}" \
    cargo llvm-cov --branch --skip-functions --workspace test "${test_args[@]}" \
      --lcov --output-path "${output_lcov}" \
      --ignore-rust-version -- --nocapture
  echo "::endgroup::"
  exit 0
fi

if command -v cargo-nextest >/dev/null 2>&1; then
  echo "::group::cargo nextest run --profile ci -p ironclaw_integration_tests ${test_args[*]}"
  status=0
  timeout --signal=INT --kill-after=30s "${test_timeout}" \
    cargo nextest run --profile ci -p ironclaw_integration_tests "${test_args[@]}" \
      --test-threads 4 --ignore-rust-version || status=$?
  echo "::endgroup::"
  exit "${status}"
fi

if [[ "${CI:-}" == "true" ]]; then
  echo "cargo-nextest is required in CI but was not found on PATH" >&2
  exit 1
fi

local_timeout="${test_timeout}"
if [[ "${run_groups}" == "true" ]]; then
  local_timeout="28m"
fi
echo "::group::cargo test -p ironclaw_integration_tests ${test_args[*]}"
status=0
timeout --signal=INT --kill-after=30s "${local_timeout}" \
  cargo test -p ironclaw_integration_tests "${test_args[@]}" \
    --no-fail-fast --ignore-rust-version -- --nocapture || status=$?
echo "::endgroup::"
exit "${status}"
