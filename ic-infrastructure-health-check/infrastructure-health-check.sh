#!/bin/bash
# Infrastructure Health Check - Main Entry Point
# Runs all component checks and aggregates results

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPORT_DIR="$HOME/.ironclaw/workspace/reports/health"
LOG_FILE="$REPORT_DIR/health.log"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

# Ensure report directory exists
mkdir -p "$REPORT_DIR"

# Initialize results
components=()
overall_status="healthy"
overall_exit_code=0
alerts=()

# Log function - logs to file and stderr (not stdout to avoid JSON contamination)
log() {
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $1" >> "$LOG_FILE"
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $1" >&2
}

# Run a health check
run_check() {
    local script=$1
    local component=$2
    
    log "Running $component health check..."
    
    local start=$(date +%s)
    local output
    local exit_code
    
    # Run the check with timeout
    output=$(timeout 30 "$SCRIPT_DIR/$script" 2>&1)
    exit_code=$?
    local end=$(date +%s)
    local duration=$((end - start))
    
    if [ $exit_code -eq 124 ]; then
        log "WARNING: $component check timed out after 30 seconds"
        echo "{\"component\": \"$component\", \"status\": \"unknown\", \"error\": \"timeout\"}"
        return 1
    fi
    
    if [ $exit_code -ne 0 ] && [ -z "$output" ]; then
        log "ERROR: $component check failed with exit code $exit_code (no output)"
        echo "{\"component\": \"$component\", \"status\": \"unknown\", \"error\": \"check_failed\"}"
        return $exit_code
    fi
    
    # Validate JSON output
    if ! echo "$output" | jq . >/dev/null 2>&1; then
        log "ERROR: $component check returned invalid JSON"
        echo "{\"component\": \"$component\", \"status\": \"unknown\", \"error\": \"invalid_json\"}"
        return 1
    fi
    
    echo "$output"
    log "$component check completed in ${duration}s (exit: $exit_code)"
    return $exit_code
}

# Run all health checks
log "=== Starting Infrastructure Health Check ==="

# Run checks in parallel for speed using temp files (stdout only for JSON)
run_check "health-gateway.sh" "gateway" > /tmp/check-gateway.tmp 2> /tmp/log-gateway.tmp &
run_check "health-xmpp.sh" "xmpp" > /tmp/check-xmpp.tmp 2> /tmp/log-xmpp.tmp run_check "health-xmpp.sh" "xmpp" > /tmp/check-xmpp.tmp 2> /tmp/log-xmpp.tmp &
run_check "health-omemo.sh" "omemo" > /tmp/check-omemo.tmp 2> /tmp/log-omemo.tmp run_check "health-xmpp.sh" "xmpp" > /tmp/check-xmpp.tmp 2> /tmp/log-xmpp.tmp &
run_check "health-ratelimit.sh" "ratelimit" > /tmp/check-ratelimit.tmp 2> /tmp/log-ratelimit.tmp run_check "health-xmpp.sh" "xmpp" > /tmp/check-xmpp.tmp 2> /tmp/log-xmpp.tmp &
run_check "health-clickhouse.sh" "clickhouse" > /tmp/check-clickhouse.tmp 2> /tmp/log-clickhouse.tmp &
run_check "health-tensorzero.sh" "tensorzero" > /tmp/check-tensorzero.tmp 2> /tmp/log-tensorzero.tmp &
run_check "health-models.sh" "models" > /tmp/check-models.tmp 2> /tmp/log-models.tmp run_check "health-models.sh" "models" > /tmp/check-models.tmp 2> /tmp/log-models.tmp &
run_check "health-systemd.sh" "systemd" > /tmp/check-systemd.tmp 2> /tmp/log-systemd.tmp run_check "health-models.sh" "models" > /tmp/check-models.tmp 2> /tmp/log-models.tmp &

# Wait for all checks to complete
wait

# Append logs to main log file
for comp in gateway xmpp omemo ratelimit clickhouse tensorzero models systemd; do
    [ -f "/tmp/log-${comp}.tmp" ] && cat "/tmp/log-${comp}.tmp" >> "$LOG_FILE" && rm -f "/tmp/log-${comp}.tmp"
done

# Read results from temp files
check_gateway=$(cat /tmp/check-gateway.tmp 2>/dev/null || echo)
check_irc=$(cat /tmp/check-irc.tmp 2>/dev/null || echo)
check_xmpp=$(cat /tmp/check-xmpp.tmp 2>/dev/null || echo)
check_omemo=$(cat /tmp/check-omemo.tmp 2>/dev/null || echo)
check_ratelimit=$(cat /tmp/check-ratelimit.tmp 2>/dev/null || echo)
check_clickhouse=$(cat /tmp/check-clickhouse.tmp 2>/dev/null || echo)
check_tensorzero=$(cat /tmp/check-tensorzero.tmp 2>/dev/null || echo)
check_models=$(cat /tmp/check-models.tmp 2>/dev/null || echo)

# Cleanup temp files
rm -f /tmp/check-*.tmp

# Collect results
components=(
    "$check_gateway"
     
    "$check_xmpp"
    "${check_omemo:-}"
    "${check_ratelimit:-}"
    "$check_clickhouse"
    "$check_tensorzero"
    "$check_models"
    "${check_systemd:-}"
)

# Aggregate results and determine overall status
for component_json in "${components[@]}"; do
    if [ -n "$component_json" ]; then
        status=$(echo "$component_json" | jq -r '.status // "unknown"')
        component_name=$(echo "$component_json" | jq -r '.component // "unknown"')
        
        # Update overall status
        case $status in
            "critical")
                overall_status="critical"
                overall_exit_code=2
                alerts+=("{\"component\": \"$component_name\", \"severity\": \"critical\", \"message\": \"$component_name is critical\"}")
                ;;
            "degraded")
                if [ "$overall_status" != "critical" ]; then
                    overall_status="degraded"
                    overall_exit_code=1
                fi
                alerts+=("{\"component\": \"$component_name\", \"severity\": \"warning\", \"message\": \"$component_name is degraded\"}")
                ;;
            "unknown")
                if [ "$overall_status" = "healthy" ]; then
                    overall_status="degraded"
                    overall_exit_code=1
                fi
                alerts+=("{\"component\": \"$component_name\", \"severity\": \"warning\", \"message\": \"$component_name status unknown\"}")
                ;;
        esac
    fi
done

# Build components array (filter out empty entries)
components_json=""
for comp in "$check_gateway"  "$check_xmpp"
    "${check_omemo:-}"
    "${check_ratelimit:-}" "$check_clickhouse" "$check_tensorzero" "$check_models"
    "${check_systemd:-}"; do
    if [ -n "$comp" ] && echo "$comp" | jq . >/dev/null 2>&1; then
        if [ -n "$components_json" ]; then
            components_json="$components_json,$comp"
        else
            components_json="$comp"
        fi
    fi
done

# Build alerts array JSON
alerts_json="[]"
if [ ${#alerts[@]} -gt 0 ]; then
    alerts_json=$(printf '%s\n' "${alerts[@]}" | jq -s .)
fi

# Generate final report
report_json=$(cat <<EOF
{
  "timestamp": "$TIMESTAMP",
  "overall_status": "$overall_status",
  "components": [$components_json],
  "alerts": $alerts_json
}
EOF
)

# Validate final JSON
if echo "$report_json" | jq . >/dev/null 2>&1; then
    # Save report
    report_file="$REPORT_DIR/$(date +%Y-%m-%dT%H:%M:%SZ).json"
    echo "$report_json" | jq . > "$report_file"
    
    # Generate human-readable summary
    summary_file="$REPORT_DIR/$(date +%Y-%m-%dT%H:%M:%SZ)-summary.md"
    echo "# Infrastructure Health Check - $(date '+%Y-%m-%d %H:%M UTC')" > "$summary_file"
    echo "" >> "$summary_file"
    echo "**Overall Status:** $overall_status" >> "$summary_file"
    echo "" >> "$summary_file"
    echo "## Component Status" >> "$summary_file"
    echo "" >> "$summary_file"
    
    for component_json in "${components[@]}"; do
        if [ -n "$component_json" ] && echo "$component_json" | jq . >/dev/null 2>&1; then
            comp=$(echo "$component_json" | jq -r '.component')
            status=$(echo "$component_json" | jq -r '.status')
            echo "- **$comp:** $status" >> "$summary_file"
        fi
    done
    
    echo "" >> "$summary_file"
    echo "## Alerts" >> "$summary_file"
    echo "" >> "$summary_file"
    
    if [ ${#alerts[@]} -gt 0 ]; then
        for alert in "${alerts[@]}"; do
            comp=$(echo "$alert" | jq -r '.component')
            severity=$(echo "$alert" | jq -r '.severity')
            message=$(echo "$alert" | jq -r '.message')
            echo "- **$severity:** $comp - $message" >> "$summary_file"
        done
    else
        echo "- No alerts" >> "$summary_file"
    fi
    
    log "Health check completed. Overall status: $overall_status"
    log "Report saved: $report_file"
    log "Summary saved: $summary_file"
    
    # Output final JSON
    echo "$report_json" | jq .
else
    log "ERROR: Failed to generate valid JSON report"
    echo "{\"error\": \"failed_to_generate_report\", \"timestamp\": \"$TIMESTAMP\"}"
    exit 2
fi

# Send notification if degraded or critical
if [ "$overall_status" != "healthy" ]; then
    send_notification "$overall_status" "$report_file"
fi

exit $overall_exit_code
