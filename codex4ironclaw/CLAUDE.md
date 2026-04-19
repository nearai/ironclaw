# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this project is

IronClaw Codex Worker — a Docker image that wraps `@openai/codex` as a managed worker. It supports two runtime modes:

- **CLI mode** (default): runs Codex as a one-shot process for a given prompt
- **WebSocket mode**: either connects to an agent hub in `client` role or listens for inbound agent connections in `server` role

In both modes a lightweight Python HTTP server (`health_server.py`) always runs on port 8443 serving `/health` and `/ready`.

## Build & run

```bash
# Build the image
docker build -t ironclaw-codex-worker:latest .

# Prepare config (first time)
mkdir -p config
cp example-codex.toml config/codex.toml

# CLI mode
docker run --rm \
  -e OPENAI_API_KEY="$OPENAI_API_KEY" \
  -v "$PWD/config:/app/config:ro" \
  -v "$PWD/workspace:/workspace" \
  ironclaw-codex-worker:latest \
  --mode cli "your task prompt here"

# WebSocket mode
docker run --rm \
  -e OPENAI_API_KEY="$OPENAI_API_KEY" \
  -e AGENT_AUTH_TOKEN="$AGENT_AUTH_TOKEN" \
  -e CODEX_MODE=websocket \
  -v "$PWD/config:/app/config:ro" \
  -p 8443:8443 \
  ironclaw-codex-worker:latest \
  --mode websocket

# Docker Compose (starts worker + example subagent client)
docker compose up --build
```

## Key environment variables

| Variable | Default | Notes |
|---|---|---|
| `OPENAI_API_KEY` | — | Required |
| `AGENT_AUTH_TOKEN` | — | WebSocket mode only |
| `CODEX_MODE` | `cli` | `cli` or `websocket` |
| `HEALTH_PORT` | `8443` | Health server port |
| `PROTOCOL_CONFIG` | `/app/config/agent_comm_protocol.json` | WebSocket protocol spec path |
| `CODEX_CONFIG` | `/home/codex/.codex/config.toml` | Symlinked from `/app/config/codex.toml` |

Copy `.env.example` to `.env` to set these for Docker Compose runs.

## Architecture

### Entrypoint flow (`entrypoint.sh`)
1. Starts `health_server.py` in the background
2. Reads `--mode` from CLI args (or `$CODEX_MODE` env)
3. **CLI mode**: `exec node .../codex "$@"` — passes remaining args directly to Codex
4. **WebSocket mode**:
   `WS_ROLE=client` reads `connection.uri` from `agent_comm_protocol.json` and runs `scripts/codex_agent_client.py`
   `WS_ROLE=server` listens on `WS_PORT`/`WS_PATH` and runs `scripts/codex_agent_server.py`

### Config symlink
`/home/codex/.codex/config.toml` → `/app/config/codex.toml`. The active config is mounted at runtime via `-v "$PWD/config:/app/config:ro"`. `config/codex.toml` is gitignored; `example-codex.toml` is the template.

The active `config.toml` in this repo (at the project root, not in `config/`) uses **TensorZero** as the model provider (`http://192.168.1.157:3001/openai/v1`) rather than OpenAI directly.

### WebSocket protocol (`agent_comm_protocol.json`)
All messages use a JSON envelope with `id`, `type`, `timestamp`, `payload`. The worker sends `ready` on connect, receives `task_request`, streams `task_progress` chunks with `done: false`, then sends a final `task_result`. Bearer token auth via `AGENT_AUTH_TOKEN`. Reconnect delay: 3000 ms.

### WebSocket runtime
`scripts/codex_agent_client.py` is the outbound worker client used when the worker connects to a hub.
`scripts/codex_agent_server.py` is the inbound worker server used when an Ironclaw agent connects to the worker.
`scripts/mock_ironclaw_agent.py` is the example test agent used by `docker-compose.yml`.

### Health server (`health_server.py`)
Pure stdlib Python. `/ready` now reflects the shared websocket state file written by the active websocket role.

## CI/CD

GitLab CI via `.gitlab-ci.yml` using `Auto-DevOps.gitlab-ci.yml` template with SAST and secret detection enabled.
