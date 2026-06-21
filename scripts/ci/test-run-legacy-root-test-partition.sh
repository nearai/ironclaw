#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
runner="${script_dir}/run-legacy-root-test-partition.sh"

tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT

bin_dir="${tmpdir}/bin"
workspace="${tmpdir}/workspace"
mkdir -p "${bin_dir}" "${workspace}/tests"

cat > "${bin_dir}/cargo" <<'CARGO'
#!/usr/bin/env bash
set -euo pipefail

case "${1:-}" in
  metadata)
    cat "${FAKE_CARGO_METADATA_JSON:?}"
    ;;
  test)
    printf '%s\n' "$*" >> "${FAKE_CARGO_TEST_LOG:?}"
    ;;
  *)
    echo "unexpected fake cargo command: $*" >&2
    exit 99
    ;;
esac
CARGO
chmod +x "${bin_dir}/cargo"

metadata="${tmpdir}/metadata.json"
test_log="${tmpdir}/cargo-test.log"
cat > "${metadata}" <<JSON
{
  "workspace_root": "${workspace}",
  "packages": [
    {
      "name": "ironclaw",
      "targets": [
        {
          "name": "base_case",
          "kind": ["test"],
          "src_path": "${workspace}/tests/base_case.rs"
        },
        {
          "name": "default_case",
          "kind": ["test"],
          "required-features": ["default"],
          "src_path": "${workspace}/tests/default_case.rs"
        },
        {
          "name": "postgres_case",
          "kind": ["test"],
          "required-features": ["postgres"],
          "src_path": "${workspace}/tests/postgres_case.rs"
        },
        {
          "name": "libsql_case",
          "kind": ["test"],
          "required-features": ["libsql"],
          "src_path": "${workspace}/tests/libsql_case.rs"
        },
        {
          "name": "combo_case",
          "kind": ["test"],
          "required-features": ["postgres", "libsql"],
          "src_path": "${workspace}/tests/combo_case.rs"
        },
        {
          "name": "disabled_case",
          "kind": ["test"],
          "required-features": ["bedrock"],
          "src_path": "${workspace}/tests/disabled_case.rs"
        },
        {
          "name": "unit_target",
          "kind": ["lib"],
          "src_path": "${workspace}/src/lib.rs"
        }
      ]
    },
    {
      "name": "ironclaw_other",
      "targets": [
        {
          "name": "other_case",
          "kind": ["test"],
          "src_path": "${workspace}/tests/other_case.rs"
        }
      ]
    }
  ]
}
JSON

run_partition() {
  local partition_count="$1"
  local partition_index="$2"
  local feature_flags="$3"

  : > "${test_log}"
  PATH="${bin_dir}:${PATH}" \
    FAKE_CARGO_METADATA_JSON="${metadata}" \
    FAKE_CARGO_TEST_LOG="${test_log}" \
    LEGACY_ROOT_TEST_PARTITIONS="${partition_count}" \
    LEGACY_ROOT_TEST_PARTITION="${partition_index}" \
    LEGACY_ROOT_TEST_FEATURE_FLAGS="${feature_flags}" \
    bash "${runner}" >/dev/null
}

assert_rejects_env() {
  local name="$1"
  local partition_count="$2"
  local partition_index="$3"
  local output
  local status

  set +e
  output="$(
    PATH="${bin_dir}:${PATH}" \
      FAKE_CARGO_METADATA_JSON="${metadata}" \
      FAKE_CARGO_TEST_LOG="${test_log}" \
      LEGACY_ROOT_TEST_PARTITIONS="${partition_count}" \
      LEGACY_ROOT_TEST_PARTITION="${partition_index}" \
      LEGACY_ROOT_TEST_FEATURE_FLAGS="" \
      bash "${runner}" 2>&1
  )"
  status=$?
  set -e

  if [ "${status}" -ne 2 ]; then
    printf 'FAIL %s: expected exit 2, got %s\n%s\n' "${name}" "${status}" "${output}" >&2
    exit 1
  fi

  printf 'PASS %s\n' "${name}"
}

assert_ran_tests() {
  local name="$1"
  local expected="$2"
  local actual

  actual="$(awk '{
    for (i = 1; i <= NF; i++) {
      if ($i == "--test" && i < NF) {
        print $(i + 1)
      }
    }
  }' "${test_log}" | sort)"
  if [ "${actual}" != "${expected}" ]; then
    printf 'FAIL %s\nExpected:\n%s\nActual:\n%s\n' "${name}" "${expected}" "${actual}" >&2
    exit 1
  fi

  printf 'PASS %s\n' "${name}"
}

assert_rejects_env "zero partition count" "0" "0"
assert_rejects_env "non-numeric partition count" "two" "0"
assert_rejects_env "non-numeric partition index" "2" "one"
assert_rejects_env "out-of-range partition index" "2" "2"

run_partition "1" "0" "--no-default-features --features postgres,libsql"
assert_ran_tests "space-separated features filter required test targets" "base_case
combo_case
libsql_case
postgres_case"

run_partition "1" "0" "--no-default-features --features=postgres,libsql"
assert_ran_tests "equals features filter required test targets" "base_case
combo_case
libsql_case
postgres_case"

run_partition "1" "0" ""
assert_ran_tests "default feature is included without no-default-features" "base_case
default_case"
