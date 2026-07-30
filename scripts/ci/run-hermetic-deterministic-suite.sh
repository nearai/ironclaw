#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
hermetic="${repo_root}/scripts/ci/run-hermetic-test-process.sh"
stage="${1:-all}"
if [[ "$#" -gt 0 ]]; then
  shift
fi

frontend_corepack_home=""

cleanup() {
  if [[ -n "${frontend_corepack_home}" && -d "${frontend_corepack_home}" ]]; then
    rm -rf -- "${frontend_corepack_home}"
  fi
}
trap cleanup EXIT

run() {
  "${hermetic}" -- "$@"
}

prepare_rust_dependencies() {
  # Dependency acquisition is setup, not test behavior. Fetch once before the
  # hermetic process switches Cargo into offline mode.
  cargo fetch --locked
}

run_root_partitions() {
  local partition
  for partition in 0 1 2 3; do
    REBORN_ROOT_TEST_PARTITIONS=4 \
      REBORN_ROOT_TEST_PARTITION="${partition}" \
      REBORN_ROOT_TEST_TIMEOUT="${REBORN_ROOT_TEST_TIMEOUT:-28m}" \
      RUST_MIN_STACK=67108864 \
      run "${repo_root}/scripts/ci/run-reborn-root-partition.sh"
  done
}

discover_reborn_packages() {
  local allowlist closure
  allowlist="$(
    cargo metadata --no-deps --format-version 1 \
      | jq -c '
          [
            .packages[]
            | select(
                (
                  (.name == "ironclaw")
                  or (.name == "ironclaw_runner")
                  or (.name | startswith("ironclaw_reborn"))
                  or (.name | startswith("ironclaw_product"))
                  or (.name == "ironclaw_architecture")
                  or (.name == "ironclaw_slack_extension")
                  or (.name == "ironclaw_telegram_extension")
                  or (.name == "ironclaw_telegram_v2_adapter")
                  or (.name | startswith("ironclaw_webui"))
                )
                and (.name != "ironclaw_reborn_integration_tests")
              )
            | .name
          ]
          | unique
        '
  )"
  closure="$(
    comm -12 \
      <(cargo tree -p ironclaw -e normal,build --prefix none \
        | grep -oE 'ironclaw_[a-z0-9_]+' \
        | LC_ALL=C sort -u) \
      <(cargo metadata --no-deps --format-version 1 \
        | jq -r '.packages[].name' \
        | LC_ALL=C sort -u) \
      | jq -R -s -c 'split("\n") | map(select(length > 0))'
  )"
  jq -n -r \
    --argjson allowlist "${allowlist}" \
    --argjson closure "${closure}" \
    '$allowlist + $closure | unique | .[]'
}

run_crate_tests() {
  local package feature_flags
  while IFS= read -r package; do
    feature_flags="$("${repo_root}/scripts/ci/package-feature-flags.sh" "${package}")"
    # shellcheck disable=SC2086 # feature_flags is the checked-in CI argument list.
    run cargo test -p "${package}" ${feature_flags} --all-targets -- --nocapture
  done < <(discover_reborn_packages)
}

run_integration_tier() {
  local test_name
  while IFS= read -r test_name; do
    [[ "${test_name}" == --test ]] && continue
    run cargo test -p ironclaw_reborn_integration_tests \
      --test "${test_name}" -- --nocapture
  done < <("${repo_root}/scripts/ci/reborn-coverage-int-tier-tests.sh")
}

run_python_e2e() {
  if [[ -z "${IRONCLAW_EMULATE_CLI:-}" ]]; then
    echo "IRONCLAW_EMULATE_CLI must name the built pinned Emulate CLI" >&2
    echo "Use the revision pinned in .github/workflows/reborn-e2e.yml." >&2
    return 2
  fi
  run pytest \
    tests/e2e/scenarios/test_reborn_webui_v2_smoke.py \
    -v --timeout=120
  run pytest \
    tests/e2e/scenarios/test_provider_capability_inventory.py \
    tests/e2e/scenarios/test_journey_coverage.py \
    tests/e2e/scenarios/test_emulate_reborn_provider_contracts.py \
    tests/e2e/scenarios/test_provider_fault_proxy.py \
    tests/e2e/scenarios/test_emulate_build_parity.py \
    tests/e2e/scenarios/test_provider_world_isolation.py \
    tests/e2e/scenarios/test_reborn_qa_trace_replay.py \
    tests/e2e/scenarios/test_reborn_qa_trace_full_path.py \
    -m "not shared_world" \
    -v --timeout=120
  run pytest tests/e2e/scenarios/test_reborn_blackbox_smoke.py -v --timeout=120
}

prepare_frontend_dependencies() {
  local package_manager
  package_manager="$(
    jq -r '.packageManager' \
      "${repo_root}/crates/ironclaw_webui/frontend/package.json"
  )"
  if [[ "${package_manager}" != pnpm@* ]]; then
    echo "frontend packageManager must pin pnpm: ${package_manager}" >&2
    return 2
  fi

  frontend_corepack_home="$(
    mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/ironclaw-corepack.XXXXXX"
  )"
  COREPACK_HOME="${frontend_corepack_home}" \
    corepack install --global "${package_manager}"
  (
    cd "${repo_root}/crates/ironclaw_webui/frontend"
    COREPACK_HOME="${frontend_corepack_home}" \
      corepack pnpm install --frozen-lockfile
  )
}

run_frontend_tests() {
  # Run from the package directory so Corepack honors its pinned packageManager
  # version and its isolated setup cache without registry access in the guard.
  COREPACK_HOME="${frontend_corepack_home}" \
    run bash -c 'cd "$1" && exec corepack pnpm test' \
      _ "${repo_root}/crates/ironclaw_webui/frontend"
}

case "${stage}" in
  self-test)
    "${repo_root}/scripts/ci/test-hermetic-test-process.sh"
    ;;
  root)
    prepare_rust_dependencies
    run_root_partitions
    ;;
  crates)
    prepare_rust_dependencies
    run_crate_tests
    ;;
  groups)
    prepare_rust_dependencies
    REBORN_GROUP_TEST_TIMEOUT="${REBORN_GROUP_TEST_TIMEOUT:-28m}" \
      RUST_MIN_STACK=67108864 \
      run "${repo_root}/scripts/ci/run-reborn-group-tests.sh"
    ;;
  integration)
    prepare_rust_dependencies
    run_integration_tier
    ;;
  qa)
    prepare_rust_dependencies
    run "${repo_root}/scripts/ci/check-reborn-qa-fixtures.sh"
    run cargo test -p ironclaw_reborn_integration_tests \
      --test reborn_qa_recorded_behavior -- --nocapture
    ;;
  rust-e2e)
    prepare_rust_dependencies
    run "${repo_root}/scripts/reborn-e2e-rust.sh" "${1:-all}"
    ;;
  frontend)
    prepare_frontend_dependencies
    run_frontend_tests
    ;;
  python-e2e)
    run_python_e2e
    ;;
  all)
    "${repo_root}/scripts/ci/test-hermetic-test-process.sh"
    prepare_rust_dependencies
    run_crate_tests
    run_root_partitions
    REBORN_GROUP_TEST_TIMEOUT="${REBORN_GROUP_TEST_TIMEOUT:-28m}" \
      RUST_MIN_STACK=67108864 \
      run "${repo_root}/scripts/ci/run-reborn-group-tests.sh"
    run_integration_tier
    run "${repo_root}/scripts/ci/check-reborn-qa-fixtures.sh"
    run cargo test -p ironclaw_reborn_integration_tests \
      --test reborn_qa_recorded_behavior -- --nocapture
    run "${repo_root}/scripts/reborn-e2e-rust.sh" all
    prepare_frontend_dependencies
    run_frontend_tests
    run cargo build -p ironclaw --bin ironclaw
    run_python_e2e
    ;;
  command)
    if [[ "$#" -eq 0 ]]; then
      echo "command stage requires a command" >&2
      exit 2
    fi
    prepare_rust_dependencies
    run "$@"
    ;;
  *)
    echo "unknown hermetic deterministic-suite stage: ${stage}" >&2
    echo "expected: all, self-test, crates, root, groups, integration, qa, rust-e2e, frontend, python-e2e, command" >&2
    exit 2
    ;;
esac
