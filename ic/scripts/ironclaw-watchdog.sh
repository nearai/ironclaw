#!/usr/bin/env bash
set -euo pipefail

SERVICE="${IRONCLAW_WATCHDOG_SERVICE:-ironclaw.service}"
LOG_FILE="${IRONCLAW_WATCHDOG_LOG:-/var/log/ironclaw-watchdog.log}"
LOCK_FILE="${IRONCLAW_WATCHDOG_LOCK:-/run/ironclaw-watchdog.lock}"
POST_RESTART_SLEEP_SECONDS="${IRONCLAW_WATCHDOG_POST_RESTART_SLEEP_SECONDS:-5}"

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
  systemctl show "$SERVICE" --no-pager \
    -p ActiveState \
    -p SubState \
    -p Result \
    -p ExecMainCode \
    -p ExecMainStatus \
    -p NRestarts \
    -p MainPID \
    2>&1 | tr '\n' ' ' | sed 's/[[:space:]]*$//'
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

if systemctl is-active --quiet "$SERVICE"; then
  log "$SERVICE active; no action"
  exit 0
fi

before_state="$(state_summary || true)"
log "$SERVICE not active; attempting restart; before_state=\"$before_state\""

if ! systemctl restart "$SERVICE"; then
  after_state="$(state_summary || true)"
  log "$SERVICE restart command failed; after_state=\"$after_state\""
  exit 1
fi

sleep "$POST_RESTART_SLEEP_SECONDS"

if systemctl is-active --quiet "$SERVICE"; then
  after_state="$(state_summary || true)"
  log "$SERVICE restart succeeded; after_state=\"$after_state\""
  exit 0
fi

after_state="$(state_summary || true)"
log "$SERVICE restart attempted but service is still not active; after_state=\"$after_state\""
exit 1
