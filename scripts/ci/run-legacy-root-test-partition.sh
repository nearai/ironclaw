#!/usr/bin/env bash
set -euo pipefail

partition_count="${LEGACY_ROOT_TEST_PARTITIONS:?LEGACY_ROOT_TEST_PARTITIONS must be set}"
partition_index="${LEGACY_ROOT_TEST_PARTITION:?LEGACY_ROOT_TEST_PARTITION must be set}"
feature_flags="${LEGACY_ROOT_TEST_FEATURE_FLAGS:-}"

if ! [[ "${partition_count}" =~ ^[0-9]+$ ]] || [ "${partition_count}" -lt 1 ]; then
  echo "LEGACY_ROOT_TEST_PARTITIONS must be a positive integer; got '${partition_count}'" >&2
  exit 2
fi

partition_count_int=$((10#${partition_count}))

if ! [[ "${partition_index}" =~ ^[0-9]+$ ]]; then
  echo "LEGACY_ROOT_TEST_PARTITION must be an integer in [0, ${partition_count_int}); got '${partition_index}'" >&2
  exit 2
fi

partition_index_int=$((10#${partition_index}))

if [ "${partition_index_int}" -ge "${partition_count_int}" ]; then
  echo "LEGACY_ROOT_TEST_PARTITION must be an integer in [0, ${partition_count}); got '${partition_index}'" >&2
  exit 2
fi

enabled_features=()
read -r -a feature_args <<< "${feature_flags}"
for index in "${!feature_args[@]}"; do
  arg="${feature_args[$index]}"
  case "${arg}" in
    --features)
      next_index=$((index + 1))
      if [ "${next_index}" -lt "${#feature_args[@]}" ]; then
        IFS=',' read -r -a parsed_features <<< "${feature_args[$next_index]}"
        enabled_features+=("${parsed_features[@]}")
      fi
      ;;
    --features=*)
      IFS=',' read -r -a parsed_features <<< "${arg#--features=}"
      enabled_features+=("${parsed_features[@]}")
      ;;
  esac
done

if [[ " ${feature_flags} " != *" --no-default-features "* ]]; then
  enabled_features+=(default)
fi

enabled_feature_csv="$(IFS=,; echo "${enabled_features[*]}")"
export ENABLED_FEATURES="${enabled_feature_csv}"

integration_tests_output="$(
  cargo metadata --no-deps --format-version=1 \
    | python3 -c '
import json
import os
import sys
from pathlib import Path

metadata = json.load(sys.stdin)
root = Path(metadata["workspace_root"]).resolve()
tests_dir = root / "tests"
enabled = {feature for feature in os.environ.get("ENABLED_FEATURES", "").split(",") if feature}

names = []
for package in metadata["packages"]:
    if package["name"] != "ironclaw":
        continue
    for target in package["targets"]:
        if "test" not in target.get("kind", []):
            continue
        src_path = Path(target["src_path"]).resolve()
        try:
            src_path.relative_to(tests_dir)
        except ValueError:
            continue
        target_name = target["name"]
        required = set(target.get("required-features") or [])
        if required.issubset(enabled):
            names.append(target_name)
        else:
            missing = ",".join(sorted(required - enabled))
            print(
                f"Skipping {target_name} because required features are not enabled: {missing}",
                file=sys.stderr,
            )

for name in sorted(names):
    print(name)
'
)" || {
  echo "Failed to enumerate legacy root integration tests" >&2
  exit 1
}

integration_tests=()
if [ -n "${integration_tests_output}" ]; then
  mapfile -t integration_tests <<< "${integration_tests_output}"
fi

ran_any=false
for index in "${!integration_tests[@]}"; do
  if (( index % partition_count_int != partition_index_int )); then
    continue
  fi

  test_name="${integration_tests[$index]}"
  ran_any=true
  echo "::group::cargo test --test ${test_name}"
  # shellcheck disable=SC2086 # feature_flags intentionally expands to zero or more Cargo args.
  cargo test ${feature_flags} --test "${test_name}" -- --nocapture
  echo "::endgroup::"
done

if [ "${ran_any}" = false ]; then
  echo "No legacy root integration tests assigned to partition ${partition_index_int} of ${partition_count_int}; passing by design"
fi
