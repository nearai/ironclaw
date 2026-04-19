#!/usr/bin/env bash
set -euo pipefail

# Unified live-canary dispatcher.
#
# This branch carries the upstream live LLM lanes plus the auth-focused lanes
# added here. Lanes write artifacts under artifacts/live-canary/.

if [[ -n "${LANE:-}" ]]; then
  lane_value="${LANE}"
elif [[ $# -gt 0 && "$1" != --* ]]; then
  lane_value="$1"
  shift
else
  lane_value="public-smoke"
fi

if [[ -n "${SCENARIO:-}" ]]; then
  scenario_value="${SCENARIO}"
elif [[ $# -gt 0 && "$1" != --* ]]; then
  scenario_value="$1"
  shift
else
  scenario_value=""
fi

LANE="${lane_value}"
SCENARIO="${scenario_value}"
passthrough_args=("$@")

PROVIDER="${PROVIDER:-default}"
PLAYWRIGHT_INSTALL="${PLAYWRIGHT_INSTALL:-auto}"
COMMAND_TIMEOUT="${COMMAND_TIMEOUT:-90m}"
ARTIFACT_ROOT="${ARTIFACT_ROOT:-artifacts/live-canary}"
TIMESTAMP="${TIMESTAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
RUN_DIR="${RUN_DIR:-${ARTIFACT_ROOT}/${LANE}/${PROVIDER}/${TIMESTAMP}}"

mkdir -p "${RUN_DIR}"

LOG_FILE="${RUN_DIR}/test-output.log"
SUMMARY_FILE="${RUN_DIR}/summary.md"
ENV_FILE="${RUN_DIR}/env-summary.txt"
TRACE_STATUS_FILE="${RUN_DIR}/trace-fixture-status.txt"

: > "${LOG_FILE}"

started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
status=0

log() {
  echo "$@" | tee -a "${LOG_FILE}"
}

finish() {
  status=$?
  record_trace_status || true
  write_summary || true
  log "[live-canary] summary=${SUMMARY_FILE}"
  log "[live-canary] log=${LOG_FILE}"
  exit "${status}"
}

write_env_summary() {
  {
    echo "lane=${LANE}"
    echo "scenario=${SCENARIO:-<default>}"
    echo "provider=${PROVIDER}"
    echo "started_at=${started_at}"
    echo "sha=$(git rev-parse HEAD 2>/dev/null || true)"
    echo "branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
    echo "rustc=$(rustc --version 2>/dev/null || true)"
    echo "cargo=$(cargo --version 2>/dev/null || true)"
    echo "IRONCLAW_LIVE_TEST=${IRONCLAW_LIVE_TEST:-<unset>}"
    echo "LLM_BACKEND=${LLM_BACKEND:-<unset>}"
    echo "LLM_MODEL=${LLM_MODEL:-<unset>}"
    echo "ANTHROPIC_MODEL=${ANTHROPIC_MODEL:-<unset>}"
    echo "OPENAI_MODEL=${OPENAI_MODEL:-<unset>}"
    echo "GEMINI_MODEL=${GEMINI_MODEL:-<unset>}"
    echo "DATABASE_BACKEND=${DATABASE_BACKEND:-<unset>}"
    echo "LIBSQL_PATH=${LIBSQL_PATH:-<unset>}"
    echo "playwright_install=${PLAYWRIGHT_INSTALL}"
    echo "cases=${CASES:-<default>}"
    echo "skip_build=${SKIP_BUILD:-0}"
    echo "skip_python_bootstrap=${SKIP_PYTHON_BOOTSTRAP:-0}"
  } > "${ENV_FILE}"
}

run_with_timeout() {
  log "[live-canary] running: $*"
  if command -v timeout >/dev/null 2>&1; then
    timeout --signal=INT --kill-after=30s "${COMMAND_TIMEOUT}" "$@" 2>&1 | tee -a "${LOG_FILE}"
  else
    "$@" 2>&1 | tee -a "${LOG_FILE}"
  fi
  return "${PIPESTATUS[0]}"
}

run_cargo_test() {
  local test_target="$1"
  local filter="${2:-}"

  if [[ -n "${filter}" ]]; then
    run_with_timeout cargo test --features libsql --test "${test_target}" "${filter}" -- --ignored --nocapture --test-threads=1
  else
    run_with_timeout cargo test --features libsql --test "${test_target}" -- --ignored --nocapture --test-threads=1
  fi
}

select_rotating_persona() {
  if [[ -n "${SCENARIO}" && "${SCENARIO}" != "auto" ]]; then
    echo "${SCENARIO}"
    return
  fi

  case "$(date -u +%u)" in
    1) echo "ceo_full_workflow" ;;
    2) echo "content_creator_full_workflow" ;;
    3) echo "trader_full_workflow" ;;
    4) echo "developer_full_workflow" ;;
    5) echo "developer_full_workflow" ;;
    6) echo "ceo_full_workflow" ;;
    *) echo "content_creator_full_workflow" ;;
  esac
}

record_trace_status() {
  git status --short tests/fixtures/llm_traces/live > "${TRACE_STATUS_FILE}" || true
  if [[ -s "${TRACE_STATUS_FILE}" ]]; then
    log "Live trace fixture changes detected:"
    tee -a "${LOG_FILE}" < "${TRACE_STATUS_FILE}"
  else
    echo "No live trace fixture changes detected." > "${TRACE_STATUS_FILE}"
  fi
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
    echo "| Scenario | \`${SCENARIO:-<default>}\` |"
    echo "| Provider | \`${PROVIDER}\` |"
    echo "| Status | \`${status}\` |"
    echo "| Started | \`${started_at}\` |"
    echo "| Finished | \`${finished_at}\` |"
    echo "| Commit | \`$(git rev-parse HEAD 2>/dev/null || true)\` |"
    echo
    echo "Artifacts:"
    echo "- \`${LOG_FILE}\`"
    echo "- \`${ENV_FILE}\`"
    echo "- \`${TRACE_STATUS_FILE}\`"
  } > "${SUMMARY_FILE}"
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

run_python_lane() {
  local script="$1"
  shift
  build_common_args
  build_case_args
  local -a safe_case_args=()
  local -a safe_passthrough_args=()
  if [[ ${case_args+x} ]]; then
    safe_case_args=("${case_args[@]}")
  fi
  if [[ ${passthrough_args+x} ]]; then
    safe_passthrough_args=("${passthrough_args[@]}")
  fi
  run_with_timeout python3 "${script}" "${common_args[@]}" "$@" \
    "${safe_case_args[@]}" "${safe_passthrough_args[@]}"
}

main() {
  write_env_summary

  log "[live-canary] lane=${LANE} scenario=${SCENARIO:-<default>} provider=${PROVIDER}"
  log "[live-canary] artifacts=${RUN_DIR}"

  case "${LANE}" in
    deterministic-replay)
      IRONCLAW_LIVE_TEST=0 run_cargo_test e2e_live "${SCENARIO}"
      IRONCLAW_LIVE_TEST=0 run_cargo_test e2e_live_personas ""
      ;;
    public-smoke)
      export IRONCLAW_LIVE_TEST=1
      run_cargo_test e2e_live "${SCENARIO:-zizmor_scan}"
      run_cargo_test e2e_live_mission "mission_daily_news_digest_with_followup"
      ;;
    persona-rotating)
      export IRONCLAW_LIVE_TEST=1
      selected="$(select_rotating_persona)"
      SCENARIO="${selected}"
      run_cargo_test e2e_live_personas "${selected}"
      ;;
    private-oauth)
      export IRONCLAW_LIVE_TEST=1
      run_cargo_test e2e_live "drive_auth_gate_roundtrip"
      run_cargo_test e2e_live "drive_transparent_oauth_refresh"
      ;;
    provider-matrix)
      export IRONCLAW_LIVE_TEST=1
      run_cargo_test "${PROVIDER_TEST_TARGET:-e2e_live}" "${SCENARIO:-zizmor_scan}"
      ;;
    release-public-full)
      export IRONCLAW_LIVE_TEST=1
      run_cargo_test e2e_live "zizmor_scan"
      run_cargo_test e2e_live "zizmor_scan_v2"
      run_cargo_test e2e_live_mission ""
      run_cargo_test e2e_live_personas ""
      ;;
    upgrade-canary)
      run_with_timeout scripts/live-canary/upgrade-canary.sh
      ;;
    auth-smoke)
      run_python_lane scripts/auth_canary/run_canary.py --profile smoke --playwright-install "${PLAYWRIGHT_INSTALL}"
      ;;
    auth-full)
      run_python_lane scripts/auth_canary/run_canary.py --profile full --playwright-install "${PLAYWRIGHT_INSTALL}"
      ;;
    auth-channels)
      run_python_lane scripts/auth_canary/run_canary.py --profile channels --playwright-install "${PLAYWRIGHT_INSTALL}"
      ;;
    auth-live-seeded)
      run_python_lane scripts/auth_live_canary/run_live_canary.py --mode seeded --playwright-install "${PLAYWRIGHT_INSTALL}"
      ;;
    auth-browser-consent)
      run_python_lane scripts/auth_live_canary/run_live_canary.py --mode browser --playwright-install "${PLAYWRIGHT_INSTALL}"
      ;;
    *)
      echo "Unknown live canary lane: ${LANE}" >&2
      echo "Known lanes: deterministic-replay, public-smoke, persona-rotating, private-oauth, provider-matrix, release-public-full, upgrade-canary, auth-smoke, auth-full, auth-channels, auth-live-seeded, auth-browser-consent" >&2
      return 2
      ;;
  esac
}

trap finish EXIT
main
