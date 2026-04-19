#!/bin/bash
# Health Check: XMPP Bridge Health
# Checks: connection status, message latency, error rate, active sessions
# Output: JSON to stdout
# Exit codes: 0=healthy, 1=degraded, 2=critical

set -euo pipefail

# Configuration
XMPP_SERVER="xmpp.sobe.world"
XMPP_PORT=5222
LATENCY_DEGRADED_MS=500
LATENCY_CRITICAL_MS=2000
ERROR_DEGRADED=1
ERROR_CRITICAL=5

# Initialize
connected=false
latency_ms=0
error_count=0
active_sessions=0
issues=()

# Check XMPP connectivity
check_connection() {
    if timeout 10 nc -z "$XMPP_SERVER" "$XMPP_PORT" 2>/dev/null; then
        echo "true"
    else
        echo "false"
        issues+=("XMPP server $XMPP_SERVER:$XMPP_PORT unreachable")
    fi
}

# Measure message latency (simplified - would use real XMPP client in production)
measure_latency() {
    local latency=0
    
    if [ "$connected" = "true" ]; then
        # Placeholder - would send test message and measure roundtrip
        # For now, simulate with a ping-like test
        local start=$(date +%s%N)
        timeout 5 bash -c "echo >/dev/tcp/$XMPP_SERVER/$XMPP_PORT" 2>/dev/null || true
        local end=$(date +%s%N)
        latency=$(( (end - start) / 1000000 ))
    fi
    
    echo $latency
}

# Check error rate (would query from logs/database in production)
check_errors() {
    local errors=0
    
    # Placeholder - would query recent errors from logs
    # For now, check if there are any recent error files
    if [ -d "$HOME/.ironclaw/workspace/logs" ]; then
        errors=$(find "$HOME/.ironclaw/workspace/logs" -name "*xmpp*error*" -mmin -15 -type f 2>/dev/null | wc -l)
    fi
    
    echo $errors
}

# Count active XMPP sessions
count_sessions() {
    local sessions=0
    
    # Placeholder - would query active sessions from bridge
    # For now, count recent activity files
    if [ -d "$HOME/.ironclaw/agents" ]; then
        sessions=$(find "$HOME/.ironclaw/agents" -name "*.jsonl" -mmin -5 -type f 2>/dev/null | wc -l)
    fi
    
    echo $sessions
}

# Run checks
connected=$(check_connection)
latency_ms=$(measure_latency)
error_count=$(check_errors)
active_sessions=$(count_sessions)

# Add latency issues
if [ $latency_ms -gt $LATENCY_CRITICAL_MS ]; then
    issues+=("XMPP latency critical: ${latency_ms}ms")
elif [ $latency_ms -gt $LATENCY_DEGRADED_MS ]; then
    issues+=("XMPP latency elevated: ${latency_ms}ms")
fi

# Add error issues
if [ $error_count -gt $ERROR_CRITICAL ]; then
    issues+=("XMPP error count critical: ${error_count}")
elif [ $error_count -gt $ERROR_DEGRADED ]; then
    issues+=("XMPP error count elevated: ${error_count}")
fi

# Determine status
status="healthy"
exit_code=0

if [ "$connected" = "false" ] || [ $latency_ms -gt $LATENCY_CRITICAL_MS ] || [ $error_count -gt $ERROR_CRITICAL ]; then
    status="critical"
    exit_code=2
elif [ $latency_ms -gt $LATENCY_DEGRADED_MS ] || [ $error_count -gt $ERROR_DEGRADED ]; then
    status="degraded"
    exit_code=1
fi

# Build issues JSON
issues_json="[]"
if [ ${#issues[@]} -gt 0 ]; then
    issues_json=$(printf '%s\n' "${issues[@]}" | jq -R . | jq -s .)
fi

# Output JSON
cat <<EOF
{
    "component": "xmpp",
    "status": "$status",
    "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
    "metrics": {
        "connected": $connected,
        "latency_ms": $latency_ms,
        "error_count": $error_count,
        "active_sessions": $active_sessions
    },
    "issues": $issues_json
}
EOF

exit $exit_code
