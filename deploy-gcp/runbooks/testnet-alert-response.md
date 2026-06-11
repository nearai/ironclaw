# t3claw-testnet Alerts Runbook

GCP Cloud Monitoring is configured with six alert policies on the `t3claw-testnet` VM. This guide explains what each alert means and how to respond.

**GCP Alerting console:**
`https://console.cloud.google.com/monitoring/alerting?project=gen-lang-client-0263867259`

---

## How Alerts Arrive

Alerts fire to Slack via two channels:

| Slack channel | Severity | Alerts |
|---|---|---|
| `#alerts-prod-critical` | CRITICAL | Uptime check down, CPU > 90%, Memory > 92%, Disk > 95% |
| `#alerts-prod` | WARNING | Disk > 80%, Network egress > 100 MiB/s |

A GCP alert notification in Slack includes the policy name, the condition that fired, a link to the incident, and a **View Incident** button that takes you to the GCP Monitoring console.

**Acknowledging / silencing:** Open the incident from the Slack link → click **Acknowledge** to suppress repeat notifications while you investigate. Click **Close** once the condition is resolved. GCP will auto-close incidents when the metric returns below threshold.

---

## SSH Access

All investigation below requires SSH access to the VM. Use IAP tunnel (no public IP):

```bash
gcloud compute ssh t3claw-testnet \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap
```

See `deploy-gcp/README.md` for full operational procedures (restart, env update, hard reset).

---

## Alert Runbooks

### 1. Uptime check down — `/api/health` CRITICAL

**What it means:** GCP's uptime check has been unable to get a `200 OK` from `https://t3claw-testnet.agent.prod.gc.terminal3.io/api/health` for at least 3 minutes. The service is unreachable from the public internet.

**Likely causes:**
- `t3claw` systemd service crashed or failed to start
- Postgres container is unhealthy (agent depends on it)
- Load balancer health check backend is unregistered (rare, usually after a hard reset)
- OOM kill of the agent process

**Investigate:**

```bash
# Is the systemd service running?
sudo systemctl status t3claw

# Are the containers up?
sudo docker compose -f /opt/t3claw/docker-compose.yml --profile app ps

# Check recent logs
sudo journalctl -u t3claw -n 100 --no-pager
sudo docker logs t3claw-t3claw-1 --tail 100
sudo docker logs t3claw-t3n-mcp-sidecar-1 --tail 50
sudo docker logs t3claw-postgres-1 --tail 50

# Manual health probe from inside the VM
curl -sf http://localhost:3000/api/health && echo OK || echo FAIL
```

**Remediate:**

```bash
# Restart the full stack
sudo systemctl restart t3claw

# If postgres is unhealthy, restart it explicitly first
sudo docker compose -f /opt/t3claw/docker-compose.yml restart postgres
sudo systemctl restart t3claw

# If the sidecar is down or crashed, restart it explicitly
sudo docker compose -f /opt/t3claw/docker-compose.yml restart t3n-mcp-sidecar
# Then verify it's up and the Unix socket is available
sudo docker logs t3claw-t3n-mcp-sidecar-1 --tail 50
ls -la /var/run/t3n-mcp/
```

If the service keeps crashing, check logs for a panic or startup error (missing env var, bad DB migration, sidecar socket not ready, etc.) before restarting in a loop.

---

### 2. CPU > 90% — CRITICAL

**What it means:** The VM's CPU utilization has been above 90% for 5 consecutive minutes. The agent is likely running a CPU-intensive task or is stuck in a loop.

**Likely causes:**
- Agent processing a long-running or runaway job
- Postgres doing a large query or vacuum
- Unexpected process on the host

**Investigate:**

```bash
# Top processes by CPU
top -bn1 | head -25

# Per-container CPU
sudo docker stats --no-stream

# Check for active agent jobs
sudo docker logs t3claw-t3claw-1 --tail 200 | grep -i "job\|task\|error"
```

**Remediate:**

If the agent is the culprit and is stuck (same job running for > 30 min with no progress in logs):

```bash
# Restarts the full stack: agent, sidecar, and postgres
sudo systemctl restart t3claw
```

If Postgres is the culprit, check for long-running queries:

```bash
sudo docker exec t3claw-postgres-1 psql -U t3claw -c \
  "SELECT pid, now() - query_start AS duration, query FROM pg_stat_activity WHERE state='active' ORDER BY duration DESC;"
```

---

### 3. Memory > 92% — CRITICAL

**What it means:** The VM's RAM usage has been above 92% for 5 consecutive minutes. The agent may be experiencing a memory leak or holding large context buffers.

**Likely causes:**
- Agent accumulating large LLM context across many jobs
- Postgres caching large datasets
- Memory leak after many hours of uptime

**Investigate:**

```bash
# Overall memory
free -h

# Per-container memory
sudo docker stats --no-stream

# Check for OOM kills in kernel log
sudo dmesg | grep -i "oom\|killed" | tail -20
```

**Remediate:**

```bash
# Restarts the full stack: agent, sidecar, and postgres (releases in-memory caches; DB state is preserved)
sudo systemctl restart t3claw
```

If OOM kills are happening frequently, the VM may need to be resized. Note the memory level when it fires and report in Slack with the timestamp.

---

### 4. Disk > 80% — WARNING

**What it means:** The root filesystem is more than 80% full. This is an early warning — the service is not yet impacted but will be if disk usage continues to grow.

The boot disk is **30 GB `pd-standard`** (`t3claw-testnet`, zone `asia-southeast1-a`). 80% = ~24 GB used.

**Likely causes:**
- Accumulated Docker image layers from repeated deploys
- Growing container log files
- Postgres data volume growth (normal over time)

**Investigate:**

```bash
# Overall disk usage
df -h /

# Find the largest directories
sudo du -sh /var/lib/docker/overlay2 /var/lib/docker/containers \
  /home/t3claw/.t3claw /var/log 2>/dev/null | sort -rh

# Check container log file sizes
sudo find /var/lib/docker/containers -name "*.log" -size +100M \
  -exec ls -lh {} \;

# Check postgres DB size
sudo docker exec t3claw-postgres-1 psql -U t3claw -c "\l+"
```

**Remediate:**

```bash
# Remove stopped containers and dangling images (safe — does NOT touch volumes)
sudo docker system prune -f

# Remove all unused images (safe if you are about to redeploy, which will re-pull :testnet)
sudo docker image prune -a -f

# Truncate oversized container logs (safe — logs are diagnostic only)
sudo find /var/lib/docker/containers -name "*.log" -size +500M \
  -exec truncate -s 0 {} \;
```

> **Never run `docker volume prune`** — the `pgdata` and `t3claw_data` volumes hold the PostgreSQL database and the agent's persistent workspace. Pruning them is permanent data loss.

---

### 5. Disk > 95% — CRITICAL

**What it means:** The root filesystem is critically full. At 100% the agent will crash (cannot write logs, temp files, or DB WAL). Act immediately.

The boot disk is **30 GB `pd-standard`**. 95% = ~28.5 GB used — less than 1.5 GB free.

Follow the same investigation steps as the 80% warning above, then apply all of the safe cleanup steps:

```bash
sudo docker system prune -f
sudo docker image prune -a -f
sudo find /var/lib/docker/containers -name "*.log" -size +100M \
  -exec truncate -s 0 {} \;
df -h /
```

If disk is still > 90% after cleanup, the VM's persistent disk may need to be resized. This requires a GCP console operation — open `Compute Engine → Disks`, select the `t3claw-testnet` boot disk, click **Edit**, and increase the size. Then SSH in and resize the filesystem:

```bash
sudo growpart /dev/sda 1
sudo resize2fs /dev/sda1
df -h /
```

---

### 6. Network egress > 100 MiB/s — WARNING

**What it means:** The VM has been sending more than 100 MiB/s outbound for 5 consecutive minutes. This is well above normal usage (LLM API calls, NEAR RPC calls, webhook delivery are all well under 1 MiB/s).

**Likely causes:**
- Agent executing a tool that streams a large file or response
- Misconfigured routine making rapid repeated HTTP calls
- Unexpected process (unlikely given the VM's restricted network config)

**Investigate:**

```bash
# Check active connections
ss -tnp | grep ESTABLISHED

# Check agent logs for outbound requests
sudo docker logs t3claw-t3claw-1 --tail 300 | grep -i "http\|request\|fetch\|upload"

# Install nethogs for per-process breakdown (if needed)
sudo apt-get install -y nethogs && sudo nethogs ens4
```

**Remediate:**

If a specific tool or job is the cause, identify it from the logs and restart the agent:

```bash
# Restarts the full stack: agent, sidecar, and postgres
sudo systemctl restart t3claw
```

If the cause is unclear and egress is still high, escalate — this could indicate unexpected data exfiltration or a misconfigured integration.
