#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../.." && pwd)"
checker="${script_dir}/check-reborn-branch-coverage-flags.py"
tmp_root="$(mktemp -d)"
trap 'rm -rf "${tmp_root}"' EXIT

pass_count=0
fail_count=0

pass() {
  pass_count=$((pass_count + 1))
  printf '  ok   %s\n' "$1"
}

fail() {
  fail_count=$((fail_count + 1))
  printf '  FAIL %s\n' "$1" >&2
}

run_case() {
  set +e
  case_output="$("$@" 2>&1)"
  case_rc=$?
  set -e
}

assert_rc() {
  local name="$1" expected="$2"
  if [ "${case_rc}" -eq "${expected}" ]; then
    pass "${name}"
  else
    fail "${name}: expected rc=${expected}, got ${case_rc}"
    printf '%s\n' "${case_output}" >&2
  fi
}

assert_contains() {
  local name="$1" expected="$2"
  if [[ "${case_output}" == *"${expected}"* ]]; then
    pass "${name}"
  else
    fail "${name}: missing ${expected}"
    printf '%s\n' "${case_output}" >&2
  fi
}

printf '▶ checked-in branch exports\n'
run_case "${checker}"
assert_rc "all branch LCOV exports use the crash-safe shape" 0
assert_contains "the discovered export count is explicit" \
  "4 exports, 4 LLVM 21.1.3 toolchains, 4 compatibility envelopes"

printf '▶ missing --skip-functions sabotage\n'
mkdir -p "${tmp_root}/.github/workflows" "${tmp_root}/scripts/ci"
cp "${repo_root}/.github/workflows/coverage.yml" \
  "${tmp_root}/.github/workflows/coverage.yml"
cp "${repo_root}/.github/workflows/reborn-tests.yml" \
  "${tmp_root}/.github/workflows/reborn-tests.yml"
cp "${repo_root}/scripts/ci/reborn-coverage-lane-run.sh" \
  "${tmp_root}/scripts/ci/reborn-coverage-lane-run.sh"
sed -i.bak 's/--skip-functions//' \
  "${tmp_root}/.github/workflows/reborn-tests.yml"
run_case "${checker}" \
  "${tmp_root}/.github/workflows/coverage.yml" \
  "${tmp_root}/.github/workflows/reborn-tests.yml" \
  "${tmp_root}/scripts/ci/reborn-coverage-lane-run.sh"
assert_rc "removing the crash-safe flag fails" 1
assert_contains "the failure names the required flag" "--skip-functions"

printf '▶ unsafe toolchain sabotage\n'
cp "${repo_root}/.github/workflows/coverage.yml" \
  "${tmp_root}/.github/workflows/coverage.yml"
sed -i.bak 's/nightly-2025-11-01/nightly-2026-07-29/' \
  "${tmp_root}/.github/workflows/coverage.yml"
run_case "${checker}" \
  "${tmp_root}/.github/workflows/coverage.yml" \
  "${tmp_root}/.github/workflows/reborn-tests.yml" \
  "${tmp_root}/scripts/ci/reborn-coverage-lane-run.sh"
assert_rc "selecting a regressed LLVM toolchain fails" 1
assert_contains "the failure names the safe LLVM release" "LLVM 21.1.3"

printf '▶ missing MSRV override sabotage\n'
cp "${repo_root}/.github/workflows/coverage.yml" \
  "${tmp_root}/.github/workflows/coverage.yml"
cp "${repo_root}/.github/workflows/reborn-tests.yml" \
  "${tmp_root}/.github/workflows/reborn-tests.yml"
sed -i.bak 's/--ignore-rust-version//' \
  "${tmp_root}/scripts/ci/reborn-coverage-lane-run.sh"
run_case "${checker}" \
  "${tmp_root}/.github/workflows/coverage.yml" \
  "${tmp_root}/.github/workflows/reborn-tests.yml" \
  "${tmp_root}/scripts/ci/reborn-coverage-lane-run.sh"
assert_rc "removing the pinned compiler override fails" 1
assert_contains "the failure names the required override" "--ignore-rust-version"

printf '▶ missing compatibility envelope sabotage\n'
cp "${repo_root}/.github/workflows/coverage.yml" \
  "${tmp_root}/.github/workflows/coverage.yml"
cp "${repo_root}/scripts/ci/reborn-coverage-lane-run.sh" \
  "${tmp_root}/scripts/ci/reborn-coverage-lane-run.sh"
sed -i.bak '/RUSTC_BOOTSTRAP/d' \
  "${tmp_root}/.github/workflows/coverage.yml"
run_case "${checker}" \
  "${tmp_root}/.github/workflows/coverage.yml" \
  "${tmp_root}/.github/workflows/reborn-tests.yml" \
  "${tmp_root}/scripts/ci/reborn-coverage-lane-run.sh"
assert_rc "removing coverage-only bootstrap fails" 1
assert_contains "the missing bootstrap is explicit" "RUSTC_BOOTSTRAP"

cp "${repo_root}/.github/workflows/coverage.yml" \
  "${tmp_root}/.github/workflows/coverage.yml"
sed -i.bak 's/,slice_as_array//' \
  "${tmp_root}/.github/workflows/reborn-tests.yml"
run_case "${checker}" \
  "${tmp_root}/.github/workflows/coverage.yml" \
  "${tmp_root}/.github/workflows/reborn-tests.yml" \
  "${tmp_root}/scripts/ci/reborn-coverage-lane-run.sh"
assert_rc "changing the compatibility feature set fails" 1
assert_contains "the exact feature set is named" "slice_as_array"

printf '▶ build-profile contamination sabotage\n'
cp "${repo_root}/.github/workflows/coverage.yml" \
  "${tmp_root}/.github/workflows/coverage.yml"
sed -i.bak '/cargo llvm-cov clean --profraw-only/d' \
  "${tmp_root}/.github/workflows/coverage.yml"
run_case "${checker}" \
  "${tmp_root}/.github/workflows/coverage.yml" \
  "${tmp_root}/.github/workflows/reborn-tests.yml" \
  "${tmp_root}/scripts/ci/reborn-coverage-lane-run.sh"
assert_rc "retaining prebuild profiles fails" 1
assert_contains "profile contamination failure is explicit" "clear build-time profiles"

printf '▶ empty discovery and stale input sabotage\n'
: > "${tmp_root}/empty"
run_case "${checker}" "${tmp_root}/empty"
assert_rc "zero branch exports fail" 1
assert_contains "empty discovery is explicit" "no branch LCOV exports discovered"
run_case "${checker}" "${tmp_root}/missing"
assert_rc "a stale source path fails" 1
assert_contains "the stale path is named" "branch coverage source is missing"

if [ "${fail_count}" -gt 0 ]; then
  printf '\n%s branch-export self-test(s) failed\n' "${fail_count}" >&2
  exit 1
fi
printf '\nall %s branch-export self-tests passed\n' "${pass_count}"
