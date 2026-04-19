# IronClaw Codex Worker

Small containerized Codex worker with two runtime modes:

- `cli`: run a one-shot Codex task inside the container
- `websocket`: keep the worker running in either outbound `client` mode or inbound `server` mode

## Repository Layout

```text
.
├── Dockerfile
├── docker-compose.yml
├── entrypoint.sh
├── health_server.py
├── agent_comm_protocol.json
├── example-codex.toml
├── config/
│   └── config.toml
└── scripts/
    ├── codex_agent_server.py
    ├── acp_bridge.py
    ├── codex_agent_client.py
    ├── internal_ironclaw_agent_example.py
    ├── external_ironclaw_agent_example.py
    ├── mock_ironclaw_agent.py
    └── mock_agent_hub.py
```

## Build The Image

```bash
docker build -t ironclaw-codex-worker:latest .
```

## Config

The container expects its active Codex config at `/app/config/codex.toml`.

Compose mounts `${CODEX_CONFIG_FILE:-./config/config.toml}` to that path. The
default `config/config.toml` uses the repo's TensorZero provider. The
`config/chatgpt-pro.toml` sample is for signing in with a ChatGPT account inside
the container.

```bash
CODEX_CONFIG_FILE=./config/chatgpt-pro.toml docker compose up -d --build ironclaw-worker
```

The Compose stack also persists `/home/codex/.codex` in the `codex-home` named
volume so Codex CLI login state survives container recreation.

To authenticate Codex CLI with a ChatGPT account:

```bash
CODEX_CONFIG_FILE=./config/chatgpt-pro.toml \
docker compose --profile cli run --rm codex-cli login --device-auth
```

Open the displayed device-pair URL, sign in with your ChatGPT account, then
restart the worker with the same `CODEX_CONFIG_FILE`.

For host bind-mounted workspaces, let the container's `codex` user write through
your host group:

```bash
export HOST_GID="$(id -g)"
chgrp -R "$HOST_GID" workspace
chmod -R g+rwX workspace
find workspace -type d -exec chmod g+s {} +
```

Compose adds `HOST_GID` as a supplemental container group and uses
`FILE_UMASK=0002` so files created by Codex remain editable from the host.

If `WS_URL` is not set, WebSocket mode falls back to the `connection.uri` value from `agent_comm_protocol.json`.

## Compose Quickstart

1. Copy `.env.example` to `.env` and set at least `AGENT_AUTH_TOKEN`.
2. Set `CODEX_CONFIG_FILE` if you want a config other than `config/config.toml`.
3. Start the worker:

```bash
docker compose up -d --build ironclaw-worker
```

4. Verify host-side access:

```bash
python3 -m pip install --break-system-packages websockets
AGENT_AUTH_TOKEN="$(grep '^AGENT_AUTH_TOKEN=' .env | cut -d= -f2-)" \
python3 scripts/check_worker_connectivity.py \
  --host 127.0.0.1 \
  --health-port 8443 \
  --ws-port 9090
```

The worker will then be reachable at:

- `http://127.0.0.1:8443/health`
- `http://127.0.0.1:8443/ready`
- `ws://<host>:9090/ws/agent`

## Use Codex CLI With Compose

Run a one-shot Codex CLI task against the mounted `./workspace` directory:

```bash
docker compose --profile cli run --rm codex-cli "fix the bug in src/app.py"
```

Useful variants:

- `docker compose --profile cli run --rm codex-cli --help`
- `docker compose --profile cli run --rm codex-cli "add tests for health_server.py"`

The CLI service uses the same image, config, and mounted workspace as the websocket worker.

## Run In WebSocket Client Mode

Use client mode when the worker should initiate the WebSocket connection to an external hub.

On Linux, `--add-host=host.docker.internal:host-gateway` lets the container reach a WebSocket server running on the host.

```bash
sudo docker run -d \
  --name codex-test \
  --add-host=host.docker.internal:host-gateway \
  -e OPENAI_API_KEY="$OPENAI_API_KEY" \
  -e AGENT_AUTH_TOKEN="test-token" \
  -e CODEX_MODE=websocket \
  -e WS_ROLE=client \
  -e WS_URL="ws://host.docker.internal:9000/codex" \
  -v "$PWD/config/config.toml:/app/config/codex.toml:ro" \
  -v "$PWD/workspace:/workspace" \
  -w /workspace \
  -p 8443:8443 \
  ironclaw-codex-worker:latest \
  --mode websocket
```

Notes:

- `WS_URL` overrides the default URI from `agent_comm_protocol.json`.
- `AGENT_AUTH_TOKEN` is sent as a bearer token when present.
- `WS_ROLE` defaults to `client` for backward compatibility.
- `CODEX_MODE=websocket` and `--mode websocket` are redundant; either is enough, and CLI args win if both are set.
- The worker exposes the health server on `http://localhost:8443`.

## Run In WebSocket Server Mode

Use server mode when an Ironclaw agent should connect directly to the worker.

```bash
sudo docker run -d \
  --name codex-test \
  -e OPENAI_API_KEY="$OPENAI_API_KEY" \
  -e AGENT_AUTH_TOKEN="test-token" \
  -e CODEX_MODE=websocket \
  -e WS_ROLE=server \
  -e WS_PORT=9090 \
  -e WS_PATH="/ws/agent" \
  -v "$PWD/config/config.toml:/app/config/codex.toml:ro" \
  -v "$PWD/workspace:/workspace" \
  -w /workspace \
  -p 8443:8443 \
  -p 9090:9090 \
  ironclaw-codex-worker:latest \
  --mode websocket
```

An Ironclaw agent should then connect to `ws://<worker-host>:9090/ws/agent`.

To start the same server mode through Compose:

```bash
docker compose up -d --build ironclaw-worker
```

Useful follow-up commands:

```bash
curl http://127.0.0.1:8443/health
curl http://127.0.0.1:8443/ready
sudo docker logs -f codex-test
sudo docker rm -f codex-test
```

If `localhost` fails but `127.0.0.1` works on your machine, you are hitting an address-family mismatch rather than a worker failure.

## Run The Mock Agent Hub

[`scripts/mock_agent_hub.py`](scripts/mock_agent_hub.py) is a small local test hub for client mode. It listens on `ws://0.0.0.0:9000/codex`, waits for the worker's `ready` message, and sends a demo `task_request`.

The worker image already installs the Python `websockets` package. If you run the mock hub on the host, install it there first:

```bash
python3 -m pip install --break-system-packages websockets
python3 scripts/mock_agent_hub.py
```

That host-side install is what makes the `WS_URL="ws://host.docker.internal:9000/codex"` example above work.

## Internal IronClaw Agent Example

[`scripts/internal_ironclaw_agent_example.py`](scripts/internal_ironclaw_agent_example.py) is the dedicated example for an IronClaw agent running inside the same Docker Compose network as the worker.

```bash
docker compose up -d --build ironclaw-worker
docker compose --profile internal-agent run --rm internal-ironclaw-agent
```

The Compose stack also includes optional helper services:

- `docker compose --profile smoke run --rm agent-smoke`
  Verifies `/health`, `/ready`, websocket connect, worker `ready`, and `pong` from inside the Compose network.
- `docker compose --profile internal-agent run --rm internal-ironclaw-agent`
  Connects to `ws://ironclaw-worker:9090/ws/agent` and sends a demo `task_request` using `TASK_PROMPT` from `.env`.

The internal agent example only proves the full task path if the worker's Codex provider in `config/config.toml` is reachable and authorized.

## External IronClaw Agent Example

[`scripts/external_ironclaw_agent_example.py`](scripts/external_ironclaw_agent_example.py) is the dedicated example for an IronClaw agent connecting from outside Docker.

```bash
python3 -m pip install --break-system-packages websockets
python3 scripts/external_ironclaw_agent_example.py \
  --ws-url ws://127.0.0.1:9090/ws/agent \
  --auth-token test-token \
  --prompt "Write a short Python function that returns the factorial of a number." \
  --exit-on-result
```

Defaults:

- URL: `ws://127.0.0.1:9090/ws/agent`
- Header: `Authorization: Bearer <AGENT_AUTH_TOKEN>`
- Subprotocol: `ironclaw-agent-v1`
- Task prompt: `Write a hello world Python script`
- Task path: `/workspace`

You can override any of these with CLI flags or env vars such as `WS_URL`,
`TASK_PROMPT`, `TASK_PATH`, `TASK_ID`, and `TASK_TIMEOUT_MS`.

For host-side prompt submission, use `scripts/external_ironclaw_agent_example.py`.
`scripts/codex_agent_client.py` is the worker's outbound hub client and defaults to
`ws://agent-hub:9000/codex` when `WS_URL` is unset.

## ACP Stdio Bridge For OpenClaw/acpx

[`scripts/acp_bridge.py`](scripts/acp_bridge.py) is a thin Agent Client Protocol
adapter. It speaks ACP JSON-RPC over stdio and translates prompt turns into this
worker's existing WebSocket `task_request` protocol.

Start the Codex worker first:

```bash
source .env 2>/dev/null || true
docker compose up -d --build ironclaw-worker
```

Then configure OpenClaw/acpx to launch the bridge as a stdio adapter:

```bash
python3 -m pip install --break-system-packages websockets
python3 /home/cmc/codex4ironclaw/scripts/acp_bridge.py \
  --ws-url ws://127.0.0.1:9090/ws/agent \
  --auth-token "$AGENT_AUTH_TOKEN" \
  --project-dir /workspace
```

Supported ACP surface:

- `initialize`
- `session/new`
- `session/prompt`
- `session/cancel`

The bridge emits `session/update` notifications with
`agent_message_chunk` content while the worker streams progress and final output.
It does not implement `session/load`, MCP server attachment, images, audio, or
client filesystem calls.

`--project-dir` is the worker-visible project directory sent as
`context.path`. Keep the default `/workspace` for the Docker setup in this repo.
Use `--use-session-cwd` only when the ACP client's `cwd` is also a valid path
inside the worker container.

Equivalent environment variables:

- `ACP_WORKER_WS_URL`
- `AGENT_AUTH_TOKEN`
- `ACP_PROJECT_DIR`
- `ACP_USE_SESSION_CWD`
- `TASK_TIMEOUT_MS`

## Host-Side Connectivity Check

[`scripts/check_worker_connectivity.py`](scripts/check_worker_connectivity.py) verifies the host-published health endpoints and the external agent websocket flow.

```bash
python3 -m pip install --break-system-packages websockets
AGENT_AUTH_TOKEN=test-token \
python3 scripts/check_worker_connectivity.py \
  --host 127.0.0.1 \
  --health-port 8443 \
  --ws-port 9090
```

It checks:

- `GET /health`
- `GET /ready`
- websocket connect to `/ws/agent`
- worker `ready`
- protocol `ping` / `pong`

If you prefer `wscat`, use the worker endpoint and the IronClaw envelope instead of a raw `{"action":"ping"}` payload:

```bash
printf '%s\n' '{"id":"manual-ping-001","type":"ping","timestamp":"2026-04-13T00:00:00Z","payload":{}}' \
  | npx wscat -c ws://127.0.0.1:9090/ws/agent \
      -H "Authorization: Bearer test-token" \
      -s ironclaw-agent-v1
```

Do not use `wscat -p 5`; that forces an obsolete websocket protocol version and will fail before the worker sees the connection.

## How External IronClaw Agents Connect

External agents should connect to:

- URL: `ws://<worker-host>:9090/ws/agent`
- Header: `Authorization: Bearer <AGENT_AUTH_TOKEN>`
- WebSocket subprotocol: `ironclaw-agent-v1`

Connection flow:

1. The agent opens the websocket.
2. The worker immediately sends a `ready` envelope.
3. The agent sends `task_request` envelopes.
4. The worker runs `codex exec --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check --cd /workspace <prompt>` by default.
5. The worker streams `task_progress` and ends with `task_result`.

Minimal `task_request` example:

```json
{
  "id": "task-msg-001",
  "type": "task_request",
  "timestamp": "2026-04-13T00:00:00Z",
  "payload": {
    "task_id": "task-001",
    "prompt": "Write a hello world Python script",
    "context": {
      "path": "/workspace"
    },
    "timeout_ms": 300000
  }
}
```

## How Internal IronClaw Agents Connect

Internal agents running inside the same Compose network should connect to:

- URL: `ws://ironclaw-worker:9090/ws/agent`
- Header: `Authorization: Bearer <AGENT_AUTH_TOKEN>`
- WebSocket subprotocol: `ironclaw-agent-v1`

The dedicated repo example for this is [`scripts/internal_ironclaw_agent_example.py`](scripts/internal_ironclaw_agent_example.py).

## Health Endpoints

The container always starts `health_server.py` on `HEALTH_PORT` (default `8443`).

| Endpoint | Status | Notes |
| --- | --- | --- |
| `GET /health` | `200` | Returns process health, uptime, mode, and version |
| `GET /ready` | `200` or `503` | Reflects the current WebSocket role state from the worker process |

## Important Environment Variables

| Variable | Default | Description |
| --- | --- | --- |
| `CODEX_CONFIG_FILE` | `./config/config.toml` | Host-side TOML file mounted as `/app/config/codex.toml` |
| `OPENAI_API_KEY` | none | API-key auth credential; leave empty for ChatGPT account sign-in |
| `CODEX_BYPASS_SANDBOX` | `true` | Disable Codex's nested sandbox; Docker is the worker sandbox |
| `CODEX_MODE` | `cli` | Runtime mode: `cli` or `websocket` |
| `AGENT_AUTH_TOKEN` | empty | Bearer token used for outbound hub auth or inbound agent validation |
| `WS_ROLE` | `client` | `client` makes the worker dial out, `server` makes it accept inbound agent connections |
| `WS_URL` | from `agent_comm_protocol.json` | WebSocket endpoint override |
| `WS_BIND_HOST` | `0.0.0.0` | Server bind host for inbound agent connections |
| `WS_PORT` | `9090` | Server listen port for inbound agent connections |
| `WS_PATH` | `/ws/agent` | Server path for inbound agent connections |
| `HEALTH_PORT` | `8443` | HTTP port for `/health` and `/ready` |
| `PROTOCOL_CONFIG` | `/app/config/agent_comm_protocol.json` | JSON file with connection defaults |
| `CODEX_CONFIG` | `/home/codex/.codex/config.toml` | Effective Codex config path inside the container |
| `RECONNECT_MS` | `3000` | Reconnect delay after an unexpected disconnect |
| `WORKSPACE_ROOT` | `/workspace` | Default execution root for task requests |
| `IRONCLAW_WORKER_ID` | `worker-codex-01` | Worker identifier sent in the `ready` payload |
| `ACP_WORKER_WS_URL` | `ws://127.0.0.1:9090/ws/agent` | Bridge worker URL |
| `ACP_PROJECT_DIR` | `/workspace` | Worker-visible path sent by the ACP bridge as `context.path` |
| `ACP_USE_SESSION_CWD` | `false` | Send ACP `cwd` as `context.path` instead of `ACP_PROJECT_DIR` |

## Protocol Reference

See [`agent_comm_protocol.json`](agent_comm_protocol.json) for the full message schema.

At a high level:

Client role:
1. The worker opens the WebSocket connection.
2. The worker sends a `ready` message.
3. The hub sends `task_request` messages.
4. The worker streams `task_progress` and finishes with `task_result`.

Server role:
1. The worker listens on `WS_PORT` and `WS_PATH`.
2. The agent connects and receives the worker `ready` message.
3. The agent sends `task_request` messages.
4. The worker streams `task_progress` and finishes with `task_result`.
