#!/bin/bash
# Health Check: Model API Health
# Checks: provider availability (OpenRouter, OpenAI, Anthropic, local models)
# Output: JSON to stdout
# Exit codes: 0=healthy, 1=degraded, 2=critical

set -euo pipefail

# Thresholds
LATENCY_DEGRADED_MS=3000
LATENCY_CRITICAL_MS=10000

# Initialize
providers=()
overall_status="healthy"
overall_exit_code=0

# Check provider health
check_provider() {
    local provider=$1
    local endpoint=$2
    local api_key_env=$3
    
    local status="healthy"
    local latency_ms=0
    local last_error=""
    local exit_code=0
    
    local start=$(date +%s%N)
    
    case $provider in
        "openrouter")
            # Test OpenRouter health
            if [ -n "${!api_key_env:-}" ]; then
                response=$(curl -s -w "%{http_code}" -H "Authorization: Bearer ${!api_key_env}" \
                    -H "Content-Type: application/json" \
                    "$endpoint/health" 2>&1) || true
                
                if [[ "$response" == *"200"* ]] && [[ "$response" == *"ok"* ]]; then
                    status="healthy"
                else
                    status="critical"
                    last_error="HTTP error or unhealthy response"
                    exit_code=2
                fi
            else
                status="unknown"
                last_error="API key not set"
                exit_code=1
            fi
            ;;
        "openai")
            # Test OpenAI health
            if [ -n "${!api_key_env:-}" ]; then
                response=$(curl -s -w "%{http_code}" -H "Authorization: Bearer ${!api_key_env}" \
                    "$endpoint/models" 2>&1) || true
                
                if [[ "$response" == *"200"* ]]; then
                    status="healthy"
                else
                    status="critical"
                    last_error="HTTP error"
                    exit_code=2
                fi
            else
                status="unknown"
                last_error="API key not set"
                exit_code=1
            fi
            ;;
        "anthropic")
            # Test Anthropic health
            if [ -n "${!api_key_env:-}" ]; then
                response=$(curl -s -w "%{http_code}" -H "x-api-key: ${!api_key_env}" \
                    -H "anthropic-version: 2023-06-01" \
                    "$endpoint" 2>&1) || true
                
                if [[ "$response" == *"200"* ]]; then
                    status="healthy"
                else
                    status="critical"
                    last_error="HTTP error"
                    exit_code=2
                fi
            else
                status="unknown"
                last_error="API key not set"
                exit_code=1
            fi
            ;;
        "local")
            # Test local model endpoints (vLLM, etc.)
            response=$(curl -s -w "%{http_code}" "$endpoint" 2>&1) || true
            
            if [[ "$response" == *"200"* ]]; then
                status="healthy"
            else
                status="critical"
                last_error="Local model endpoint unreachable"
                exit_code=2
            fi
            ;;
    esac
    
    local end=$(date +%s%N)
    latency_ms=$(( (end - start) / 1000000 ))
    
    # Check latency thresholds
    if [ $latency_ms -gt $LATENCY_CRITICAL_MS ]; then
        status="critical"
        last_error="Latency critical: ${latency_ms}ms"
        exit_code=2
    elif [ $latency_ms -gt $LATENCY_DEGRADED_MS ]; then
        if [ "$status" = "healthy" ]; then
            status="degraded"
            last_error="Latency elevated: ${latency_ms}ms"
            exit_code=1
        fi
    fi
    
    # Update overall status
    if [ $exit_code -gt $overall_exit_code ]; then
        overall_exit_code=$exit_code
        if [ $exit_code -eq 2 ]; then
            overall_status="critical"
        elif [ $exit_code -eq 1 ] && [ "$overall_status" != "critical" ]; then
            overall_status="degraded"
        fi
    fi
    
    # Output provider result
    cat <<PROVIDER_EOF
    {
        "name": "$provider",
        "status": "$status",
        "latency_ms": $latency_ms,
        "last_error": "$last_error"
    }
PROVIDER_EOF
}

# Check all providers
providers_json=$(
    check_provider "openrouter" "https://openrouter.ai/api/v1" "OPENROUTER_API_KEY"
    echo ","
    check_provider "openai" "https://api.openai.com/v1" "OPENAI_API_KEY"  
    echo ","
    check_provider "anthropic" "https://api.anthropic.com/v1" "ANTHROPIC_API_KEY"
    echo ","
    check_provider "local" "http://localhost:8000/v1" ""
)

# Output JSON
cat <<EOF
{
    "component": "models",
    "status": "$overall_status",
    "timestamp": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
    "providers": [
        $providers_json
    ]
}
EOF

exit $overall_exit_code
