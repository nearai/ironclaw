#!/usr/bin/env bash
#
# Run one batch of IronClaw integration-tier lanes. Pull requests and merge
# groups batch every selected lane into one compile; main keeps one batch per
# lane so instrumented coverage still produces five mergeable tracefiles.
#
# The tests/integration/ suites (see reborn-coverage-int-tier-tests.sh for
# the canonical, registration-driven enumeration — flat, domain-folder, and
# group bins alike) are split across 5 lanes in
# .github/workflows/reborn-tests.yml's `reborn-integration-coverage` matrix
# job: 4 modulo-partitions of the `reborn_integration_*` suites, plus one
# dedicated lane for the `reborn_group_*` suites.
#
# Main runs use this script for instrumented pass/fail and coverage. Pull
# requests and merge groups set REBORN_COV_COLLECT=false and use the same test
# inventory with nextest for fast pass/fail feedback.
#
# All of this lane's assigned suites run in ONE `cargo llvm-cov ... test`
# invocation, with one repeated `--test <name>` per suite, `--workspace` so
# the report covers every linked workspace crate (not just the root
# package), and `--lcov --output-path` attached directly to that same
# invocation. This mirrors the retired reborn-coverage.yml workflow's
# working `cargo llvm-cov --workspace "${test_args[@]}" --json ...` shape —
# deliberately NOT split into a `--no-report test` pass followed by a
# separate `cargo llvm-cov report` call, because the standalone `report`
# subcommand has no `--workspace`/`-p` flag of its own (confirmed via `cargo
# llvm-cov report --help`) and empirically defaults to reporting only the
# current/root package, silently dropping every crates/ironclaw_* file. The
# combined single-invocation form is the only one observed to include the
# other workspace crates' coverage.
#
# Reuses reborn-coverage-int-tier-tests.sh as the single source of truth for
# suite discovery/naming (the tests/integration/ -> reborn_integration_*/
# reborn_group_* rewrite rules), so this script never re-derives that mapping.
#
# REBORN_COV_LANES_JSON is a non-empty JSON array containing flat partition
# indices and/or "groups". Instrumented coverage accepts exactly one lane;
# uninstrumented gates combine all selected flat suites into one nextest
# invocation and then run group suites through their required sequential Cargo
# runner.

set -euo pipefail

output_lcov="${1:?usage: reborn-coverage-lane-run.sh <output-lcov-path>}"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
test_timeout="${REBORN_COV_LANE_TEST_TIMEOUT:-45m}"
group_test_timeout="${REBORN_GROUP_TEST_TIMEOUT:-28m}"
collect_coverage="${REBORN_COV_COLLECT:-true}"
lanes_json="${REBORN_COV_LANES_JSON:?REBORN_COV_LANES_JSON must select at least one lane}"
partition_count="${REBORN_COV_LANE_PARTITIONS:?REBORN_COV_LANE_PARTITIONS must be set}"

if [[ "${collect_coverage}" != "true" && "${collect_coverage}" != "false" ]]; then
  echo "REBORN_COV_COLLECT must be 'true' or 'false'; got '${collect_coverage}'" >&2
  exit 1
fi

if ! [[ "${partition_count}" =~ ^[0-9]+$ ]] || [[ "${partition_count}" -lt 1 ]]; then
  echo "REBORN_COV_LANE_PARTITIONS must be a positive integer; got '${partition_count}'" >&2
  exit 1
fi
partition_count_int=$((10#${partition_count}))

if ! jq -e --argjson count "${partition_count_int}" '
  type == "array" and length > 0 and
  all(.[]; . == "groups" or (type == "number" and floor == . and . >= 0 and . < $count)) and
  length == (unique | length)
' <<< "${lanes_json}" > /dev/null; then
  echo "REBORN_COV_LANES_JSON must contain unique lane indices in [0, ${partition_count_int}) and/or \"groups\"; got ${lanes_json}" >&2
  exit 1
fi

if [[ "${collect_coverage}" == "true" ]] &&
   ! jq -e 'length == 1' <<< "${lanes_json}" > /dev/null; then
  echo "coverage batches must contain exactly one lane; got ${lanes_json}" >&2
  exit 1
fi

mapfile -t lanes < <(jq -r '.[]' <<< "${lanes_json}")
run_groups=false
has_flat_lanes=false
for lane in "${lanes[@]}"; do
  if [[ "${lane}" == "groups" ]]; then
    run_groups=true
  else
    has_flat_lanes=true
  fi
done

if [[ "${run_groups}" == "true" ]]; then
  python3 "${script_dir}/lib/integration_test_inventory.py" \
    --validate-group-topology "${PWD}"
fi

# reborn-coverage-int-tier-tests.sh prints alternating "--test"/"<name>"
# lines; keep only the name lines (every 2nd line, portable awk — no GNU-only
# `sed -n 2~2p`).
all_names=()
if [[ "${collect_coverage}" == "true" || "${has_flat_lanes}" == "true" ]]; then
  mapfile -t all_names < <("${script_dir}/reborn-coverage-int-tier-tests.sh" | awk 'NR % 2 == 0')

  if [ "${#all_names[@]}" -eq 0 ]; then
    echo "No Reborn integration-tier test binaries discovered" >&2
    exit 1
  fi
fi

mapfile -t flat_names < <(
  printf '%s\n' "${all_names[@]}" \
    | grep -E '^reborn_(integration_|generated_)' \
    | LC_ALL=C sort
)

selected_flat_names=()
selected_group_names=()
for lane in "${lanes[@]}"; do
  if [[ "${lane}" == "groups" ]]; then
    if [[ "${collect_coverage}" == "true" ]]; then
      mapfile -t selected_group_names < <(
        printf '%s\n' "${all_names[@]}" | grep '^reborn_group_' | LC_ALL=C sort
      )
    fi
    continue
  fi
  lane_int=$((10#${lane}))
  for index in "${!flat_names[@]}"; do
    if (( index % partition_count_int == lane_int )); then
      selected_flat_names+=("${flat_names[$index]}")
    fi
  done
done

selected_names=("${selected_flat_names[@]}" "${selected_group_names[@]}")

if [[ "${collect_coverage}" == "true" && "${#selected_names[@]}" -eq 0 ]] ||
   [[ "${collect_coverage}" == "false" && "${#selected_flat_names[@]}" -eq 0 && "${run_groups}" == "false" ]]; then
  # Empty partitions are valid when the matrix has more partitions than tests
  # or when the sorted test list leaves a sparse tail for this partition
  # (mirrors run-reborn-root-partition.sh). Write an empty tracefile so the
  # caller's `cargo llvm-cov report`-less contract (this script always
  # produces output_lcov) holds even in the empty case.
  echo "No Reborn integration-tier suites assigned to batch ${lanes_json}; passing by design"
  if [[ "${collect_coverage}" == "true" ]]; then
    : > "${output_lcov}"
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
else
  status=0
  if [[ "${#selected_flat_names[@]}" -gt 0 ]]; then
    if command -v cargo-nextest >/dev/null 2>&1; then
      runner="nextest"
    elif [[ "${CI:-}" == "true" ]]; then
      echo "cargo-nextest is required in CI but was not found on PATH" >&2
      exit 1
    else
      runner="cargo"
    fi
    flat_test_args=()
    for test_name in "${selected_flat_names[@]}"; do
      flat_test_args+=(--test "${test_name}")
    done
    if [[ "${runner}" == "nextest" ]]; then
      echo "::group::cargo nextest run --profile ci -p ironclaw_integration_tests ${flat_test_args[*]}"
      timeout --signal=INT --kill-after=30s "${test_timeout}" \
        cargo nextest run --profile ci -p ironclaw_integration_tests "${flat_test_args[@]}" \
          --ignore-rust-version || status=$?
    else
      echo "::group::cargo test -p ironclaw_integration_tests ${flat_test_args[*]}"
      timeout --signal=INT --kill-after=30s "${test_timeout}" \
        cargo test -p ironclaw_integration_tests "${flat_test_args[@]}" \
          --no-fail-fast --ignore-rust-version -- --nocapture || status=$?
    fi
    echo "::endgroup::"
  fi

  if [[ "${run_groups}" == "true" ]]; then
    group_status=0
    REBORN_GROUP_TEST_TIMEOUT="${group_test_timeout}" \
      RUST_MIN_STACK=67108864 \
      "${script_dir}/run-reborn-group-tests.sh" || group_status=$?
    if [[ "${status}" -eq 0 ]]; then
      status="${group_status}"
    fi
  fi
  exit "${status}"
fi
