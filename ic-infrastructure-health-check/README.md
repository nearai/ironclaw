# Infrastructure Health Check

Consolidated health check system for monitoring OpenClaw/Ironclaw infrastructure health across multiple services.

## Overview

Automated, repeatable diagnostics for early detection of service degradation before users notice.

## Components Monitored

| Component | Script | Checks |
|-----------|--------|--------|
| **Gateway** | `health-gateway.sh` | Active sessions, stale locks, orphan transcripts, oversized sessions |
| **IRC Bridge** | `health-irc.sh` | Connection status, nick availability, message success rate |
| **XMPP Bridge** | `health-xmpp.sh` | Connectivity, message latency, error count, active sessions |
| **ClickHouse** | `health-clickhouse.sh` | Query response times, disk usage, memory usage |
| **TensorZero** | `health-tensorzero.sh` | P50/P95 latency, error rate, queue depth, GPU utilization |
| **Model APIs** | `health-models.sh` | OpenRouter, OpenAI, Anthropic, local model availability |

## Quick Start

### Run Full Health Check

```bash
./infrastructure-health-check.sh
```

### Run Individual Check

```bash
./health-gateway.sh
./health-xmpp.sh
./health-irc.sh
# etc...
```

## Output

### JSON Report

Each check outputs structured JSON:

```json
{
  "component": "gateway",
  "status": "degraded",
  "timestamp": "2026-04-17T02:58:46Z",
  "metrics": {
    "active_sessions": 4,
    "stale_locks": 0,
    "orphan_transcripts": 0,
    "oversized_sessions": 56
  },
  "issues": []
}
```

### Status Levels

- 🟢 **healthy** - All metrics within normal thresholds
- 🟡 **degraded** - One or more metrics elevated but service functional
- 🔴 **critical** - Service impaired or failing

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Healthy |
| 1 | Degraded |
| 2 | Critical |

## Configuration

### Environment Variables

```bash
# Gotify notifications (optional)
export GOTIFY_URL="http://localhost:3000"
export GOTIFY_TOKEN="your-token-here"

# API keys for model checks (optional)
export OPENROUTER_API_KEY="..."
export OPENAI_API_KEY="..."
export ANTHROPIC_API_KEY="..."
```

### Thresholds

Edit individual scripts to customize thresholds:

```bash
# health-gateway.sh
STALE_LOCK_MINUTES=10
ORPHAN_THRESHOLD_DEGRADED=10
ORPHAN_THRESHOLD_CRITICAL=50

# health-tensorzero.sh
P95_DEGRADED_MS=5000
P95_CRITICAL_MS=15000
```

## Scheduling

### Cron Job

```bash
# Run every 30 minutes
0,30 * * * * /home/openjaw/.openclaw/workspace/infrastructure-health-check/infrastructure-health-check.sh
```

### OpenClaw Heartbeat

Add to `HEARTBEAT.md`:

```markdown
### Infrastructure Health Check
- **Frequency**: Every 30 minutes
- **Script**: `/home/openjaw/.openclaw/workspace/infrastructure-health-check/infrastructure-health-check.sh`
- **Alert on**: degraded or critical status
```

## Reports

Reports are saved to:

```
reports/
  health/
    2026-04-17T02:58:46Z.json          # Full check results
    2026-04-17T02:58:46Z-summary.md    # Human-readable summary
    health.log                          # Execution log
```

## Notifications

Gotify notifications are sent automatically when status is **degraded** or **critical**:

- **Degraded**: Priority 6 (warning)
- **Critical**: Priority 8 (urgent)

## Testing

```bash
# Run single check
./health-gateway.sh

# Run full check (no notifications)
./infrastructure-health-check.sh

# Check exit code
./health-gateway.sh; echo "Exit code: $?"
```

## Dependencies

- `bash` >= 4.0
- `jq` - JSON processing
- `curl` - HTTP requests
- `nc` (netcat) - TCP connectivity tests
- `clickhouse-client` - ClickHouse queries (optional)
- `nvidia-smi` or `rocm-smi` - GPU monitoring (optional)

## Design

Based on design document by Baud (2026-04-16).

## License

Part of OpenClaw workspace infrastructure.

---

**Status**: Phase 1 Complete ✅
**Author**: Volta (with Baud's design)
**Created**: 2026-04-17
