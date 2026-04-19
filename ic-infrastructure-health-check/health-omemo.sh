#!/bin/bash
# Health Check: OMEMO Encryption Status
# Checks: OMEMO capability and device consistency across agents
# Output: JSON to stdout
# Exit codes: 0=healthy, 1=degraded, 2=critical

set -euo pipefail

# Config
AGENT_DIR="${AGENT_DIR:-$HOME/.ironclaw/agents}"
OMEMO_STORE="${OMEMO_STORE:-$HOME/.ironclaw/omemo}"

issues=()
devices=0
bundles=0
sessions=0
status="healthy"
exit_code=0

# Check OMEMO store exists
if [ -d "$OMEMO_STORE" ]; then
  # Count device keys
  devices=$(find "$OMEMO_STORE" -name "*.key" -type f 2>/dev/null | wc -l || echo 0)
  # Count bundles
  bundles=$(find "$OMEMO_STORE" -name "bundle*.json" -type f 2>/dev/null | wc -l || echo 0)
  # Count sessions
  sessions=$(find "$OMEMO_STORE" -name "session*.json" -type f 2>/dev/null | wc -l || echo 0)
else
  issues+=("OMEMO store directory not found")
  status="critical"
  exit_code=2
fi

# Check for recent OMEMO activity (sessions updated in last hour)
recent_sessions=0
if [ -d "$OMEMO_STORE" ]; then
  recent_sessions=$(find "$OMEMO_STORE" -name "session*.json" -mmin -60 -type f 2>/dev/null | wc -l || echo 0)
fi

if [ $devices -eq 0 ]; then
  issues+=("No OMEMO device keys found")
  status="degraded"
  exit_code=1
fi

# Output JSON
cat <<EOF
{
  "component": "omemo",
  "status": "$status",
  "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "metrics": {
    "devices": $devices,
    "bundles": $bundles,
    "sessions": $sessions,
    "recent_sessions": $recent_sessions
  },
  "issues": $(printf '%s\n' "${issues[@]}" | jq -R . | jq -s . 2>/dev/null || echo '[]')
}
EOF

exit $exit_code
