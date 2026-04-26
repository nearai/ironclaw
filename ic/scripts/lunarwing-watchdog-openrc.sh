#!/usr/bin/env bash
set -euo pipefail

WATCHDOG_CONFD="${LUNARWING_WATCHDOG_CONFD:-/etc/conf.d/lunarwing-watchdog}"

if [[ -r "$WATCHDOG_CONFD" ]]; then
  # shellcheck disable=SC1090
  . "$WATCHDOG_CONFD"
fi

SERVICE="${lunarwing_watchdog_service:-${LUNARWING_WATCHDOG_SERVICE:-${IRONCLAW_WATCHDOG_SERVICE:-lunarwing}}}"
SERVICE="${SERVICE%.service}"
LOG_FILE="${lunarwing_watchdog_log:-${LUNARWING_WATCHDOG_LOG:-${IRONCLAW_WATCHDOG_LOG:-/var/log/lunarwing-watchdog.log}}}"
LOCK_FILE="${lunarwing_watchdog_lock:-${LUNARWING_WATCHDOG_LOCK:-${IRONCLAW_WATCHDOG_LOCK:-/run/lunarwing-watchdog.lock}}}"
POST_RESTART_SLEEP_SECONDS="${lunarwing_watchdog_post_restart_sleep_seconds:-${LUNARWING_WATCHDOG_POST_RESTART_SLEEP_SECONDS:-${IRONCLAW_WATCHDOG_POST_RESTART_SLEEP_SECONDS:-5}}}"

timestamp() {
  date '+%Y-%m-%dT%H:%M:%S%z'
}

prepare_log_file() {
  local log_dir
  log_dir="$(dirname -- "$LOG_FILE")"

  if [[ -d "$log_dir" && -w "$log_dir" ]]; then
    touch "$LOG_FILE" 2>/dev/null || LOG_FILE=""
  else
    LOG_FILE=""
  fi
}

log() {
  local line
  line="$(timestamp) $*"

  if [[ -n "$LOG_FILE" ]]; then
    printf '%s\n' "$line" | tee -a "$LOG_FILE"
  else
    printf '%s\n' "$line"
  fi
}

state_summary() {
  rc-service "$SERVICE" status 2>&1 | tr '\n' ' ' | sed 's/[[:space:]]*$//'
}

prepare_log_file

if command -v flock >/dev/null 2>&1; then
  if exec 9>"$LOCK_FILE"; then
    if ! flock -n 9; then
      log "another watchdog run is already active; exiting"
      exit 0
    fi
  else
    log "lock file unavailable at $LOCK_FILE; continuing without lock"
  fi
fi

if rc-service "$SERVICE" status >/dev/null 2>&1; then
  log "$SERVICE active; no action"
  exit 0
fi

before_state="$(state_summary || true)"
log "$SERVICE not active; attempting start; before_state=\"$before_state\""

if ! rc-service "$SERVICE" start >/dev/null 2>&1; then
  after_state="$(state_summary || true)"
  log "$SERVICE start command failed; after_state=\"$after_state\""
  exit 1
fi

sleep "$POST_RESTART_SLEEP_SECONDS"

if rc-service "$SERVICE" status >/dev/null 2>&1; then
  after_state="$(state_summary || true)"
  log "$SERVICE start succeeded; after_state=\"$after_state\""
  exit 0
fi

after_state="$(state_summary || true)"
log "$SERVICE start attempted but service is still not active; after_state=\"$after_state\""
exit 1
