# IronClaw Codebase Analysis — Configuration System

> Updated: 2026-02-24 | Version: v0.11.1

## 1. Overview

IronClaw's configuration system is built around a layered priority model where
environment variables always win. The entry points are `Config::from_env()` for
early startup (before the database is available) and `Config::from_db()` for
normal runtime operation.

At the lowest level, `~/.ironclaw/.env` stores the database connection string and
other bootstrap variables. This file is loaded by `bootstrap::load_ironclaw_env()`
using dotenvy, which never overwrites variables already present in the process
environment. A standard `./.env` in the current working directory is loaded first
(also via dotenvy), so it takes priority over `~/.ironclaw/.env`.

On top of the two `.env` files, the operator can place a TOML config file at
`~/.ironclaw/config.toml` (or pass an explicit path via `--config`). TOML values
win over database settings and disk-based `settings.json`, but lose to env vars.

All config structs are built at startup in `Config::build()` inside `config/mod.rs`.
Each sub-module exposes a `resolve()` function that reads env vars via `optional_env()`
(which checks real env vars and then the injected-secrets overlay) and falls back
to `Settings` values loaded from the database or disk.

The `inject_llm_keys_from_secrets()` function bridges the encrypted secrets store
with the env-var-first resolution: API keys saved during onboarding are loaded into
a `INJECTED_VARS` overlay so they are visible to `optional_env()` without unsafe
`set_var` calls. Explicitly set env vars always win over injected secrets.

## 2. Configuration Priority

Priority order (highest to lowest):

1. Shell environment variables set before running IronClaw
2. `./.env` in the current working directory (loaded via dotenvy)
3. `~/.ironclaw/.env` — IronClaw-specific bootstrap file (loaded by `bootstrap.rs`)
4. `~/.ironclaw/config.toml` — TOML config file overlay (optional)
5. Database settings table (key-value pairs stored per user)
6. `~/.ironclaw/settings.json` — legacy disk fallback (read-only on existing installs)
7. Compiled-in defaults

The `~/.ironclaw/.env` file is the recommended place for operators to put stable
secrets (database URL, API keys). Its permissions are set to `0o600` on Unix.

## 3. Environment Variable Reference

The table below covers every env var found in the source. "Required" means the
process will exit with `ConfigError::MissingRequired` if the var is absent and no
default is possible.

| Env Var | Type | Default | Required | Description |
|---------|------|---------|----------|-------------|
| **Database** | | | | |
| `DATABASE_BACKEND` | string | `postgres` | No | `postgres` (or `pg`, `postgresql`) or `libsql` (or `turso`, `sqlite`) |
| `DATABASE_URL` | secret | — | If postgres | PostgreSQL connection string (e.g. `postgres://user:pass@host/db`) |
| `DATABASE_POOL_SIZE` | u32 | `10` | No | PostgreSQL connection pool size |
| `LIBSQL_PATH` | path | `~/.ironclaw/ironclaw.db` | No | Path to local libSQL/SQLite database file |
| `LIBSQL_URL` | string | — | No | Turso cloud URL for remote sync (e.g. `libsql://xxx.turso.io`) |
| `LIBSQL_AUTH_TOKEN` | secret | — | If `LIBSQL_URL` is set | Turso authentication token |
| **LLM Backend** | | | | |
| `LLM_BACKEND` | string | `nearai` | No | `nearai`, `openai`, `anthropic`, `ollama`, `openai_compatible`, or `tinfoil` |
| `NEARAI_MODEL` | string | `fireworks::accounts/fireworks/models/llama4-maverick-instruct-basic` | No | Model name for NEAR AI |
| `NEARAI_CHEAP_MODEL` | string | — | No | Optional model name for SmartRoutingProvider cheap-model path (e.g., `claude-haiku-4-20250514`). Added v0.10.0. |
| `NEARAI_BASE_URL` | string | `https://private.near.ai` (Responses) or `https://cloud-api.near.ai` (Chat) | No | NEAR AI API base URL |
| `NEARAI_AUTH_URL` | string | `https://private.near.ai` | No | NEAR AI auth/refresh endpoint base URL |
| `NEARAI_SESSION_PATH` | path | `~/.ironclaw/session.json` | No | Path to NEAR AI session token file |
| `NEARAI_API_KEY` | secret | — | No | API key for NEAR AI Cloud (Chat Completions mode) |
| `NEARAI_SESSION_TOKEN` | secret | — | No | Optional session token env override used by session manager |
| `NEARAI_FALLBACK_MODEL` | string | — | No | Optional secondary model for failover |
| `NEARAI_MAX_RETRIES` | u32 | `3` | No | Maximum retries for transient NEAR AI errors |
| `OPENAI_API_KEY` | secret | — | If `LLM_BACKEND=openai` | OpenAI API key |
| `OPENAI_MODEL` | string | `gpt-4o` | No | OpenAI model name |
| `OPENAI_BASE_URL` | string | — | No | Optional base URL override for OpenAI (e.g. proxies) |
| `ANTHROPIC_API_KEY` | secret | — | If `LLM_BACKEND=anthropic` | Anthropic API key |
| `ANTHROPIC_MODEL` | string | `claude-sonnet-4-20250514` | No | Anthropic model name |
| `ANTHROPIC_BASE_URL` | string | — | No | Optional base URL override for Anthropic |
| `OLLAMA_BASE_URL` | string | `http://localhost:11434` | No | Ollama server base URL (used for LLM and embeddings) |
| `OLLAMA_MODEL` | string | `llama3` | No | Ollama model name |
| `LLM_BASE_URL` | string | — | If `LLM_BACKEND=openai_compatible` | Base URL for OpenAI-compatible endpoint |
| `LLM_API_KEY` | secret | — | No | API key for OpenAI-compatible endpoint |
| `LLM_MODEL` | string | `default` | No | Model name for OpenAI-compatible endpoint |
| `LLM_EXTRA_HEADERS` | string | — | No | Comma-separated `Key:Value` HTTP headers injected into OpenAI-compatible provider requests. Example: `"HTTP-Referer:https://myapp.com,X-Title:MyApp"`. Added v0.10.0. |
| `TINFOIL_API_KEY` | secret | — | If `LLM_BACKEND=tinfoil` | Tinfoil private inference API key |
| `TINFOIL_MODEL` | string | `kimi-k2-5` | No | Tinfoil model name |
| **LLM Resilience** | | | | |
| `CIRCUIT_BREAKER_THRESHOLD` | u32 | disabled | No | Consecutive failures before circuit breaker opens. Omit to disable |
| `CIRCUIT_BREAKER_RECOVERY_SECS` | u64 | `30` | No | Seconds before circuit allows a probe after opening |
| `RESPONSE_CACHE_ENABLED` | bool | `false` | No | Enable in-memory LLM response cache within a session |
| `RESPONSE_CACHE_TTL_SECS` | u64 | `3600` | No | TTL in seconds for cached responses |
| `RESPONSE_CACHE_MAX_ENTRIES` | usize | `1000` | No | Max cached responses before LRU eviction |
| `LLM_FAILOVER_COOLDOWN_SECS` | u64 | `300` | No | Seconds a failed provider stays in cooldown |
| `LLM_FAILOVER_THRESHOLD` | u32 | `3` | No | Consecutive retryable failures before provider enters cooldown |
| `SMART_ROUTING_CASCADE` | bool | `true` | No | When enabled, uncertain responses from the cheap model trigger escalation to the primary model. Added v0.10.0. |
| **Embeddings** | | | | |
| `EMBEDDING_ENABLED` | bool | `false` | No | Enable vector embeddings for semantic memory search |
| `EMBEDDING_PROVIDER` | string | `nearai` | No | Provider: `openai`, `nearai`, or `ollama` |
| `EMBEDDING_MODEL` | string | `text-embedding-3-small` | No | Embedding model name |
| `EMBEDDING_DIMENSION` | usize | inferred | No | Vector dimension. Inferred from model name if unset (1536 for `text-embedding-3-small`, 3072 for `text-embedding-3-large`, 768 for `nomic-embed-text`, 1024 for `mxbai-embed-large`, 384 for `all-minilm`) |
| **Agent** | | | | |
| `AGENT_NAME` | string | `ironclaw` | No | Agent display name |
| `AGENT_MAX_PARALLEL_JOBS` | usize | `5` | No | Maximum concurrent jobs |
| `AGENT_JOB_TIMEOUT_SECS` | u64 | `3600` | No | Per-job hard timeout in seconds |
| `AGENT_MAX_TOOL_ITERATIONS` | usize | `50` | No | Maximum tool-call iterations per agent loop invocation |
| `AGENT_AUTO_APPROVE_TOOLS` | bool | `false` | No | Skip tool approval checks entirely (for benchmarks/CI) |
| `AGENT_STUCK_THRESHOLD_SECS` | u64 | `300` | No | Seconds without progress before a job is considered stuck |
| `AGENT_USE_PLANNING` | bool | `true` | No | Whether the agent uses planning before tool execution |
| `SESSION_IDLE_TIMEOUT_SECS` | u64 | `604800` (7 days) | No | Sessions idle longer than this are pruned from memory |
| `ALLOW_LOCAL_TOOLS` | bool | `false` | No | Allow chat to use filesystem/shell tools directly (bypasses sandbox) |
| `MAX_COST_PER_DAY_CENTS` | u64 | unlimited | No | Maximum daily LLM spend in cents (e.g. `10000` = $100). Unset = no limit |
| `MAX_ACTIONS_PER_HOUR` | u64 | unlimited | No | Maximum LLM/tool actions per hour. Unset = no limit |
| **Self-Repair** | | | | |
| `SELF_REPAIR_CHECK_INTERVAL_SECS` | u64 | `60` | No | How often (seconds) to check for stuck jobs |
| `SELF_REPAIR_MAX_ATTEMPTS` | u32 | `3` | No | Maximum repair attempts before marking a job failed |
| **Channels: CLI** | | | | |
| `CLI_ENABLED` | bool | `true` | No | Enable the REPL/TUI channel. Set `false` when running headless (systemd/launchd) |
| **Channels: HTTP Webhook** | | | | |
| `HTTP_HOST` | string | `0.0.0.0` | No | HTTP webhook listen address. Setting this or `HTTP_PORT` enables the channel |
| `HTTP_PORT` | u16 | `8080` | No | HTTP webhook listen port |
| `HTTP_WEBHOOK_SECRET` | secret | — | No | Shared secret for validating webhook signatures |
| `HTTP_USER_ID` | string | `http` | No | User ID assigned to messages from the HTTP channel |
| **Channels: Web Gateway** | | | | |
| `GATEWAY_ENABLED` | bool | `true` | No | Enable the web gateway (browser UI + REST API). Defaults to enabled |
| `GATEWAY_HOST` | string | `127.0.0.1` | No | Gateway listen address |
| `GATEWAY_PORT` | u16 | `3000` | No | Gateway listen port |
| `GATEWAY_AUTH_TOKEN` | string | — | No | Bearer token for gateway API authentication. A random token is generated at startup if unset |
| `GATEWAY_USER_ID` | string | `default` | No | User ID for messages arriving via gateway |
| **Channels: WASM Channels** | | | | |
| `WASM_CHANNELS_DIR` | path | `~/.ironclaw/channels/` | No | Directory containing WASM channel modules (Telegram, Slack, etc.) |
| `WASM_CHANNELS_ENABLED` | bool | `true` | No | Enable WASM channel modules |
| `TELEGRAM_OWNER_ID` | i64 | — | No | Telegram user ID. When set, bot only responds to this user |
| **Tunnel** | | | | |
| `TUNNEL_URL` | string | — | No | Static public HTTPS URL (externally managed tunnel). Must start with `https://` |
| `TUNNEL_PROVIDER` | string | — | No | Managed tunnel provider: `ngrok`, `cloudflare`, `tailscale`, or `custom` |
| `TUNNEL_CF_TOKEN` | secret | — | If provider=cloudflare | Cloudflare tunnel token |
| `TUNNEL_NGROK_TOKEN` | secret | — | If provider=ngrok | ngrok auth token |
| `TUNNEL_NGROK_DOMAIN` | string | — | No | ngrok custom domain (paid plans) |
| `TUNNEL_TS_FUNNEL` | bool | `false` | No | Use Tailscale Funnel (public) instead of Serve (tailnet-only) |
| `TUNNEL_TS_HOSTNAME` | string | — | No | Tailscale hostname override |
| `TUNNEL_CUSTOM_COMMAND` | string | — | If provider=custom | Shell command to start custom tunnel (supports `{port}` and `{host}` placeholders) |
| `TUNNEL_CUSTOM_HEALTH_URL` | string | — | No | Health check URL for custom tunnel |
| `TUNNEL_CUSTOM_URL_PATTERN` | string | — | No | Substring pattern for extracting tunnel URL from stdout |
| **Docker Sandbox** | | | | |
| `SANDBOX_ENABLED` | bool | `true` | No | Enable Docker sandbox for job execution |
| `SANDBOX_POLICY` | string | `readonly` | No | `readonly`, `workspace_write`, or `full_access` |
| `SANDBOX_TIMEOUT_SECS` | u64 | `120` | No | Container execution timeout in seconds |
| `SANDBOX_MEMORY_LIMIT_MB` | u64 | `2048` | No | Container memory limit in megabytes |
| `SANDBOX_CPU_SHARES` | u32 | `1024` | No | CPU shares (relative weight, Docker `--cpu-shares`) |
| `SANDBOX_IMAGE` | string | `ironclaw-worker:latest` | No | Docker image for sandbox containers |
| `SANDBOX_AUTO_PULL` | bool | `true` | No | Automatically pull the image if not found locally |
| `SANDBOX_EXTRA_DOMAINS` | string | — | No | Comma-separated list of extra domains to allow through the network proxy |
| **Claude Code Sandbox** | | | | |
| `CLAUDE_CODE_ENABLED` | bool | `false` | No | Enable Claude Code mode (delegates jobs to `claude` CLI inside containers) |
| `CLAUDE_CONFIG_DIR` | path | `~/.claude` | No | Host directory for Claude auth config |
| `CLAUDE_CODE_MODEL` | string | `sonnet` | No | Claude model for Claude Code mode |
| `CLAUDE_CODE_MAX_TURNS` | u32 | `50` | No | Maximum agentic turns before stopping |
| `CLAUDE_CODE_MEMORY_LIMIT_MB` | u64 | `4096` | No | Memory limit in MB for Claude Code containers |
| `CLAUDE_CODE_ALLOWED_TOOLS` | string | see note | No | Comma-separated tool patterns auto-approved in containers. Default: `Read(*)`, `Write(*)`, `Edit(*)`, `Glob(*)`, `Grep(*)`, `NotebookEdit(*)`, `Bash(*)`, `Task(*)`, `WebFetch(*)`, `WebSearch(*)` |
| **WASM Tool Runtime** | | | | |
| `WASM_ENABLED` | bool | `true` | No | Enable WASM tool execution (wasmtime sandbox) |
| `WASM_TOOLS_DIR` | path | `~/.ironclaw/tools/` | No | Directory containing installed WASM tools |
| `WASM_DEFAULT_MEMORY_LIMIT` | u64 | `10485760` (10 MB) | No | Default memory limit in bytes per WASM module |
| `WASM_DEFAULT_TIMEOUT_SECS` | u64 | `60` | No | Default execution timeout per WASM tool call |
| `WASM_DEFAULT_FUEL_LIMIT` | u64 | `10000000` | No | Default fuel (CPU instruction budget) per WASM call |
| `WASM_CACHE_COMPILED` | bool | `true` | No | Cache compiled WASM modules to disk |
| `WASM_CACHE_DIR` | path | — | No | Directory for compiled module cache. Defaults to a system temp path |
| **Heartbeat** | | | | |
| `HEARTBEAT_ENABLED` | bool | `false` | No | Enable proactive periodic heartbeat execution |
| `HEARTBEAT_INTERVAL_SECS` | u64 | `1800` (30 min) | No | Interval between heartbeat checks |
| `HEARTBEAT_NOTIFY_CHANNEL` | string | — | No | Channel name to notify on heartbeat findings (e.g. `tui`, `gateway`) |
| `HEARTBEAT_NOTIFY_USER` | string | — | No | User ID to notify on heartbeat findings |
| **Memory Hygiene** | | | | |
| `MEMORY_HYGIENE_ENABLED` | bool | `true` | No | Enable automatic cleanup of stale workspace documents |
| `MEMORY_HYGIENE_RETENTION_DAYS` | u32 | `30` | No | Days before `daily/` documents are deleted |
| `MEMORY_HYGIENE_CADENCE_HOURS` | u32 | `12` | No | Minimum hours between hygiene passes |
| **Routines** | | | | |
| `ROUTINES_ENABLED` | bool | `true` | No | Enable the scheduled/reactive routines system |
| `ROUTINES_CRON_INTERVAL` | u64 | `15` | No | How often (seconds) to poll for cron routines needing execution |
| `ROUTINES_MAX_CONCURRENT` | usize | `10` | No | Maximum routines executing concurrently |
| `ROUTINES_DEFAULT_COOLDOWN` | u64 | `300` | No | Default cooldown in seconds between routine firings |
| `ROUTINES_MAX_TOKENS` | u32 | `4096` | No | Max output tokens for lightweight routine LLM calls |
| **Skills** | | | | |
| `SKILLS_ENABLED` | bool | `false` | No | Enable the SKILL.md prompt extension system |
| `SKILLS_DIR` | path | `~/.ironclaw/skills/` | No | Directory containing local (trusted) skills |
| `SKILLS_MAX_ACTIVE` | usize | `3` | No | Maximum skills active simultaneously |
| `SKILLS_MAX_CONTEXT_TOKENS` | usize | `4000` | No | Maximum total context tokens allocated to skill prompts |
| **Safety** | | | | |
| `SAFETY_MAX_OUTPUT_LENGTH` | usize | `100000` | No | Maximum bytes allowed in tool output before truncation |
| `SAFETY_INJECTION_CHECK_ENABLED` | bool | `true` | No | Enable prompt injection detection on tool output and LLM responses |
| **Secrets** | | | | |
| `SECRETS_MASTER_KEY` | secret | — | No | AES-256-GCM master key for encrypting stored secrets. Minimum 32 bytes. Falls back to OS keychain if unset |
| **Builder** | | | | |
| `BUILDER_ENABLED` | bool | `true` | No | Enable the dynamic WASM tool builder |
| `BUILDER_DIR` | path | system temp | No | Directory for build artifacts |
| `BUILDER_MAX_ITERATIONS` | u32 | `20` | No | Maximum iterations in the build-test-fix loop |
| `BUILDER_TIMEOUT_SECS` | u64 | `600` | No | Build operation timeout in seconds |
| `BUILDER_AUTO_REGISTER` | bool | `true` | No | Automatically register successfully built WASM tools |
| **Observability** | | | | |
| `OBSERVABILITY_BACKEND` | string | `none` | No | Observability/tracing backend. Currently `none` or custom value |
| `RUST_LOG` | string | `info` | No | Log filter in tracing/env-logger format (e.g. `ironclaw=debug,tower_http=info`) |

## 4. Configuration Structs

### Config (`config/mod.rs`)

The root configuration struct assembled at startup.

```rust
pub struct Config {
    pub database: DatabaseConfig,
    pub llm: LlmConfig,
    pub embeddings: EmbeddingsConfig,
    pub tunnel: TunnelConfig,
    pub channels: ChannelsConfig,
    pub agent: AgentConfig,
    pub safety: SafetyConfig,
    pub wasm: WasmConfig,
    pub secrets: SecretsConfig,
    pub builder: BuilderModeConfig,
    pub heartbeat: HeartbeatConfig,
    pub hygiene: HygieneConfig,
    pub routines: RoutineConfig,
    pub sandbox: SandboxModeConfig,
    pub claude_code: ClaudeCodeConfig,
    pub skills: SkillsConfig,
    pub observability: ObservabilityConfig,
}
```

### AgentConfig (`config/agent.rs`)

Controls job scheduling, timeouts, cost limiting, and self-repair behavior.

```rust
pub struct AgentConfig {
    pub name: String,                        // AGENT_NAME
    pub max_parallel_jobs: usize,            // AGENT_MAX_PARALLEL_JOBS
    pub job_timeout: Duration,               // AGENT_JOB_TIMEOUT_SECS
    pub stuck_threshold: Duration,           // AGENT_STUCK_THRESHOLD_SECS
    pub repair_check_interval: Duration,     // SELF_REPAIR_CHECK_INTERVAL_SECS
    pub max_repair_attempts: u32,            // SELF_REPAIR_MAX_ATTEMPTS
    pub use_planning: bool,                  // AGENT_USE_PLANNING
    pub session_idle_timeout: Duration,      // SESSION_IDLE_TIMEOUT_SECS
    pub allow_local_tools: bool,             // ALLOW_LOCAL_TOOLS
    pub max_cost_per_day_cents: Option<u64>, // MAX_COST_PER_DAY_CENTS
    pub max_actions_per_hour: Option<u64>,   // MAX_ACTIONS_PER_HOUR
}
```

### LlmConfig (`config/llm.rs`)

Multi-provider LLM configuration. Only the sub-struct matching the active backend
is populated; the others are `None`.

```rust
pub enum LlmBackend {
    NearAi,           // "nearai" (default)
    OpenAi,           // "openai"
    Anthropic,        // "anthropic" or "claude"
    Ollama,           // "ollama"
    OpenAiCompatible, // "openai_compatible" or "compatible"
    Tinfoil,          // "tinfoil"
}

pub enum NearAiApiMode {
    Responses,        // NEAR AI Chat (Responses API, session token auth) — default
    ChatCompletions,  // NEAR AI Cloud (Chat Completions API, API key auth)
}

pub struct NearAiConfig {
    pub model: String,
    pub cheap_model: Option<String>,
    pub base_url: String,
    pub auth_base_url: String,
    pub session_path: PathBuf,
    pub api_mode: NearAiApiMode,
    pub api_key: Option<SecretString>,
    pub fallback_model: Option<String>,
    pub max_retries: u32,
    pub circuit_breaker_threshold: Option<u32>,
    pub circuit_breaker_recovery_secs: u64,
    pub response_cache_enabled: bool,
    pub response_cache_ttl_secs: u64,
    pub response_cache_max_entries: usize,
    pub failover_cooldown_secs: u64,
    pub failover_cooldown_threshold: u32,
}

pub struct LlmConfig {
    pub backend: LlmBackend,
    pub nearai: NearAiConfig,                           // always populated
    pub openai: Option<OpenAiDirectConfig>,             // Some when backend=openai
    pub anthropic: Option<AnthropicDirectConfig>,       // Some when backend=anthropic
    pub ollama: Option<OllamaConfig>,                   // Some when backend=ollama
    pub openai_compatible: Option<OpenAiCompatibleConfig>, // Some when backend=openai_compatible
    pub tinfoil: Option<TinfoilConfig>,                 // Some when backend=tinfoil
}

pub struct OpenAiDirectConfig {
    pub api_key: SecretString,   // OPENAI_API_KEY
    pub model: String,           // OPENAI_MODEL
    pub base_url: Option<String>, // OPENAI_BASE_URL
}

pub struct AnthropicDirectConfig {
    pub api_key: SecretString,   // ANTHROPIC_API_KEY
    pub model: String,           // ANTHROPIC_MODEL
    pub base_url: Option<String>, // ANTHROPIC_BASE_URL
}

pub struct OllamaConfig {
    pub base_url: String, // OLLAMA_BASE_URL
    pub model: String,    // OLLAMA_MODEL
}

pub struct OpenAiCompatibleConfig {
    pub base_url: String,         // LLM_BASE_URL (required)
    pub api_key: Option<SecretString>, // LLM_API_KEY
    pub model: String,            // LLM_MODEL
}

pub struct TinfoilConfig {
    pub api_key: SecretString, // TINFOIL_API_KEY
    pub model: String,         // TINFOIL_MODEL
}
```

### DatabaseConfig (`config/database.rs`)

```rust
pub enum DatabaseBackend {
    Postgres, // default — "postgres", "pg", "postgresql"
    LibSql,   // "libsql", "turso", "sqlite"
}

pub struct DatabaseConfig {
    pub backend: DatabaseBackend,      // DATABASE_BACKEND
    pub url: SecretString,             // DATABASE_URL (required for postgres)
    pub pool_size: usize,              // DATABASE_POOL_SIZE (default: 10)
    pub libsql_path: Option<PathBuf>, // LIBSQL_PATH (default: ~/.ironclaw/ironclaw.db)
    pub libsql_url: Option<String>,   // LIBSQL_URL (Turso cloud, optional)
    pub libsql_auth_token: Option<SecretString>, // LIBSQL_AUTH_TOKEN (required with libsql_url)
}
```

### ChannelsConfig (`config/channels.rs`)

```rust
pub struct ChannelsConfig {
    pub cli: CliConfig,
    pub http: Option<HttpConfig>,        // Some when HTTP_HOST or HTTP_PORT is set
    pub gateway: Option<GatewayConfig>,  // Some when GATEWAY_ENABLED != false
    pub wasm_channels_dir: PathBuf,      // WASM_CHANNELS_DIR
    pub wasm_channels_enabled: bool,     // WASM_CHANNELS_ENABLED
    pub telegram_owner_id: Option<i64>,  // TELEGRAM_OWNER_ID
}

pub struct CliConfig {
    pub enabled: bool, // CLI_ENABLED
}

pub struct HttpConfig {
    pub host: String,                     // HTTP_HOST
    pub port: u16,                        // HTTP_PORT
    pub webhook_secret: Option<SecretString>, // HTTP_WEBHOOK_SECRET
    pub user_id: String,                  // HTTP_USER_ID
}

pub struct GatewayConfig {
    pub host: String,             // GATEWAY_HOST
    pub port: u16,                // GATEWAY_PORT
    pub auth_token: Option<String>, // GATEWAY_AUTH_TOKEN
    pub user_id: String,          // GATEWAY_USER_ID
}
```

Note: `GATEWAY_ENABLED` defaults to `true`, meaning the gateway is on by default.
Set `GATEWAY_ENABLED=false` to disable it entirely.

### EmbeddingsConfig (`config/embeddings.rs`)

```rust
pub struct EmbeddingsConfig {
    pub enabled: bool,                      // EMBEDDING_ENABLED
    pub provider: String,                   // EMBEDDING_PROVIDER ("openai", "nearai", "ollama")
    pub openai_api_key: Option<SecretString>, // OPENAI_API_KEY (shared with LLM)
    pub model: String,                      // EMBEDDING_MODEL
    pub ollama_base_url: String,            // OLLAMA_BASE_URL
    pub dimension: usize,                   // EMBEDDING_DIMENSION (inferred from model)
}
```

Known dimension inference (used when `EMBEDDING_DIMENSION` is not set):

| Model | Dimension |
|-------|-----------|
| `text-embedding-3-small` | 1536 |
| `text-embedding-3-large` | 3072 |
| `text-embedding-ada-002` | 1536 |
| `nomic-embed-text` | 768 |
| `mxbai-embed-large` | 1024 |
| `all-minilm` | 384 |
| (unknown) | 1536 |

### HeartbeatConfig (`config/heartbeat.rs`)

```rust
pub struct HeartbeatConfig {
    pub enabled: bool,                   // HEARTBEAT_ENABLED (default: false)
    pub interval_secs: u64,              // HEARTBEAT_INTERVAL_SECS (default: 1800)
    pub notify_channel: Option<String>,  // HEARTBEAT_NOTIFY_CHANNEL
    pub notify_user: Option<String>,     // HEARTBEAT_NOTIFY_USER
}
```

### HygieneConfig (`config/hygiene.rs`)

Controls automatic cleanup of stale `daily/` workspace documents.

```rust
pub struct HygieneConfig {
    pub enabled: bool,        // MEMORY_HYGIENE_ENABLED (default: true)
    pub retention_days: u32,  // MEMORY_HYGIENE_RETENTION_DAYS (default: 30)
    pub cadence_hours: u32,   // MEMORY_HYGIENE_CADENCE_HOURS (default: 12)
}
```

The state directory for tracking last-run timestamps is fixed at `~/.ironclaw/`.

### RoutineConfig (`config/routines.rs`)

```rust
pub struct RoutineConfig {
    pub enabled: bool,                    // ROUTINES_ENABLED (default: true)
    pub cron_check_interval_secs: u64,    // ROUTINES_CRON_INTERVAL (default: 15)
    pub max_concurrent_routines: usize,   // ROUTINES_MAX_CONCURRENT (default: 10)
    pub default_cooldown_secs: u64,       // ROUTINES_DEFAULT_COOLDOWN (default: 300)
    pub max_lightweight_tokens: u32,      // ROUTINES_MAX_TOKENS (default: 4096)
}
```

### SafetyConfig (`config/safety.rs`)

```rust
pub struct SafetyConfig {
    pub max_output_length: usize,       // SAFETY_MAX_OUTPUT_LENGTH (default: 100000)
    pub injection_check_enabled: bool,  // SAFETY_INJECTION_CHECK_ENABLED (default: true)
}
```

### WasmConfig (`config/wasm.rs`)

Controls the wasmtime-based tool sandbox.

```rust
pub struct WasmConfig {
    pub enabled: bool,               // WASM_ENABLED (default: true)
    pub tools_dir: PathBuf,          // WASM_TOOLS_DIR (default: ~/.ironclaw/tools/)
    pub default_memory_limit: u64,   // WASM_DEFAULT_MEMORY_LIMIT (default: 10485760)
    pub default_timeout_secs: u64,   // WASM_DEFAULT_TIMEOUT_SECS (default: 60)
    pub default_fuel_limit: u64,     // WASM_DEFAULT_FUEL_LIMIT (default: 10000000)
    pub cache_compiled: bool,        // WASM_CACHE_COMPILED (default: true)
    pub cache_dir: Option<PathBuf>,  // WASM_CACHE_DIR
}
```

### SandboxModeConfig (`config/sandbox.rs`)

Controls the Docker-based execution sandbox and its network proxy.

```rust
pub struct SandboxModeConfig {
    pub enabled: bool,                        // SANDBOX_ENABLED (default: true)
    pub policy: String,                       // SANDBOX_POLICY (default: "readonly")
    pub timeout_secs: u64,                    // SANDBOX_TIMEOUT_SECS (default: 120)
    pub memory_limit_mb: u64,                 // SANDBOX_MEMORY_LIMIT_MB (default: 2048)
    pub cpu_shares: u32,                      // SANDBOX_CPU_SHARES (default: 1024)
    pub image: String,                        // SANDBOX_IMAGE (default: "ironclaw-worker:latest")
    pub auto_pull_image: bool,                // SANDBOX_AUTO_PULL (default: true)
    pub extra_allowed_domains: Vec<String>,   // SANDBOX_EXTRA_DOMAINS (comma-separated)
}
```

Valid policy values: `readonly` (no writes, allowlisted network), `workspace_write`
(read-write workspace mount, allowlisted network), `full_access` (unrestricted).

### ClaudeCodeConfig (`config/sandbox.rs`)

```rust
pub struct ClaudeCodeConfig {
    pub enabled: bool,              // CLAUDE_CODE_ENABLED (default: false)
    pub config_dir: PathBuf,        // CLAUDE_CONFIG_DIR (default: ~/.claude)
    pub model: String,              // CLAUDE_CODE_MODEL (default: "sonnet")
    pub max_turns: u32,             // CLAUDE_CODE_MAX_TURNS (default: 50)
    pub memory_limit_mb: u64,       // CLAUDE_CODE_MEMORY_LIMIT_MB (default: 4096)
    pub allowed_tools: Vec<String>, // CLAUDE_CODE_ALLOWED_TOOLS (comma-separated)
}
```

### SkillsConfig (`config/skills.rs`)

```rust
pub struct SkillsConfig {
    pub enabled: bool,              // SKILLS_ENABLED (default: false)
    pub local_dir: PathBuf,         // SKILLS_DIR (default: ~/.ironclaw/skills/)
    pub max_active_skills: usize,   // SKILLS_MAX_ACTIVE (default: 3)
    pub max_context_tokens: usize,  // SKILLS_MAX_CONTEXT_TOKENS (default: 4000)
}
```

### SecretsConfig (`config/secrets.rs`)

```rust
pub struct SecretsConfig {
    pub master_key: Option<SecretString>, // SECRETS_MASTER_KEY or OS keychain
    pub enabled: bool,                    // true when master_key is Some
    pub source: KeySource,                // Env, Keychain, or None
}
```

### BuilderModeConfig (`config/builder.rs`)

```rust
pub struct BuilderModeConfig {
    pub enabled: bool,          // BUILDER_ENABLED (default: true)
    pub build_dir: Option<PathBuf>, // BUILDER_DIR (default: system temp)
    pub max_iterations: u32,    // BUILDER_MAX_ITERATIONS (default: 20)
    pub timeout_secs: u64,      // BUILDER_TIMEOUT_SECS (default: 600)
    pub auto_register: bool,    // BUILDER_AUTO_REGISTER (default: true)
}
```

### TunnelConfig (`config/tunnel.rs`)

```rust
pub struct TunnelConfig {
    pub public_url: Option<String>,                   // TUNNEL_URL
    pub provider: Option<TunnelProviderConfig>,       // TUNNEL_PROVIDER + sub-vars
}
```

## 5. AppBuilder Pattern (`config/builder.rs`)

IronClaw does not use a traditional builder pattern for config assembly. Instead,
`Config::build()` (private, in `config/mod.rs`) calls each sub-module's `resolve()`
function in sequence and assembles the final `Config` struct. The loading sequence is:

```
startup
  └─ Config::from_env() or Config::from_db()
       ├─ dotenvy::dotenv()                        # load ./.env
       ├─ bootstrap::load_ironclaw_env()           # load ~/.ironclaw/.env
       ├─ Settings::load() or store.get_all_settings()  # disk or DB
       ├─ Settings::load_toml()                    # ~/.ironclaw/config.toml (optional)
       ├─ settings.merge_from(toml_settings)       # TOML wins over DB/disk
       └─ Config::build(&settings)
            ├─ DatabaseConfig::resolve()
            ├─ LlmConfig::resolve(&settings)
            ├─ EmbeddingsConfig::resolve(&settings)
            ├─ TunnelConfig::resolve(&settings)
            ├─ ChannelsConfig::resolve(&settings)
            ├─ AgentConfig::resolve(&settings)
            ├─ SafetyConfig::resolve()
            ├─ WasmConfig::resolve()
            ├─ SecretsConfig::resolve().await      # probes env then keychain
            ├─ BuilderModeConfig::resolve()
            ├─ HeartbeatConfig::resolve(&settings)
            ├─ HygieneConfig::resolve()
            ├─ RoutineConfig::resolve()
            ├─ SandboxModeConfig::resolve()
            ├─ ClaudeCodeConfig::resolve()
            ├─ SkillsConfig::resolve()
            └─ ObservabilityConfig { backend: env("OBSERVABILITY_BACKEND") }
```

After `Config` is built, the agent startup sequence (in `main.rs`) calls
`inject_llm_keys_from_secrets()` to load any API keys from the encrypted secrets
store into the `INJECTED_VARS` overlay. A second call to `Config::from_db()` then
picks up those keys without needing unsafe `set_var` calls.

## 6. Settings Store (`settings.rs`)

`Settings` is a Rust struct that represents user preferences persisted to disk
(`~/.ironclaw/settings.json`) or to the database `settings` table. It is the
bridge between runtime-adjustable values and the env-var-first config system.

The config system reads from `Settings` as a fallback when an env var is not set.
This means most fields can be changed at runtime via `ironclaw config set <path> <value>`
without restarting — the new value is stored in `Settings` and picked up on the
next `Config::from_db()` call (which happens at the start of each agent session).

Fields that require a full restart are those read only during early bootstrap
(before the DB is available), primarily:

| Setting | Stored in | Hot-reload? |
|---------|-----------|-------------|
| `DATABASE_URL` | `~/.ironclaw/.env` | No — restart required |
| `DATABASE_BACKEND` | `~/.ironclaw/.env` | No — restart required |
| `LIBSQL_PATH` / `LIBSQL_URL` | `~/.ironclaw/.env` | No — restart required |
| `LLM_BACKEND` | DB / settings.json | Yes — next session |
| `AGENT_MAX_PARALLEL_JOBS` | DB / settings.json | Yes — next session |
| `HEARTBEAT_ENABLED` | DB / settings.json | Yes — next session |
| All other settings | DB / settings.json | Yes — next session |

The `Settings` struct supports dotted-path get/set/reset operations used by the
`ironclaw config` CLI subcommand:

```
ironclaw config get agent.max_parallel_jobs
ironclaw config set heartbeat.enabled true
ironclaw config reset agent.name
ironclaw config list
```

Settings are also serializable to a TOML file via `Settings::save_toml()`, enabling
`ironclaw config init` to write an annotated `~/.ironclaw/config.toml` with all
current values.

### Settings Sections

| Struct | DB path prefix | Description |
|--------|---------------|-------------|
| `AgentSettings` | `agent.*` | Name, parallelism, timeouts, repair |
| `EmbeddingsSettings` | `embeddings.*` | Provider, model, enabled |
| `HeartbeatSettings` | `heartbeat.*` | Enabled, interval, notify targets |
| `ChannelSettings` | `channels.*` | HTTP, Telegram owner, WASM channels |
| `TunnelSettings` | `tunnel.*` | Public URL, provider, tokens |
| `WasmSettings` | `wasm.*` | WASM runtime limits |
| `SandboxSettings` | `sandbox.*` | Docker sandbox policy, limits |
| `SafetySettings` | `safety.*` | Output length, injection check |
| `BuilderSettings` | `builder.*` | Builder iterations, timeout |

## 7. Config Hygiene (`config/hygiene.rs`)

`HygieneConfig` controls the workspace memory cleanup subsystem. When enabled,
a background task scans `daily/` documents in the workspace and deletes entries
older than `retention_days`. The `cadence_hours` setting prevents the hygiene
pass from running more frequently than necessary.

The hygiene state (last-run timestamp) is written to `~/.ironclaw/` so it
persists across restarts. The actual cleanup operates on the database's
`memory_documents` and `memory_chunks` tables.

At startup, HygieneConfig is converted to a `workspace::hygiene::HygieneConfig`
struct (which adds the `state_dir` field pointing to `~/.ironclaw/`) and passed
to the workspace subsystem.

## 8. Sample .env File

The following is a complete annotated `~/.ironclaw/.env` covering every supported
env var. Copy and edit as needed; lines beginning with `#` are comments and are
ignored by dotenvy.

```bash
# ~/.ironclaw/.env — IronClaw configuration
# Format: KEY="VALUE" (double-quoted to handle special chars like # in passwords)
# Priority: shell env > ./.env > this file > config.toml > database > defaults

##############################################
# Database
##############################################
# Options: postgres (default), libsql (zero-dependency local mode)
DATABASE_BACKEND="libsql"

# PostgreSQL connection string (required when DATABASE_BACKEND=postgres)
# DATABASE_URL="postgresql://user:pass@localhost:5432/ironclaw"

# libSQL local file (default: ~/.ironclaw/ironclaw.db)
# LIBSQL_PATH="/home/user/.ironclaw/ironclaw.db"

# Turso cloud sync (optional, requires LIBSQL_AUTH_TOKEN)
# LIBSQL_URL="libsql://mydb-myorg.turso.io"
# LIBSQL_AUTH_TOKEN="eyJh..."

# PostgreSQL pool size (default: 10)
# DATABASE_POOL_SIZE="10"

##############################################
# LLM Backend
##############################################
# Options: nearai (default), openai, anthropic, ollama, openai_compatible, tinfoil
LLM_BACKEND="openai"

# OpenAI
OPENAI_API_KEY="sk-proj-..."
OPENAI_MODEL="gpt-4o"
# OPENAI_BASE_URL="https://openai-compatible-proxy.example.com/v1"

# Anthropic
# LLM_BACKEND="anthropic"
# ANTHROPIC_API_KEY="sk-ant-..."
# ANTHROPIC_MODEL="claude-sonnet-4-20250514"
# ANTHROPIC_BASE_URL="https://anthropic-proxy.example.com"

# NEAR AI Chat (Responses API — session token auth, default mode)
# LLM_BACKEND="nearai"
# NEARAI_MODEL="fireworks::accounts/fireworks/models/llama4-maverick-instruct-basic"
# NEARAI_SESSION_PATH="~/.ironclaw/session.json"
# NEARAI_BASE_URL="https://private.near.ai"
# NEARAI_AUTH_URL="https://private.near.ai"
# NEARAI_CHEAP_MODEL="some/cheap-model"
# NEARAI_FALLBACK_MODEL="backup/model"

# NEAR AI Cloud (Chat Completions API — API key auth)
# LLM_BACKEND="nearai"
# NEARAI_API_KEY="your-nearai-api-key"
# NEARAI_BASE_URL="https://cloud-api.near.ai"

# Local Ollama
# LLM_BACKEND="ollama"
# OLLAMA_BASE_URL="http://localhost:11434"
# OLLAMA_MODEL="llama3"

# OpenAI-compatible endpoint (vLLM, LiteLLM, Together, OpenRouter, etc.)
# LLM_BACKEND="openai_compatible"
# LLM_BASE_URL="https://api.together.xyz/v1"
# LLM_API_KEY="your-api-key"
# LLM_MODEL="meta-llama/Llama-3-70b-chat-hf"

# Tinfoil private inference (hardware-attested TEE)
# LLM_BACKEND="tinfoil"
# TINFOIL_API_KEY="your-tinfoil-key"
# TINFOIL_MODEL="kimi-k2-5"

##############################################
# LLM Resilience (optional, advanced)
##############################################
# Circuit breaker: open after N consecutive failures
# CIRCUIT_BREAKER_THRESHOLD="5"
# CIRCUIT_BREAKER_RECOVERY_SECS="30"

# In-memory response cache (saves tokens on repeated prompts)
# RESPONSE_CACHE_ENABLED="false"
# RESPONSE_CACHE_TTL_SECS="3600"
# RESPONSE_CACHE_MAX_ENTRIES="1000"

# Failover provider cooldown
# LLM_FAILOVER_COOLDOWN_SECS="300"
# LLM_FAILOVER_THRESHOLD="3"
# NEARAI_MAX_RETRIES="3"

##############################################
# Embeddings (Semantic Memory)
##############################################
EMBEDDING_ENABLED="true"
EMBEDDING_PROVIDER="openai"
EMBEDDING_MODEL="text-embedding-3-small"
# EMBEDDING_DIMENSION="1536"

# For Ollama embeddings:
# EMBEDDING_PROVIDER="ollama"
# OLLAMA_BASE_URL="http://localhost:11434"
# EMBEDDING_MODEL="nomic-embed-text"

##############################################
# Agent
##############################################
AGENT_NAME="ironclaw"
# AGENT_MAX_PARALLEL_JOBS="5"
# AGENT_JOB_TIMEOUT_SECS="3600"
# AGENT_STUCK_THRESHOLD_SECS="300"
# AGENT_USE_PLANNING="true"
# SESSION_IDLE_TIMEOUT_SECS="604800"

# Cost/rate limiting (unset = unlimited)
# MAX_COST_PER_DAY_CENTS="10000"   # $100/day
# MAX_ACTIONS_PER_HOUR="500"

# Self-repair
# SELF_REPAIR_CHECK_INTERVAL_SECS="60"
# SELF_REPAIR_MAX_ATTEMPTS="3"

# Allow chat to bypass sandbox (use with caution)
# ALLOW_LOCAL_TOOLS="false"

##############################################
# Web Gateway (browser UI)
##############################################
# GATEWAY_ENABLED defaults to true — set false to disable entirely
# GATEWAY_ENABLED="true"
GATEWAY_HOST="127.0.0.1"
GATEWAY_PORT="3000"
# Generate with: openssl rand -hex 32
GATEWAY_AUTH_TOKEN="replace-with-random-hex-token"
# GATEWAY_USER_ID="default"

##############################################
# HTTP Webhook Channel
##############################################
# Only enabled when HTTP_HOST or HTTP_PORT is set
# HTTP_HOST="0.0.0.0"
# HTTP_PORT="8080"
# HTTP_WEBHOOK_SECRET="shared-webhook-secret"
# HTTP_USER_ID="http"

##############################################
# WASM Channels (Telegram, Slack, etc.)
##############################################
# WASM_CHANNELS_ENABLED="true"
# WASM_CHANNELS_DIR="~/.ironclaw/channels/"
# TELEGRAM_OWNER_ID="123456789"

##############################################
# Tunnel (for public webhook endpoints)
##############################################
# Static URL (you manage the tunnel externally):
# TUNNEL_URL="https://abc123.ngrok.io"

# Managed ngrok tunnel:
# TUNNEL_PROVIDER="ngrok"
# TUNNEL_NGROK_TOKEN="ngrok-auth-token"
# TUNNEL_NGROK_DOMAIN="my.ngrok.dev"

# Managed Cloudflare tunnel:
# TUNNEL_PROVIDER="cloudflare"
# TUNNEL_CF_TOKEN="cloudflare-tunnel-token"

# Tailscale:
# TUNNEL_PROVIDER="tailscale"
# TUNNEL_TS_FUNNEL="true"
# TUNNEL_TS_HOSTNAME="ironclaw"

##############################################
# Channels: CLI/TUI
##############################################
# Set false when running as a background service (launchd/systemd)
# CLI_ENABLED="false"

##############################################
# Docker Sandbox
##############################################
# SANDBOX_ENABLED="true"
# SANDBOX_POLICY="readonly"           # readonly, workspace_write, full_access
# SANDBOX_IMAGE="ironclaw-worker:latest"
# SANDBOX_TIMEOUT_SECS="120"
# SANDBOX_MEMORY_LIMIT_MB="2048"
# SANDBOX_CPU_SHARES="1024"
# SANDBOX_AUTO_PULL="true"
# SANDBOX_EXTRA_DOMAINS="api.example.com,cdn.example.com"

##############################################
# Claude Code Sandbox
##############################################
# CLAUDE_CODE_ENABLED="false"
# CLAUDE_CODE_MODEL="sonnet"
# CLAUDE_CODE_MAX_TURNS="50"
# CLAUDE_CODE_MEMORY_LIMIT_MB="4096"
# CLAUDE_CONFIG_DIR="/home/user/.claude"

##############################################
# WASM Tool Runtime
##############################################
# WASM_ENABLED="true"
# WASM_TOOLS_DIR="~/.ironclaw/tools/"
# WASM_DEFAULT_MEMORY_LIMIT="10485760"   # 10 MB
# WASM_DEFAULT_TIMEOUT_SECS="60"
# WASM_DEFAULT_FUEL_LIMIT="10000000"
# WASM_CACHE_COMPILED="true"
# WASM_CACHE_DIR=""                      # defaults to system temp

##############################################
# Heartbeat (proactive periodic execution)
##############################################
# HEARTBEAT_ENABLED="false"
# HEARTBEAT_INTERVAL_SECS="1800"        # 30 minutes
# HEARTBEAT_NOTIFY_CHANNEL="gateway"    # or "tui"
# HEARTBEAT_NOTIFY_USER="default"

##############################################
# Memory Hygiene (workspace cleanup)
##############################################
# MEMORY_HYGIENE_ENABLED="true"
# MEMORY_HYGIENE_RETENTION_DAYS="30"
# MEMORY_HYGIENE_CADENCE_HOURS="12"

##############################################
# Routines (scheduled/reactive execution)
##############################################
# ROUTINES_ENABLED="true"
# ROUTINES_CRON_INTERVAL="15"
# ROUTINES_MAX_CONCURRENT="10"
# ROUTINES_DEFAULT_COOLDOWN="300"
# ROUTINES_MAX_TOKENS="4096"

##############################################
# Skills
##############################################
# SKILLS_ENABLED="false"
# SKILLS_DIR="~/.ironclaw/skills/"
# SKILLS_MAX_ACTIVE="3"
# SKILLS_MAX_CONTEXT_TOKENS="4000"

##############################################
# Security
##############################################
# Master key for encrypted secrets store.
# Minimum 32 bytes. Falls back to OS keychain if unset.
# Generate with: openssl rand -hex 32
# SECRETS_MASTER_KEY="64-hex-char-string-here"

##############################################
# Safety / Prompt Injection Defense
##############################################
# SAFETY_MAX_OUTPUT_LENGTH="100000"
# SAFETY_INJECTION_CHECK_ENABLED="true"

##############################################
# Dynamic Tool Builder
##############################################
# BUILDER_ENABLED="true"
# BUILDER_MAX_ITERATIONS="20"
# BUILDER_TIMEOUT_SECS="600"
# BUILDER_AUTO_REGISTER="true"
# BUILDER_DIR=""                          # defaults to system temp

##############################################
# Observability
##############################################
# OBSERVABILITY_BACKEND="none"

##############################################
# Logging
##############################################
RUST_LOG="ironclaw=info,tower_http=info"
# Verbose: RUST_LOG="ironclaw=debug,tower_http=debug"
# Trace:   RUST_LOG="ironclaw=trace"
```
