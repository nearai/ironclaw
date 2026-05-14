# Running T3Claw in Docker

## Prerequisites

- Docker Desktop (or Docker Engine + Compose plugin)
- An LLM API key (Anthropic, OpenAI, NearAI, etc.)

---

## Quick start

### 1. Generate required secrets

Two values **must** be set before first boot and never changed afterwards:

```bash
# GATEWAY_AUTH_TOKEN — your Bearer token for the web UI and API
openssl rand -hex 32

# SECRETS_MASTER_KEY — AES-256-GCM key for the encrypted secrets store (must be 64 hex chars)
openssl rand -hex 32
```

> **Why upfront?**
> `GATEWAY_AUTH_TOKEN` is hashed into the database on first boot as the admin credential — there is no unauthenticated endpoint to set it afterwards. `SECRETS_MASTER_KEY` encrypts every secret stored by the agent; changing it after first run makes previously stored secrets unreadable.

### 2. Create your `.env`

```bash
cp .env.example .env
```

Set these in `.env` (minimum viable config):

```bash
# Paste values from the openssl commands above
GATEWAY_AUTH_TOKEN=<your-64-hex-char-token>
SECRETS_MASTER_KEY=<your-64-hex-char-key>

# Pick one LLM backend
LLM_BACKEND=anthropic
ANTHROPIC_API_KEY=sk-ant-...

# Optional: change from dev default for any real deployment
POSTGRES_PASSWORD=<strong-password>
```

Everything else has a working default. See `.env.example` for the full reference.

### 2b. Trinity MCP sidecar

When you run the `app` profile, Docker also starts a `t3n-mcp-sidecar` alongside
`t3claw`. The sidecar pulls `@terminal-3/t3n-mcp` from the GitHub npm registry
at build time — **no sibling repository is required**.

Required in `.env` before building:

```bash
# read:packages access on the Terminal-3 GitHub org
GITHUB_TOKEN=ghp_...
```

Optional runtime settings:

```bash
# Default is staging if omitted
T3N_MCP_ENV=staging

# Override if you need a specific live Trinity instance
# T3N_MCP_RPC_URL=https://your-rpc-endpoint
# T3N_MCP_DASHBOARD_URL=https://your-dashboard-url

# Required for authenticated Trinity operations (session creation etc.)
# T3N_MCP_PRIVATE_KEY=0x...
```

Notes:

- The sidecar starts automatically with `docker compose --profile app up`.
- `T3N_MCP_PRIVATE_KEY` is **not** required just to prove connectivity.
- The two containers communicate via a shared Unix socket volume (`t3n_mcp_socket`).
  No network ports are needed between them.
- After first boot, register the MCP server with T3Claw (one-time):
  ```bash
  docker compose exec t3claw \
    t3claw mcp add t3n-mcp --transport unix \
    --socket /var/run/t3n-mcp/t3n-mcp.sock
  ```

### 3. Start

```bash
docker compose --profile app up --build
```

If you want a fresh rebuild so code and Dockerfile changes definitely propagate
through both `t3claw` and the Trinity MCP sidecar, use:

```bash
docker compose --profile app down
docker compose --profile app build --no-cache
docker compose --profile app up
```

Gateway is at **http://127.0.0.1:3000**. Authenticate with:

```
Authorization: Bearer <your GATEWAY_AUTH_TOKEN>
```

### 4. One-time Trinity MCP registration

Starting the sidecar is not enough on its own. T3Claw still needs a
one-time MCP server registration inside its own config:

```bash
docker compose exec t3claw \
  t3claw mcp add t3n-mcp \
  --transport unix \
  --socket /var/run/t3n-mcp/t3n-mcp.sock
```

Then verify the connection:

```bash
docker compose exec t3claw t3claw mcp test t3n-mcp
```

If the test succeeds, the Trinity MCP tools should be available to the running
instance. The MCP server config is persisted, so you only need to add it again
if you reset T3Claw's persisted state.

When the server is registered as `t3n-mcp`, T3Claw now attaches built-in
Trinity setup guidance for local auth. If a user has not finished Trinity
onboarding and granted the agent profile/data permissions yet, T3Claw will
direct them to [staging.network.terminal3.io/login](https://staging.network.terminal3.io/login)
and ask them to confirm once they have completed the step.

If you added `t3n-mcp` before this support existed, remove and re-add it once so
the saved MCP config picks up the built-in Trinity setup metadata:

```bash
docker compose exec t3claw t3claw mcp remove t3n-mcp
docker compose exec t3claw \
  t3claw mcp add t3n-mcp \
  --transport unix \
  --socket /var/run/t3n-mcp/t3n-mcp.sock
```

---

## Common commands

| Action | Command |
|---|---|
| Start (after first build) | `docker compose --profile app up` |
| Start in background | `docker compose --profile app up -d` |
| View logs | `docker compose logs -f t3claw` |
| View Trinity MCP sidecar logs | `docker compose logs -f t3n-mcp-sidecar` |
| Stop (keep data) | `docker compose --profile app down` |
| Full reset (destroys all data) | `docker compose --profile app down -v` |
| Rebuild after code changes | `docker compose --profile app up --build` |
| Fresh rebuild after Docker/code changes | `docker compose --profile app down && docker compose --profile app build --no-cache && docker compose --profile app up` |
| Postgres only (for `cargo run`) | `docker compose up` |

---

## Local iteration

When editing Rust crates or the sidecar bridge, `docker compose up -d` on its own
will not rebuild the images. The `Makefile` exposes two targets that combine the
build and restart steps:

- `make rebuild-claw` — rebuild `t3claw` + `t3n-mcp-sidecar`, then `up -d`.
- `make rebuild-sidecar` — rebuild `t3n-mcp-sidecar` only, then restart
  `t3claw` so the lazy-spawn re-execs the new sidecar binary.

`make help` lists every target.

---

## Multi-tenancy

To run a shared instance with per-user isolation, add to `.env`:

```bash
AGENT_MULTI_TENANT=true
HEARTBEAT_MULTI_TENANT=true  # if heartbeat is enabled
```

Create additional users after boot:

```bash
curl -sS -X POST http://127.0.0.1:3000/api/admin/users \
  -H "Authorization: Bearer <GATEWAY_AUTH_TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{"display_name":"Alice","email":"alice@example.com","role":"member"}'
```

---

## Sandbox (Docker-in-Docker)

To allow the agent to spin up Docker containers for job isolation, mount the host socket. Add to the `t3claw` service in `docker-compose.yml`:

```yaml
volumes:
  - /var/run/docker.sock:/var/run/docker.sock
```

Then set in `.env`:

```bash
SANDBOX_ENABLED=true
```

> This gives the container control over the host Docker daemon — equivalent to root access. Only enable if you need job sandboxing.

---

## Backups

```bash
# Database
docker compose exec postgres pg_dump -U t3claw -d t3claw \
  --format=custom > backup-$(date +%Y%m%d).dump

# Workspace / skills volume
docker run --rm \
  -v t3claw-claw_t3claw_data:/source:ro \
  -v $(pwd)/backups:/dest \
  alpine tar czf /dest/t3claw-$(date +%Y%m%d).tar.gz -C /source .
```
