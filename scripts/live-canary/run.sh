#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${LANE:-}" ]]; then
  lane_value="${LANE}"
elif [[ $# -gt 0 && "$1" != --* ]]; then
  lane_value="$1"
  shift
else
  lane_value="auth-smoke"
fi

LANE="${lane_value}"
passthrough_args=("$@")
PROVIDER="${PROVIDER:-auth}"
PLAYWRIGHT_INSTALL="${PLAYWRIGHT_INSTALL:-auto}"
ARTIFACT_ROOT="${ARTIFACT_ROOT:-artifacts/live-canary}"
TIMESTAMP="${TIMESTAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
RUN_DIR="${RUN_DIR:-${ARTIFACT_ROOT}/${LANE}/${PROVIDER}/${TIMESTAMP}}"

mkdir -p "${RUN_DIR}"

LOG_FILE="${RUN_DIR}/test-output.log"
SUMMARY_FILE="${RUN_DIR}/summary.md"
ENV_FILE="${RUN_DIR}/env-summary.txt"

exec 3>&1 4>&2
exec >"${LOG_FILE}" 2>&1

started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
status=0

finish() {
  status=$?
  write_summary || true
  cat "${LOG_FILE}" >&3
  exec 3>&- 4>&-
  exit "${status}"
}

write_env_summary() {
  {
    echo "lane=${LANE}"
    echo "provider=${PROVIDER}"
    echo "started_at=${started_at}"
    echo "sha=$(git rev-parse HEAD 2>/dev/null || true)"
    echo "branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
    echo "playwright_install=${PLAYWRIGHT_INSTALL}"
    echo "cases=${CASES:-<default>}"
    echo "skip_build=${SKIP_BUILD:-0}"
    echo "skip_python_bootstrap=${SKIP_PYTHON_BOOTSTRAP:-0}"
  } >"${ENV_FILE}"
}

write_summary() {
  local finished_at
  finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  {
    echo "## Live Canary Summary"
    echo
    echo "| Field | Value |"
    echo "| --- | --- |"
    echo "| Lane | \`${LANE}\` |"
    echo "| Provider | \`${PROVIDER}\` |"
    echo "| Status | \`${status}\` |"
    echo "| Started | \`${started_at}\` |"
    echo "| Finished | \`${finished_at}\` |"
    echo "| Commit | \`$(git rev-parse HEAD 2>/dev/null || true)\` |"
    echo
    echo "Artifacts:"
    echo "- \`${LOG_FILE}\`"
    echo "- \`${ENV_FILE}\`"
    echo "- \`${RUN_DIR}\`"
  } >"${SUMMARY_FILE}"
}

build_common_args() {
  common_args=(--output-dir "${RUN_DIR}")
  if [[ "${SKIP_BUILD:-0}" == "1" ]]; then
    common_args+=(--skip-build)
  fi
  if [[ "${SKIP_PYTHON_BOOTSTRAP:-0}" == "1" ]]; then
    common_args+=(--skip-python-bootstrap)
  fi
}

build_case_args() {
  case_args=()
  if [[ -n "${CASES:-}" ]]; then
    IFS=',' read -ra raw_cases <<< "${CASES}"
    for case_name in "${raw_cases[@]}"; do
      trimmed="$(echo "$case_name" | xargs)"
      if [[ -n "${trimmed}" ]]; then
        case_args+=(--case "${trimmed}")
      fi
    done
  fi
}

run_lane() {
  build_common_args
  build_case_args

  case "${LANE}" in
    auth-smoke)
      python3 scripts/auth_canary/run_canary.py \
        --profile smoke \
        --playwright-install "${PLAYWRIGHT_INSTALL}" \
        "${common_args[@]}" \
        "${passthrough_args[@]}"
      ;;
    auth-full)
      python3 scripts/auth_canary/run_canary.py \
        --profile full \
        --playwright-install "${PLAYWRIGHT_INSTALL}" \
        "${common_args[@]}" \
        "${passthrough_args[@]}"
      ;;
    auth-channels)
      python3 scripts/auth_canary/run_canary.py \
        --profile channels \
        --playwright-install "${PLAYWRIGHT_INSTALL}" \
        "${common_args[@]}" \
        "${passthrough_args[@]}"
      ;;
    auth-live-seeded)
      python3 scripts/auth_live_canary/run_live_canary.py \
        --playwright-install "${PLAYWRIGHT_INSTALL}" \
        "${common_args[@]}" \
        "${case_args[@]}" \
        "${passthrough_args[@]}"
      ;;
    auth-browser-consent)
      python3 scripts/auth_browser_canary/run_browser_canary.py \
        --playwright-install "${PLAYWRIGHT_INSTALL}" \
        "${common_args[@]}" \
        "${case_args[@]}" \
        "${passthrough_args[@]}"
      ;;
    *)
      echo "Unknown live canary lane: ${LANE}" >&2
      echo "Known lanes: auth-smoke, auth-full, auth-channels, auth-live-seeded, auth-browser-consent" >&2
      return 2
      ;;
  esac
}

trap finish EXIT
write_env_summary
run_lane
