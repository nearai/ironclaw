#!/usr/bin/env bash
set -euo pipefail

job_result_ok() {
  local name="$1"
  local result="$2"
  local allow_skipped="${3:-false}"
  local cancelled_mode="${4:-none}"
  local event_name="${GITHUB_EVENT_NAME:-}"
  local ref_name="${GITHUB_REF:-}"
  local current_sha="${GITHUB_SHA:-}"

  if [[ "$result" == "success" ]]; then
    return 0
  fi

  if [[ "$allow_skipped" == "true" && "$result" == "skipped" ]]; then
    return 0
  fi

  if [[ "$result" != "cancelled" ]]; then
    return 1
  fi

  if [[ "$event_name" != "push" || "$ref_name" != "refs/heads/main" ]]; then
    return 1
  fi

  case "$cancelled_mode" in
    allow)
      echo "$name was cancelled on push to main; treating as non-blocking"
      return 0
      ;;
    superseded_only)
      if git fetch --no-tags --depth=2 origin +refs/heads/main:refs/remotes/origin/main >/dev/null 2>&1; then
        local latest_sha
        latest_sha="$(git rev-parse refs/remotes/origin/main)"
        if [[ "${current_sha}" != "$latest_sha" ]]; then
          echo "$name was cancelled on a superseded push-to-main run; treating as non-blocking"
          return 0
        fi
      fi
      return 1
      ;;
    none)
      return 1
      ;;
    *)
      return 1
      ;;
  esac
}
