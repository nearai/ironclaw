---
type: "Guide"
title: "Deployment & Configuration"
description: "Build, deploy, and configure IronClaw across development, staging, and production environments. Covers Docker builds, runtime profiles, configuration management, and environment-variable injection."
tags: [deployment, configuration, docker, operators, production]
verified:
  - by: openwiki/0.6.0
    at: 2026-09-28T08:16:43.565Z
sources:
  - id: openwiki-source-03992b8dfeea5b7c8c634d9c
    resource: repo://crates/app/ironclaw_cli/Cargo.toml
  - id: openwiki-source-245e37521806083040609b2d
    resource: repo://crates/app/ironclaw_cli/README.md
  - id: openwiki-source-6139b30bb29d75d66444194a
    resource: repo://crates/app/ironclaw_cli/src/webui_token.rs
  - id: openwiki-source-d1e95c32bc4b193f2d78e9c8
    resource: repo://crates/app/ironclaw_composition/README.md
  - id: openwiki-source-4d5f53253b98cb7f45169e4c
    resource: repo://crates/app/ironclaw_composition/src/deployment.rs
  - id: openwiki-source-198456486e807b1fa0c29b6c
    resource: repo://crates/app/ironclaw_composition/src/lib.rs
  - id: openwiki-source-4a0be14ce4289d33058bc3a4
    resource: repo://crates/app/ironclaw_config/src/budget.rs
  - id: openwiki-source-9e59ea3dae9b7cdac551ff90
    resource: repo://crates/app/ironclaw_config/src/config_file.rs
  - id: openwiki-source-51c31e44d078b4fab07b9848
    resource: repo://crates/app/ironclaw_config/src/home.rs
  - id: openwiki-source-7b349b0ada3c7821ff40a022
    resource: repo://crates/app/ironclaw_config/src/profile.rs
  - id: openwiki-source-b4a5b9ccbfeb23f1ee1ff2e2
    resource: repo://crates/app/ironclaw_config/src/retired_sections.rs
  - id: openwiki-source-2449d28148b0434accd30572
    resource: repo://crates/app/ironclaw_config/src/secrets_guard.rs
  - id: openwiki-source-20ba14fec31b0eec3b7b37ac
    resource: repo://docker/reborn/config.toml
  - id: openwiki-source-3eb24a24468c00eb3a849086
    resource: repo://docker/reborn/entrypoint.sh
  - id: openwiki-source-bb1ebe868e35e9e500714501
    resource: repo://Dockerfile
  - id: openwiki-source-18544723259db8b2000607f0
    resource: repo://docs/internal/reborn/deploy-reborn-cli-docker.md
generated: { by: "openwiki/0.6.0", at: "2026-09-28T08:16:43.565Z" }
---

# Deployment & Configuration

This guide covers building, deploying, and configuring IronClaw in development, staging, and production environments. It explains the three-layer configuration model, deployment profiles, runtime substrates, and the operational responsibilities for managing IronClaw instances.

## Architecture Overview

IronClaw deployment follows a **three-layer configuration model**:

1. **Catalog** — `providers.json` (provider registry, loaded at boot via `ironclaw_llm::ProviderRegistry`)
2. **Selection** — `config.toml` (operator-edited boot-time config in `$IRONCLAW_REBORN_HOME/config.toml`)
3. **Runtime** — computed in the composition root by resolving selection against catalog

Field precedence within each layer:
```
compiled defaults  <  config.toml  <  environment variables  <  CLI flags
```

Secrets are **environment-only by policy** — raw secret values pasted into `config.toml` are rejected at parse time by the `reject_inline_secret` guard.

## Build System

### Docker Build: Multi-Stage Pipeline

The production Dockerfile (`/Dockerfile`) uses a four-stage pipeline:

1. **`node_toolchain`** — Node 22.23.1 for WebUI frontend build
2. **`railway_cli`** — Railway CLI binary (for Railway deployments)
3. **`chef` → `planner` → `deps`** — Rust dependency caching via cargo-chef
4. **`builder`** — Compiles `ironclaw` binary with `--profile dist`
5. **`runtime`** — Minimal Debian runtime with SSH, PostgreSQL client, SQLite

**Build command:**
```bash
docker build -f Dockerfile -t ironclaw-reborn:latest .
```

**Key build features:**

- **WASM sandbox**: Wasmtime 47.0.4+ for untrusted tool execution with WASI p2 host API
- **Node toolchain**: WebUI v2 frontend bundled at build time
- **Railway CLI**: Pre-installed for Railway Sandbox deployments
- **Base image**: `debian:bookworm-slim` for runtime, pinned to specific digest for reproducibility
- **Build profile**: `dist` profile with `CARGO_PROFILE_DIST_PANIC=abort` and `CARGO_PROFILE_DIST_CODEGEN_UNITS=1` for optimized size/startup

**Entrypoint (`/docker/reborn/entrypoint.sh`):**

The container starts as root and the entrypoint:
1. Resolves `IRONCLAW_REBORN_HOME` (Railway volume mount or `/data/ironclaw-reborn`)
2. Creates and chowns directories to unprivileged `ironclaw` user (uid 1000)
3. Installs default `config.toml` if missing (selected by profile)
4. Migrates legacy `[llm.default]` stubs and retired `[slack]` sections
5. Optionally starts SSH daemon on port 2222 if `IRONCLAW_REBORN_SSH_PUBLIC_KEY` is set
6. Drops privilege with `gosu ironclaw` and execs the `ironclaw serve` command

**SSH access:**

- Disabled by default; enable by setting `IRONCLAW_REBORN_SSH_PUBLIC_KEY` (the public key is the sole `authorized_keys` entry)
- Username is `agent` (uid 1000 alias of `ironclaw` runtime user — SSH sessions hold full runtime identity)
- Port 2222 must be explicitly published (`docker run -p 2222:2222` or Railway TCP proxy)
- Authentication is public-key-only; no password or keyboard-interactive fallback

## Deployment Profiles

Eight boot profiles control deployment mode, storage backend, resource limits, and policy constraints. Profiles are selected via `IRONCLAW_REBORN_PROFILE` environment variable; if unset, defaults to `local-dev`.

| Profile | Storage | Sandbox | HTTP Listener | Use Case | Status |
|---------|---------|---------|---------------|----------|--------|
| `local-dev` | libSQL (volume-backed) | None (host access only) | Loopback only | Development laptop / CI | Default |
| `local-dev-yolo` | libSQL (volume-backed) | None (unrestricted host) | Loopback only | Development with full trust | Dev-only |
| `hosted-single-tenant` | PostgreSQL (durable) | None | Public (all binds) | Production-grade single-tenant | Stable |
| `hosted-single-tenant-volume` | libSQL (persistent volume) | None | Public (all binds) | Railway preview (volume + SSO) | Preview |
| `hosted-single-tenant-volume-sandboxed` | libSQL (persistent volume) | Docker (local) | Public (all binds) | Local Docker multi-tenant dev | Preview |
| `hosted-single-tenant-volume-sandboxed-railway` | libSQL (persistent volume) | Railway Sandbox | Public (all binds) | Railway Sandbox preview | Preview |
| `production` | PostgreSQL (durable) | Railway Sandbox (optional) | Public (all binds) | Future multi-replica production | Reserved |
| `migration-dry-run` | PostgreSQL (durable) | None | Disabled | Validate production boot/migrations | Reserved |

### Profile Characteristics

**Standalone profiles** (`local-dev`, `local-dev-yolo`):
- Trusted development environment; no multi-tenant isolation
- libSQL filesystem storage under `$IRONCLAW_REBORN_HOME/reborn-data/`
- Dev-only profiles refuse to bind HTTP listener to non-loopback addresses
- `local-dev-yolo` grants trusted-host access; `local-dev` enforces stricter capability policy

**Hosted single-tenant profiles** (`hosted-single-tenant*`):
- Single isolated user/tenant; durable state storage
- `hosted-single-tenant`: PostgreSQL-backed, production-ready
- `hosted-single-tenant-volume`: libSQL on persistent mount (Railway preview)
- Sandboxed variants: per-user shell/process isolation via Docker or Railway Sandbox
- All hosted profiles start a WebUI listener (healthcheck at `/api/health` before DB assembly completes)

**Production/migration profiles**:
- Reserved for future multi-replica deployments
- `migration-dry-run`: validates config and runs migrations without serving traffic
- Fail-closed when required services are absent

## Configuration: `config.toml`

The operator-edited configuration file at `$IRONCLAW_REBORN_HOME/config.toml` controls:

- **Boot**: Profile selection (overridable by env var)
- **Identity**: Tenant, owner, default agent IDs
- **Policy**: Runtime behavior constraints (task limits, resource governance)
- **Drivers**: Stateless agent loop driver and harness selection
- **Runner**: Concurrent turn-runner workers, max concurrent runs per user
- **Skills**: Regex-based skill activation enablement
- **Storage**: Durable backend (libSQL or PostgreSQL)
- **LLM slots**: Per-slot provider selection (keyed by `default`, `mission`, etc.)
- **WebUI**: HTTP gateway bind address, token/user ID env vars
- **Google OAuth**: Public client IDs for Gmail/Calendar/Drive extensions
- **Budget**: Cost-based spend limits (USD per day/user/tick, thresholds)
- **Trigger poller**: Scheduled trigger polling configuration
- **Memory**: Memory capability profile bindings (mem0, memory-native, etc.)

### Three-Layer LLM Configuration

The LLM system uses three layers resolved at runtime:

1. **Catalog (`providers.json`)** — all available providers (NearAI, OpenAI, Anthropic, etc.)
2. **Selection (`[llm.default]` in `config.toml`)** — operator chooses provider + model for each slot
3. **Runtime (`resolve_reborn_runtime_llm`)** — if no slot is configured, env-var fallback (e.g., `NEARAI_API_KEY`)

**Key fields in `[llm.<slot>]`:**

```toml
[llm.default]
provider_id = "nearai"
model = "deepseek-ai/DeepSeek-V4-Flash"
api_key_env = "NEARAI_API_KEY"
```

No implicit slot is shipped in bundled configs — the operator must explicitly configure `[llm.default]` or rely on environment-variable resolution.

### Secret Handling

- Secrets (API keys, master keys, passwords) must **not** appear in `config.toml`
- All secret values are **environment-only** (`IRONCLAW_*`, `NEARAI_API_KEY`, `POSTGRES_URL`, etc.)
- At parse time, the `reject_inline_secret` guard fails closed if a value looks like a secret (e.g., hex string ≥32 chars, base64, or matches known patterns)
- The `[google]` section contains **public** client IDs only; secrets stay in `IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET`, `IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET`, etc.

### Config Seeding & Migrations

**First boot:**

The entrypoint checks if `$IRONCLAW_REBORN_HOME/config.toml` exists. If missing, it:
1. Selects a default config by profile (from `/opt/ironclaw/reborn/config*.toml`)
2. Copies it into `$IRONCLAW_REBORN_HOME/config.toml`
3. On subsequent boots, the existing home config is preserved

**Custom config mount:**

To seed a custom config instead of the bundled default:
1. Mount the file under `/opt/ironclaw/`
2. Set `IRONCLAW_REBORN_DEFAULT_CONFIG` to that path
3. On first start, the entrypoint copies it into the home

**Legacy migrations:**

The entrypoint runs two one-time migrations:

1. **`[llm.default]` stub migration**: Removes the old baked-in LLM stub (exact byte-for-byte match) if it exists, enabling new env-var-driven resolution. A backup (`config.toml.pre-llm-migration`) is saved.
2. **`[slack]` section migration**: Removes legacy `[slack]` setup fields (`signing_secret_env`, `bot_token_env`) when `enabled = false` is present. Other setups fail boot closed with a migration pointer.

Retired sections (`[slack]`, `[telegram]`) parse but emit deprecation notices. Presence of setup keys (e.g., `installation_id`, `bot_token_env`) fails boot closed with guidance to use the extension system.

## Environment Variables

All IronClaw environment variables use the `IRONCLAW_` prefix (or vendor-specific keys like `NEARAI_API_KEY` for LLM providers).

### Boot Configuration

| Variable | Purpose | Example |
|----------|---------|---------|
| `IRONCLAW_REBORN_HOME` | State root directory | `/data/ironclaw-reborn` or `$HOME/.ironclaw-reborn` |
| `IRONCLAW_REBORN_PROFILE` | Deployment profile (overrides `config.toml`) | `hosted-single-tenant`, `local-dev-yolo` |
| `IRONCLAW_REBORN_WORKSPACE_ROOT` | Project/workspace filesystem root | `$IRONCLAW_REBORN_HOME/workspace` |

### HTTP Gateway

| Variable | Purpose | Example |
|----------|---------|---------|
| `IRONCLAW_REBORN_SERVE_HOST` | Bind address (HTTP listener) | `127.0.0.1` (default), `0.0.0.0` (public) |
| `IRONCLAW_REBORN_SERVE_PORT` | Bind port (overridden by Railway `PORT`) | `3000` |
| `IRONCLAW_REBORN_LOG` | Tracing level | `info`, `debug`, `warn` |

### WebUI Authentication

| Variable | Purpose | Requirement |
|----------|---------|-------------|
| `IRONCLAW_REBORN_WEBUI_TOKEN` | HTTP bearer token (32+ hex bytes) | **Required** for `serve` to start |
| `IRONCLAW_REBORN_WEBUI_USER_ID` | Authenticated user ID | **Required** for `serve` to start |
| `IRONCLAW_REBORN_WEBUI_BASE_URL` | Public HTTPS base URL | e.g., `https://myapp.railway.app` (for OAuth callbacks) |

### WebUI Google OAuth (SSO login)

| Variable | Purpose | Example |
|----------|---------|---------|
| `IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID` | Google OAuth public client ID | From Google Cloud console |
| `IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET` | Google OAuth client secret | **Secret; env-only** |
| `IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS` | Whitelist of allowed email domains | `near.ai,example.com` |

**OAuth callback registration:**

Register this URL in the Google OAuth client:
```
https://<IRONCLAW_REBORN_WEBUI_BASE_URL>/auth/callback/google
```

### Product-Auth Google OAuth (Agent credential linking)

| Variable | Purpose | Example |
|----------|---------|---------|
| `IRONCLAW_REBORN_GOOGLE_CLIENT_ID` | Google OAuth public client ID for product-auth | From Google Cloud console |
| `IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET` | Google OAuth client secret for product-auth | **Secret; env-only** |
| `IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI` | Callback URL (agent linking, not WebUI SSO) | `https://myapp.railway.app/api/reborn/product-auth/oauth/google/callback` |

### LLM Providers

| Variable | Purpose | Example |
|----------|---------|---------|
| `NEARAI_BASE_URL` | NearAI API endpoint | `https://cloud-api.near.ai` |
| `NEARAI_API_KEY` | NearAI API key | **Secret; env-only** |
| `OPENAI_API_KEY` | OpenAI API key | **Secret; env-only** |
| `ANTHROPIC_API_KEY` | Anthropic API key | **Secret; env-only** |

### Storage Backend

| Variable | Purpose | Example |
|----------|---------|---------|
| `IRONCLAW_REBORN_POSTGRES_URL` | PostgreSQL connection URL | `postgresql://user:pass@host:5432/dbname` |
| `IRONCLAW_REBORN_POSTGRES_POOL_MAX_SIZE` | Connection pool size | `1` (for managed Postgres with low session limits) |
| `IRONCLAW_FILESYSTEM_POSTGRES_MIGRATION_CONNECT_MAX_WAIT_SECS` | DB connection timeout during assembly | `300` (default 5 min) |

### Master Key / Secrets Encryption

| Variable | Purpose | Example |
|----------|---------|---------|
| `IRONCLAW_REBORN_SECRET_MASTER_KEY` | Encryption key for stored credentials | 32+ hex bytes |
| `IRONCLAW_REBORN_ALLOW_EPHEMERAL_RAILWAY` | Allow ephemeral storage on Railway (dev-only) | `true` |

### SSH Access

| Variable | Purpose | Example |
|----------|---------|---------|
| `IRONCLAW_REBORN_SSH_PUBLIC_KEY` | SSH public key (enables sshd on port 2222) | SSH RSA/Ed25519 public key |

### SSH Slack Integration

Slack webhooks answer `503 temporarily_unavailable` until the extension is installed:

1. **No config file setup** — Slack routes are compiled unconditionally
2. **No env var** — `IRONCLAW_REBORN_SLACK_ENABLED` has no reader; do not add it
3. **Extension-driven** — Set signing secret and bot token in WebUI `/extensions` after installation
4. **No shared channels** — Each user OAuth-connects individually; per-channel routes are legacy

Legacy `[slack]` sections in `config.toml` cause boot failures unless they only contain `enabled = false` (which the entrypoint migrates).

### Budget Defaults

Budget-layer environment variables override compiled defaults and the `[budget]` section:

```bash
IRONCLAW_BUDGET_USER_DAILY_USD=5.00              # Per-user daily ceiling
IRONCLAW_BUDGET_PROJECT_DAILY_USD=2.00           # Per-project daily ceiling
IRONCLAW_BUDGET_MISSION_PER_TICK_USD=0.50        # Per-mission-tick budget
IRONCLAW_BUDGET_HEARTBEAT_PER_TICK_USD=0.05      # Per-heartbeat-tick budget
IRONCLAW_BUDGET_ROUTINE_LIGHTWEIGHT_USD=0.02     # Per-lightweight-routine budget
IRONCLAW_BUDGET_ROUTINE_STANDARD_USD=0.10        # Per-standard-routine budget
IRONCLAW_BUDGET_BACKGROUND_JOB_DEFAULT_USD=1.00  # Per-background-job budget
IRONCLAW_BUDGET_DEFAULT_TZ=UTC                   # IANA timezone for period rollover
IRONCLAW_BUDGET_WARN_AT=0.75                     # Warn threshold (0.0–1.0)
IRONCLAW_BUDGET_PAUSE_AT=0.90                    # Pause-with-approval threshold (0.0–1.0)
IRONCLAW_BUDGET_OVERESTIMATE_FACTOR=1.2          # Pre-call estimate multiplier
```

Setting any USD field to `0` disables that limit (unlimited spending).

## Deployment Scenarios

### Local Development

**Profile:** `local-dev` (default)

```bash
# Set home (optional; defaults to ~/.ironclaw-reborn)
export IRONCLAW_REBORN_HOME=~/.ironclaw-reborn

# Start the server (picks up default config or ~/.ironclaw-reborn/config.toml)
ironclaw serve
```

**First boot** installs a default `config.toml` and libSQL database.

### Docker (Local Development)

```bash
# Build image
docker build -f Dockerfile -t ironclaw-reborn:local .

# Create env file (outside git)
cat > .env.reborn <<EOF
IRONCLAW_REBORN_SERVE_HOST=127.0.0.1
IRONCLAW_REBORN_SERVE_PORT=3000
IRONCLAW_REBORN_PROFILE=local-dev
IRONCLAW_REBORN_WEBUI_TOKEN=$(openssl rand -hex 32)
IRONCLAW_REBORN_WEBUI_USER_ID=reborn-cli
NEARAI_API_KEY=<your-nearai-key>
EOF

# Run container
docker run --rm \
  --env-file .env.reborn \
  -p 127.0.0.1:3000:3000 \
  ironclaw-reborn:local
```

**Key points:**

- Loopback binding (`127.0.0.1:3000`) requires `IRONCLAW_REBORN_SERVE_HOST=127.0.0.1`
- Token and user ID are required; `serve` exits if missing
- For custom config, mount it at `/opt/ironclaw/` and set `IRONCLAW_REBORN_DEFAULT_CONFIG`
- libSQL data persists under `/data/ironclaw-reborn` (ephemeral unless a volume is mounted)

### Railway Deployment (Production)

**Profile:** `hosted-single-tenant` (PostgreSQL) or `hosted-single-tenant-volume` (libSQL on managed volume)

**Railway variables:**

```bash
IRONCLAW_REBORN_SERVE_HOST=0.0.0.0              # Railway NAT requires public binding
IRONCLAW_REBORN_PROFILE=hosted-single-tenant    # or hosted-single-tenant-volume
IRONCLAW_REBORN_POSTGRES_URL=$POSTGRES_URL      # (for hosted-single-tenant only)
IRONCLAW_REBORN_SECRET_MASTER_KEY=<random-key>  # Encryption key for secrets
IRONCLAW_REBORN_WEBUI_TOKEN=<32+-hex-bytes>
IRONCLAW_REBORN_WEBUI_USER_ID=reborn-cli
NEARAI_API_KEY=<your-api-key>
```

**Healthcheck:**

Railway automatically detects the `/api/health` endpoint on port 3000. The WebUI listener starts before the database is fully initialized, allowing Railway to drain the old deployment and release PgBouncer connections.

**Volume management:**

- If using `hosted-single-tenant-volume`, mount a Railway volume and it auto-detects `RAILWAY_VOLUME_MOUNT_PATH`
- Attach a Railway PostgreSQL service for `hosted-single-tenant`
- Without a volume or database, deployments fail closed (use `IRONCLAW_REBORN_ALLOW_EPHEMERAL_RAILWAY=true` for disposable test deployments only)

### Railway Sandbox Preview

**Profile:** `hosted-single-tenant-volume-sandboxed-railway`

<!-- openwiki: broken internal link [../internal/reborn/railway-sandbox-operator.md] file "../internal/reborn/railway-sandbox-operator.md" does not exist. Fix the href or restore the target, then delete this comment. -->
Each command runs in a fresh inner Docker worker inside a Railway Sandbox. Per-user isolation is provided by the Railway Sandbox lifecycle; see [railway-sandbox-operator.md](../internal/reborn/railway-sandbox-operator.md) for setup.

## Runtime Profiles & Policy

The `DeploymentConfig` struct controls:

- **Sandbox type**: `SandboxType::None` (host), `SandboxType::Docker` (local Docker), or Railway Sandbox
- **Storage backend**: libSQL (volume-backed) or PostgreSQL
- **Policy profile**: `Standalone`, `StandaloneUnrestricted`, `HostedSingleTenant`, `Production`, etc.
- **Traffic policy**: `Enabled` (serve requests), `Disabled` (no traffic), or `HealthcheckOnly` (serve `/api/health` before DB ready)

The runtime policy resolver (in `ironclaw_runtime_policy`) is the **only** producer of `EffectiveRuntimePolicy`, which enforces:

- Capability whitelists (which extensions can use which capabilities)
- Resource limits (concurrent tasks, memory, CPU)
- Approval workflows (user/task approval gates)
- Feature gates (skill activation, model access, etc.)

## Feature Flags

The `ironclaw` binary is built with feature toggles:

- **`memory-mem0`** — Compile mem0 third-party memory provider; without it, a mem0 binding fails closed
- **`test-support`** — Dev-only seam for hermetic E2E tests (loopback-only OAuth endpoint); release builds fail closed if the seam vars are present

These are set during `cargo build --features ...`.

## Multi-Package Deployment

The `ironclaw` **CLI binary contains all features** — concrete extension packages (Slack, Telegram, web-app), first-party capabilities (Google, memory), and the WebUI are all compiled in. The binary is the sole entrypoint; unlike multi-package architectures, there are no separate service binaries.

The binary's dependencies are pinned in `crates/app/ironclaw_cli/Cargo.toml` and validated by architecture tests (`reborn_dependency_boundaries.rs`) — a new direct dependency requires a reviewed gate change.

## Starting & Stopping

### Command-Line Start

```bash
ironclaw serve \
  --host 127.0.0.1 \
  --port 3000
```

Arguments override environment variables.

### Systemd Service (Unix)

Use `ironclaw service install` to register a systemd unit (on Linux) or LaunchAgent (on macOS). The service resolves the current home/profile via `IRONCLAW_REBORN_HOME` and `IRONCLAW_REBORN_PROFILE` environment variables.

### Docker Container

```bash
# Foreground
docker run --rm --env-file .env.reborn ironclaw-reborn:local

# Background
docker run -d --env-file .env.reborn -p 127.0.0.1:3000:3000 ironclaw-reborn:local

# Stop container
docker stop <container-id>
```

### Graceful Shutdown

Send SIGTERM to the process; `ironclaw serve` shuts down:
1. Stops accepting new requests
2. Drains in-flight turns
3. Flushes event logs
4. Closes database connections

## Operationalizing IronClaw

### Configuration Management

- **Version control**: Commit `config.toml` to a private repo (or exclude secrets, then commit)
- **Secret injection**: Pass secrets via environment variables at container start
- **Rollback**: Keep backup of `config.toml` before migrations (entrypoint creates `config.toml.pre-llm-migration`)
- **Observability**: Set `IRONCLAW_REBORN_LOG=debug` for detailed logs during troubleshooting

### Monitoring

- **Health endpoint**: `/api/health` returns `200` when ready (or `503` if db assembly incomplete)
- **Logs**: Check stderr/stdout; set `IRONCLAW_REBORN_LOG=info` or `debug`
- **Readiness state**: The operator can query `/api/status` (admin-only) for detailed readiness diagnostics

### Backing Up State

**LibSQL (volume-backed profiles):**

```bash
# Copy the entire home directory to durable storage
cp -r $IRONCLAW_REBORN_HOME /backup/ironclaw-reborn-$(date +%Y%m%d)
```

**PostgreSQL (hosted-single-tenant):**

```bash
# Use standard PostgreSQL backup tools
pg_dump $IRONCLAW_REBORN_POSTGRES_URL > backup.sql
```

### Capacity Planning

**Disk:**
- `$IRONCLAW_REBORN_HOME/reborn-data/` (libSQL): ~10 MB baseline + conversation history + attachments
- `$IRONCLAW_REBORN_HOME/workspace/`: Projects, attachments, materialized extensions
- libSQL files auto-checkpoint; growth is gradual

**Memory:**
- Baseline: ~100–200 MB (Rust async runtime, extension host)
- Per-turn: +20–50 MB (turn state, LLM context, extension state)
- Set container limits (e.g., `--memory=512m` in Docker)

**CPU:**
- Baseline: low (waiting for I/O)
- Per-turn: moderate (LLM calls are network-bound; local tool execution is CPU-bound)
- Concurrent tasks are limited by `max_concurrent_runs_per_user` and `max_concurrent_trigger_runs` in `[runner]`

**Network:**
- Outbound: LLM provider APIs, extension APIs (Slack, Google, etc.)
- Inbound: HTTP/2 (WebUI) + SSH (if `IRONCLAW_REBORN_SSH_PUBLIC_KEY` is set)

## Related Documentation

- **Architecture**: `/docs/internal/reborn/target-architecture/families/app.md` (composition, CLI, config)
- **Docker build**: `/docs/internal/reborn/deploy-reborn-cli-docker.md`
- **Railway Sandbox**: `/docs/internal/reborn/railway-sandbox-operator.md`
- **Development setup**: `/openwiki/development/setup.md`
- **Security**: `/openwiki/operations/security.md`

## Troubleshooting

### `serve` exits before binding listener

**Cause:** Missing `IRONCLAW_REBORN_WEBUI_TOKEN` or `IRONCLAW_REBORN_WEBUI_USER_ID`

**Fix:** Set both variables and retry.

### `denied: default` config parse error

**Cause:** An inline secret was detected in `config.toml`

**Fix:** Move the value to an environment variable and reference it via `env_var` field instead.

### `IRONCLAW_REBORN_HOME must not be the filesystem root`

**Cause:** `IRONCLAW_REBORN_HOME` is set to `/`

**Fix:** Use a subdirectory (e.g., `/data/ironclaw-reborn` or `/home/user/.ironclaw-reborn`)

### PostgreSQL connection pool exhausted

**Cause:** Too many concurrent connections for the managed database service

**Fix:** Set `IRONCLAW_REBORN_POSTGRES_POOL_MAX_SIZE=1` or `2` to avoid overwhelming the connection quota.

### Stale `[slack]` section causes boot failure

**Cause:** Old `config.toml` carries legacy `[slack]` setup keys

**Fix:** Delete the entire `[slack]` section (or just the setup keys if you want to keep a deprecated inert section). The entrypoint migrates the old-shipped default config automatically.
