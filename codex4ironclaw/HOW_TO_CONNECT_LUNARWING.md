# IronClaw Agent Worker Connection Runbook

Use this when an IronClaw agent needs to connect directly to the Codex worker.

## Start The Worker

```bash
cd ~/.ironclaw/projects/codex4ironclaw
source .env 2>/dev/null || true

export CODEX_CONFIG_FILE=./config/chatgpt-pro.toml
export HOST_GID="$(id -g)"

docker compose up -d --build ironclaw-worker
```

## Check The Worker

```bash
curl http://127.0.0.1:8443/health
curl http://127.0.0.1:8443/ready
```

## Connection Details

For a normal IronClaw agent, connect directly to the worker WebSocket endpoint.

```text
URL: ws://<worker-host-ip>:9090/ws/agent
Auth header: Authorization: Bearer <AGENT_AUTH_TOKEN>
WebSocket subprotocol: ironclaw-agent-v1
Task context path: /workspace
```

If IronClaw is on the same machine, use:

```text
ws://127.0.0.1:9090/ws/agent
```

If IronClaw is on another machine or container on your LAN, use the worker host
IP, for example:

```text
ws://192.168.1.157:9090/ws/agent
```

## Agent Flow

1. Open the WebSocket with the bearer token and subprotocol.
2. Wait for the worker message `type: "ready"`.
3. Optionally send `type: "ping"` and expect `type: "pong"`.
4. Send `type: "task_request"`.

## Minimal Task Request

```json
{
  "id": "task-msg-001",
  "type": "task_request",
  "timestamp": "2026-04-19T00:00:00Z",
  "payload": {
    "task_id": "ironclaw-test-001",
    "prompt": "Say connected and create /workspace/ironclaw_agent_test.txt containing ironclaw-ok.",
    "context": {
      "path": "/workspace"
    },
    "timeout_ms": 300000
  }
}
```

## Quick Local Test

Run this from the repo:

```bash
source .env 2>/dev/null || true

python3 scripts/codex_agent_client.py \
  --ws-url ws://127.0.0.1:9090/ws/agent \
  --auth-token "$AGENT_AUTH_TOKEN" \
  --mode cli \
  --timeout-ms 300000 \
  --prompt "Say connected and create /workspace/ironclaw_agent_test.txt containing ironclaw-ok. Do not just describe it."
```

## OpenClaw/acpx ACP Bridge

For IronClaw/acpx/ACP specifically, use the stdio bridge instead:

```bash
source .env 2>/dev/null || true

python3 scripts/acp_bridge.py \
  --ws-url ws://127.0.0.1:9090/ws/agent \
  --auth-token "$AGENT_AUTH_TOKEN" \
  --project-dir /workspace
```
