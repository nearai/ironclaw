#!/bin/bash
# Send Gotify Notification for Health Check Results

set -euo pipefail

# Configuration
GOTIFY_URL="${GOTIFY_URL:-http://localhost:3000}"
GOTIFY_TOKEN="${GOTIFY_TOKEN:-}"
REPORT_DIR="$HOME/.ironclaw/workspace/reports/health"

# Arguments
status="${1:-unknown}"
report_file="${2:-}"

# Exit if no Gotify token configured
if [ -z "$GOTIFY_TOKEN" ]; then
    echo "GOTIFY_TOKEN not set, skipping notification"
    exit 0
fi

# Determine priority based on status
priority=3
title="Infrastructure Health Check"
message=""

case $status in
    "healthy")
        # Don't send notification for healthy status
        exit 0
        ;;
    "degraded")
        priority=6
        title="⚠️ Infrastructure Degraded"
        ;;
    "critical")
        priority=8
        title="🚨 Infrastructure Critical"
        ;;
    *)
        priority=5
        title="📊 Infrastructure Health Unknown"
        ;;
esac

# Build message from latest report
if [ -n "$report_file" ] && [ -f "$report_file" ]; then
    # Extract component statuses
    message=$(jq -r '
        "Overall: " + (.overall_status | ascii_upcase) + "\n\n" +
        "Components:\n" +
        (.components[] | "- " + .component + ": " + .status) +
        "\n\n" +
        (if .alerts | length > 0 then "Issues:\n" + (.alerts[] | "- " + .message) else "" end) +
        "\n\nFull report: " + $report_file
    ' "$report_file")
else
    message="Infrastructure health check completed with status: $status"
fi

# Send to Gotify
curl -s -X POST "$GOTIFY_URL/message" \
    -H "Content-Type: application/json" \
    -d "{
        \"title\": \"$title\",
        \"message\": \"$message\",
        \"priority\": $priority
    }" \
    -H "X-Gotify-Key: $GOTIFY_TOKEN" \
    >/dev/null 2>&1

echo "Notification sent: $title"
