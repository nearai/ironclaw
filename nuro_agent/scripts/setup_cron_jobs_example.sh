#!/usr/bin/env bash
set -euo pipefail

if ! command -v openclaw >/dev/null 2>&1; then
  echo "openclaw CLI not found in PATH. Install OpenClaw first."
  exit 1
fi

APPLY=0
if [[ "${1:-}" == "--apply" ]]; then
  APPLY=1
fi

TZ_NAME="${TZ_NAME:-America/Los_Angeles}"
DELIVERY_CHANNEL="${DELIVERY_CHANNEL:-telegram}"
DELIVERY_TO="${DELIVERY_TO:-REPLACE_TARGET_ID}"

run_cmd() {
  echo "$*"
  if [[ "$APPLY" -eq 1 ]]; then
    "$@"
  fi
}

echo "Mode: $([[ "$APPLY" -eq 1 ]] && echo APPLY || echo DRY-RUN)"
echo "TZ: $TZ_NAME"
echo "Delivery: ${DELIVERY_CHANNEL}:${DELIVERY_TO}"

# 1) Hourly integrity + backup reminder (internal)
run_cmd openclaw cron add \
  --name nuro-hourly-integrity \
  --cron "0 * * * *" \
  --session isolated \
  --message "Run integrity pass: git status, private backup readiness, and summarize anomalies." \
  --no-deliver

# 2) Daily security and health sweep
run_cmd openclaw cron add \
  --name nuro-daily-security-health \
  --cron "15 6 * * *" \
  --tz "$TZ_NAME" \
  --session isolated \
  --message "Run status --deep, channels probe, and security audit. Summarize urgent fixes." \
  --announce \
  --channel "$DELIVERY_CHANNEL" \
  --to "$DELIVERY_TO"

# 3) Daily spend + usage review
run_cmd openclaw cron add \
  --name nuro-daily-usage-cost \
  --cron "45 6 * * *" \
  --tz "$TZ_NAME" \
  --session isolated \
  --message "Review token and API usage trends. Flag unusual spikes and expensive workflows." \
  --announce \
  --channel "$DELIVERY_CHANNEL" \
  --to "$DELIVERY_TO"

# 4) Daily markdown drift check against best practices
run_cmd openclaw cron add \
  --name nuro-daily-doc-drift \
  --cron "30 7 * * *" \
  --tz "$TZ_NAME" \
  --session isolated \
  --message "Cross-check AGENTS.md, SOUL.md, TOOLS.md, HEARTBEAT.md, and MEMORY.md against latest OpenClaw/provider best practices. Propose minimal diffs." \
  --announce \
  --channel "$DELIVERY_CHANNEL" \
  --to "$DELIVERY_TO"

# 5) Weekly long-term synthesis
run_cmd openclaw cron add \
  --name nuro-weekly-memory-synthesis \
  --cron "0 8 * * 1" \
  --tz "$TZ_NAME" \
  --session isolated \
  --message "Synthesize weekly learnings into durable memory updates and open actions." \
  --announce \
  --channel "$DELIVERY_CHANNEL" \
  --to "$DELIVERY_TO"

if [[ "$APPLY" -eq 0 ]]; then
  echo
  echo "Dry run complete. Re-run with --apply to create the jobs."
fi
