#!/usr/bin/env bash
#
# Run one instrumented Reborn integration-tier coverage lane and produce that
# lane's lcov tracefile.
#
# The 34 tests/integration/ suites (see reborn-coverage-int-tier-tests.sh for
# the canonical enumeration) are split across 5 lanes in
# .github/workflows/reborn-tests.yml's `reborn-integration-coverage` matrix
# job: 4 modulo-partitions of the 27 flat `reborn_integration_*` suites, plus
# one dedicated lane for all 7 `reborn_group_*` suites.
#
# This script is the SINGLE execution of the 34 int-tier suites for pass/fail
# purposes too: `cargo llvm-cov ... test` has the same pass/fail semantics as
# `cargo test`, so there is no separate uninstrumented run of the 27 flat
# suites. (The 7 group suites also still have their own uninstrumented
# `reborn-group-tests` job via run-reborn-group-tests.sh, which stays as the
# fast low-contention pass/fail signal for that suite; this lane additionally
# runs them once more, instrumented, for coverage.)
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
# Modes (REBORN_COV_LANE_MODE):
#   flat-partition  Modulo-partitions the 27 reborn_integration_* suites
#                   across REBORN_COV_LANE_PARTITIONS lanes; REBORN_COV_LANE_INDEX
#                   (0-based) selects this lane's slice — mirrors
#                   scripts/ci/run-reborn-root-partition.sh's partitioning.
#   group           Runs all reborn_group_* suites (7 total) — the one
#                   dedicated group coverage lane.
#
# Usage: REBORN_COV_LANE_MODE=... [other env] reborn-coverage-lane-run.sh <output-lcov-path>

set -euo pipefail

output_lcov="${1:?usage: reborn-coverage-lane-run.sh <output-lcov-path>}"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mode="${REBORN_COV_LANE_MODE:?REBORN_COV_LANE_MODE must be set to 'flat-partition' or 'group'}"
test_timeout="${REBORN_COV_LANE_TEST_TIMEOUT:-45m}"

# reborn-coverage-int-tier-tests.sh prints alternating "--test"/"<name>"
# lines; keep only the name lines (every 2nd line, portable awk — no GNU-only
# `sed -n 2~2p`).
mapfile -t all_names < <("${script_dir}/reborn-coverage-int-tier-tests.sh" | awk 'NR % 2 == 0')

if [ "${#all_names[@]}" -eq 0 ]; then
  echo "No Reborn integration-tier test binaries discovered" >&2
  exit 1
fi

selected_names=()

case "${mode}" in
  flat-partition)
    partition_count="${REBORN_COV_LANE_PARTITIONS:?REBORN_COV_LANE_PARTITIONS must be set for flat-partition mode}"
    partition_index="${REBORN_COV_LANE_INDEX:?REBORN_COV_LANE_INDEX must be set for flat-partition mode}"

    if ! [[ "${partition_count}" =~ ^[0-9]+$ ]] || [ "${partition_count}" -lt 1 ]; then
      echo "REBORN_COV_LANE_PARTITIONS must be a positive integer; got '${partition_count}'" >&2
      exit 1
    fi
    partition_count_int=$((10#${partition_count}))

    if ! [[ "${partition_index}" =~ ^[0-9]+$ ]]; then
      echo "REBORN_COV_LANE_INDEX must be an integer in [0, ${partition_count_int}); got '${partition_index}'" >&2
      exit 1
    fi
    partition_index_int=$((10#${partition_index}))

    if [ "${partition_index_int}" -ge "${partition_count_int}" ]; then
      echo "REBORN_COV_LANE_INDEX must be an integer in [0, ${partition_count}); got '${partition_index}'" >&2
      exit 1
    fi

    mapfile -t flat_names < <(printf '%s\n' "${all_names[@]}" | grep '^reborn_integration_' | LC_ALL=C sort)

    for index in "${!flat_names[@]}"; do
      if (( index % partition_count_int != partition_index_int )); then
        continue
      fi
      selected_names+=("${flat_names[$index]}")
    done
    ;;
  group)
    mapfile -t selected_names < <(printf '%s\n' "${all_names[@]}" | grep '^reborn_group_' | LC_ALL=C sort)
    ;;
  *)
    echo "Unknown REBORN_COV_LANE_MODE: ${mode} (expected 'flat-partition' or 'group')" >&2
    exit 1
    ;;
esac

if [ "${#selected_names[@]}" -eq 0 ]; then
  # Empty partitions are valid when the matrix has more partitions than tests
  # or when the sorted test list leaves a sparse tail for this partition
  # (mirrors run-reborn-root-partition.sh). Write an empty tracefile so the
  # caller's `cargo llvm-cov report`-less contract (this script always
  # produces output_lcov) holds even in the empty case.
  echo "No Reborn integration-tier suites assigned to this coverage lane (mode=${mode}); passing by design"
  : > "${output_lcov}"
  exit 0
fi

test_args=()
for test_name in "${selected_names[@]}"; do
  test_args+=(--test "${test_name}")
done

echo "::group::cargo llvm-cov --workspace test ${test_args[*]}"
timeout --signal=INT --kill-after=30s "${test_timeout}" \
  cargo llvm-cov --workspace test "${test_args[@]}" \
    --lcov --output-path "${output_lcov}" \
    -- --nocapture
echo "::endgroup::"
