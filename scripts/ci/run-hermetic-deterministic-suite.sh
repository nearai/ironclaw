#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
hermetic="${repo_root}/scripts/ci/run-hermetic-test-process.sh"
stage="${1:-all}"
if [[ "$#" -gt 0 ]]; then
  shift
fi

frontend_corepack_home=""
postgres_image_prepared=0
webui_frontend_dir=""

cleanup() {
  if [[ -n "${frontend_corepack_home}" && -d "${frontend_corepack_home}" ]]; then
    rm -rf -- "${frontend_corepack_home}"
  fi
}
trap cleanup EXIT

run() {
  "${hermetic}" -- "$@"
}

# Resolved by crate NAME through the shared inventory
# (scripts/ci/lib/crate_tree.py) rather than a literal `crates/ironclaw_webui`
# path, so the target-architecture family move (PROPOSAL §5) cannot leave the
# frontend prep/test stages pointed at a directory that no longer exists
# (docs/internal/reborn/target-architecture/CHECKLIST.md WS10). Memoized: called from
# both prepare_frontend_dependencies and run_frontend_tests.
resolve_webui_frontend_dir() {
  if [[ -n "${webui_frontend_dir}" ]]; then
    return
  fi
  webui_frontend_dir="$("${repo_root}/scripts/ci/crate-dir.sh" ironclaw_webui "${repo_root}")/frontend"
}

prepare_rust_dependencies() {
  # Dependency acquisition is setup, not test behavior. Fetch once before the
  # hermetic process switches Cargo into offline mode. Rust builds can invoke
  # the WebUI build script, so prepare its pinned package manager too.
  prepare_command_dependencies
  prepare_frontend_dependencies
}

prepare_command_dependencies() {
  cargo fetch --locked
}

prepare_postgres_test_image() {
  if [[ "${postgres_image_prepared}" == "1" ]]; then
    return
  fi
  if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required to prepare postgres:16-alpine for hermetic tests" >&2
    return 2
  fi
  docker pull postgres:16-alpine
  postgres_image_prepared=1
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
  "${repo_root}/scripts/ci/discover-reborn-package-crates.sh" | jq -r '.[]'
}

run_crate_tests() {
  local package feature_flags
  prepare_postgres_test_image
  while IFS= read -r package; do
    feature_flags="$("${repo_root}/scripts/ci/package-feature-flags.sh" "${package}")"
    # shellcheck disable=SC2086 # feature_flags is the checked-in CI argument list.
    run cargo test -p "${package}" ${feature_flags} --all-targets -- --nocapture
  done < <(discover_reborn_packages)
}

run_integration_tier() {
  local test_name
  prepare_postgres_test_image
  while IFS= read -r test_name; do
    [[ "${test_name}" == --test ]] && continue
    run cargo test -p ironclaw_integration_tests \
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
    tests/e2e/scenarios/test_reborn_webui_v2_sso.py \
    -v --timeout=120
  run pytest \
    tests/e2e/scenarios/test_product_surface_coverage.py \
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
  if [[ -n "${frontend_corepack_home}" ]]; then
    return
  fi

  resolve_webui_frontend_dir
  package_manager="$(
    jq -r '.packageManager' \
      "${webui_frontend_dir}/package.json"
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
  export COREPACK_HOME="${frontend_corepack_home}"
  (
    cd "${webui_frontend_dir}"
    COREPACK_HOME="${frontend_corepack_home}" \
      corepack pnpm install --frozen-lockfile
  )
}

run_frontend_tests() {
  # Run from the package directory so Corepack honors its pinned packageManager
  # version and its isolated setup cache without registry access in the guard.
  resolve_webui_frontend_dir
  COREPACK_HOME="${frontend_corepack_home}" \
    run bash -c 'cd "$1" && exec corepack pnpm test' \
      _ "${webui_frontend_dir}"
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
    run cargo test -p ironclaw_integration_tests \
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
    prepare_rust_dependencies
    run_python_e2e
    ;;
  prepare-command)
    prepare_command_dependencies
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
    run cargo test -p ironclaw_integration_tests \
      --test reborn_qa_recorded_behavior -- --nocapture
    run "${repo_root}/scripts/reborn-e2e-rust.sh" all
    run_frontend_tests
    run cargo build -p ironclaw --bin ironclaw
    run_python_e2e
    ;;
  command)
    if [[ "$#" -eq 0 ]]; then
      echo "command stage requires a command" >&2
      exit 2
    fi
    case "${IRONCLAW_HERMETIC_SUITE_SKIP_PREPARE:-0}" in
      0) prepare_rust_dependencies ;;
      1) ;;
      *)
        echo "IRONCLAW_HERMETIC_SUITE_SKIP_PREPARE must be 0 or 1" >&2
        exit 2
        ;;
    esac
    run "$@"
    ;;
  *)
    echo "unknown hermetic deterministic-suite stage: ${stage}" >&2
    echo "expected: all, self-test, crates, root, groups, integration, qa, rust-e2e, frontend, python-e2e, prepare-command, command" >&2
    exit 2
    ;;
esac
