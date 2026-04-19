#!/bin/bash
# Health Check: ClickHouse Performance
# Checks: query response times, disk usage, memory usage
# Output: JSON to stdout
# Exit codes: 0=healthy, 1=degraded, 2=critical

set -euo pipefail

# Thresholds
DISK_DEGRADED=70
DISK_CRITICAL=85
MEMORY_DEGRADED=80
MEMORY_CRITICAL=95
QUERY_DEGRADED_MS=1000
QUERY_CRITICAL_MS=5000

# Initialize
query_time_ms=0
disk_pct=0
memory_pct=0
issues=()

# Check query response time
check_query_time() {
    local start=$(date +%s%N)
    
    # Simple query to test responsiveness
    if command -v clickhouse-client &>/dev/null; then
        clickhouse-client -q "SELECT 1" &>/dev/null 2>&1 || true
    fi
    
    local end=$(date +%s%N)
    local elapsed=$(( (end - start) / 1000000 ))  # Convert to ms
    
    echo $elapsed
}

# Check disk usage
check_disk() {
    local usage=0
    
    if command -v clickhouse-client &>/dev/null; then
        usage=$(clickhouse-client -q "SELECT round(100 - free_space / total_space * 100, 2) FROM system.disks WHERE name = 'default'" 2>/dev/null || echo 0)
    else
        # Fallback: check /var/lib/clickhouse if it exists
        if [ -d "/var/lib/clickhouse" ]; then
            usage=$(df /var/lib/clickhouse | tail -1 | awk '{print $5}' | tr -d '%')
        fi
    fi
    
    echo ${usage:-0}
}

# Check memory usage (system-level for ClickHouse process)
check_memory() {
    local mem_pct=0
    
    if command -v clickhouse-client &>/dev/null; then
        mem_pct=$(clickhouse-client -q "SELECT round(memory_usage / (SELECT total_memory FROM system.metrics WHERE metric = 'MemoryTotal') * 100, 2) FROM system.metrics WHERE metric = 'MemoryTracking'" 2>/dev/null || echo 0)
    else
        # Fallback: check system memory
        mem_pct=$(free | grep Mem | awk '{printf "%.0f", $3/$2 * 100}')
    fi
    
    echo ${mem_pct:-0}
}

# Run checks
query_time_ms=$(check_query_time)
disk_pct=$(check_disk)
memory_pct=$(check_memory)

# Add issues if thresholds exceeded
if [ $disk_pct -gt $DISK_CRITICAL ]; then
    issues+=("Disk usage critical: ${disk_pct}%")
elif [ $disk_pct -gt $DISK_DEGRADED ]; then
    issues+=("Disk usage elevated: ${disk_pct}%")
fi

if [ $memory_pct -gt $MEMORY_CRITICAL ]; then
    issues+=("Memory usage critical: ${memory_pct}%")
elif [ $memory_pct -gt $MEMORY_DEGRADED ]; then
    issues+=("Memory usage elevated: ${memory_pct}%")
fi

if [ $query_time_ms -gt $QUERY_CRITICAL_MS ]; then
    issues+=("Query time critical: ${query_time_ms}ms")
elif [ $query_time_ms -gt $QUERY_DEGRADED_MS ]; then
    issues+=("Query time elevated: ${query_time_ms}ms")
fi

# Determine status
status="healthy"
exit_code=0

if [ $disk_pct -gt $DISK_CRITICAL ] || [ $memory_pct -gt $MEMORY_CRITICAL ] || [ $query_time_ms -gt $QUERY_CRITICAL_MS ]; then
    status="critical"
    exit_code=2
elif [ $disk_pct -gt $DISK_DEGRADED ] || [ $memory_pct -gt $MEMORY_DEGRADED ] || [ $query_time_ms -gt $QUERY_DEGRADED_MS ]; then
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
    "component": "clickhouse",
    "status": "$status",
    "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
    "metrics": {
        "query_time_ms": $query_time_ms,
        "disk_pct": $disk_pct,
        "memory_pct": $memory_pct
    },
    "issues": $issues_json
}
EOF

exit $exit_code
