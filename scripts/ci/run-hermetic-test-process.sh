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
  while IFS='=' read -r key _; do
    case "${key}" in
      IRONCLAW_E2E_EMULATE_SLACK_CHANNEL_BEARER|IRONCLAW_EMULATE_CLI|\
      IRONCLAW_GENERATED_SEQUENCE_DEPTH|IRONCLAW_JOURNEY_ORDER)
        ;;
      IRONCLAW_*)
        env_args+=("-u" "${key}")
        ;;
      *_API_KEY|*_TOKEN|*_SECRET|*_CREDENTIALS|*_PASSWORD|*_PRIVATE_KEY)
        env_args+=("-u" "${key}")
        ;;
      ANTHROPIC_*|OPENAI_*|NEARAI_*|GOOGLE_*|GITHUB_TOKEN|GH_TOKEN|SLACK_*|TELEGRAM_*|NOTION_*|EXA_*|BRAVE_*|AWS_*|AZURE_*|COHERE_*|MISTRAL_*|GROQ_*|\
      LLM_*|OLLAMA_*|REBORN_TOOL_DISCLOSURE|DATABASE_URL|LIBSQL_PATH|SECRETS_MASTER_KEY|\
      HTTP_PROXY|HTTPS_PROXY|ALL_PROXY|NO_PROXY|http_proxy|https_proxy|all_proxy|no_proxy)
        env_args+=("-u" "${key}")
        ;;
    esac
  done <<<"$(env)"
fi

original_home="${HOME:-}"
if [[ -n "${CARGO_HOME:-}" ]]; then
  cargo_home="${CARGO_HOME}"
elif [[ -n "${original_home}" ]]; then
  cargo_home="${original_home}/.cargo"
else
  cargo_home=""
fi
if [[ -n "${RUSTUP_HOME:-}" ]]; then
  rustup_home="${RUSTUP_HOME}"
elif [[ -n "${original_home}" ]]; then
  rustup_home="${original_home}/.rustup"
else
  rustup_home=""
fi

env_args+=(
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
fi
if [[ -n "${cargo_home}" ]]; then
  env_args+=("CARGO_HOME=${cargo_home}")
fi
if [[ -n "${rustup_home}" ]]; then
  env_args+=("RUSTUP_HOME=${rustup_home}")
fi

if [[ "${sabotage}" != "python-seed" ]]; then
  env_args+=(
    "PYTHONHASHSEED=0"
  )
fi

if [[ "${sabotage}" != "network" ]]; then
  host_triple="$(rustc -vV | sed -n 's/^host: //p')"
  if [[ -z "${host_triple}" ]]; then
    echo "unable to determine Rust host triple for the hermetic test runner" >&2
    exit 1
  fi
  cargo_runner_key="CARGO_TARGET_$(tr '[:lower:]-' '[:upper:]_' <<<"${host_triple}")_RUNNER"
  env_args+=(
    "IRONCLAW_HERMETIC_NETWORK_GUARD_LIBRARY=${guard_library}"
    "${cargo_runner_key}=${network_runner}"
  )
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
