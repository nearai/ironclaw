#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
under_test="${repo_root}/scripts/ci/reborn-coverage-lane-run.sh"
sandbox="$(mktemp -d)"
trap 'rm -rf "${sandbox}"' EXIT

mkdir -p "${sandbox}/bin"

cat >"${sandbox}/bin/cargo-nextest" <<'STUB'
#!/usr/bin/env bash
exit 0
STUB

cat >"${sandbox}/bin/cargo" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${INTEGRATION_BATCH_LOG}"
if [[ "$*" == nextest\ run* ]]; then
  exit 17
fi
exit 0
STUB

cat >"${sandbox}/bin/timeout" <<'STUB'
#!/usr/bin/env bash
while [[ "$1" == --* || "$1" == *[smhd] ]]; do
  case "$1" in
    --signal=*|--kill-after=*) shift ;;
    *) shift; break ;;
  esac
done
exec "$@"
STUB

chmod +x "${sandbox}/bin/cargo-nextest" "${sandbox}/bin/cargo" "${sandbox}/bin/timeout"

mkdir -p "${sandbox}/without-nextest-bin"
cp "${sandbox}/bin/cargo" "${sandbox}/bin/timeout" "${sandbox}/without-nextest-bin/"
chmod +x "${sandbox}/without-nextest-bin/cargo" "${sandbox}/without-nextest-bin/timeout"

missing_nextest_path="${sandbox}/without-nextest-bin:/usr/bin:/bin"
if PATH="${missing_nextest_path}" command -v cargo-nextest >/dev/null 2>&1; then
  echo "FAIL: missing-nextest probe PATH unexpectedly contains cargo-nextest" >&2
  exit 1
fi

status=0
mkdir -p "${sandbox}/group-only/lib"
cp "${under_test}" "${sandbox}/group-only/reborn-coverage-lane-run.sh"
cat >"${sandbox}/group-only/lib/integration_test_inventory.py" <<'STUB'
import os
import sys

with open(os.environ["INTEGRATION_BATCH_LOG"], "a", encoding="utf-8") as log:
    log.write("topology-check\n")
if sys.argv[1:] != ["--validate-group-topology", os.getcwd()]:
    sys.exit(24)
if os.environ.get("FAIL_GROUP_TOPOLOGY") == "true":
    sys.exit(23)
STUB
cat >"${sandbox}/group-only/reborn-coverage-int-tier-tests.sh" <<'STUB'
#!/usr/bin/env bash
if [[ "${VALID_GROUP_COVERAGE:-false}" == "true" ]]; then
  printf '%s\n' --test reborn_group_valid
  exit 0
fi
echo "FAIL: groups-only pass/fail batch rediscovered the coverage inventory" >&2
exit 19
STUB
cat >"${sandbox}/group-only/run-reborn-group-tests.sh" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' group-run >>"${INTEGRATION_BATCH_LOG}"
STUB
chmod +x "${sandbox}/group-only/"*.sh "${sandbox}/group-only/lib/"*.py

if ! (
  PATH="${sandbox}/bin:/usr/bin:/bin" \
    INTEGRATION_BATCH_LOG="${sandbox}/group-only.log" \
    REBORN_COV_COLLECT=false \
    REBORN_COV_LANES_JSON='["groups"]' \
    REBORN_COV_LANE_PARTITIONS=4 \
    bash "${sandbox}/group-only/reborn-coverage-lane-run.sh" "${sandbox}/unused-group.lcov"
); then
  echo "FAIL: groups-only pass/fail batch did not delegate directly to the canonical group runner" >&2
  status=1
elif [[ "$(cat "${sandbox}/group-only.log")" != $'topology-check\ngroup-run' ]]; then
  echo "FAIL: groups-only pass/fail batch did not invoke the canonical group runner exactly once" >&2
  status=1
fi

coverage_status=0
FAIL_GROUP_TOPOLOGY=true \
  PATH="${sandbox}/bin:/usr/bin:/bin" \
  INTEGRATION_BATCH_LOG="${sandbox}/group-coverage.log" \
  REBORN_COV_COLLECT=true \
  REBORN_COV_LANES_JSON='["groups"]' \
  REBORN_COV_LANE_PARTITIONS=4 \
  bash "${sandbox}/group-only/reborn-coverage-lane-run.sh" \
    "${sandbox}/unused-group-coverage.lcov" || coverage_status=$?
if [[ "${coverage_status}" -ne 23 ]] ||
   [[ "$(cat "${sandbox}/group-coverage.log" 2>/dev/null || true)" != "topology-check" ]]; then
  echo "FAIL: coverage groups lane did not stop at topology validation" >&2
  status=1
fi

if ! VALID_GROUP_COVERAGE=true \
  PATH="${sandbox}/bin:/usr/bin:/bin" \
  INTEGRATION_BATCH_LOG="${sandbox}/valid-group-coverage.log" \
  REBORN_COV_COLLECT=true \
  REBORN_COV_LANES_JSON='["groups"]' \
  REBORN_COV_LANE_PARTITIONS=4 \
  bash "${sandbox}/group-only/reborn-coverage-lane-run.sh" \
    "${sandbox}/valid-group-coverage.lcov"; then
  echo "FAIL: valid coverage groups lane did not reach llvm-cov" >&2
  status=1
elif ! grep -q '^llvm-cov .*--test reborn_group_valid' "${sandbox}/valid-group-coverage.log"; then
  echo "FAIL: valid coverage groups lane omitted its registered target" >&2
  status=1
fi
if (
  cd "${repo_root}"
  PATH="${sandbox}/bin:/usr/bin:/bin" \
    CI=true \
    INTEGRATION_BATCH_LOG="${sandbox}/commands.log" \
    REBORN_COV_COLLECT=false \
    REBORN_COV_LANES_JSON='[0,"groups"]' \
    REBORN_COV_LANE_PARTITIONS=4 \
    bash "${under_test}" "${sandbox}/unused.lcov"
); then
  echo "FAIL: a failed flat batch did not fail the combined runner" >&2
  status=1
fi

if [[ "${status}" -eq 0 ]]; then
  if [[ "$(grep -c '^nextest run ' "${sandbox}/commands.log")" -ne 1 ]]; then
    echo "FAIL: selected flat lanes did not use one nextest invocation" >&2
    status=1
  fi
  if ! grep -q -- '--test reborn_group_' "${sandbox}/commands.log"; then
    echo "FAIL: group suites did not run after the flat batch failed" >&2
    status=1
  fi
fi

ci_missing_output="${sandbox}/ci-missing-nextest.log"
if (
  cd "${repo_root}"
  PATH="${missing_nextest_path}" \
    CI=true \
    INTEGRATION_BATCH_LOG="${sandbox}/ci-missing-nextest.commands" \
    REBORN_COV_COLLECT=false \
    REBORN_COV_LANES_JSON='[0]' \
    REBORN_COV_LANE_PARTITIONS=4 \
    bash "${under_test}" "${sandbox}/ci-missing-nextest.lcov"
) >"${ci_missing_output}" 2>&1; then
  echo "FAIL: CI accepted a flat batch without cargo-nextest" >&2
  status=1
elif ! grep -qx 'cargo-nextest is required in CI but was not found on PATH' "${ci_missing_output}"; then
  echo "FAIL: CI missing-nextest failure did not emit the required diagnostic" >&2
  status=1
fi

local_fallback_log="${sandbox}/local-fallback.commands"
if ! (
  cd "${repo_root}"
  env -u CI \
    PATH="${missing_nextest_path}" \
    INTEGRATION_BATCH_LOG="${local_fallback_log}" \
    REBORN_COV_COLLECT=false \
    REBORN_COV_LANES_JSON='[0]' \
    REBORN_COV_LANE_PARTITIONS=4 \
    bash "${under_test}" "${sandbox}/local-fallback.lcov"
); then
  echo "FAIL: local flat batch without cargo-nextest did not use cargo test" >&2
  status=1
elif ! grep -q '^test -p ironclaw_integration_tests .* --no-fail-fast' "${local_fallback_log}"; then
  echo "FAIL: local flat batch did not run cargo test with --no-fail-fast" >&2
  status=1
elif grep -q 'nextest' "${local_fallback_log}"; then
  echo "FAIL: local flat batch without cargo-nextest logged nextest" >&2
  status=1
fi

coverage_output="${sandbox}/coverage-output.log"
if (
  cd "${repo_root}"
  PATH="${sandbox}/bin:/usr/bin:/bin" \
    REBORN_COV_COLLECT=true \
    REBORN_COV_LANES_JSON='[0,1]' \
    REBORN_COV_LANE_PARTITIONS=4 \
    bash "${under_test}" "${sandbox}/invalid.lcov"
) >"${coverage_output}" 2>&1; then
  echo "FAIL: coverage accepted a multi-lane batch" >&2
  status=1
elif ! grep -q "coverage batches must contain exactly one lane" "${coverage_output}"; then
  echo "FAIL: coverage rejected the batch without a useful diagnostic" >&2
  status=1
fi

if [[ "${status}" -ne 0 ]]; then
  test ! -f "${sandbox}/commands.log" || cat "${sandbox}/commands.log" >&2
  cat "${coverage_output}" >&2
  exit "${status}"
fi

echo "IronClaw integration batch runner: OK"
