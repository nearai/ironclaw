#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
runner="${repo_root}/scripts/ci/run-hermetic-test-process.sh"
sabotage="${IRONCLAW_HERMETIC_SELF_TEST_SABOTAGE:-}"
probe_dir="$(mktemp -d "${TMPDIR:-/tmp}/hermetic-network-probe.XXXXXX")"
trap 'rm -rf "${probe_dir}"' EXIT
network_probe="${probe_dir}/hermetic-network-probe"
"${CC:-cc}" -O2 -Wall -Wextra -Werror \
  -o "${network_probe}" "${repo_root}/scripts/ci/hermetic-network-probe.c"

if [[ ! -x "${runner}" ]]; then
  echo "hermetic test-process runner is missing or not executable: ${runner}" >&2
  exit 1
fi

run_probe() {
  env \
    ANTHROPIC_API_KEY="must-not-leak" \
    GITHUB_TOKEN="must-not-leak" \
    GOOGLE_APPLICATION_CREDENTIALS="/developer/credential.json" \
    PERPLEXITY_KEY="must-not-leak" \
    DEEPSEEK_APIKEY="must-not-leak" \
    HF_TOKEN_FILE="/developer/hf-token" \
    SSH_AUTH_SOCK="/developer/ssh-agent.sock" \
    GPG_AGENT_INFO="/developer/gpg-agent" \
    BASH_ENV="/developer/shell-startup" \
    CARGO_TARGET_DIR="/developer/target" \
    AMBIENT_MULTILINE=$'first line\nPATH=/developer/injected' \
    LLM_BACKEND="ambient-provider" \
    REBORN_TOOL_DISCLOSURE="Bridged" \
    COREPACK_HOME="${probe_dir}/corepack" \
    PLAYWRIGHT_BROWSERS_PATH="${probe_dir}/playwright-browsers" \
    IRONCLAW_E2E_EMULATE_SLACK_CHANNEL_BEARER="emulate-slack-channel-token" \
    IRONCLAW_HERMETIC_SABOTAGE="${sabotage}" \
    "${runner}" -- bash -c '
      set -euo pipefail
      for key in \
        ANTHROPIC_API_KEY \
        GITHUB_TOKEN \
        GOOGLE_APPLICATION_CREDENTIALS \
        PERPLEXITY_KEY \
        DEEPSEEK_APIKEY \
        HF_TOKEN_FILE \
        SSH_AUTH_SOCK \
        GPG_AGENT_INFO \
        BASH_ENV \
        AMBIENT_MULTILINE \
        LLM_BACKEND \
        REBORN_TOOL_DISCLOSURE
      do
        if [[ -n "${!key+x}" ]]; then
          echo "ambient variable leaked into hermetic process: ${key}" >&2
          exit 31
        fi
      done

      case "${CARGO_HOME}" in
        "${IRONCLAW_HERMETIC_ROOT}"/*) ;;
        *)
          echo "CARGO_HOME is outside the hermetic root: ${CARGO_HOME}" >&2
          exit 38
          ;;
      esac
      if [[ -n "${RUSTUP_HOME+x}" ]]; then
        echo "ambient RUSTUP_HOME leaked into the hermetic process" >&2
        exit 39
      fi
      if [[ "${CARGO_TARGET_DIR}" != */target || "${CARGO_TARGET_DIR}" == "/developer/target" ]]; then
        echo "ambient CARGO_TARGET_DIR leaked into the hermetic process" >&2
        exit 40
      fi

      case "${HOME}" in
        "${IRONCLAW_HERMETIC_ROOT}"/*) ;;
        *)
          echo "HOME is outside the hermetic root: ${HOME}" >&2
          exit 32
          ;;
      esac
      for path in \
        "${IRONCLAW_BASE_DIR}" \
        "${IRONCLAW_REBORN_HOME}" \
        "${IRONCLAW_TEST_WORKSPACE}" \
        "${TMPDIR}"
      do
        case "${path}" in
          "${IRONCLAW_HERMETIC_ROOT}"/*) ;;
          *)
            echo "mutable path is outside the hermetic root: ${path}" >&2
            exit 33
            ;;
        esac
      done

      [[ "${TZ}" == "UTC" ]]
      [[ "${LANG}" == "C.UTF-8" ]]
      [[ "${LC_ALL}" == "C.UTF-8" ]]
      if [[ "${COREPACK_HOME:-}" != */corepack ]]; then
        echo "explicit Corepack toolchain path was not preserved" >&2
        exit 36
      fi
      if [[ "${PLAYWRIGHT_BROWSERS_PATH:-}" != */playwright-browsers ]]; then
        echo "explicit Playwright browser toolchain path was not preserved" >&2
        exit 35
      fi
      if [[ "${IRONCLAW_E2E_EMULATE_SLACK_CHANNEL_BEARER:-}" != "emulate-slack-channel-token" ]]; then
        echo "explicit Emulate Slack fixture bearer was not preserved" >&2
        exit 37
      fi
      if [[ "${PYTHONHASHSEED:-}" != "0" ]]; then
        echo "deterministic Python hash seed is not injected" >&2
        exit 34
      fi

      printf "%s\n" "${IRONCLAW_HERMETIC_ROOT}"
    '
}

first_root="$(run_probe)"
second_root="$(run_probe)"
if [[ "${first_root}" == "${second_root}" ]]; then
  echo "hermetic invocations reused mutable state root: ${first_root}" >&2
  exit 1
fi

parallel_dir="${probe_dir}/parallel-roots"
mkdir -p "${parallel_dir}"
parallel_pids=()
for index in 1 2 3 4; do
  run_probe > "${parallel_dir}/${index}" &
  parallel_pids+=("$!")
done
for pid in "${parallel_pids[@]}"; do
  wait "${pid}"
done
parallel_root_count="$(LC_ALL=C sort -u "${parallel_dir}"/* | wc -l | tr -d '[:space:]')"
if [[ "${parallel_root_count}" != "4" ]]; then
  echo "parallel hermetic invocations did not receive four isolated roots" >&2
  LC_ALL=C sort "${parallel_dir}"/* >&2
  exit 1
fi

no_prepare_bin="${probe_dir}/no-prepare-bin"
mkdir -p "${no_prepare_bin}"
cat > "${no_prepare_bin}/cargo" <<'SH'
#!/usr/bin/env bash
echo "command stage unexpectedly repeated dependency preparation" >&2
exit 97
SH
chmod +x "${no_prepare_bin}/cargo"
PATH="${no_prepare_bin}:${PATH}" \
  IRONCLAW_HERMETIC_SUITE_SKIP_PREPARE=1 \
  "${repo_root}/scripts/ci/run-hermetic-deterministic-suite.sh" \
  command bash -c 'test -n "${IRONCLAW_HERMETIC_ROOT:-}"'

set +e
network_output="$(
  IRONCLAW_HERMETIC_SABOTAGE="${sabotage}" \
    "${runner}" -- "${network_probe}" 192.0.2.1 2>&1
)"
network_status=$?
set -e
if [[ "${network_status}" -eq 0 ]]; then
  echo "unexpected non-loopback network attempt was not reported" >&2
  exit 1
fi

# The guard applies to arbitrary launchers, not just known test executables.
set +e
shell_output="$(
  IRONCLAW_HERMETIC_SABOTAGE="${sabotage}" \
    "${runner}" -- bash -c '"$1" 192.0.2.1' _ "${network_probe}" 2>&1
)"
shell_status=$?
set -e
if [[ "${shell_status}" -eq 0 || "${shell_output}" != *"non-loopback network attempt"* ]]; then
  echo "shell-launched non-loopback network attempt was not reported" >&2
  printf '%s\n' "${shell_output}" >&2
  exit 1
fi
if [[ "${network_output}" != *"non-loopback network attempt"* ]]; then
  echo "network guard failed without its actionable diagnostic" >&2
  printf '%s\n' "${network_output}" >&2
  exit 1
fi

set +e
udp_output="$(
  IRONCLAW_HERMETIC_SABOTAGE="${sabotage}" \
    "${runner}" -- "${network_probe}" 192.0.2.1 udp 2>&1
)"
udp_status=$?
set -e
if [[ "${udp_status}" -eq 0 || "${udp_output}" != *"non-loopback network attempt"* ]]; then
  echo "unexpected non-loopback UDP attempt was not reported" >&2
  printf '%s\n' "${udp_output}" >&2
  exit 1
fi

# UDP connect() only selects a route and sends no packet, so browser and PAC
# source-address inspection remains usable. A subsequent connected send is
# still external I/O and must fail loudly.
if [[ "$(uname -s)" == "Linux" ]]; then
  "${runner}" -- "${network_probe}" 192.0.2.1 udp-connect-only
else
  # The macOS process sandbox may reject even a route-only association. That is
  # safe; unlike the interposer it cannot distinguish association from I/O.
  set +e
  "${runner}" -- "${network_probe}" 192.0.2.1 udp-connect-only >/dev/null 2>&1
  route_probe_status=$?
  set -e
  if [[ "${route_probe_status}" -ne 0 && "${route_probe_status}" -ne 90 ]]; then
    echo "unexpected macOS UDP route-probe result: ${route_probe_status}" >&2
    exit 1
  fi
fi
connected_udp_modes=(udp-connected udp-sendfile udp-write udp-writev)
if [[ "$(uname -s)" == "Linux" ]]; then
  connected_udp_modes+=(udp-sendmmsg)
fi
for connected_udp_mode in "${connected_udp_modes[@]}"; do
  set +e
  connected_udp_output="$(
    IRONCLAW_HERMETIC_SABOTAGE="${sabotage}" \
      "${runner}" -- "${network_probe}" 192.0.2.1 "${connected_udp_mode}" 2>&1
  )"
  connected_udp_status=$?
  set -e
  if [[ "${connected_udp_status}" -eq 0 || "${connected_udp_output}" != *"non-loopback network attempt"* ]]; then
    echo "unexpected ${connected_udp_mode} non-loopback UDP send was not reported" >&2
    printf '%s\n' "${connected_udp_output}" >&2
    exit 1
  fi
done

# Deliberate localhost fakes remain usable. A refused port is sufficient: the
# guard must allow the connect syscall to reach the kernel rather than report it.
"${runner}" -- "${network_probe}" 127.0.0.1
if [[ "$(uname -s)" == "Linux" ]]; then
  "${runner}" -- "${network_probe}" ::ffff:127.0.0.1
else
  # sandbox-exec may reject IPv4-mapped IPv6 before the interposer observes it.
  set +e
  "${runner}" -- "${network_probe}" ::ffff:127.0.0.1 >/dev/null 2>&1
  mapped_loopback_status=$?
  set -e
  if [[ "${mapped_loopback_status}" -ne 0 && "${mapped_loopback_status}" -ne 90 ]]; then
    echo "unexpected macOS IPv4-mapped loopback result: ${mapped_loopback_status}" >&2
    exit 1
  fi
fi

"${runner}" -- python3 - "${repo_root}/tests/e2e" <<'PY'
import sys

sys.path.insert(0, sys.argv[1])
from hermetic_process import forward_hermetic_process_env

source = {
    "IRONCLAW_HERMETIC_NETWORK_GUARD_LIBRARY": "/tmp/guard.so",
    "IRONCLAW_HERMETIC_NETWORK_VIOLATIONS": "/tmp/violations",
    "LD_PRELOAD": "/tmp/existing.so",
    "DYLD_INSERT_LIBRARIES": "/tmp/existing.dylib",
    "DYLD_FORCE_FLAT_NAMESPACE": "0",
    "ANTHROPIC_API_KEY": "must-not-forward",
}
child = {}
forward_hermetic_process_env(child, source)
assert child["IRONCLAW_HERMETIC_NETWORK_VIOLATIONS"] == "/tmp/violations"
assert "ANTHROPIC_API_KEY" not in child
if sys.platform == "darwin":
    assert child["DYLD_INSERT_LIBRARIES"] == "/tmp/guard.so:/tmp/existing.dylib"
    assert child["LD_PRELOAD"] == "/tmp/existing.so"
    assert child["DYLD_FORCE_FLAT_NAMESPACE"] == "1"
elif sys.platform.startswith("linux"):
    assert child["LD_PRELOAD"] == "/tmp/guard.so:/tmp/existing.so"
    assert child["DYLD_INSERT_LIBRARIES"] == "/tmp/existing.dylib"

try:
    forward_hermetic_process_env(
        {},
        {"IRONCLAW_HERMETIC_NETWORK_GUARD_LIBRARY": "  "},
    )
except ValueError as error:
    assert "must be non-empty" in str(error)
else:
    raise AssertionError("empty network guard library did not fail closed")
PY

set +e
child_output="$(
  "${runner}" -- python3 - "${repo_root}/tests/e2e" "${network_probe}" 2>&1 <<'PY'
import subprocess
import sys

sys.path.insert(0, sys.argv[1])
from hermetic_process import forward_hermetic_process_env

child = {}
forward_hermetic_process_env(child)
subprocess.run([sys.argv[2], "192.0.2.1"], env=child, check=True)
PY
)"
child_status=$?
set -e
if [[ "${child_status}" -eq 0 || "${child_output}" != *"non-loopback network attempt"* ]]; then
  echo "minimal-env E2E child lost the non-loopback network guard" >&2
  printf '%s\n' "${child_output}" >&2
  exit 1
fi

for workflow_contract in \
  ".github/workflows/reborn-tests.yml:scripts/ci/run-hermetic-deterministic-suite.sh groups" \
  ".github/workflows/reborn-tests.yml:scripts/ci/discover-reborn-package-crates.sh" \
  ".github/workflows/reborn-tests.yml:scripts/ci/run-hermetic-deterministic-suite.sh command" \
  ".github/workflows/reborn-tests.yml:scripts/ci/run-hermetic-deterministic-suite.sh prepare-command" \
  ".github/workflows/reborn-tests.yml:IRONCLAW_HERMETIC_SUITE_SKIP_PREPARE=1" \
  ".github/workflows/reborn-tests.yml:Install WebUI frontend dependencies for coverage" \
  ".github/workflows/reborn-tests.yml:Install WebUI frontend dependencies for QA replay" \
  ".github/workflows/reborn-tests.yml:ironclaw|ironclaw_reborn_composition|ironclaw_webui)" \
  ".github/workflows/reborn-e2e.yml:scripts/ci/run-hermetic-deterministic-suite.sh" \
  ".github/workflows/reborn-e2e.yml:IRONCLAW_HERMETIC_SUITE_SKIP_PREPARE" \
  ".github/workflows/code_style.yml:scripts/ci/test-hermetic-test-process.sh"
do
  workflow="${workflow_contract%%:*}"
  needle="${workflow_contract#*:}"
  if ! grep -Fq "${needle}" "${repo_root}/${workflow}"; then
    echo "CI/local hermetic-suite parity lost: ${workflow} lacks '${needle}'" >&2
    exit 1
  fi
done

evidence_contract_step="$(
  sed -n \
    '/- name: Validate product-surface evidence contracts/,/- id: product_surface_coverage/p' \
    "${repo_root}/.github/workflows/reborn-e2e.yml"
)"
if [[ "${evidence_contract_step}" != *"run-hermetic-deterministic-suite.sh"* ]]; then
  echo "product-surface evidence contracts bypass the hermetic process boundary" >&2
  exit 1
fi

corepack_path_count="$(
  grep -Fc \
    'COREPACK_HOME=${RUNNER_TEMP}/ironclaw-corepack' \
    "${repo_root}/.github/workflows/reborn-tests.yml"
)"
if [[ "${corepack_path_count}" != "3" ]]; then
  echo "expected three guarded Rust lanes to pin COREPACK_HOME, found ${corepack_path_count}" >&2
  exit 1
fi

postgres_pull_count="$(
  grep -Fc \
    'run: docker pull postgres:16-alpine' \
    "${repo_root}/.github/workflows/reborn-tests.yml"
)"
if [[ "${postgres_pull_count}" != "2" ]]; then
  echo "expected two guarded Rust lanes to pre-pull Postgres, found ${postgres_pull_count}" >&2
  exit 1
fi

for stage_call in \
  "prepare_command_dependencies" \
  "prepare_postgres_test_image" \
  "discover-reborn-package-crates.sh" \
  "run_crate_tests" \
  "run_root_partitions" \
  "run_integration_tier" \
  "prepare_frontend_dependencies" \
  "run-reborn-group-tests.sh" \
  "check-reborn-qa-fixtures.sh" \
  "reborn-e2e-rust.sh" \
  "test_product_surface_coverage.py" \
  "test_reborn_webui_v2_sso.py" \
  "run_python_e2e"
do
  if ! grep -Fq "${stage_call}" "${repo_root}/scripts/ci/run-hermetic-deterministic-suite.sh"; then
    echo "canonical complete suite lost required stage: ${stage_call}" >&2
    exit 1
  fi
done

echo "hermetic test-process self-test: OK"
