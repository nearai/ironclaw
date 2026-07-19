#!/usr/bin/env bash
set +x
set -euo pipefail

# The canonical live path is the Reborn WebUI QA runner. Legacy runtime lanes
# were retired with v1 and must not be reintroduced through this dispatcher.
lane="${LANE:-reborn-webui-v2-live-qa}"
if [[ "${lane}" != "reborn-webui-v2-live-qa" ]]; then
  echo "Unknown live canary lane: ${lane}" >&2
  echo "Known lanes: reborn-webui-v2-live-qa" >&2
  exit 2
fi

python_bin="${PYTHON_BIN:-python3}"
playwright_install="${PLAYWRIGHT_INSTALL:-auto}"
command_timeout="${COMMAND_TIMEOUT:-90m}"
artifact_root="${ARTIFACT_ROOT:-artifacts/live-canary}"
provider="${PROVIDER:-reborn-webui-v2}"
timestamp="${TIMESTAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
run_dir="${RUN_DIR:-${artifact_root}/${lane}/${provider}/${timestamp}}"
mkdir -p "${run_dir}"

args=(
  scripts/reborn_webui_v2_live_qa/run_live_qa.py
  --output-dir "${run_dir}"
  --playwright-install "${playwright_install}"
)

if [[ "${SKIP_BUILD:-0}" == "1" ]]; then
  args+=(--skip-build)
fi
if [[ "${SKIP_PYTHON_BOOTSTRAP:-0}" == "1" ]]; then
  args+=(--skip-python-bootstrap)
fi

if [[ -n "${CASES:-}" ]]; then
  IFS=',' read -ra raw_cases <<< "${CASES}"
  for case_name in "${raw_cases[@]}"; do
    trimmed="$(echo "${case_name}" | xargs)"
    [[ -n "${trimmed}" ]] || continue
    if [[ "${trimmed}" == "all" || "${trimmed}" == "ALL" || "${trimmed}" == "*" ]]; then
      args+=(--non-telegram-qa-cases)
    else
      args+=(--case "${trimmed}")
    fi
  done
fi

if (($# > 0)); then
  args+=("$@")
fi

echo "[live-canary] lane=${lane} provider=${provider} artifacts=${run_dir}"
if command -v timeout >/dev/null 2>&1; then
  timeout --signal=INT --kill-after=30s "${command_timeout}" "${python_bin}" "${args[@]}"
else
  "${python_bin}" "${args[@]}"
fi
