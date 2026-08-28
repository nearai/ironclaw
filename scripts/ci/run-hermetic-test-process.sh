#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" != "--" || "$#" -lt 2 ]]; then
  echo "usage: scripts/ci/run-hermetic-test-process.sh -- COMMAND [ARGS...]" >&2
  exit 2
fi
shift

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
network_runner="${repo_root}/scripts/ci/hermetic-network-runner.sh"
network_source="${repo_root}/scripts/ci/hermetic-network-guard.c"
sabotage="${IRONCLAW_HERMETIC_SABOTAGE:-}"

temp_parent="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
hermetic_root="$(mktemp -d "${temp_parent%/}/ironclaw-hermetic.XXXXXX")"
trap 'rm -rf "${hermetic_root}"' EXIT

mkdir -p \
  "${hermetic_root}/home" \
  "${hermetic_root}/base" \
  "${hermetic_root}/reborn-home" \
  "${hermetic_root}/workspace" \
  "${hermetic_root}/tmp" \
  "${hermetic_root}/xdg-cache" \
  "${hermetic_root}/xdg-config" \
  "${hermetic_root}/xdg-data"

guard_library=""
if [[ "${sabotage}" != "network" ]]; then
  case "$(uname -s)" in
    Linux)
      guard_library="${hermetic_root}/hermetic-network-guard.so"
      "${CC:-cc}" -shared -fPIC -O2 -Wall -Wextra -Werror \
        -o "${guard_library}" "${network_source}" -ldl
      ;;
    Darwin)
      guard_library="${hermetic_root}/hermetic-network-guard.dylib"
      "${CC:-cc}" -dynamiclib -fPIC -O2 -Wall -Wextra -Werror \
        -Wno-deprecated-declarations \
        -o "${guard_library}" "${network_source}"
      ;;
    *)
      echo "unsupported platform for hermetic network guard: $(uname -s)" >&2
      exit 2
      ;;
  esac
fi

env_args=()
if [[ "${sabotage}" != "env" ]]; then
  while IFS= read -r key; do
    case "${key}" in
      AR|CC|CXX|LD|RANLIB|STRIP|PKG_CONFIG|PKG_CONFIG_PATH|\
      OPENSSL_DIR|OPENSSL_INCLUDE_DIR|OPENSSL_LIB_DIR|LIBCLANG_PATH|\
      SDKROOT|MACOSX_DEPLOYMENT_TARGET|DEVELOPER_DIR|\
      LD_LIBRARY_PATH|DYLD_LIBRARY_PATH|LIBRARY_PATH|CPATH|\
      CI|GITHUB_ACTIONS|TERM|COLORTERM|NO_COLOR|FORCE_COLOR|\
      CARGO_INCREMENTAL|CARGO_PROFILE_DEV_DEBUG|CARGO_PROFILE_TEST_DEBUG|CARGO_TEST_ARGS|\
      RUSTFLAGS|RUST_MIN_STACK|COREPACK_HOME|PLAYWRIGHT_BROWSERS_PATH|\
      PROPTEST_CASES|REBORN_COV_COLLECT|REBORN_COV_LANES_JSON|\
      REBORN_COV_LANE_PARTITIONS|REBORN_COV_LANE_TEST_TIMEOUT|\
      REBORN_GROUP_TEST_TIMEOUT|REBORN_ROOT_TEST_PARTITION|\
      REBORN_ROOT_TEST_PARTITIONS|REBORN_ROOT_TEST_TIMEOUT|\
      IRONCLAW_E2E_EMULATE_SLACK_CHANNEL_BEARER|IRONCLAW_EMULATE_CLI|\
      IRONCLAW_GENERATED_SEQUENCE_DEPTH|IRONCLAW_JOURNEY_ORDER|\
      IRONCLAW_PROVIDER_OPERATION_SHARD)
        ;;
      *)
        env_args+=("-u" "${key}")
        ;;
    esac
  done < <(compgen -e)
fi

original_home="${HOME:-}"
original_cargo_home="${CARGO_HOME:-${original_home:+${original_home}/.cargo}}"
sanitized_cargo_home="${hermetic_root}/cargo-home"
mkdir -p "${sanitized_cargo_home}"
for cargo_cache in registry git; do
  if [[ -n "${original_cargo_home}" && -d "${original_cargo_home}/${cargo_cache}" ]]; then
    ln -s "${original_cargo_home}/${cargo_cache}" "${sanitized_cargo_home}/${cargo_cache}"
  fi
done

tool_path="${PATH:-/usr/bin:/bin}"
rust_sysroot=""
if command -v rustc >/dev/null 2>&1; then
  rust_sysroot="$(rustc --print sysroot)"
  if [[ -z "${rust_sysroot}" || ! -d "${rust_sysroot}/bin" ]]; then
    echo "unable to resolve the installed Rust toolchain for hermetic tests" >&2
    exit 1
  fi
  tool_path="${rust_sysroot}/bin:${tool_path}"
fi

env_args+=(
  "PATH=${tool_path}"
  "IRONCLAW_HERMETIC_ROOT=${hermetic_root}"
  "IRONCLAW_DISABLE_OS_KEYCHAIN=1"
  "LLM_MAX_RETRIES=0"
  "IRONCLAW_REBORN_MODEL_AVAILABILITY_RETRY_ATTEMPTS=1"
  "NO_PROXY=127.0.0.1,localhost,::1"
  "no_proxy=127.0.0.1,localhost,::1"
  "CARGO_NET_OFFLINE=true"
  "TZ=UTC"
  "LANG=C.UTF-8"
  "LC_ALL=C.UTF-8"
  "CARGO_HOME=${sanitized_cargo_home}"
  "CARGO_TARGET_DIR=${repo_root}/target"
)

if [[ "${sabotage}" != "temp" ]]; then
  env_args+=(
    "HOME=${hermetic_root}/home"
    "IRONCLAW_BASE_DIR=${hermetic_root}/base"
    "IRONCLAW_REBORN_HOME=${hermetic_root}/reborn-home"
    "IRONCLAW_TEST_WORKSPACE=${hermetic_root}/workspace"
    "TMPDIR=${hermetic_root}/tmp"
    "XDG_CACHE_HOME=${hermetic_root}/xdg-cache"
    "XDG_CONFIG_HOME=${hermetic_root}/xdg-config"
    "XDG_DATA_HOME=${hermetic_root}/xdg-data"
  )
else
  env_args+=("HOME=${original_home:-/}")
fi
if [[ "${sabotage}" != "python-seed" ]]; then
  env_args+=(
    "PYTHONHASHSEED=0"
  )
fi

if [[ "${sabotage}" != "network" ]]; then
  env_args+=("IRONCLAW_HERMETIC_NETWORK_GUARD_LIBRARY=${guard_library}")
  if [[ -n "${rust_sysroot}" ]]; then
    host_triple="$("${rust_sysroot}/bin/rustc" -vV | sed -n 's/^host: //p')"
    if [[ -z "${host_triple}" ]]; then
      echo "unable to determine Rust host triple for the hermetic test runner" >&2
      exit 1
    fi
    cargo_runner_key="CARGO_TARGET_$(tr '[:lower:]-' '[:upper:]_' <<<"${host_triple}")_RUNNER"
    env_args+=("${cargo_runner_key}=${network_runner}")
  fi
fi

command_prefix=()
if [[ "${sabotage}" != "network" ]]; then
  command_prefix=("${network_runner}")
  # Fetch dependencies and prepare compiler caches before this boundary.
  # A remote compiler wrapper must not become a hidden network exception.
  env_args+=("RUSTC_WRAPPER=")
fi

set +e
if [[ "${#command_prefix[@]}" -gt 0 ]]; then
  env "${env_args[@]}" "${command_prefix[@]}" "$@"
else
  env "${env_args[@]}" "$@"
fi
command_status=$?
set -e

exit "${command_status}"
