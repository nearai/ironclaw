#!/bin/bash
# Health Check: TensorZero Proxy Health
# Checks: API response times, error rate, queue depth, GPU utilization
# Output: JSON to stdout
# Exit codes: 0=healthy, 1=degraded, 2=critical

set -euo pipefail

# Thresholds
P95_DEGRADED_MS=5000
P95_CRITICAL_MS=15000
ERROR_DEGRADED_PCT=1
ERROR_CRITICAL_PCT=5
QUEUE_DEGRADED=5
QUEUE_CRITICAL=20

# Initialize
p50_ms=0
p95_ms=0
error_rate=0.0
queue_depth=0
gpu_util=0
issues=()

# Query ClickHouse for TensorZero metrics
query_clickhouse() {
    local query_result=""
    
    if command -v clickhouse-client &>/dev/null; then
        # Query for recent TensorZero metrics
        query_result=$(clickhouse-client --format JSONEachRow <<'EOF'
SELECT 
    quantile(0.5)(response_time_ms) as p50_ms,
    quantile(0.95)(response_time_ms) as p95_ms,
    avg(CASE WHEN status_code >= 400 THEN 1.0 ELSE 0.0 END) * 100 as error_rate,
    countIf(status_code = 0) as queue_depth,
    max(gpu_utilization) as gpu_util
FROM tensorzero_requests 
WHERE timestamp >= now() - INTERVAL 15 MINUTE
EOF
2>/dev/null) || query_result=""
    fi
    
    echo "$query_result"
}

# Get GPU utilization (fallback)
get_gpu_util() {
    local util=0
    
    # Try nvidia-smi first
    if command -v nvidia-smi &>/dev/null; then
        util=$(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader,nounits | head -1 | tr -d ' ' 2>/dev/null || echo 0)
    # Try AMD ROCm
    elif command -v rocm-smi &>/dev/null; then
        util=$(rocm-smi --showuse --csv | grep -E "^[0-9]" | head -1 | cut -d',' -f2 | tr -d '%' 2>/dev/null || echo 0)
    fi
    
    echo ${util:-0}
}

# Run checks
metrics=$(query_clickhouse)

if [ -n "$metrics" ]; then
    # Parse ClickHouse results
    p50_ms=$(echo "$metrics" | jq -r '.p50_ms // 0')
    p95_ms=$(echo "$metrics" | jq -r '.p95_ms // 0')
    error_rate=$(echo "$metrics" | jq -r '.error_rate // 0')
    queue_depth=$(echo "$metrics" | jq -r '.queue_depth // 0')
    gpu_util=$(echo "$metrics" | jq -r '.gpu_util // 0')
else
    # Fallback: use GPU utilization only
    gpu_util=$(get_gpu_util)
fi

# Add issues if thresholds exceeded
if [ $(echo "$p95_ms > $P95_CRITICAL_MS" | bc -l 2>/dev/null || echo 0) -eq 1 ]; then
    issues+=("P95 latency critical: ${p95_ms}ms")
elif [ $(echo "$p95_ms > $P95_DEGRADED_MS" | bc -l 2>/dev/null || echo 0) -eq 1 ]; then
    issues+=("P95 latency elevated: ${p95_ms}ms")
fi

if [ $(echo "$error_rate > $ERROR_CRITICAL_PCT" | bc -l 2>/dev/null || echo 0) -eq 1 ]; then
    issues+=("Error rate critical: ${error_rate}%")
elif [ $(echo "$error_rate > $ERROR_DEGRADED_PCT" | bc -l 2>/dev/null || echo 0) -eq 1 ]; then
    issues+=("Error rate elevated: ${error_rate}%")
fi

if [ $queue_depth -gt $QUEUE_CRITICAL ]; then
    issues+=("Queue depth critical: ${queue_depth}")
elif [ $queue_depth -gt $QUEUE_DEGRADED ]; then
    issues+=("Queue depth elevated: ${queue_depth}")
fi

# GPU utilization warning (if high but not necessarily critical)
if [ $gpu_util -gt 90 ]; then
    issues+=("GPU utilization high: ${gpu_util}%")
fi

# Determine status
status="healthy"
exit_code=0

if [ $(echo "$p95_ms > $P95_CRITICAL_MS" | bc -l 2>/dev/null || echo 0) -eq 1 ] || \
   [ $(echo "$error_rate > $ERROR_CRITICAL_PCT" | bc -l 2>/dev/null || echo 0) -eq 1 ] || \
   [ $queue_depth -gt $QUEUE_CRITICAL ]; then
    status="critical"
    exit_code=2
elif [ $(echo "$p95_ms > $P95_DEGRADED_MS" | bc -l 2>/dev/null || echo 0) -eq 1 ] || \
     [ $(echo "$error_rate > $ERROR_DEGRADED_PCT" | bc -l 2>/dev/null || echo 0) -eq 1 ] || \
     [ $queue_depth -gt $QUEUE_DEGRADED ]; then
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
    "component": "tensorzero",
    "status": "$status",
    "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
    "metrics": {
        "p50_ms": $p50_ms,
        "p95_ms": $p95_ms,
        "error_rate": $error_rate,
        "queue_depth": $queue_depth,
        "gpu_utilization": $gpu_util
    },
    "issues": $issues_json
}
EOF

exit $exit_code
