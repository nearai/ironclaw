# IronClaw Developer Reference

> **AI Agent Use**: Optimized for code review, bug triage, and targeted fixes.
> Jump directly to the section relevant to the error or task — no narrative reading required.

**Source**: IronClaw v0.9.0 · `~/src/ironclaw/` · ~115K lines Rust

---

## Table of Contents

1. [Quick Navigation Map](#1-quick-navigation-map)
2. [Module Responsibilities](#2-module-responsibilities)
3. [Error Catalog](#3-error-catalog)
4. [Configuration Reference](#4-configuration-reference)
5. [LLM Backend Matrix](#5-llm-backend-matrix)
6. [Database Dual-Backend Rules](#6-database-dual-backend-rules)
7. [Job State Machine](#7-job-state-machine)
8. [Tool System Reference](#8-tool-system-reference)
9. [Safety Layer Pipeline](#9-safety-layer-pipeline)
10. [Skills Trust Model](#10-skills-trust-model)
11. [Docker Sandbox Policies](#11-docker-sandbox-policies)
12. [Worker and Claude Bridge Modes](#12-worker-and-claude-bridge-modes)
13. [Code Review Checklist](#13-code-review-checklist)
14. [Bug Fix Patterns](#14-bug-fix-patterns)
15. [Anti-Patterns](#15-anti-patterns)
16. [Key Grep Queries](#16-key-grep-queries)
17. [Feature Flag Testing](#17-feature-flag-testing)
18. [Module Spec Files](#18-module-spec-files)

---

## 1. Quick Navigation Map

| "I need to find..." | Open this file |
|---------------------|----------------|
| Main agent loop / message dispatch | `src/agent/agent_loop.rs` |
| Job scheduling, parallel execution | `src/agent/scheduler.rs` |
| Per-job LLM reasoning loop | `src/agent/worker.rs` |
| Stuck job detection / recovery | `src/agent/self_repair.rs` |
| Session / conversation model | `src/agent/session.rs` |
| Context window compaction | `src/agent/compaction.rs` |
| Memory pressure monitoring | `src/agent/context_monitor.rs` |
| Routine (cron/event/webhook) engine | `src/agent/routine_engine.rs` |
| Proactive heartbeat logic | `src/agent/heartbeat.rs` |
| Interactive REPL channel | `src/channels/repl.rs` |
| Web gateway routes (40+ endpoints) | `src/channels/web/server.rs` |
| SSE broadcast | `src/channels/web/sse.rs` |
| WebSocket | `src/channels/web/ws.rs` |
| HTTP webhook channel | `src/channels/http.rs` |
| WASM channel runtime | `src/channels/wasm/` |
| All error types | `src/error.rs` |
| All config structs / env var loading | `src/config/mod.rs` |
| Tool trait definition | `src/tools/tool.rs` |
| Tool registry / discovery | `src/tools/registry.rs` |
| Built-in tool implementations | `src/tools/builtin/` |
| Shell tool (env scrubbing) | `src/tools/builtin/shell.rs` |
| HTML-to-Markdown converter (for HTTP responses) | `src/tools/builtin/html_converter.rs` |
| HTTP tool (external requests) | `src/tools/builtin/http.rs` |
| File tools (read/write/patch/list) | `src/tools/builtin/file.rs` |
| Memory tools (search/write/read) | `src/tools/builtin/memory.rs` |
| Job management tools | `src/tools/builtin/job.rs` |
| Routine management tools | `src/tools/builtin/routine.rs` |
| WASM sandbox runtime | `src/tools/wasm/runtime.rs` |
| WASM tool host functions | `src/tools/wasm/host.rs` |
| WASM fuel / memory limits | `src/tools/wasm/limits.rs` |
| WASM network allowlist | `src/tools/wasm/allowlist.rs` |
| WASM credential injection | `src/tools/wasm/credential_injector.rs` |
| Dynamic tool builder | `src/tools/builder/core.rs` |
| MCP client (HTTP only) | `src/tools/mcp/client.rs` |
| Prompt injection sanitizer | `src/safety/sanitizer.rs` |
| Input validator | `src/safety/validator.rs` |
| Policy rules engine | `src/safety/policy.rs` |
| Secret leak detector (15+ patterns) | `src/safety/leak_detector.rs` |
| Credential detection in HTTP params | `src/safety/credential_detect.rs` |
| LLM provider trait | `src/llm/provider.rs` |
| LLM provider factory / backend enum | `src/llm/mod.rs` |
| NEAR AI provider (Chat Completions, dual auth) | `src/llm/nearai_chat.rs` |
| Smart routing (cheap/primary cascade) | `src/llm/smart_routing.rs` |
| Circuit breaker | `src/llm/circuit_breaker.rs` |
| Retry + exponential backoff | `src/llm/retry.rs` |
| Multi-provider failover | `src/llm/failover.rs` |
| LLM response cache | `src/llm/response_cache.rs` |
| Token cost tracking | `src/llm/costs.rs` |
| Session token auto-renewal | `src/llm/session.rs` |
| Database trait (~60 async methods) | `src/db/mod.rs` |
| PostgreSQL backend | `src/db/postgres.rs` |
| libSQL/Turso backend | `src/db/libsql_backend.rs` |
| libSQL schema (SQLite-dialect) | `src/db/libsql_migrations.rs` |
| PostgreSQL migrations | `migrations/V1__initial.sql` |
| Workspace / memory system | `src/workspace/mod.rs` |
| Document chunker (800 tok, 15% overlap) | `src/workspace/chunker.rs` |
| Hybrid FTS+vector search (RRF) | `src/workspace/search.rs` |
| Embedding providers | `src/workspace/embeddings.rs` |
| Job context / state machine | `src/context/state.rs` |
| Concurrent job context manager | `src/context/manager.rs` |
| Docker sandbox manager | `src/sandbox/manager.rs` |
| Container lifecycle (bollard) | `src/sandbox/container.rs` |
| Sandbox network proxy | `src/sandbox/proxy/http.rs` |
| Domain allowlist (sandbox) | `src/sandbox/proxy/allowlist.rs` |
| Sandbox policies | `src/sandbox/config.rs` |
| AES-256-GCM crypto | `src/secrets/crypto.rs` |
| Secret store | `src/secrets/store.rs` |
| Skills registry | `src/skills/registry.rs` |
| Skill scoring / selection | `src/skills/selector.rs` |
| Trust-based tool attenuation | `src/skills/attenuation.rs` |
| ClawHub registry client | `src/skills/catalog.rs` |
| Onboarding wizard (7-step) | `src/setup/wizard.rs` |
| Worker runtime (inside containers) | `src/worker/runtime.rs` |
| Claude Code bridge | `src/worker/claude_bridge.rs` |
| Orchestrator internal API | `src/orchestrator/api.rs` |
| Per-job bearer token store | `src/orchestrator/auth.rs` |
| Entry point, CLI arg parsing | `src/main.rs` |
| Library root, module declarations | `src/lib.rs` |

---

## 2. Module Responsibilities

| Module | Path | Responsibility |
|--------|------|----------------|
| `agent` | `src/agent/` | Core loop, job scheduling, sessions, routines, heartbeat |
| `channels` | `src/channels/` | REPL, web gateway, HTTP webhooks, WASM plugin channels |
| `llm` | `src/llm/` | Multi-provider LLM: retry, circuit breaker, cache, failover |
| `tools` | `src/tools/` | Built-in tools, WASM sandbox, MCP client, dynamic builder |
| `safety` | `src/safety/` | Prompt injection defense: sanitize, validate, policy, leak-detect |
| `db` | `src/db/` | Database abstraction: PostgreSQL + libSQL backends |
| `workspace` | `src/workspace/` | Persistent memory: chunking, embeddings, hybrid RRF search |
| `context` | `src/context/` | Job context isolation, state machine, conversation memory |
| `sandbox` | `src/sandbox/` | Docker isolation, network proxy, credential injection |
| `worker` | `src/worker/` | Container-side execution loop, Claude Code bridge, LLM proxy |
| `orchestrator` | `src/orchestrator/` | Host-side internal API for container ↔ host communication |
| `secrets` | `src/secrets/` | AES-256-GCM secret storage, keychain, credential types |
| `skills` | `src/skills/` | SKILL.md extension system, trust model, ClawHub registry |
| `estimation` | `src/estimation/` | Cost/time/value estimation with EMA learner |
| `evaluation` | `src/evaluation/` | Job success evaluation (rule-based + LLM) |
| `history` | `src/history/` | PostgreSQL repositories, analytics aggregation |
| `setup` | `src/setup/` | 7-step interactive onboarding wizard |
| `config` | `src/config/` | All env var loading and sub-config structs |
| `error` | `src/error.rs` | All error types via `thiserror` |

---

## 3. Error Catalog

All error types defined in `src/error.rs`. Top-level `Error` wraps domain errors via `#[from]`.

### 3.1 ConfigError

| Variant | Message Pattern | Root Cause | Fix Location |
|---------|-----------------|------------|--------------|
| `MissingEnvVar(String)` | `Missing required environment variable: {0}` | Env var not set at all | Set in `.env` / LaunchAgent plist |
| `MissingRequired { key, hint }` | `Missing required configuration: {key}. {hint}` | Config field required but value absent | Check `hint` for which backend/mode requires it |
| `InvalidValue { key, message }` | `Invalid configuration value for {key}: {message}` | Env var set but cannot be parsed/validated | Fix value in `.env` |
| `ParseError(String)` | `Failed to parse configuration: {0}` | `.env` file malformed or TOML parse error | Check file syntax |
| `Io(io::Error)` | `IO error: {0}` | Cannot read `.env` or config file | Check file permissions and path |

### 3.2 DatabaseError

| Variant | Message Pattern | Root Cause | Fix Location |
|---------|-----------------|------------|--------------|
| `Pool(String)` | `Connection pool error: {0}` | DB unreachable, wrong URL, pool exhausted | `DATABASE_URL` / `LIBSQL_PATH` env vars |
| `Query(String)` | `Query failed: {0}` | SQL syntax error or schema mismatch | Check `libsql_migrations.rs` or `V1__initial.sql` |
| `NotFound { entity, id }` | `Entity not found: {entity} with id {id}` | Row missing in DB | Expected — caller should handle |
| `Constraint(String)` | `Constraint violation: {0}` | Duplicate key, FK violation | Schema design issue — check query |
| `Migration(String)` | `Migration failed: {0}` | Schema migration error | Check migration files |
| `Serialization(String)` | `Serialization error: {0}` | JSON de/serialization from DB column | Check JSONB/TEXT column content |
| `Postgres(tokio_postgres::Error)` | `PostgreSQL error: {0}` | Low-level Postgres error | Only with `#[cfg(feature = "postgres")]` |
| `LibSql(libsql::Error)` | `LibSQL error: {0}` | Low-level libSQL error | Only with `#[cfg(feature = "libsql")]` |

### 3.3 ChannelError

| Variant | Root Cause | Fix |
|---------|------------|-----|
| `StartupFailed { name, reason }` | Channel config missing, port in use | Check channel-specific env vars |
| `Disconnected { name, reason }` | Network drop, peer closed | Expected transient — handled by retry |
| `SendFailed { name, reason }` | Channel closed before response sent | Race condition — check session lifecycle |
| `InvalidMessage(String)` | Malformed incoming message | Client-side bug |
| `AuthFailed { name, reason }` | Wrong `GATEWAY_AUTH_TOKEN` or channel secret | Check auth env var for channel |
| `RateLimited { name }` | Too many messages to channel | Back off |
| `HealthCheckFailed { name }` | Channel health endpoint not responding | Restart channel |

### 3.4 LlmError

| Variant | Root Cause | Fix |
|---------|------------|-----|
| `RequestFailed { provider, reason }` | HTTP error, network, DNS | Check provider URL and API key |
| `RateLimited { provider, retry_after }` | Provider quota exceeded | Wait `retry_after` duration; retry logic in `src/llm/retry.rs` |
| `InvalidResponse { provider, reason }` | Unexpected JSON schema from provider | Provider API change — update parser |
| `ContextLengthExceeded { used, limit }` | Conversation too long | Compaction triggered in `src/agent/compaction.rs` |
| `ModelNotAvailable { provider, model }` | `*_MODEL` env var wrong | Check model name for provider |
| `AuthFailed { provider }` | Wrong API key or expired session | Check `*_API_KEY` and NEAR AI session/API key config |
| `SessionExpired { provider }` | NEAR AI session token expired | Re-authenticate; session renewal in `src/llm/session.rs` |
| `SessionRenewalFailed { provider, reason }` | Auto-renewal failed | Manual re-auth required |

### 3.5 ToolError

| Variant | Root Cause | Fix |
|---------|------------|-----|
| `NotFound { name }` | Tool name not in registry | Check registration in `src/tools/registry.rs` |
| `ExecutionFailed { name, reason }` | Tool logic threw error | Check `reason` string and tool source |
| `Timeout { name, timeout }` | Tool exceeded time limit | Increase `SANDBOX_TIMEOUT_SECS` or fix slow logic |
| `InvalidParameters { name, reason }` | JSON params don't match schema | Fix LLM prompt or tool schema |
| `Disabled { name, reason }` | Tool gated behind feature flag or config | Check tool registration conditions |
| `Sandbox { name, reason }` | WASM sandbox error | Check `src/tools/wasm/` for details |
| `AuthRequired { name }` | Tool needs credentials not set | Set required secret via `ironclaw secret set` |
| `BuilderFailed(String)` | Dynamic tool build failed | Check `src/tools/builder/core.rs` |

### 3.6 SafetyError

| Variant | Root Cause | Fix |
|---------|------------|-----|
| `InjectionDetected { pattern }` | Prompt injection in tool output | Output passes through `src/safety/sanitizer.rs` |
| `OutputTooLarge { length, max }` | Tool output exceeds max length | Tool must trim output before returning |
| `BlockedContent { pattern }` | Policy rule blocked content | Check `src/safety/policy.rs` rules |
| `ValidationFailed { reason }` | Input validation failed | Check `src/safety/validator.rs` |
| `PolicyViolation { rule }` | Named policy rule triggered | Inspect rule in `PolicyRule` registry |

### 3.7 JobError

| Variant | Root Cause | Fix |
|---------|------------|-----|
| `NotFound { id }` | Job ID invalid or expired | Check job was created successfully |
| `InvalidTransition { id, state, target }` | Illegal state machine transition | See state machine in §7 |
| `Failed { id, reason }` | Job execution failed | Inspect `reason`; check `tool_failures` table |
| `Stuck { id, duration }` | Job in `InProgress` too long | Self-repair in `src/agent/self_repair.rs` |
| `MaxJobsExceeded { max }` | `AGENT_MAX_PARALLEL_JOBS` hit | Increase env var or queue jobs |
| `ContextError { id, reason }` | Context manager error | Check `src/context/manager.rs` |

### 3.8 WorkerError (container-side)

| Variant | Root Cause | Fix |
|---------|------------|-----|
| `ConnectionFailed { url, reason }` | Container cannot reach orchestrator | Check `IRONCLAW_ORCHESTRATOR_URL` inside container |
| `LlmProxyFailed { reason }` | LLM proxy request to orchestrator failed | Check orchestrator API in `src/orchestrator/api.rs` |
| `SecretResolveFailed { secret_name, reason }` | Secret not found for injection | Register secret via host `ironclaw secret set` |
| `OrchestratorRejected { job_id, reason }` | Orchestrator rejected request | Check job state and orchestrator logs |
| `MissingToken` | `IRONCLAW_WORKER_TOKEN` not set in container | Container startup misconfiguration |

### 3.9 RoutineError

| Variant | Root Cause | Fix |
|---------|------------|-----|
| `InvalidCron { reason }` | Bad cron expression | Fix expression; valid format: `"0 */2 * * *"` |
| `MaxConcurrent { name }` | `ROUTINES_MAX_CONCURRENT` exceeded | Increase limit or reduce routine frequency |
| `EmptyResponse` | LLM returned empty content for routine | Check prompt template |
| `TruncatedResponse` | LLM finish_reason=length with no content | Shorten routine prompt or increase context |
| `Disabled { name }` | Routine disabled flag set | Enable via `routine_update` tool |

---

## 4. Configuration Reference

Config loaded in priority order: **shell env → ./.env → ~/.ironclaw/.env → config.toml → DB settings → defaults**

Config struct: `src/config/mod.rs` · `INJECTED_VARS: OnceLock<HashMap<String,String>>` for secret overlay

### 4.1 Database

| Env Var | Type | Default | Required | Notes |
|---------|------|---------|----------|-------|
| `DATABASE_BACKEND` | `"postgres"\|"libsql"\|"turso"` | `postgres` | No | Selects backend at runtime |
| `DATABASE_URL` | string (URL) | — | Yes (postgres) | `postgres://user:pass@host/db` |
| `LIBSQL_PATH` | string (path) | `~/.ironclaw/ironclaw.db` | No | Local libSQL file path |
| `LIBSQL_URL` | string (URL) | — | No | Turso cloud URL (overrides LIBSQL_PATH) |
| `LIBSQL_AUTH_TOKEN` | string | — | Yes (with LIBSQL_URL) | Turso auth token |

### 4.2 LLM

| Env Var | Type | Default | Required | Notes |
|---------|------|---------|----------|-------|
| `LLM_BACKEND` | enum | `nearai` | No | See §5 for all options |
| `NEARAI_API_KEY` | string | — | No | Enables API-key mode for NEAR AI cloud |
| `NEARAI_BASE_URL` | URL | `https://private.near.ai` | No | Override for cloud: `https://cloud-api.near.ai` |
| `NEARAI_MODEL` | string | `fireworks::accounts/fireworks/models/llama4-maverick-instruct-basic` | No | Model name |
| `NEARAI_SESSION_PATH` | path | `~/.ironclaw/session.json` | No | Session file location |
| `OPENAI_API_KEY` | string | — | Yes (openai) | `sk-...` |
| `OPENAI_BASE_URL` | URL | provider default | No | Optional override |
| `OPENAI_MODEL` | string | `gpt-4o` | No | Model name |
| `ANTHROPIC_API_KEY` | string | — | Yes (anthropic) | |
| `ANTHROPIC_BASE_URL` | URL | provider default | No | |
| `ANTHROPIC_MODEL` | string | `claude-sonnet-4-20250514` | No | |
| `OLLAMA_BASE_URL` | URL | `http://localhost:11434` | Yes (ollama) | |
| `OLLAMA_MODEL` | string | `llama3` | No | |
| `LLM_BASE_URL` | URL | — | Yes (openai_compatible) | Custom base URL |
| `LLM_API_KEY` | string | — | No | |
| `LLM_MODEL` | string | `default` | No | Falls back to selected model from settings |
| `LLM_EXTRA_HEADERS` | string | — | No | Extra HTTP headers injected into every LLM request. Format: `Key:Value,Key2:Value2`. Useful for OpenRouter attribution. |
| `NEARAI_CHEAP_MODEL` | string | — | No | Cheap model for smart routing, evaluation, heartbeat tasks |
| `TINFOIL_API_KEY` | string | — | Yes (tinfoil) | |
| `TINFOIL_MODEL` | string | `kimi-k2-5` | No | |

### 4.3 LLM Resilience

| Env Var | Type | Default | Notes |
|---------|------|---------|-------|
| `CIRCUIT_BREAKER_THRESHOLD` | u32 | unset | Failures before circuit opens |
| `CIRCUIT_BREAKER_RECOVERY_SECS` | u64 | `30` | Seconds before half-open |
| `NEARAI_MAX_RETRIES` | u32 | `3` | Max retry count |
| `RESPONSE_CACHE_ENABLED` | bool | `false` | Cache LLM responses |
| `RESPONSE_CACHE_TTL_SECS` | u64 | `3600` | Cache TTL |
| `RESPONSE_CACHE_MAX_ENTRIES` | usize | `1000` | Cache size cap |
| `LLM_FAILOVER_COOLDOWN_SECS` | u64 | `300` | Provider cooldown after repeated failures |
| `LLM_FAILOVER_THRESHOLD` | u32 | `3` | Failures before provider cooldown |
| `SMART_ROUTING_CASCADE` | bool | `true` | Cheap-model cascade behavior |

### 4.4 Agent

| Env Var | Type | Default | Notes |
|---------|------|---------|-------|
| `AGENT_NAME` | string | `ironclaw` | Display name |
| `AGENT_MAX_PARALLEL_JOBS` | usize | `5` | Job concurrency limit |
| `AGENT_JOB_TIMEOUT_SECS` | u64 | `1800` | Per-job timeout |
| `AGENT_STUCK_THRESHOLD_SECS` | u64 | `1800` | Stuck-job detector threshold |
| `AGENT_MAX_TOOL_ITERATIONS` | usize | `50` | Max agentic tool-call loop iterations |
| `AGENT_AUTO_APPROVE_TOOLS` | bool | `false` | Skip tool approvals (CI/benchmarks) |

### 4.5 Web Gateway

| Env Var | Type | Default | Required | Notes |
|---------|------|---------|----------|-------|
| `GATEWAY_ENABLED` | bool | `true` | No | Must be `true` to start gateway |
| `GATEWAY_HOST` | string | `127.0.0.1` | No | Bind address |
| `GATEWAY_PORT` | u16 | `3000` | No | Listen port |
| `GATEWAY_AUTH_TOKEN` | string | random if unset | No | Bearer token for protected API calls |
| `GATEWAY_USER_ID` | string | `default` | No | Default user context |

### 4.6 CLI / REPL

| Env Var | Type | Default | Notes |
|---------|------|---------|-------|
| `CLI_ENABLED` | bool | `true` | **Set `false` for service mode** (prevents REPL EOF crash with `/dev/null` stdin) |

### 4.7 Docker Sandbox

| Env Var | Type | Default | Notes |
|---------|------|---------|-------|
| `SANDBOX_ENABLED` | bool | `true` | Enable Docker sandbox |
| `SANDBOX_IMAGE` | string | `ironclaw-worker:latest` | Container image |
| `SANDBOX_MEMORY_LIMIT_MB` | u64 | `2048` | Container memory cap |
| `SANDBOX_TIMEOUT_SECS` | u64 | `120` | Container execution timeout |
| `SANDBOX_CPU_SHARES` | u32 | `1024` | Relative CPU weight |
| `SANDBOX_POLICY` | enum | `readonly` | `readonly\|workspace_write\|full_access` |
| `SANDBOX_AUTO_PULL` | bool | `true` | Auto-pull missing image |
| `DOCKER_HOST` | string | system default | Set to Podman socket for Podman users |

### 4.8 Claude Code Mode (in containers)

| Env Var | Type | Default | Notes |
|---------|------|---------|-------|
| `CLAUDE_CODE_ENABLED` | bool | `false` | Enable Claude Code bridge |
| `CLAUDE_CODE_MODEL` | string | `sonnet` | Model for Claude Code |
| `CLAUDE_CODE_MAX_TURNS` | u32 | `50` | Max turns per job |
| `CLAUDE_CONFIG_DIR` | path | `~/.claude` | Host config dir for credential extraction |

### 4.9 Routines

| Env Var | Type | Default | Notes |
|---------|------|---------|-------|
| `ROUTINES_ENABLED` | bool | `true` | Enable routine engine |
| `ROUTINES_CRON_INTERVAL` | u64 | `15` | Tick interval (seconds) |
| `ROUTINES_MAX_CONCURRENT` | usize | `10` | Max concurrent routine runs |
| `ROUTINES_DEFAULT_COOLDOWN` | u64 | `300` | Default cooldown between runs |
| `ROUTINES_MAX_TOKENS` | u32 | `4096` | Lightweight routine token budget |

### 4.10 Skills

| Env Var | Type | Default | Notes |
|---------|------|---------|-------|
| `SKILLS_ENABLED` | bool | `false` | Enable skills system |
| `SKILLS_DIR` | path | `~/.ironclaw/skills` | Local skill directory |
| `SKILLS_MAX_ACTIVE` | usize | `3` | Max active skills per request |
| `SKILLS_MAX_CONTEXT_TOKENS` | usize | `4000` | Max prompt budget per turn |

### 4.11 Workspace / Memory

| Env Var | Type | Default | Notes |
|---------|------|---------|-------|
| `EMBEDDING_ENABLED` | bool | `false` | Enable vector embeddings |
| `EMBEDDING_PROVIDER` | enum | `nearai` | `openai\|nearai\|ollama` |
| `EMBEDDING_MODEL` | string | `text-embedding-3-small` | Embedding model name |
| `EMBEDDING_DIMENSION` | usize | model-derived | Explicit vector size override |
| `HEARTBEAT_ENABLED` | bool | `false` | Enable proactive execution |
| `HEARTBEAT_INTERVAL_SECS` | u64 | `1800` | 30 minutes default |
| `HEARTBEAT_NOTIFY_CHANNEL` | string | unset | Channel to send findings |
| `HEARTBEAT_NOTIFY_USER` | string | unset | User to notify |

### 4.12 Tunnel

| Env Var | Type | Default | Notes |
|---------|------|---------|-------|
| `TUNNEL_PROVIDER` | enum | — | `cloudflare\|ngrok\|tailscale\|custom` |
| `TUNNEL_URL` | string | — | Static public URL (manual tunnel) |
| `TUNNEL_NGROK_TOKEN` | string | — | Required for ngrok |
| `TUNNEL_NGROK_DOMAIN` | string | — | Optional ngrok domain |
| `TUNNEL_CF_TOKEN` | string | — | Required for Cloudflare |
| `TUNNEL_TS_FUNNEL` | bool | `false` | Use tailscale funnel |
| `TUNNEL_TS_HOSTNAME` | string | — | Tailscale hostname |

### 4.13 WASM Runtime

| Env Var | Type | Default | Notes |
|---------|------|---------|-------|
| `WASM_ENABLED` | bool | `true` | Enable WASM tools |
| `WASM_TOOLS_DIR` | path | `~/.ironclaw/tools` | Tool directory |
| `WASM_DEFAULT_FUEL_LIMIT` | u64 | `10_000_000` | Execution fuel cap |
| `WASM_DEFAULT_MEMORY_LIMIT` | u64 | `10485760` | Memory cap in bytes (10MB) |
| `WASM_DEFAULT_TIMEOUT_SECS` | u64 | `60` | Execution timeout |
| `WASM_CACHE_DIR` | path | unset | Compiled module cache override |

### 4.14 Rate Limiting

| Env Var | Type | Default | Notes |
|---------|------|---------|-------|
| Built-in/WASM tool rate limiting is configured in tool/runtime capabilities and code defaults (`src/tools/rate_limiter.rs`, `src/tools/wasm/capabilities.rs`). |

### 4.15 Logging

| Env Var | Type | Default | Notes |
|---------|------|---------|-------|
| `RUST_LOG` | string | `ironclaw=info` | See §Debugging for patterns |

---

## 5. LLM Backend Matrix

| Backend | `LLM_BACKEND` value | Required Env Vars | API Protocol | Tool Call Format |
|---------|---------------------|-------------------|--------------|------------------|
| NEAR AI | `nearai` (default) | `NEARAI_SESSION_TOKEN` **or** `NEARAI_API_KEY` | Chat Completions | Text-flattened |
| OpenAI | `openai` | `OPENAI_API_KEY` | Chat Completions | Native |
| Anthropic | `anthropic` | `ANTHROPIC_API_KEY` | Messages API | Native |
| Ollama | `ollama` | `OLLAMA_BASE_URL` | Chat Completions | Native |
| OpenAI-compatible | `openai_compatible` | `LLM_BASE_URL`, `LLM_MODEL` | Chat Completions | Native |
| Tinfoil (TEE) | `tinfoil` | `TINFOIL_API_KEY` | Chat Completions (adapted) | Chat-format |

**NEAR AI auth mode selection** (`src/llm/nearai_chat.rs`):
- If `NEARAI_API_KEY` set → Bearer API key auth (base URL defaults to `https://cloud-api.near.ai`)
- Otherwise → session token auth via `SessionManager` with auto-renewal on 401 (base URL defaults to `https://private.near.ai`)
- Both modes use Chat Completions API (`/v1/chat/completions`) with tool-message flattening

**Five-tier wrapper chain** (all backends):

```
Request → SmartRoutingProvider → RetryProvider → CircuitBreakerProvider → ResponseCacheProvider → FailoverProvider → actual backend
```

Source: `src/llm/smart_routing.rs`, `src/llm/retry.rs`, `src/llm/circuit_breaker.rs`, `src/llm/response_cache.rs`, `src/llm/failover.rs`

**SmartRoutingProvider** (`src/llm/smart_routing.rs`): Routes requests to cheap vs primary model based on message complexity.
- `Simple` (greetings, yes/no, ≤10 chars, simple keywords) → cheap model (`NEARAI_CHEAP_MODEL`)
- `Complex` (code blocks, implementation/refactor/debug/analyze keywords, >1000 chars) → primary model
- `Moderate` (everything else) → cheap model first; if response contains uncertainty phrases → escalate to primary (cascade)
- Tool calls (`complete_with_tools`) always go to primary — reliable structured output required
- Config: `SMART_ROUTING_CASCADE=true` (enable cascade), `simple_max_chars=200`, `complex_min_chars=1000`

---

## 6. Database Dual-Backend Rules

> **CRITICAL**: All new persistence features MUST support both backends.

### Required Steps for Any DB Change

1. Add method to `Database` trait in `src/db/mod.rs`
2. Implement in `src/db/postgres.rs` (delegate to Store/Repository pattern)
3. Implement in `src/db/libsql_backend.rs` (native SQLite-dialect SQL)
4. Add schema in `migrations/V1__initial.sql` (PostgreSQL)
5. Add schema in `src/db/libsql_migrations.rs` (SQLite-dialect)
6. Test with both feature flags (see §16)

### Schema Translation Rules

| PostgreSQL | libSQL/SQLite |
|-----------|---------------|
| `UUID` | `TEXT` |
| `TIMESTAMPTZ` | `TEXT` (ISO-8601) |
| `JSONB` | `TEXT` |
| `VECTOR(1536)` | `F32_BLOB(1536)` with `libsql_vector_idx` |
| `tsvector` / `ts_rank_cd` | FTS5 virtual table + sync triggers |
| PL/pgSQL functions | SQLite triggers |
| `jsonb_set` (path-targeted) | `json_patch` (RFC 7396 merge patch — replaces top-level keys entirely) |

### Database Tables

| Table | Purpose |
|-------|---------|
| `conversations` | Multi-channel conversation tracking |
| `agent_jobs` | Job metadata and status |
| `job_actions` | Event-sourced tool executions |
| `dynamic_tools` | Agent-built tools |
| `llm_calls` | Cost tracking |
| `estimation_snapshots` | EMA learning data |
| `memory_documents` | Workspace files (path-based, e.g., `"context/vision.md"`) |
| `memory_chunks` | Chunked content (FTS + vector indexes) |
| `heartbeat_state` | Periodic execution tracking |
| `routines` | Scheduled/reactive routine definitions |
| `routine_runs` | Routine execution history |
| `settings` | Per-user key-value settings |
| `tool_failures` | Self-repair tracking |
| `secrets` | Encrypted credential storage |
| `wasm_tools` | WASM tool registry |
| `tool_capabilities` | Tool capability declarations |

### libSQL Known Limitations

| Limitation | Impact |
|-----------|--------|
| Workspace/memory not wired via Database trait | `EMBEDDING_ENABLED=true` requires PostgreSQL |
| Secrets store not available | AES-GCM encrypted secrets require PostgreSQL |
| Hybrid search: FTS5 only (no vector) | Semantic search unavailable |
| Settings reload from DB skipped | Config changes require restart |
| No incremental migrations | Schema uses `CREATE IF NOT EXISTS`; no `ALTER TABLE` |
| No encryption at rest | SQLite file is plaintext; use FileVault / LUKS |
| `json_patch` vs `jsonb_set` semantics | Partial nested object updates may drop keys |

---

## 7. Job State Machine

Source: `src/context/state.rs`

```
Pending
  └─→ InProgress
        ├─→ Completed
        │     └─→ Submitted
        │           └─→ Accepted
        ├─→ Failed
        └─→ Stuck
              ├─→ InProgress  (recovery attempt via self_repair.rs)
              └─→ Failed
```

| Transition | Trigger | Handler |
|-----------|---------|---------|
| `Pending → InProgress` | Job dispatched | `src/agent/scheduler.rs` |
| `InProgress → Completed` | Worker loop exits cleanly | `src/agent/worker.rs` |
| `InProgress → Failed` | Worker error, panic, timeout | `src/agent/worker.rs` |
| `InProgress → Stuck` | Heartbeat detects stale job | `src/agent/self_repair.rs` |
| `Stuck → InProgress` | Recovery attempt starts | `src/agent/self_repair.rs` |
| `Stuck → Failed` | Max recovery attempts exceeded | `src/agent/self_repair.rs` (`RepairError::MaxAttemptsExceeded`) |
| `Completed → Submitted` | Job output submitted to user | `src/agent/submission.rs` |
| `Submitted → Accepted` | User confirms acceptance | `src/agent/submission.rs` |

**Invalid transitions** throw `JobError::InvalidTransition { id, state, target }`.

---

## 8. Tool System Reference

### 8.1 Tool Trait (required interface)

Source: `src/tools/tool.rs`

```rust
#[async_trait]
impl Tool for MyTool {
    fn name(&self) -> &str { "my_tool" }           // must be unique, lowercase_snake
    fn description(&self) -> &str { "..." }         // used in LLM system prompt
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "param": { "type": "string" } },
            "required": ["param"]
        })
    }
    async fn execute(&self, params: serde_json::Value, ctx: &JobContext)
        -> Result<ToolOutput, ToolError> { ... }
    fn requires_sanitization(&self) -> bool { true }  // true = output from external sources
}
```

**Schema rules**:

- Top-level must be `"type": "object"`
- Property types: `"string"`, `"integer"`, `"boolean"`, `"array"`, `"object"` (never `["string", "null"]` array form — OpenAI 400)
- For optional string fields: omit from `"required"`, do not use array type syntax

### 8.2 Core Tool Groups

| Tool Name | Source File | Category |
|-----------|-------------|----------|
| `echo` | `builtin/echo.rs` | Debug |
| `time` | `builtin/time.rs` | Utility |
| `json` | `builtin/json.rs` | Data |
| `http` | `builtin/http.rs` | Network |
| `read_file`, `write_file`, `list_dir`, `apply_patch` | `builtin/file.rs` | Filesystem |
| `shell` | `builtin/shell.rs` | Execution |
| `memory_search`, `memory_write`, `memory_read`, `memory_tree` | `builtin/memory.rs` | Workspace |
| `create_job`, `list_jobs`, `job_status`, `cancel_job` | `builtin/job.rs` | Agent |
| `routine_create`, `routine_list`, `routine_update`, `routine_delete`, `routine_history` | `builtin/routine.rs` | Routines |
| `tool_search`, `tool_install`, `tool_auth`, `tool_activate`, `tool_list`, `tool_remove` | `builtin/extension_tools.rs` | Extensions |
| `skill_list`, `skill_search`, `skill_install`, `skill_remove` | `builtin/skill_tools.rs` | Skills |
| `html_to_markdown` | `builtin/html_converter.rs` | Utility |

### 8.3 Protected Tool Names

These names cannot be shadowed by WASM or dynamically-built tools.
The protected list is defined in `src/tools/registry.rs` (`PROTECTED_TOOL_NAMES`).

### 8.4 Tool Registration

Tools are registered in `src/tools/registry.rs` via `ToolRegistry::register()`. Discovery order:

1. Built-in tools (hardcoded, always present)
2. WASM tools (loaded from `~/.ironclaw/tools/*.wasm` and workspace `tools/`)
3. MCP tools (from configured MCP server URLs)
4. Dynamically-built tools (from `dynamic_tools` DB table)

### 8.5 WASM Tool Constraints

Source: `src/tools/wasm/`

| Constraint | Value |
|-----------|-------|
| Fuel limit | `WASM_DEFAULT_FUEL_LIMIT` (default: 10,000,000) |
| Memory limit | `WASM_DEFAULT_MEMORY_LIMIT` (default: 10MB in bytes) |
| Network | Allowlisted domains only (`src/tools/wasm/allowlist.rs`) |
| Credentials | Injected via proxy; never in WASM env |
| Rate limit | Capability-driven per-tool limits (`capabilities.json`) |
| Module cache | `WASM_CACHE_DIR` (compiled `.cwasm` files) |
| Component model | wasmtime component model (WASM P2) |

### 8.6 MCP Client

Source: `src/tools/mcp/client.rs`

- **Transport**: HTTP only (no stdio)
- **Protocol**: JSON-RPC 2.0
- **Tool discovery**: `tools/list` RPC method on startup
- **Execution**: `tools/call` RPC method per tool invocation
- Auth: Bearer token in `Authorization` header

---

## 9. Safety Layer Pipeline

Source: `src/safety/`

All external tool output passes through the pipeline in this order:

```
Tool Output
    │
    ▼
[1] Sanitizer (src/safety/sanitizer.rs)
    • Detects injection patterns (command chaining, subshells, path traversal)
    • Escapes dangerous content
    • Wraps output: <tool_output name="{}" sanitized="true">[escaped]</tool_output>
    │
    ▼
[2] Validator (src/safety/validator.rs)
    • Checks length (→ SafetyError::OutputTooLarge)
    • Encoding validation
    • Forbidden pattern matching
    │
    ▼
[3] Policy Engine (src/safety/policy.rs)
    • PolicyRule system: severity (Critical/High/Medium/Low) + action (Block/Warn/Review/Sanitize)
    • Critical = Block immediately (→ SafetyError::PolicyViolation)
    │
    ▼
[4] Leak Detector (src/safety/leak_detector.rs)
    • 15+ secret patterns: API keys, session tokens, private keys, connection strings, JWTs
    • Per-pattern action: Block (reject) | Redact (mask) | Warn (flag)
    • Runs at two points: before LLM sees tool output AND before user sees LLM response
    │
    ▼
LLM context (sanitized)
```

**Credential detect** (`src/safety/credential_detect.rs`): Used by the HTTP tool specifically to detect manually-provided credentials in request parameters (headers, URL query params, URL userinfo). Checks for auth header names (Authorization, X-Api-Key, etc.), auth value prefixes (Bearer, Basic, Token), credential query params (api_key, access_token, etc.), and embedded URL userinfo. Triggers approval prompt before executing the HTTP request.

**Shell tool** (`src/tools/builtin/shell.rs`): scrubs sensitive env vars before command execution to prevent `env` / `printenv` / `$VAR` leakage.

---

## 10. Skills Trust Model

Source: `src/skills/`

| Trust Level | Source Directory | Tool Access |
|-------------|-----------------|-------------|
| `Trusted` | `~/.ironclaw/skills/` or `<workspace>/skills/` | All tools (shell, file write, HTTP, etc.) |
| `Installed` | `~/.ironclaw/installed_skills/` (from ClawHub) | Read-only tools only |

**Selection pipeline** (per-request):

1. **Gating** (`src/skills/gating.rs`): Check `bins`, `env`, `config` requirements; skip if missing
2. **Scoring** (`src/skills/selector.rs`): Deterministic score against message keywords/patterns
3. **Budget**: Select top skills within `SKILLS_MAX_CONTEXT_TOKENS`
4. **Attenuation** (`src/skills/attenuation.rs`): Strip dangerous tool access for `Installed` skills

**SKILL.md format** (frontmatter + markdown body):

```yaml
---
name: my-skill
version: 0.1.0
description: ...
activation:
  patterns: ["deploy to.*production"]
  keywords: ["deployment"]
  max_context_tokens: 2000
metadata:
  openclaw:
    requires:
      bins: [docker, kubectl]
      env: [KUBECONFIG]
---
```

---

## 11. Docker Sandbox Policies

Source: `src/sandbox/config.rs`

| Policy | `SANDBOX_POLICY` | Filesystem | Network |
|--------|--------------------------|-----------|---------|
| `ReadOnly` | `readonly` | Read-only workspace mount | Allowlisted domains only |
| `WorkspaceWrite` | `workspace_write` | Read-write workspace mount | Allowlisted domains only |
| `FullAccess` | `full_access` | Full filesystem | Unrestricted |

**Network proxy credential model** (`src/sandbox/proxy/`):

- All container HTTP/HTTPS routes through host proxy on `SANDBOX_PROXY_PORT`
- CONNECT method validates target domain against `DomainAllowlist`
- `CredentialResolver` trait injects auth headers at transit — containers never see raw keys
- Custom policy via `NetworkPolicyDecider` trait

**Default allowlisted domains** (defined in `src/sandbox/mod.rs`): package registries (crates.io, npmjs.com, pypi.org), GitHub, common API endpoints.

---

## 12. Worker and Claude Bridge Modes

IronClaw supports two internal execution modes that run inside Docker containers. These are not user-facing commands but are essential for understanding the sandbox architecture.

### Worker Mode

**Purpose**: Standard agentic execution inside a container.

**Source**: `src/worker/runtime.rs`

**Command** (internal, invoked by orchestrator):
```bash
ironclaw worker --job-id <uuid> --orchestrator-url <url>
```

**Characteristics**:
- No TUI, no DB connection, no channels
- Communicates with host via orchestrator HTTP API
- LLM requests proxied through host (credential isolation)
- Tools execute within container filesystem

**Environment variables (set by orchestrator)**:
| Variable | Purpose |
|----------|---------|
| `IRONCLAW_WORKER_TOKEN` | Bearer token for orchestrator auth |
| `IRONCLAW_ORCHESTRATOR_URL` | Host-side API endpoint |

### Claude Bridge Mode

**Purpose**: Delegates execution to Anthropic's `claude` CLI inside a container.

**Source**: `src/worker/claude_bridge.rs`

**Command** (internal):
```bash
ironclaw claude-bridge --job-id <uuid> --orchestrator-url <url> [--model sonnet] [--max-turns 50]
```

**Enabling**:
```bash
CLAUDE_CODE_ENABLED=true
CLAUDE_CODE_MODEL=sonnet      # or opus, haiku
CLAUDE_CODE_MAX_TURNS=50
CLAUDE_CONFIG_DIR=~/.claude   # host dir for credential extraction
```

**How it works**:
1. Orchestrator starts container with Claude Bridge command
2. Bridge extracts credentials from host's `~/.claude/` directory
3. Spawns `claude` CLI with job prompt
4. Streams output back to orchestrator
5. Orchestrator forwards to user

**Use case**: Leverage Claude Code's agentic capabilities with IronClaw's sandbox isolation.

---

## 13. Code Review Checklist

From `src/CLAUDE.md` review discipline. Run these on every changed file set:

### Mechanical Pre-Commit Checks

```bash
# 1. No panics in production code
grep -rnE '\.unwrap\(\)|\.expect\(' <changed_files>
# Expected: zero hits (tests are exception)

# 2. No super:: imports (use crate:: instead)
grep -rn 'super::' <changed_files>
# Expected: zero hits

# 3. Pattern propagation — if you fixed a bug pattern, find all instances
grep -rn '<the_pattern>' src/
# Fix ALL instances, not just the reported one
```

### Architectural Checks

| Check | Description |
|-------|-------------|
| Both DB backends | Any new persistence method in `Database` trait? → Must be in BOTH `postgres.rs` AND `libsql_backend.rs` |
| Schema sync | New table/index? → Must be in BOTH `migrations/V1__initial.sql` AND `libsql_migrations.rs` |
| Seed data | Any `INSERT INTO` in migrations? → Check libSQL migration for same seed data |
| Index parity | Diff `CREATE INDEX` between the two schema files |
| Feature flag coverage | Code behind `#[cfg(feature)]`? → Test with each feature in isolation (§16) |
| Concurrency model | Changed resource sharing model? → Grep for all types that held references to old model |
| Tool names | New tool? → Name must not be in protected list; check `registry.rs` |
| Tool schema | New tool schema? → No array type syntax (`["string","null"]`); use single type strings |
| `requires_sanitization` | Tool fetches external data? → Must return `true` |

### Safety Layer Checks

| Check | Rule |
|-------|------|
| External data in tools | Must pass through safety layer (`requires_sanitization() = true`) |
| New shell commands | Check env var scrubbing in `shell.rs` |
| Credential handling | No secrets in container env — use proxy injection model |
| New leak patterns | Add to `leak_detector.rs` pattern list if new secret format detected |

---

## 14. Bug Fix Patterns

### Pattern: "Tool schema 400 Bad Request from OpenAI"

**Symptom**: OpenAI returns 400 with schema validation error
**Root cause**: Tool schema uses array type syntax `"type": ["string", "null"]`
**File**: `src/tools/builtin/http.rs` or `src/tools/builtin/json.rs`
**Fix**: Replace array type with single string type; for optional fields, remove from `"required"` list
**Grep**: `grep -rn '"type": \[' src/tools/`

### Pattern: "REPL EOF crash on service start"

**Symptom**: Service starts then immediately exits when launched via launchd/systemd
**Root cause**: `CLI_ENABLED=true` + stdin from `/dev/null` → REPL reads EOF → graceful shutdown
**Fix**: Set `CLI_ENABLED=false` in service environment
**File**: `src/channels/repl.rs` (`ReplChannel` startup), `src/config/channels.rs` (`CliConfig.enabled`)

### Pattern: "Job stuck, never completes"

**Symptom**: `agent_jobs` table shows `InProgress` for hours
**Root cause**: Worker panicked but state not updated, OR DB write failed silently
**Handler**: `src/agent/self_repair.rs` — detects after `AGENT_JOB_TIMEOUT_SECS`
**Fix**: Check `job_actions` for last action; look at `tool_failures` table; restart service

### Pattern: "NEAR AI session expired"

**Symptom**: `LlmError::SessionExpired { provider: "nearai" }`
**Root cause**: NEAR AI session credentials expired
**Handler**: `src/llm/session.rs` — auto-renewal attempted
**Fix if auto-renewal fails**: Re-authenticate via `ironclaw onboard` or use `NEARAI_API_KEY`

### Pattern: "libSQL workspace search returns no results"

**Symptom**: `memory_search` returns empty even when documents exist
**Root cause**: libSQL backend uses FTS5 only; vector search not implemented
**Impact**: Semantic queries don't match; only exact keyword matches work
**Fix**: Use PostgreSQL backend for full hybrid search, or phrase queries for FTS

### Pattern: "Config value silently ignored"

**Symptom**: Env var set but behavior unchanged
**Root cause**: Wrong priority level; `.env` file in wrong location overriding shell env
**Priority order**: shell env > `./.env` > `~/.ironclaw/.env` > config.toml > DB > defaults
**Grep**: `grep -rn 'INJECTED_VARS\|from_env\|env::var' src/config/`

### Pattern: "TOCTOU race in DB operations"

**Symptom**: Duplicate rows or lost updates under concurrent load
**Root cause**: INSERT + SELECT-back pattern — not atomic
**Fix pattern**: Use INSERT ... RETURNING or UPSERT; propagate to ALL similar sites
**Grep**: `grep -rn 'INSERT.*SELECT' src/db/`
**Rule**: Fix the pattern everywhere, not just the reported instance

### Pattern: "Secret leaked in tool output"

**Symptom**: API key visible in LLM response or user output
**Root cause**: Tool returns raw credential; `requires_sanitization()` returns `false`
**Fix**: Set `requires_sanitization() = true`; add pattern to `src/safety/leak_detector.rs`

---

## 15. Anti-Patterns

### Code Anti-Patterns

| Anti-Pattern | Why Wrong | Correct |
|-------------|-----------|---------|
| `.unwrap()` in production | Panics entire process | Return `Result`, use `?` |
| `.expect("...")` in production | Same as unwrap | Return `Result`, use `?` |
| `super::` imports | Fragile on refactor | Use `crate::` |
| `pub use` re-exports (unnecessary) | Hidden coupling | Only re-export for public API consumers |
| `std::env::set_var` for secrets | Not thread-safe | Use `INJECTED_VARS: OnceLock<HashMap<...>>` |
| String types for known enums | No type safety | Define enum, implement `FromStr` |
| Hardcoded provider URL | Config change breaks build | Use config struct, env var |
| Tool outputs raw external data | Safety bypass | Set `requires_sanitization() = true` |
| Fixing one instance of a pattern bug | Other instances remain | Grep and fix all instances |

### Database Anti-Patterns

| Anti-Pattern | Why Wrong | Correct |
|-------------|-----------|---------|
| Method on only one backend | Breaks at runtime with other backend | Implement in both backends |
| PG-only schema change | libSQL builds break | Update both migration files |
| `json_patch` for partial nested update | Drops top-level keys not in patch | Reconstruct full object or use PG backend |
| Holding single `Connection` across async points | Connection not Send across await | Use connection pool, get per-operation |

### Safety Anti-Patterns

| Anti-Pattern | Why Wrong | Correct |
|-------------|-----------|---------|
| Credential in container env | Container code can read it | Proxy injection model |
| Raw tool output to LLM | Prompt injection risk | Safety layer pipeline (§9) |
| New shell command without env scrub | Secret leakage via `env` | Check shell.rs scrubbing |
| MCP server over stdio | Not implemented | HTTP transport only |

### Operational Anti-Patterns

| Anti-Pattern | Why Wrong | Correct |
|-------------|-----------|---------|
| `CLI_ENABLED=true` in service mode | REPL crashes on `/dev/null` stdin | `CLI_ENABLED=false` |
| Single feature build assumption | Dead code behind wrong `#[cfg]` | Test all feature combos (§16) |
| Docker with Podman without `DOCKER_HOST` | bollard uses wrong socket | Set `DOCKER_HOST` to Podman socket |

---

## 16. Key Grep Queries

Pre-built search patterns for common review/debug tasks:

```bash
# Find all .unwrap() and .expect() in production code
grep -rnE '\.unwrap\(\)|\.expect\(' src/ --include="*.rs" | grep -v '#\[test\]' | grep -v 'mod tests'

# Find super:: imports (should use crate::)
grep -rn 'super::' src/ --include="*.rs"

# Find all tool schema definitions (check for array type syntax)
grep -rn '"type": \[' src/tools/ --include="*.rs"

# Find all tools that may need sanitization review
grep -rn 'requires_sanitization' src/tools/ --include="*.rs"

# Find all env var reads (config coverage check)
grep -rn 'env::var\|std::env' src/config/

# Find all Database trait methods (check both backends implement)
grep -n 'async fn ' src/db/mod.rs

# Find INSERT patterns for TOCTOU review
grep -rn 'INSERT INTO' src/db/ --include="*.rs"

# Find all error propagations (catch missing ? operator)
grep -rn 'unwrap\|panic!' src/ --include="*.rs" | grep -v test

# Find all feature-gated code blocks
grep -rn '#\[cfg(feature' src/ --include="*.rs"

# Find all hardcoded URLs (should be config)
grep -rnE '"https?://[^"]+\.(com|ai|io|dev)' src/ --include="*.rs" | grep -v test | grep -v doc

# Find all credential/secret handling
grep -rn 'api_key\|auth_token\|password\|secret' src/ --include="*.rs" -i | grep -v test | grep -vE '^\s*//'

# Find all WASM tool registrations
grep -rn 'wasm.*register\|register.*wasm' src/tools/ --include="*.rs"

# Find all channel startup sites
grep -rn 'ChannelManager\|channel.*start\|start.*channel' src/ --include="*.rs"

# Find all DB trait method calls (both backends should handle)
grep -rn '\.db\.' src/agent/ --include="*.rs" | head -30

# Check libSQL migration for missing indexes vs PostgreSQL
diff <(grep 'CREATE INDEX' migrations/V1__initial.sql) <(grep 'CREATE INDEX' src/db/libsql_migrations.rs)
```

---

## 17. Feature Flag Testing

**Required before any commit touching persistence, config, or feature-gated code:**

```bash
# Default (PostgreSQL only)
cargo check

# libSQL only (zero-dependency mode)
cargo check --no-default-features --features libsql

# Both backends available
cargo check --features "postgres,libsql"

# All features
cargo check --all-features

# Run tests for each
cargo test
cargo test --no-default-features --features libsql
cargo test --all-features
```

**Dead code risk**: Code behind wrong `#[cfg(feature)]` gate compiles silently with default features but breaks single-feature builds. Always test the feature-specific build.

---

## 18. Module Spec Files

Some modules have authoritative spec files. **Code must match spec** — spec is the tiebreaker when code and spec disagree.

| Module | Spec File | When to Read |
|--------|-----------|--------------|
| `src/setup/` | `src/setup/README.md` | Before modifying onboarding wizard |
| `src/workspace/` | `src/workspace/README.md` | Before modifying memory/search/chunking |
| `src/tools/` | `src/tools/README.md` | Before adding built-in tools, WASM tools, or MCP |

**Update both sides**: When changing behavior, update spec AND code. If spec and code disagree, fix spec first (or explicitly mark spec as outdated), then fix code.

---

## Debugging

```bash
# All ironclaw logs
RUST_LOG=ironclaw=debug cargo run

# Specific module
RUST_LOG=ironclaw::agent=debug cargo run
RUST_LOG=ironclaw::llm=trace cargo run
RUST_LOG=ironclaw::tools=debug cargo run
RUST_LOG=ironclaw::safety=debug cargo run

# With HTTP request logging
RUST_LOG=ironclaw=debug,tower_http=debug cargo run

# Service mode (launchd/systemd)
tail -f ~/.ironclaw/logs/stdout.log
tail -f ~/.ironclaw/logs/stderr.log

# Gateway health check
curl -H "Authorization: Bearer $GATEWAY_AUTH_TOKEN" http://127.0.0.1:3002/api/health

# Check job state in DB (libSQL)
sqlite3 ~/.ironclaw/ironclaw.db "SELECT id, status, created_at FROM agent_jobs ORDER BY created_at DESC LIMIT 10;"
```

---

*Source: IronClaw v0.9.0 · Docs: github.com/mudrii/ironclaw-docs · Generated: 2026-02-21*
