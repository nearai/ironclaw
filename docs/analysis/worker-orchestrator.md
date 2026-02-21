# IronClaw Codebase Analysis — Worker & Orchestrator (Docker Sandbox)

> Updated: 2026-02-21 | Version: v0.9.0

## 1. Overview

IronClaw's Docker sandbox system provides isolated execution of untrusted or
resource-intensive code. When a job requires executing arbitrary shell commands,
building software, or running a full Claude Code session, the main IronClaw
process delegates that work to a short-lived Docker container.

The system uses a two-process model:

- **Orchestrator**: runs inside the main IronClaw process. It manages container
  lifecycle via the bollard crate, holds all secrets and LLM credentials, and
  exposes an internal HTTP API that containers call back on.

- **Worker**: runs inside a Docker container as a separate invocation of the
  `ironclaw` binary (in `--worker-mode` or `--claude-bridge` mode). It has no
  direct access to the database, secrets store, or LLM API keys. All access
  goes through the orchestrator.

Two worker modes exist:

- **Worker mode** (`ironclaw worker`): runs a full tool-execution loop with
  proxied LLM calls. Tools available are a restricted container-safe set
  (shell, file I/O, apply_patch). The LLM provider is `ProxyLlmProvider`,
  which routes all completions through the orchestrator.

- **Claude Code bridge mode** (`ironclaw claude-bridge`): spawns the `claude`
  CLI inside the container and streams its NDJSON output back to the
  orchestrator. Used when `CLAUDE_CODE_ENABLED=true`.

The sandbox is activated by the shell tool, code execution requests, or
explicit user-requested compute jobs. Each job gets a fresh container,
a unique per-job auth token, and its own credential grants scoped to that
specific job only.

---

## 2. Architecture

```
[IronClaw Main Process (orchestrator)]
    │
    │  bollard (Docker API via Unix socket or TCP)
    ▼
[Docker Container (per-job, bridge network)]
    │  runs: ironclaw worker  OR  ironclaw claude-bridge
    │
    │  IRONCLAW_WORKER_TOKEN  (env, per-job, ephemeral)
    │  IRONCLAW_JOB_ID        (env)
    │  IRONCLAW_ORCHESTRATOR_URL  (env → host.docker.internal:50051)
    │
    ▼
[Worker Runtime / Claude Bridge Runtime]
    │
    │  HTTP to orchestrator (Bearer <job-token>)
    │
    ├── GET  /worker/{id}/job             — fetch task description
    ├── GET  /worker/{id}/credentials     — fetch decrypted secrets
    ├── POST /worker/{id}/llm/complete    — proxy LLM completion
    ├── POST /worker/{id}/llm/complete_with_tools
    ├── POST /worker/{id}/status          — report progress
    ├── POST /worker/{id}/event           — stream events to UI
    ├── GET  /worker/{id}/prompt          — poll for follow-up prompts
    └── POST /worker/{id}/complete        — signal job done / trigger cleanup
```

The orchestrator listens on port 50051 (configurable). On macOS/Windows
(Docker Desktop), it binds to `127.0.0.1` because Docker Desktop routes
`host.docker.internal` through its VM to the host loopback. On Linux, it binds
to `0.0.0.0` because containers reach the host via the docker bridge gateway
(`172.17.0.1`), not loopback.

---

## 3. Orchestrator (`orchestrator/`)

Source files:

- `/Users/mudrii/src/ironclaw/src/orchestrator/mod.rs`
- `/Users/mudrii/src/ironclaw/src/orchestrator/api.rs`
- `/Users/mudrii/src/ironclaw/src/orchestrator/auth.rs`
- `/Users/mudrii/src/ironclaw/src/orchestrator/job_manager.rs`

### Container lifecycle (`job_manager.rs`)

`ContainerJobManager` handles the full Docker container lifecycle using the
bollard crate. It connects to Docker lazily (on first use) and caches the
connection.

When `create_job()` is called:

1. A cryptographically random 32-byte per-job token is generated and stored
   in `TokenStore` (in-memory only, never persisted or logged).
2. Credential grants for the job are stored in `TokenStore` alongside the token.
3. A `ContainerHandle` is recorded (state: `Creating`).
4. The container is configured with:
   - Image: `ironclaw-worker:latest` (configurable)
   - Environment: `IRONCLAW_WORKER_TOKEN`, `IRONCLAW_JOB_ID`,
     `IRONCLAW_ORCHESTRATOR_URL`
   - For `ClaudeCode` mode: `ANTHROPIC_API_KEY` or `CLAUDE_CODE_OAUTH_TOKEN`,
     `CLAUDE_CODE_ALLOWED_TOOLS`
   - Volume mounts: project directory at `/workspace:rw`
     (validated to be under `~/.ironclaw/projects/`)
   - Security: `cap_drop: ALL`, `cap_add: CHOWN`,
     `security_opt: no-new-privileges:true`
   - Tmpfs: `/tmp` at 512 MB
   - Network: `bridge` mode
   - User: UID/GID `1000:1000` (non-root `sandbox` user)
5. Container is started; handle is updated to `Running`.

On completion or explicit stop, `complete_job()` or `stop_job()` stops and
removes the container via Docker API, then calls `token_store.revoke(job_id)`
which atomically deletes the token and all credential grants for that job.
The handle is kept briefly so the calling tool can read the completion result,
then `cleanup_job()` removes it from memory.

Two job modes are supported:

- `JobMode::Worker`: CMD is `ironclaw worker --job-id <uuid> --orchestrator-url <url>`
- `JobMode::ClaudeCode`: CMD is `ironclaw claude-bridge --job-id <uuid> --orchestrator-url <url> --max-turns <n> --model <m>`

Worker containers receive 2 GB RAM by default; Claude Code containers receive
4 GB (configurable, since the `claude` CLI itself is heavier).

### Per-job token authentication (`auth.rs`)

`TokenStore` is an in-memory `HashMap<Uuid, String>` protected by `RwLock`.
Each token is a 32-byte cryptographically random value, hex-encoded to 64
characters. Token comparison uses `subtle::ConstantTimeEq` to prevent
timing-side-channel attacks.

A token for Job A cannot authenticate against endpoints scoped to Job B.
The `worker_auth_middleware` axum middleware extracts the `{job_id}` from the
URL path and validates that the `Authorization: Bearer <token>` header matches
the stored token for exactly that job ID.

Tokens are revoked together with their credential grants when the container
is stopped or cleaned up.

### Internal API server (`api.rs`)

`OrchestratorApi::router()` builds an axum `Router` with all `/worker/` routes
protected by the `worker_auth_middleware` route layer. The `/health` endpoint
is unauthenticated.

Full endpoint list:

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/health` | Liveness probe (no auth) |
| `GET` | `/worker/{id}/job` | Return task description to worker |
| `POST` | `/worker/{id}/llm/complete` | Proxy basic LLM completion |
| `POST` | `/worker/{id}/llm/complete_with_tools` | Proxy tool-use completion |
| `POST` | `/worker/{id}/status` | Receive progress update, update handle |
| `POST` | `/worker/{id}/complete` | Mark job done, trigger container cleanup |
| `POST` | `/worker/{id}/event` | Receive SSE event, broadcast + persist |
| `GET` | `/worker/{id}/prompt` | Pop next follow-up prompt from queue |
| `GET` | `/worker/{id}/credentials` | Serve decrypted secrets to container |

Job events received via `/event` are:

- Persisted to the database (fire-and-forget tokio::spawn)
- Converted to `SseEvent` variants and broadcast on the web gateway SSE channel

This gives the browser UI real-time visibility into container activity.

---

## 4. Worker Runtime (`worker/runtime.rs`)

Source file: `/Users/mudrii/src/ironclaw/src/worker/runtime.rs`

`WorkerRuntime` is the entry point when `ironclaw worker` is invoked inside
a container. It reads `IRONCLAW_WORKER_TOKEN` from the environment and
constructs a `WorkerHttpClient` bound to that job ID.

Startup sequence:

1. `WorkerHttpClient::from_env()` reads the token from `IRONCLAW_WORKER_TOKEN`.
2. `get_job()` fetches the task title and description from the orchestrator.
3. `fetch_credentials()` fetches decrypted secrets as `{env_var, value}` pairs.
   Credentials are stored in an `Arc<HashMap>` and injected into child
   processes via `Command::envs()` — never via `std::env::set_var()`, which
   is unsafe in a multi-threaded runtime.
4. `report_status()` signals `in_progress` to the orchestrator.
5. The `execution_loop()` runs up to `max_iterations` (default 50) turns with
   a configurable timeout (default 600 seconds).

The execution loop:

- Calls `Reasoning::select_tools()` to ask the LLM (via `ProxyLlmProvider`)
  which tool to use next.
- Executes the selected tool with a per-tool timeout.
- Validates tool parameters through `SafetyLayer` before execution.
- Sanitizes tool output through `SafetyLayer` before feeding it back to
  the LLM.
- Streams `tool_use`, `tool_result`, and `message` events to the orchestrator
  after each step.
- Polls for follow-up prompts every iteration and injects them as user messages.

Tools available inside the worker are registered by
`ToolRegistry::register_container_tools()`. This is a restricted set:
`shell`, `read_file`, `write_file`, `list_dir`, `apply_patch`. Tools that
require database access, secrets store access, or network calls to external
APIs are not available.

What the worker cannot do:

- Access the database directly
- Access the secrets store directly
- Call LLM APIs directly (all calls are proxied through the orchestrator)
- Reach other containers (network mode is `bridge`, not `host`)
- Escalate privileges (cap_drop ALL, non-root user)

---

## 5. Worker API (`worker/api.rs`)

Source file: `/Users/mudrii/src/ironclaw/src/worker/api.rs`

`WorkerHttpClient` is the HTTP client used by both `WorkerRuntime` and
`ClaudeBridgeRuntime` to talk to the orchestrator. Every request includes
`Authorization: Bearer <token>` where the token is the per-job value read
from `IRONCLAW_WORKER_TOKEN` at startup.

URL pattern: `{orchestrator_url}/worker/{job_id}/{endpoint}`

Key types:

- `JobDescription`: title, description, optional project_dir path
- `StatusUpdate`: state string, optional message, iteration counter
- `CompletionReport`: success bool, optional message, iteration count
- `JobEventPayload`: event_type string + arbitrary JSON data blob
- `PromptResponse`: content string + done bool
- `CredentialResponse`: env_var name + plaintext value (decrypted by orchestrator)
- `ProxyCompletionRequest` / `ProxyCompletionResponse`: LLM passthrough
- `ProxyToolCompletionRequest` / `ProxyToolCompletionResponse`: LLM tool passthrough

The `poll_prompt()` method returns `None` on HTTP 204 (no prompt available)
and `Some(PromptResponse)` on HTTP 200. The worker polls this every 2 seconds
during the follow-up loop in `ClaudeBridgeRuntime`, and every iteration in
`WorkerRuntime::poll_and_inject_prompt()`.

The `fetch_credentials()` method treats HTTP 204 and 404 as "no credentials
granted" rather than errors, so containers that don't need secrets start
cleanly without error logs.

---

## 6. Claude Bridge (`worker/claude_bridge.rs`)

Source file: `/Users/mudrii/src/ironclaw/src/worker/claude_bridge.rs`

`ClaudeBridgeRuntime` is the alternative to `WorkerRuntime`. Instead of
running an internal LLM reasoning loop, it spawns the `claude` CLI and
streams its output back to the orchestrator.

Startup sequence:

1. `copy_auth_from_mount()`: if the host bind-mounts `~/.claude` at
   `/home/sandbox/.claude-host:ro`, copies auth files into the container's
   writable `/home/sandbox/.claude`. This is a no-op when the orchestrator
   injects credentials via environment variables instead.
2. `write_permission_settings()`: writes `/workspace/.claude/settings.json`
   with an explicit tool allowlist (`permissions.allow` array). This replaces
   `--dangerously-skip-permissions`. The Docker container remains the primary
   security boundary; the settings file is defense-in-depth.
3. `get_job()` and `fetch_credentials()`: same as worker runtime.
4. `report_status("running", "Spawning Claude Code")`.
5. `run_claude_session()`: spawns `claude -p "<task>" --output-format stream-json --verbose --max-turns <n> --model <m>`, injects credential env vars via `Command::envs()`.

The `claude` CLI emits NDJSON (one JSON object per line) on stdout with these
top-level event types:

- `system`: session init with `session_id`, tools list, model
- `assistant`: LLM response; content blocks under `message.content[]`
  as `text` or `tool_use` blocks
- `user`: tool results; `tool_result` blocks under `message.content[]`
- `result`: final summary with `is_error`, `duration_ms`, `num_turns`, result text

`stream_event_to_payloads()` converts each NDJSON line into one or more
`JobEventPayload` objects which are POSTed to `/worker/{id}/event`.

The `session_id` captured from the initial `system` event is passed to
`--resume <session_id>` for follow-up turns in the prompt polling loop.

Follow-up prompt loop:

- Polls `/worker/{id}/prompt` every 2 seconds.
- On `done: true`, breaks and reports completion.
- On a new prompt content, calls `run_claude_session()` with `--resume <sid>`.
- On a follow-up session failure, logs the error and continues polling
  (does not fail the whole job).

Stderr from `claude` is captured in a separate tokio task and forwarded as
`status` events to the orchestrator.

---

## 7. Proxy LLM (`worker/proxy_llm.rs`)

Source file: `/Users/mudrii/src/ironclaw/src/worker/proxy_llm.rs`

`ProxyLlmProvider` implements the `LlmProvider` trait by routing all calls
through `WorkerHttpClient` instead of calling any LLM API directly.

- `complete()` calls `client.llm_complete()` → POST `/worker/{id}/llm/complete`
- `complete_with_tools()` calls `client.llm_complete_with_tools()` →
  POST `/worker/{id}/llm/complete_with_tools`

The orchestrator-side handlers (`llm_complete`, `llm_complete_with_tools` in
`api.rs`) forward these to the real `LlmProvider` held in `OrchestratorState`,
which has the actual API keys and credentials.

Cost tracking (`cost_per_token()`) returns `(Decimal::ZERO, Decimal::ZERO)`
because billing and token accounting happen on the orchestrator side, not in
the container.

This design means:

- The container process never holds API keys for Anthropic, NEAR AI, OpenAI,
  or any other provider.
- The orchestrator can apply rate limiting, audit logging, and cost tracking
  to all LLM calls made by containers.
- The same `ProxyLlmProvider` works regardless of which backend the
  orchestrator is configured to use.

Token validation at the orchestrator ensures a container running a compromised
or manipulated job cannot use another job's LLM quota.

---

## 8. Dockerfile.worker

Source file: `/Users/mudrii/src/ironclaw/Dockerfile.worker`

The Dockerfile uses a two-stage build:

**Stage 1 — Builder** (`rust:1.92-bookworm`):

- Copies the full source tree into `/build`
- Runs `cargo build --release --bin ironclaw`
- Produces `/build/target/release/ironclaw`

**Stage 2 — Runtime** (`debian:bookworm-slim`):

Installed packages (all via apt):

- `ca-certificates`, `curl` — TLS and package fetching
- `git` — version control in container tasks
- `build-essential`, `pkg-config`, `libssl-dev` — C/C++ compilation
- `nodejs`, `npm` — Node.js toolchain
- `python3`, `python3-pip`, `python3-venv` — Python toolchain
- `gh` — GitHub CLI (from the official GitHub apt repository)

Additional installs:

- Rust toolchain (`rustup`, toolchain `1.92.0`) installed into
  `/usr/local/rustup` and `/usr/local/cargo`, world-readable. This allows
  workers to compile Rust code inside the container.
- `@anthropic-ai/claude-code` (latest) installed globally via npm — required
  for `claude-bridge` mode.

Security hardening at the Dockerfile level:

- The final image runs as the non-root `sandbox` user (UID 1000, matching
  the orchestrator's `user: "1000:1000"` config).
- `/workspace` is owned by `sandbox`.
- `/home/sandbox/.claude` is pre-created and owned by `sandbox` so Claude
  Code can write its state files without needing root.

The entrypoint is `ironclaw`. The orchestrator passes the full command
(`worker --job-id ...` or `claude-bridge --job-id ...`) as Docker CMD,
allowing a single image to serve both modes.

Build command:

```bash
docker build -f Dockerfile.worker -t ironclaw-worker .
```

---

## 9. Docker Compose (`docker-compose.yml`)

Source file: `/Users/mudrii/src/ironclaw/docker-compose.yml`

The `docker-compose.yml` is scoped to local development only. It does not
define the worker container — workers are created on-demand by the orchestrator
at runtime. The compose file defines only the infrastructure services that the
main IronClaw process requires:

**`postgres`** service:

- Image: `pgvector/pgvector:pg16` (PostgreSQL 16 with the pgvector extension
  for semantic search)
- Port: `5432:5432`
- Credentials: `ironclaw / ironclaw` (development only, not for production)
- Health check: `pg_isready -U ironclaw` every 5 seconds
- Volume: named `pgdata` for persistence across restarts

Worker containers are not defined in docker-compose.yml because they are
created dynamically by `ContainerJobManager` using the bollard Docker API.
The compose file is only for bootstrapping the development database.

For production, run PostgreSQL separately (or use Turso/libSQL) and deploy
the main IronClaw binary with `SANDBOX_ENABLED=true` and the appropriate
`SANDBOX_IMAGE` pointing to a pre-built `ironclaw-worker` image.

---

## 10. Security Properties

The sandbox system provides the following security guarantees:

### Container isolation — fresh container per job

Each job gets a brand-new container. There is no shared mutable state between
jobs. A container cannot observe or modify the state of another job's
container. Containers are fully removed (not just stopped) after job
completion via `docker.remove_container(..., force: true)`.

### No cross-job token access

Per-job tokens are bound to the job UUID at the path level. The
`worker_auth_middleware` rejects a valid token for Job A when it is presented
on a Job B URL (`StatusCode::UNAUTHORIZED`). Constant-time comparison prevents
timing attacks. Tokens are revoked atomically with credential grants when a
container completes.

### No direct database access

The worker has no `DATABASE_URL` or database credentials. All persistence
(job events, status) goes through the orchestrator's `/event` and `/status`
endpoints, which are authenticated and rate-limited to the active job.

### No direct secrets access

Secrets (API keys, OAuth tokens) are stored encrypted in the host's
`SecretsStore`. The worker fetches only the secrets explicitly granted to its
job via `/credentials`. The orchestrator decrypts them at serve time and sends
plaintext values in the HTTP response body — which is only reachable over the
internal loopback or bridge network (not exposed publicly). Usage of each
secret is recorded in the audit trail.

### Network isolation between containers

Containers use Docker `bridge` network mode. They cannot reach each other
directly. Each container can only reach `host.docker.internal:50051` (the
orchestrator API) and external networks (controlled by the sandbox network
proxy allowlist when `SANDBOX_NETWORK_PROXY=true`).

### Capability drop

Container host config sets `cap_drop: ALL` (drops all Linux capabilities) and
`cap_add: CHOWN` (adds back only what is needed for file ownership operations).
`security_opt: no-new-privileges:true` prevents privilege escalation via
setuid binaries.

### Non-root execution

The container runs as UID/GID `1000:1000` (the `sandbox` user). The worker
binary is in `/usr/local/bin/ironclaw`, owned by root, and not writable by
`sandbox`. The only writable areas for the `sandbox` user are `/workspace`
(the job's project directory) and `/tmp` (tmpfs, limited to 512 MB).

### Cleanup on failure

If container creation fails after the token is issued, `create_job()` catches
the error, calls `token_store.revoke(job_id)`, and removes the handle from
the in-memory map. There is no leaked token or dangling handle.

### Defense-in-depth for Claude Code bridge

In `claude-bridge` mode, the `claude` CLI is given an explicit tool allowlist
via `/workspace/.claude/settings.json` rather than
`--dangerously-skip-permissions`. Unknown or future Claude Code tools are not
auto-approved and would require interactive confirmation, which times out
harmlessly in a non-interactive container. The Docker container boundary is
still the primary security mechanism; the settings file adds a second layer.
