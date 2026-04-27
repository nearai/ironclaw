# Debugging BastionClaw

Practical commands for "what is actually happening inside this stack right
now". Scoped to the docker-compose deployment (`make up` / `make rebuild`).
For bare-metal `cargo run` debugging the log-level tips still apply; the
container-specific commands don't.

Service names used below (from `docker-compose.yml`):

- `bastionclaw` — the agent itself
- `t3n-mcp-sidecar` — Trinity MCP server (stdio → Unix socket bridge)
- `postgres` — main database

---

## Viewing logs

```bash
# Follow agent logs live (ctrl-C to stop)
make logs
# same as: docker compose --profile app logs -f bastionclaw

# Follow sidecar logs
docker compose --profile app logs -f t3n-mcp-sidecar

# Last 200 lines, no follow
docker compose --profile app logs --tail=200 bastionclaw

# Everything since a time point
docker compose --profile app logs --since=5m bastionclaw
docker compose --profile app logs --since=2026-04-23T10:00:00 bastionclaw

# All services at once (timestamps help correlate)
docker compose --profile app logs -f --timestamps
```

The sidecar's bridge prefixes its own lines with `[t3n-mcp-bridge]`; anything
without that prefix is coming from the Trinity MCP child process itself
(raw stderr passthrough).

---

## Increasing agent log verbosity

The agent reads `RUST_LOG` from the environment. Default is roughly
`bastionclaw=info`. To get more detail, set it in `.env` and restart:

```bash
# Everything in bastionclaw at debug level
RUST_LOG=bastionclaw=debug

# Just the agent loop
RUST_LOG=bastionclaw::agent=debug

# Agent + HTTP request traces from tower
RUST_LOG=bastionclaw=debug,tower_http=debug

# Absolutely everything (very noisy)
RUST_LOG=bastionclaw=trace
```

Apply with:

```bash
# Edit .env, then
make restart   # restarts only bastionclaw, keeps postgres + sidecar up
```

Note: inside the REPL/TUI, `info!` and `warn!` corrupt the terminal UI. If
you're doing ad-hoc debugging from `cargo run`, prefer `debug!` for internal
traces (see CLAUDE.md "Logging levels matter for REPL/TUI").

---

## Shell into a container

```bash
# Interactive shell in bastionclaw
make shell
# same as: docker exec -it bastion-claw-bastionclaw-1 sh

# One-off command in bastionclaw
docker compose --profile app exec bastionclaw bastionclaw mcp list

# Shell in the sidecar (root, since docker-compose.yml sets user: "0:0")
docker exec -it bastion-claw-t3n-mcp-sidecar-1 sh

# Shell in postgres (use psql directly below — this rarely needed)
docker exec -it bastion-claw-postgres-1 sh
```

---

## Inspecting tool call history (inputs & outputs)

Every tool call the agent makes lands in the `job_actions` table.
LLM data is never deleted (see CLAUDE.md), so this is a permanent audit
trail — inputs, raw outputs, sanitised outputs, duration, success flag.

```bash
# Open a psql shell against the main DB
docker compose exec postgres psql -U bastionclaw -d bastionclaw
```

Useful queries once you're in psql:

```sql
-- Last 10 tool calls across all jobs, newest first
SELECT
  created_at,
  tool_name,
  success,
  duration_ms,
  error_message,
  job_id
FROM job_actions
ORDER BY created_at DESC
LIMIT 10;

-- Full input/output for a specific tool call
SELECT
  tool_name,
  jsonb_pretty(input)            AS input,
  output_raw,
  jsonb_pretty(output_sanitized) AS output_sanitized,
  error_message
FROM job_actions
WHERE id = 'PASTE-UUID-HERE';

-- All calls to a specific tool (e.g. everything hitting t3n-mcp)
SELECT created_at, success, duration_ms, input->>'params' AS params
FROM job_actions
WHERE tool_name LIKE 't3n%'
ORDER BY created_at DESC
LIMIT 20;

-- Every tool call for the most recent job
SELECT sequence_num, tool_name, success, duration_ms
FROM job_actions
WHERE job_id = (SELECT id FROM agent_jobs ORDER BY created_at DESC LIMIT 1)
ORDER BY sequence_num;
```

`input` and `output_sanitized` are JSONB — use `jsonb_pretty(...)` when
reading them interactively, or `->>` / `->` for projection.

One-liner variant when you don't want the psql prompt:

```bash
docker compose exec -T postgres psql -U bastionclaw -d bastionclaw \
  -c "SELECT created_at, tool_name, success FROM job_actions ORDER BY created_at DESC LIMIT 10;"
```

---

## MCP-specific debugging

### What's configured

```bash
# All registered MCP servers for the default user
docker compose exec bastionclaw bastionclaw mcp list

# Verbose — shows transport, auth state, cached tool list
docker compose exec bastionclaw bastionclaw mcp list --verbose
```

### Ping / handshake test

```bash
docker compose exec bastionclaw bastionclaw mcp test t3n-mcp
```

If this fails, the agent cannot reach the MCP — debug in that order:

1. Sidecar logs (`docker compose logs t3n-mcp-sidecar`) — is the bridge
   listening? Is the child process exiting on start-up?
2. Socket existence from inside bastionclaw:
   ```bash
   docker compose exec bastionclaw ls -la /var/run/t3n-mcp/
   ```
   Should show `t3n-mcp.sock` with permissive perms.
3. Raw connectivity from inside bastionclaw:
   ```bash
   docker compose exec bastionclaw sh -c \
     'echo "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{}}}" | nc -U /var/run/t3n-mcp/t3n-mcp.sock'
   ```
   (Install `nc` in the image first if needed, or use `socat`.)

### Re-bootstrap a stale MCP entry

The t3n-mcp entry is written to the DB on first boot from
`T3N_MCP_SOCKET_PATH` (see `bootstrap_t3n_mcp_server` in
`src/tools/mcp/config.rs`). Bootstrap never rewrites an existing entry —
so if the persisted config has an out-of-date shape (e.g. leftover
`local_auth` from an older build), remove it and let bootstrap recreate
it on next start:

```bash
docker compose exec bastionclaw bastionclaw mcp remove t3n-mcp
make restart
```

---

## Sidecar child-process debugging

The bridge in `docker/t3n-mcp-bridge.mjs` spawns one MCP child per
incoming Unix-socket connection. If it's crashing instantly, the
bridge's `spawn npx ENOENT` log is sometimes misleading — the real
error is downstream (module resolution, missing file, bad config). Run
tsx directly against the source to see the real stack:

```bash
docker exec bastion-claw-t3n-mcp-sidecar-1 sh -c 'cd /app && npx tsx src/index.ts'
```

Hit Ctrl-C after you see the error line.

---

## Database quick-reference

Tables you'll actually open when debugging:

| Table | What's in it |
|---|---|
| `job_actions` | Every tool call: input, output, sanitised output, duration, success/error |
| `agent_jobs` | Per-job state machine (Pending → InProgress → Completed/Failed) |
| `conversation_messages` | Raw LLM exchange |
| `llm_calls` | LLM provider request/response metadata (model, tokens, cost) |
| `memory_documents` | Workspace files (AGENTS.md, USER.md, etc.) |

See `src/db/CLAUDE.md` and `migrations/V1__initial.sql` for the full
schema. Schema changes happen via new `migrations/VN__*.sql` files —
never edit an applied one in place.

---

## Nuclear options

```bash
# Stop everything, keep data
make down

# Stop AND wipe all volumes (postgres + workspace). All state lost.
make wipe

# No-cache rebuild of a specific service (use when Docker layer caching
# is fooling you after changing something subtle like a .dockerignore)
docker compose --profile app build --no-cache t3n-mcp-sidecar && make up
```

---

## Common failure modes we've hit

- **Sidecar log spams `Failed to spawn Trinity MCP child {error:"spawn npx ENOENT"}`** —
  misleading error. Usually means tsx crashed on module resolution because
  trinity's `.dockerignore` stripped a file from the build context. Check
  which files actually made it into the image with `docker exec bastion-claw-t3n-mcp-sidecar-1 find /app/src -type f | sort` and diff against the host. Fix by narrowing the build context to a subdirectory that doesn't have the offending `.dockerignore` (see `docker-compose.yml`'s `additional_contexts`).

- **MCP activation returns `awaiting_token` with Trinity sign-in instructions** —
  there's a stale MCP entry in the DB with `local_auth` attached.
  `bastionclaw mcp remove t3n-mcp` + `make restart` to re-bootstrap clean.

- **`make rebuild` doesn't pick up a change you're sure you made** —
  Docker layer cache. Use `docker compose --profile app build --no-cache <service>`
  to force a clean rebuild of just that service.

- **`GATEWAY_AUTH_TOKEN` / `SECRETS_MASTER_KEY` changed and the agent
  won't start** — these are hashed/used on first boot and can't be
  rotated without a `make wipe`. Keep them stable or plan for state loss.
