#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
under_test="${repo_root}/scripts/ci/reborn-coverage-lane-run.sh"
sandbox="$(mktemp -d)"
trap 'rm -rf "${sandbox}"' EXIT
status=0

mkdir -p "${sandbox}/bin" "${sandbox}/without-nextest-bin"
cat >"${sandbox}/bin/cargo-nextest" <<'STUB'
#!/usr/bin/env bash
exit 0
STUB
cat >"${sandbox}/bin/cargo" <<'STUB'
#!/usr/bin/env bash
printf 'cargo:%s\n' "$*" >>"${INTEGRATION_BATCH_LOG}"
if [[ "${FAIL_NEXTEST:-false}" == "true" && "$*" == nextest\ run* ]]; then
  exit 17
fi
STUB
cat >"${sandbox}/bin/timeout" <<'STUB'
#!/usr/bin/env bash
printf 'timeout:%s\n' "$*" >>"${INTEGRATION_BATCH_LOG}"
while [[ "$1" == --* || "$1" == *[smhd] ]]; do
  case "$1" in
    --signal=*|--kill-after=*) shift ;;
    *) shift; break ;;
  esac
done
exec "$@"
STUB
chmod +x "${sandbox}/bin/"*
cp "${sandbox}/bin/cargo" "${sandbox}/bin/timeout" "${sandbox}/without-nextest-bin/"
missing_nextest_path="${sandbox}/without-nextest-bin:/usr/bin:/bin"

run_batch() {
  local log="$1"
  shift
  (
    cd "${repo_root}"
    PATH="${sandbox}/bin:/usr/bin:/bin" \
      INTEGRATION_BATCH_LOG="${log}" \
      "$@"
  )
}

mkdir -p "${sandbox}/aliases/lib"
cp "${under_test}" "${sandbox}/aliases/reborn-coverage-lane-run.sh"
cp "${repo_root}/scripts/ci/lib/integration_test_inventory.py" \
  "${sandbox}/aliases/lib/integration_test_inventory.py"
cat >"${sandbox}/aliases/Cargo.toml" <<'TOML'
test = [
  { name = "reborn_integration_before", path = "tests/integration/alias.rs" },
  { name = "reborn_integration_after", path = "tests/integration/alias.rs" },
]
TOML
aliases_log="${sandbox}/aliases.log"
if ! (
  cd "${sandbox}/aliases"
  PATH="${sandbox}/bin:/usr/bin:/bin" \
    INTEGRATION_BATCH_LOG="${aliases_log}" \
    CI=true REBORN_COV_COLLECT=false REBORN_COV_LANES_JSON='[0]' \
    bash "${sandbox}/aliases/reborn-coverage-lane-run.sh"
); then
  echo "FAIL: aliased integration targets did not execute" >&2
  status=1
elif ! grep -q -- '--test reborn_integration_before' "${aliases_log}" ||
     ! grep -q -- '--test reborn_integration_after' "${aliases_log}"; then
  echo "FAIL: execution inventory dropped a Cargo target sharing a source path" >&2
  status=1
fi

mkdir -p "${sandbox}/topology/lib"
cp "${under_test}" "${sandbox}/topology/reborn-coverage-lane-run.sh"
cat >"${sandbox}/topology/lib/integration_test_inventory.py" <<'STUB'
#!/usr/bin/env python3
import json
import os
import sys

if sys.argv[1] == "--json":
    print(json.dumps({"partition_count": 4, "tests": [
        {"name": "reborn_group_stub", "lane": "groups"}
    ]}))
    raise SystemExit(0)
with open(os.environ["TOPOLOGY_LOG"], "w", encoding="utf-8") as log:
    log.write("validated-before-cargo\n")
raise SystemExit(23)
STUB
topology_status=0
TOPOLOGY_LOG="${sandbox}/topology.log" \
  PATH="${sandbox}/bin:/usr/bin:/bin" \
  INTEGRATION_BATCH_LOG="${sandbox}/topology-cargo.log" \
  CI=true REBORN_COV_COLLECT=false REBORN_COV_LANES_JSON='["groups"]' \
  bash "${sandbox}/topology/reborn-coverage-lane-run.sh" || topology_status=$?
if [[ "${topology_status}" -ne 23 ]] ||
   [[ "$(cat "${sandbox}/topology.log" 2>/dev/null || true)" != "validated-before-cargo" ]] ||
   [[ -e "${sandbox}/topology-cargo.log" ]]; then
  echo "FAIL: invalid group topology did not stop before Cargo" >&2
  status=1
fi

mixed_log="${sandbox}/mixed.log"
if FAIL_NEXTEST=true run_batch "${mixed_log}" env \
  CI=true REBORN_COV_COLLECT=false REBORN_COV_LANES_JSON='[0,"groups"]' \
  bash "${under_test}"; then
  echo "FAIL: failed mixed nextest batch returned success" >&2
  status=1
fi
if [[ "$(grep -c '^cargo:nextest run ' "${mixed_log}")" -ne 1 ]]; then
  echo "FAIL: mixed selection did not issue exactly one nextest command" >&2
  status=1
fi
for required in '--profile ci' '--test-threads 4' '--test reborn_group_' '--test reborn_integration_'; do
  if ! grep -q -- "${required}" "${mixed_log}"; then
    echo "FAIL: mixed nextest command omitted ${required}" >&2
    status=1
  fi
done

groups_log="${sandbox}/groups.log"
if ! run_batch "${groups_log}" env \
  CI=true REBORN_COV_COLLECT=false REBORN_COV_LANES_JSON='["groups"]' \
  bash "${under_test}"; then
  echo "FAIL: groups-only nextest selection failed" >&2
  status=1
elif [[ "$(grep -c '^cargo:nextest run ' "${groups_log}")" -ne 1 ]] ||
     grep -q -- '--test reborn_integration_' "${groups_log}"; then
  echo "FAIL: groups-only selection was not one exact nextest command" >&2
  status=1
fi

ci_missing_output="${sandbox}/ci-missing-nextest.out"
if (
  cd "${repo_root}"
  PATH="${missing_nextest_path}" CI=true \
    INTEGRATION_BATCH_LOG="${sandbox}/ci-missing.log" \
    REBORN_COV_COLLECT=false REBORN_COV_LANES_JSON='[0]' \
    bash "${under_test}"
) >"${ci_missing_output}" 2>&1; then
  echo "FAIL: CI accepted a batch without cargo-nextest" >&2
  status=1
elif ! grep -q 'cargo-nextest is required in CI' "${ci_missing_output}"; then
  echo "FAIL: missing-nextest CI diagnostic was not actionable" >&2
  status=1
fi

fallback_log="${sandbox}/fallback.log"
if ! (
  cd "${repo_root}"
  env -u CI PATH="${missing_nextest_path}" \
    INTEGRATION_BATCH_LOG="${fallback_log}" \
    REBORN_COV_COLLECT=false REBORN_COV_LANES_JSON='[0,"groups"]' \
    bash "${under_test}"
); then
  echo "FAIL: local no-nextest compatibility command failed" >&2
  status=1
elif [[ "$(grep -c '^cargo:test ' "${fallback_log}")" -ne 1 ]] ||
     ! grep -q -- '--no-fail-fast' "${fallback_log}" ||
     ! grep -q '^timeout:--signal=INT --kill-after=30s 28m cargo test ' "${fallback_log}"; then
  echo "FAIL: local group fallback was not one bounded no-fail-fast Cargo command" >&2
  status=1
fi

noncoverage_log="${sandbox}/noncoverage-no-output.log"
if ! run_batch "${noncoverage_log}" env \
  REBORN_COV_COLLECT=false REBORN_COV_LANES_JSON='[0]' \
  bash "${under_test}"; then
  echo "FAIL: non-coverage runner required a dummy LCOV path" >&2
  status=1
fi

coverage_missing_output="${sandbox}/coverage-missing-output.out"
if run_batch "${sandbox}/coverage-missing-output.log" env \
  REBORN_COV_COLLECT=true REBORN_COV_LANES_JSON='[0]' \
  bash "${under_test}" >"${coverage_missing_output}" 2>&1; then
  echo "FAIL: coverage runner accepted a missing LCOV path" >&2
  status=1
elif ! grep -q 'coverage requires an output LCOV path' "${coverage_missing_output}"; then
  echo "FAIL: missing coverage output diagnostic was not actionable" >&2
  status=1
fi

coverage_log="${sandbox}/coverage.log"
if ! run_batch "${coverage_log}" env \
  REBORN_COV_COLLECT=true REBORN_COV_LANES_JSON='["groups"]' \
  bash "${under_test}" "${sandbox}/groups.lcov"; then
  echo "FAIL: coverage group lane failed" >&2
  status=1
elif [[ "$(grep -c '^cargo:llvm-cov ' "${coverage_log}")" -ne 1 ]] ||
     ! grep -q -- '--test reborn_group_' "${coverage_log}"; then
  echo "FAIL: coverage group lane changed its llvm-cov command shape" >&2
  status=1
fi

invalid_output="${sandbox}/invalid.out"
if run_batch "${sandbox}/invalid.log" env \
  REBORN_COV_COLLECT=false REBORN_COV_LANES_JSON='[4]' \
  bash "${under_test}" >"${invalid_output}" 2>&1; then
  echo "FAIL: runner accepted a lane outside the inventory partition count" >&2
  status=1
elif ! grep -q 'unique lane indices' "${invalid_output}"; then
  echo "FAIL: invalid-lane diagnostic was not actionable" >&2
  status=1
fi

if [[ "${status}" -ne 0 ]]; then
  for log in "${sandbox}"/*.log "${sandbox}"/*.out; do
    [[ -f "${log}" ]] && { echo "==> ${log}" >&2; cat "${log}" >&2; }
  done
  exit "${status}"
fi

echo "IronClaw integration batch runner: OK"
