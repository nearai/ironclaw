#!/usr/bin/env bash
set -euo pipefail

partition_count="${REBORN_ROOT_TEST_PARTITIONS:?REBORN_ROOT_TEST_PARTITIONS must be set}"
partition_index="${REBORN_ROOT_TEST_PARTITION:?REBORN_ROOT_TEST_PARTITION must be set}"
test_timeout="${REBORN_ROOT_TEST_TIMEOUT:-12m}"

if ! [[ "${partition_count}" =~ ^[0-9]+$ ]] || [ "${partition_count}" -lt 1 ]; then
  echo "REBORN_ROOT_TEST_PARTITIONS must be a positive integer; got '${partition_count}'" >&2
  exit 1
fi

partition_count_int=$((10#${partition_count}))

if ! [[ "${partition_index}" =~ ^[0-9]+$ ]]; then
  echo "REBORN_ROOT_TEST_PARTITION must be an integer in [0, ${partition_count_int}); got '${partition_index}'" >&2
  exit 1
fi

partition_index_int=$((10#${partition_index}))

if [ "${partition_index_int}" -ge "${partition_count_int}" ]; then
  echo "REBORN_ROOT_TEST_PARTITION must be an integer in [0, ${partition_count}); got '${partition_index}'" >&2
  exit 1
fi

test_names=()
while IFS= read -r test_name; do
  test_names+=("${test_name}")
done < <(
  {
    find tests -maxdepth 1 -type f -name 'reborn_*.rs' -print
    if [ -f tests/support_unit_tests.rs ]; then
      printf '%s\n' tests/support_unit_tests.rs
    fi
  } \
    | sed -E 's#^tests/##; s#\.rs$##' \
    | LC_ALL=C sort
)

if [ "${#test_names[@]}" -eq 0 ]; then
  echo "No Reborn root tests discovered" >&2
  exit 1
fi

selected_tests=()
for index in "${!test_names[@]}"; do
  if (( index % partition_count_int != partition_index_int )); then
    continue
  fi

  selected_tests+=("${test_names[$index]}")
done

if [ "${#selected_tests[@]}" -eq 0 ]; then
  # Empty partitions are valid when the matrix has more partitions than tests
  # or when the sorted test list leaves a sparse tail for this partition.
  echo "No Reborn root tests assigned to partition ${partition_index_int} of ${partition_count_int}; passing by design"
  exit 0
fi

cargo_args=(cargo nextest run --profile ci)
for test_name in "${selected_tests[@]}"; do
  cargo_args+=(--test "${test_name}")
done

echo "Running Reborn root test partition ${partition_index_int} of ${partition_count_int}:"
printf '  - %s\n' "${selected_tests[@]}"

echo "::group::${cargo_args[*]}"
timeout --signal=INT --kill-after=30s "${test_timeout}" "${cargo_args[@]}"
echo "::endgroup::"
