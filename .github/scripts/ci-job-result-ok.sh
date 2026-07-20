#!/usr/bin/env bash

job_result_ok() {
  local name="$1"
  local result="$2"
  local allow_skipped="${3:-false}"
  local cancelled_mode="${4:-none}"
  local event_name="${GITHUB_EVENT_NAME:-}"
  local ref_name="${GITHUB_REF:-}"
  local current_sha="${GITHUB_SHA:-}"
  local latest_sha=

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
      if [[ -z "${_CI_JOB_RESULT_OK_MAIN_SHA_CACHE:-}" ]]; then
        if ! timeout 10s git fetch --no-tags --depth=2 origin +refs/heads/main:refs/remotes/origin/main >/dev/null 2>&1; then
          echo "Could not refresh refs/remotes/origin/main; treating cancelled job as blocking for $name"
          return 1
        fi
        latest_sha="$(git rev-parse refs/remotes/origin/main)"
        if [[ -z "$latest_sha" ]]; then
          echo "Could not resolve refs/remotes/origin/main; treating cancelled job as blocking for $name"
          return 1
        fi
        _CI_JOB_RESULT_OK_MAIN_SHA_CACHE="${latest_sha}"
      else
        latest_sha="${_CI_JOB_RESULT_OK_MAIN_SHA_CACHE}"
      fi

      if [[ "${current_sha}" == "$latest_sha" ]]; then
        echo "$name was cancelled on a non-superseded push-to-main run; treating as non-blocking"
        return 0
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
