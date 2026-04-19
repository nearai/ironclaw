#!/bin/bash
# Health Check: Rate Limit Status
# Checks: Current rate limit counters, queue depth, throttled messages
# Output: JSON to stdout
# Exit codes: 0=healthy, 1=degraded, 2=critical

set -euo pipefail

# Config
RATELIMIT_DIR="${RATELIMIT_DIR:-$HOME/.ironclaw/ratelimit}"
THROTTLED_DEGRADED=10
THROTTLED_CRITICAL=50

issues=()
throttled=0
queued=0
status="healthy"
exit_code=0

# Check rate limit state files
if [ -d "$RATELIMIT_DIR" ]; then
  # Count throttled events in last hour
  throttled=$(find "$RATELIMIT_DIR" -name "throttled*.log" -mmin -60 -exec cat {} \; 2>/dev/null | wc -l || echo 0)
  # Count queued messages
  if [ -f "$RATELIMIT_DIR/queue.json" ]; then
    queued=$(jq 'length' "$RATELIMIT_DIR/queue.json" 2>/dev/null || echo 0)
  fi
fi

if [ $throttled -gt $THROTTLED_CRITICAL ]; then
  issues+=("Rate limit throttles critical: $throttled in last hour")
  status="critical"
  exit_code=2
elif [ $throttled -gt $THROTTLED_DEGRADED ]; then
  issues+=("Rate limit throttles elevated: $throttled in last hour")
  status="degraded"
  exit_code=1
fi

if [ $queued -gt 100 ]; then
  issues+=("Rate limit queue deep: $queued messages")
  [ "$status" = "healthy" ] && status="degraded" && exit_code=1
fi

# Output JSON
cat <<EOF
{
  "component": "ratelimit",
  "status": "$status",
  "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "metrics": {
    "throttled_last_hour": $throttled,
    "queue_depth": $queued
  },
  "issues": $(printf '%s\n' "${issues[@]}" | jq -R . | jq -s . 2>/dev/null || echo '[]')
}
EOF

exit $exit_code
