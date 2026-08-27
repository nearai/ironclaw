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

status=0
mkdir -p "${sandbox}/group-only"
cp "${under_test}" "${sandbox}/group-only/reborn-coverage-lane-run.sh"
cat >"${sandbox}/group-only/reborn-coverage-int-tier-tests.sh" <<'STUB'
#!/usr/bin/env bash
echo "FAIL: groups-only pass/fail batch rediscovered the coverage inventory" >&2
exit 19
STUB
cat >"${sandbox}/group-only/run-reborn-group-tests.sh" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' group-run >>"${INTEGRATION_BATCH_LOG}"
STUB
chmod +x "${sandbox}/group-only/"*.sh

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
elif [[ "$(cat "${sandbox}/group-only.log")" != "group-run" ]]; then
  echo "FAIL: groups-only pass/fail batch did not invoke the canonical group runner exactly once" >&2
  status=1
fi
if (
  cd "${repo_root}"
  PATH="${sandbox}/bin:/usr/bin:/bin" \
    CI=true \
    IRONCLAW_GATE_TEST_RUNNER=nextest \
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
