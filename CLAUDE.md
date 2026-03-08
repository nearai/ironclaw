# IronClaw Development Guide

## Project Overview

**IronClaw** is a secure personal AI assistant that protects your data and expands its capabilities on the fly.

### Core Philosophy
- **User-first security** - Your data stays yours, encrypted and local
- **Self-expanding** - Build new tools dynamically without vendor dependency
- **Defense in depth** - Multiple security layers against prompt injection and data exfiltration
- **Always available** - Multi-channel access with proactive background execution

### Features
- **Multi-channel input**: TUI (Ratatui), HTTP webhooks, WASM channels (Telegram, Slack), web gateway
- **Parallel job execution** with state machine and self-repair for stuck jobs
- **Sandbox execution**: Docker container isolation with network proxy and credential injection
- **Claude Code mode**: Delegate jobs to Claude CLI inside containers
- **Skills system**: SKILL.md prompt extensions with trust model, tool attenuation, and ClawHub registry
- **Routines**: Scheduled (cron) and reactive (event, webhook) task execution
- **Web gateway**: Browser UI with SSE/WebSocket real-time streaming
- **Extension management**: Install, auth, activate MCP/WASM extensions
- **Extensible tools**: Built-in tools, WASM sandbox, MCP client, dynamic builder
- **Persistent memory**: Workspace with hybrid search (FTS + vector via RRF)
- **Prompt injection defense**: Sanitizer, validator, policy rules, leak detection, shell env scrubbing
- **Multi-provider LLM**: NEAR AI, OpenAI, Anthropic, Ollama, OpenAI-compatible, Tinfoil private inference
- **Setup wizard**: 7-step interactive onboarding for first-run configuration
- **Heartbeat system**: Proactive periodic execution with checklist

## Build & Test

```bash
# Format code
cargo fmt

# Lint (fix ALL warnings before committing, including pre-existing ones)
cargo clippy --all --benches --tests --examples --all-features

# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with logging
RUST_LOG=ironclaw=debug cargo run

# Run integration tests (may require running services/DB)
cargo test --test workspace_integration
cargo test --test ws_gateway_integration
cargo test --test heartbeat_integration

# Run E2E tests (Python/Playwright — requires a running ironclaw instance)
# See tests/e2e/CLAUDE.md for full setup instructions
cd tests/e2e
python -m venv .venv && source .venv/bin/activate  # On Windows: .venv\Scripts\activate
pip install -e .
playwright install chromium
pytest scenarios/                    # all scenarios
pytest scenarios/test_chat.py        # specific scenario
```

### Test Tiers

| Tier | Command | What runs | External deps |
|------|---------|-----------|---------------|
| Unit | `cargo test` | All `mod tests` + self-contained integration tests | None |
| Integration | `cargo test --features integration` | + PostgreSQL-dependent tests | Running PostgreSQL |
| Live | `cargo test --features integration -- --ignored` | + LLM-dependent tests | PostgreSQL + LLM API keys |

Run `bash scripts/check-boundaries.sh` to verify test tier gating and other architecture rules.

## Project Structure

```
src/
├── lib.rs              # Library root, module declarations
├── main.rs             # Entry point, CLI args, startup
├── app.rs              # App startup orchestration (channel wiring, DB init)
├── bootstrap.rs        # Base directory resolution (~/.ironclaw), early .env loading
├── settings.rs         # User settings persistence (~/.ironclaw/settings.json)
├── service.rs          # OS service management (launchd/systemd daemon install)
├── tracing_fmt.rs      # Custom tracing formatter
├── util.rs             # Shared utilities
├── config/             # Configuration from env vars (split by subsystem)
│   ├── mod.rs          # Re-exports all config types; top-level Config struct
│   ├── agent.rs, llm.rs, channels.rs, database.rs, sandbox.rs, skills.rs
│   ├── heartbeat.rs, routines.rs, safety.rs, embeddings.rs, wasm.rs
│   ├── tunnel.rs       # Tunnel provider config (TUNNEL_PROVIDER, TUNNEL_URL, etc.)
│   └── secrets.rs, hygiene.rs, builder.rs, helpers.rs
├── error.rs            # Error types (thiserror)
│
├── agent/              # Core agent loop, dispatcher, scheduler, sessions — see src/agent/CLAUDE.md
│
├── channels/           # Multi-channel input
│   ├── channel.rs      # Channel trait, IncomingMessage, OutgoingResponse
│   ├── manager.rs      # ChannelManager merges streams
│   ├── cli/            # Full TUI with Ratatui
│   │   ├── mod.rs      # TuiChannel implementation
│   │   ├── app.rs      # Application state
│   │   ├── render.rs   # UI rendering
│   │   ├── events.rs   # Input handling
│   │   ├── overlay.rs  # Approval overlays
│   │   └── composer.rs # Message composition
│   ├── http.rs         # HTTP webhook (axum) with secret validation
│   ├── webhook_server.rs # Unified HTTP server composing all webhook routes
│   ├── repl.rs         # Simple REPL (for testing)
│   ├── web/            # Web gateway (browser UI) — see src/channels/web/CLAUDE.md
│   └── wasm/           # WASM channel runtime
│       ├── mod.rs
│       ├── bundled.rs  # Bundled channel discovery
│       ├── capabilities.rs # Channel-specific capabilities (HTTP endpoint, emit rate)
│       ├── error.rs    # WASM channel error types
│       ├── runtime.rs  # WASM channel execution runtime
│       └── wrapper.rs  # Channel trait wrapper for WASM modules
│
├── cli/                # CLI subcommands (clap)
│   ├── mod.rs          # Cli struct, Command enum (run/onboard/config/tool/registry/mcp/memory/pairing/service/doctor/status/completion)
│   ├── config.rs       # config list/get/set subcommands
│   ├── tool.rs         # tool install/list/remove subcommands
│   ├── registry.rs     # registry list/install subcommands
│   ├── mcp.rs          # mcp add/auth/list/test subcommands
│   ├── memory.rs       # memory search/read/write subcommands
│   ├── pairing.rs      # pairing list/approve subcommands
│   ├── service.rs      # service install/start/stop subcommands
│   ├── doctor.rs       # Active health diagnostics
│   ├── status.rs       # System health/status display
│   ├── completion.rs   # Shell completion script generation
│   └── oauth_defaults.rs # Default OAuth redirect URIs
│
├── registry/           # Extension registry catalog
│   ├── mod.rs          # Public API; re-exports RegistryCatalog, RegistryInstaller, manifest types
│   ├── manifest.rs     # ExtensionManifest, ArtifactSpec, BundleDefinition types
│   ├── catalog.rs      # RegistryCatalog: load from filesystem and embedded JSON
│   ├── installer.rs    # RegistryInstaller: download, verify, install WASM artifacts
│   ├── artifacts.rs    # Artifact download and caching
│   └── embedded.rs     # Catalog compiled into binary at build time (via build.rs)
│
├── hooks/              # Lifecycle hooks for intercepting agent operations
│   ├── mod.rs          # 6 HookPoints: BeforeInbound, BeforeToolCall, BeforeOutbound, OnSessionStart, OnSessionEnd, TransformResponse
│   ├── hook.rs         # Hook trait, HookContext, HookEvent, HookOutcome, HookFailureMode
│   ├── registry.rs     # HookRegistry: register, prioritize, execute hooks
│   └── bundled.rs      # Built-in hooks: rule-based filters, webhook forwarders, HookBundleConfig
│
├── tunnel/             # Tunnel abstraction for public internet exposure
│   ├── mod.rs          # Tunnel trait, TunnelProviderConfig, create_tunnel() factory
│   ├── cloudflare.rs   # CloudflareTunnel (cloudflared binary)
│   ├── ngrok.rs        # NgrokTunnel
│   ├── tailscale.rs    # TailscaleTunnel (serve/funnel modes)
│   ├── custom.rs       # CustomTunnel (arbitrary command with {host}/{port})
│   └── none.rs         # NoneTunnel (local-only, no exposure)
│
├── observability/      # Pluggable event/metric recording
│   ├── mod.rs          # create_observer() factory, ObservabilityConfig
│   ├── traits.rs       # Observer trait, ObserverEvent, ObserverMetric
│   ├── noop.rs         # NoopObserver (zero overhead, default)
│   ├── log.rs          # LogObserver (tracing-based)
│   └── multi.rs        # MultiObserver (fan-out to multiple backends)
│
├── orchestrator/       # Internal HTTP API for sandbox containers
│   ├── mod.rs
│   ├── api.rs          # Axum endpoints (LLM proxy, events, prompts)
│   ├── auth.rs         # Per-job bearer token store
│   └── job_manager.rs  # Container lifecycle (create, stop, cleanup)
│
├── worker/             # Runs inside Docker containers
│   ├── mod.rs
│   ├── runtime.rs      # Worker execution loop (tool calls, LLM)
│   ├── claude_bridge.rs # Claude Code bridge (spawns claude CLI)
│   ├── api.rs          # HTTP client to orchestrator
│   └── proxy_llm.rs    # LlmProvider that proxies through orchestrator
│
├── safety/             # Prompt injection defense
│   ├── sanitizer.rs    # Pattern detection, content escaping
│   ├── validator.rs    # Input validation (length, encoding, patterns)
│   ├── policy.rs       # PolicyRule system with severity/actions
│   ├── leak_detector.rs # Secret detection (API keys, tokens, etc.)
│   └── credential_detect.rs # HTTP request credential detection (headers, URL params)
│
├── llm/                # Multi-provider LLM integration — see src/llm/CLAUDE.md
│
├── tools/              # Extensible tool system
│   ├── tool.rs         # Tool trait, ToolOutput, ToolError
│   ├── registry.rs     # ToolRegistry for discovery
│   ├── sandbox.rs      # Process-based sandbox (stub, superseded by wasm/)
│   ├── rate_limiter.rs # Shared sliding-window rate limiter for built-in and WASM tools
│   ├── builtin/        # Built-in tools
│   │   ├── echo.rs, time.rs, json.rs, http.rs
│   │   ├── web_fetch.rs # GET URL → clean Markdown (readability + html-to-md conversion)
│   │   ├── file.rs     # ReadFile, WriteFile, ListDir, ApplyPatch
│   │   ├── shell.rs    # Shell command execution
│   │   ├── memory.rs   # Memory tools (search, write, read, tree)
│   │   ├── message.rs  # MessageTool: agent proactively messages users on any channel
│   │   ├── job.rs      # CreateJob, ListJobs, JobStatus, CancelJob
│   │   ├── routine.rs  # routine_create/list/update/delete/history
│   │   ├── extension_tools.rs # Extension install/auth/activate/remove
│   │   ├── skill_tools.rs # skill_list/search/install/remove tools
│   │   ├── secrets_tools.rs # secret_list/secret_delete (zero-exposure: no values exposed)
│   │   ├── html_converter.rs # HTML→Markdown via readability + html-to-markdown-rs
│   │   ├── path_utils.rs # Shared path validation/canonicalization helpers
│   │   └── marketplace.rs, ecommerce.rs, taskrabbit.rs, restaurant.rs (stubs)
│   ├── builder/        # Dynamic tool building
│   │   ├── core.rs     # BuildRequirement, SoftwareType, Language
│   │   ├── templates.rs # Project scaffolding
│   │   ├── testing.rs  # Test harness integration
│   │   └── validation.rs # WASM validation
│   ├── mcp/            # Model Context Protocol
│   │   ├── client.rs   # MCP client over HTTP
│   │   ├── protocol.rs # JSON-RPC types
│   │   └── session.rs  # MCP session management (Mcp-Session-Id header, per-server state)
│   └── wasm/           # Full WASM sandbox (wasmtime)
│       ├── runtime.rs  # Module compilation and caching
│       ├── wrapper.rs  # Tool trait wrapper for WASM modules
│       ├── host.rs     # Host functions (logging, time, workspace)
│       ├── limits.rs   # Fuel metering and memory limiting
│       ├── allowlist.rs # Network endpoint allowlisting
│       ├── credential_injector.rs # Safe credential injection
│       ├── loader.rs   # WASM tool discovery from filesystem
│       ├── rate_limiter.rs # Per-tool rate limiting
│       ├── error.rs    # WASM-specific error types
│       └── storage.rs  # Linear memory persistence
│
├── db/                 # Dual-backend persistence (PostgreSQL + libSQL) — see src/db/CLAUDE.md
│
├── workspace/          # Persistent memory system (OpenClaw-inspired)
│   ├── mod.rs          # Workspace struct, memory operations
│   ├── document.rs     # MemoryDocument, MemoryChunk, WorkspaceEntry
│   ├── chunker.rs      # Document chunking (800 tokens, 15% overlap)
│   ├── embeddings.rs   # EmbeddingProvider trait, OpenAI implementation
│   ├── search.rs       # Hybrid search with RRF algorithm
│   └── repository.rs   # PostgreSQL CRUD and search operations
│
├── context/            # Job context isolation
│   ├── state.rs        # JobState enum, JobContext, state machine
│   ├── memory.rs       # ActionRecord, ConversationMemory
│   └── manager.rs      # ContextManager for concurrent jobs
│
├── estimation/         # Cost/time/value estimation
│   ├── cost.rs         # CostEstimator
│   ├── time.rs         # TimeEstimator
│   ├── value.rs        # ValueEstimator (profit margins)
│   └── learner.rs      # Exponential moving average learning
│
├── evaluation/         # Success evaluation
│   ├── success.rs      # SuccessEvaluator trait, RuleBasedEvaluator, LlmEvaluator
│   └── metrics.rs      # MetricsCollector, QualityMetrics
│
├── sandbox/            # Docker execution sandbox
│   ├── mod.rs          # Public API, default allowlist
│   ├── config.rs       # SandboxConfig, SandboxPolicy enum
│   ├── manager.rs      # SandboxManager orchestration
│   ├── container.rs    # ContainerRunner, Docker lifecycle
│   ├── error.rs        # SandboxError types
│   └── proxy/          # Network proxy for containers
│       ├── mod.rs      # NetworkProxyBuilder
│       ├── http.rs     # HttpProxy, CredentialResolver trait
│       ├── policy.rs   # NetworkPolicyDecider trait
│       └── allowlist.rs # DomainAllowlist validation
│
├── secrets/            # Secrets management
│   ├── mod.rs          # SecretsStore trait, public API
│   ├── types.rs        # Core types (Secret, SecretRef, SecretMetadata)
│   ├── crypto.rs       # AES-256-GCM encryption
│   ├── keychain.rs     # OS keychain integration (macOS Keychain, GNOME Keyring) for master key
│   └── store.rs        # Encrypted secret storage
│
├── setup/              # Onboarding wizard (spec: src/setup/README.md)
│   ├── mod.rs          # Entry point, check_onboard_needed()
│   ├── wizard.rs       # 7-step interactive wizard
│   ├── channels.rs     # Channel setup helpers
│   └── prompts.rs      # Terminal prompts (select, confirm, secret)
│
├── skills/             # SKILL.md prompt extension system
│   ├── mod.rs          # Core types (SkillTrust, LoadedSkill)
│   ├── registry.rs     # SkillRegistry: discover, install, remove
│   ├── selector.rs     # Deterministic scoring prefilter
│   ├── attenuation.rs  # Trust-based tool ceiling
│   ├── gating.rs       # Requirement checks (bins, env, config)
│   ├── parser.rs       # SKILL.md frontmatter + markdown parser
│   └── catalog.rs      # ClawHub registry client
│
└── history/            # Persistence
    ├── store.rs        # PostgreSQL repositories
    └── analytics.rs    # Aggregation queries (JobStats, ToolStats)

tests/
├── *.rs                # Integration tests (workspace, heartbeat, WS gateway, pairing, etc.)
├── test-pages/         # HTML→Markdown conversion fixtures (CNN, Medium, Yahoo)
└── e2e/                # Python/Playwright E2E scenarios (see tests/e2e/CLAUDE.md)
```

## Key Patterns

### Architecture

When designing new features or systems, always prefer generic/extensible architectures over hardcoding specific integrations. Ask clarifying questions about the desired abstraction level before implementing.

### Error Handling
- Use `thiserror` for error types in `error.rs`
- Never use `.unwrap()` or `.expect()` in production code (tests are fine)
- Map errors with context: `.map_err(|e| SomeError::Variant { reason: e.to_string() })?`
- Before committing, grep for `.unwrap()` and `.expect(` in changed files to catch violations mechanically

### Async
- All I/O is async with tokio
- Use `Arc<T>` for shared state across tasks
- Use `RwLock` for concurrent read/write access

### Traits for Extensibility
- `Database` - Add new database backends (must implement all ~78 methods)
- `Channel` - Add new input sources
- `Tool` - Add new capabilities
- `LlmProvider` - Add new LLM backends
- `SuccessEvaluator` - Custom evaluation logic
- `EmbeddingProvider` - Add embedding backends (workspace search)
- `NetworkPolicyDecider` - Custom network access policies for sandbox containers
- `Hook` - Lifecycle hook at 6 interception points (BeforeInbound, BeforeToolCall, BeforeOutbound, OnSessionStart, OnSessionEnd, TransformResponse)
- `Observer` - Observability backend (noop/log/multi; future: OpenTelemetry, Prometheus)
- `Tunnel` - Tunnel provider for public internet exposure

### Tool Implementation
```rust
#[async_trait]
impl Tool for MyTool {
    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "Does something useful" }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "param": { "type": "string", "description": "A parameter" }
            },
            "required": ["param"]
        })
    }

    async fn execute(&self, params: serde_json::Value, ctx: &JobContext)
        -> Result<ToolOutput, ToolError>
    {
        let start = std::time::Instant::now();
        // ... do work ...
        Ok(ToolOutput::text("result", start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool { true } // External data
}
```

### State Transitions
Job states follow a defined state machine in `context/state.rs`:
```
Pending -> InProgress -> Completed -> Submitted -> Accepted
                     \-> Failed
                     \-> Stuck -> InProgress (recovery)
                              \-> Failed
```

### Code Style

- Use `crate::` imports, not `super::`
- No `pub use` re-exports unless exposing to downstream consumers
- Prefer strong types over strings (enums, newtypes)
- Keep functions focused, extract helpers when logic is reused
- Comments for non-obvious logic only

### Review & Fix Discipline

Hard-won lessons from code review -- follow these when fixing bugs or addressing review feedback.

**Fix the pattern, not just the instance:** When a reviewer flags a bug (e.g., TOCTOU race in INSERT + SELECT-back), search the entire codebase for all instances of that same pattern. A fix in `SecretsStore::create()` that doesn't also fix `WasmToolStore::store()` is half a fix.

**Propagate architectural fixes to satellite types:** If a core type changes its concurrency model (e.g., `LibSqlBackend` switches to connection-per-operation), every type that was handed a resource from the old model (e.g., `LibSqlSecretsStore`, `LibSqlWasmToolStore` holding a single `Connection`) must also be updated. Grep for the old type across the codebase.

**Schema translation is more than DDL:** When translating a database schema between backends (PostgreSQL to libSQL, etc.), check for:
- **Indexes** -- diff `CREATE INDEX` statements between the two schemas
- **Seed data** -- check for `INSERT INTO` in migrations (e.g., `leak_detection_patterns`)
- **Semantic differences** -- document where SQL functions behave differently (e.g., `json_patch` vs `jsonb_set`)

**Feature flag testing:** When adding feature-gated code, test compilation with each feature in isolation:
```bash
cargo check                                          # default features
cargo check --no-default-features --features libsql  # libsql only
cargo check --all-features                           # all features
```
Dead code behind the wrong `#[cfg]` gate will only show up when building with a single feature.

**Regression test with every fix:** Every bug fix must include a test that would have caught the bug. Add a `#[test]` or `#[tokio::test]` that reproduces the original failure. Exempt: changes limited to `src/channels/web/static/` or `.md` files. Use `[skip-regression-check]` in commit message or PR label if genuinely not feasible. The `commit-msg` hook and CI workflow enforce this automatically.

**Zero clippy warnings policy:** Fix ALL clippy warnings before committing, including pre-existing ones in files you didn't change. Never leave warnings behind — treat `cargo clippy` output as a zero-tolerance gate.

**Mechanical verification before committing:** Run these checks on changed files before committing:
- `cargo clippy --all --benches --tests --examples --all-features` -- zero warnings
- `grep -rnE '\.unwrap\(|\.expect\(' <files>` -- no panics in production
- `grep -rn 'super::' <files>` -- use `crate::` imports
- If you fixed a pattern bug, `grep` for other instances of that pattern across `src/`
- Fix commits must include regression tests (enforced by `commit-msg` hook; bypass with `[skip-regression-check]`)

## Configuration

Environment variables (see `.env.example`):
```bash
# Database backend (default: postgres)
DATABASE_BACKEND=postgres               # or "libsql" / "turso"
DATABASE_URL=postgres://user:pass@localhost/ironclaw
LIBSQL_PATH=~/.ironclaw/ironclaw.db    # libSQL local path (default)
# LIBSQL_URL=libsql://xxx.turso.io    # Turso cloud (optional)
# LIBSQL_AUTH_TOKEN=xxx                # Required with LIBSQL_URL

# NEAR AI (when LLM_BACKEND=nearai, the default)
# Two auth modes: session token (default) or API key
# Session token auth (default): uses browser OAuth on first run
NEARAI_SESSION_TOKEN=sess_...           # hosting providers: set this
NEARAI_BASE_URL=https://private.near.ai
# API key auth: set NEARAI_API_KEY, base URL defaults to cloud-api.near.ai
# NEARAI_API_KEY=...                    # API key from cloud.near.ai
NEARAI_MODEL=claude-3-5-sonnet-20241022

# Agent settings
AGENT_NAME=ironclaw
MAX_PARALLEL_JOBS=5

# Embeddings (for semantic memory search)
OPENAI_API_KEY=sk-...                   # For OpenAI embeddings
# Or use NEAR AI embeddings:
# EMBEDDING_PROVIDER=nearai
# EMBEDDING_ENABLED=true
EMBEDDING_MODEL=text-embedding-3-small  # or text-embedding-3-large

# Heartbeat (proactive periodic execution)
HEARTBEAT_ENABLED=true
HEARTBEAT_INTERVAL_SECS=1800            # 30 minutes
HEARTBEAT_NOTIFY_CHANNEL=tui
HEARTBEAT_NOTIFY_USER=default

# Web gateway
GATEWAY_ENABLED=true
GATEWAY_HOST=127.0.0.1
GATEWAY_PORT=3001
GATEWAY_AUTH_TOKEN=changeme           # Required for API access
GATEWAY_USER_ID=default

# Docker sandbox
SANDBOX_ENABLED=true
SANDBOX_IMAGE=ironclaw-worker:latest
SANDBOX_MEMORY_LIMIT_MB=512
SANDBOX_TIMEOUT_SECS=1800
SANDBOX_CPU_LIMIT=1.0                  # CPU cores per container
SANDBOX_NETWORK_PROXY=true             # Enable network proxy for containers
SANDBOX_PROXY_PORT=8080                # Proxy listener port
SANDBOX_DEFAULT_POLICY=workspace_write # ReadOnly, WorkspaceWrite, FullAccess

# Claude Code mode (runs inside sandbox containers)
CLAUDE_CODE_ENABLED=false
CLAUDE_CODE_MODEL=claude-sonnet-4-20250514
CLAUDE_CODE_MAX_TURNS=50
CLAUDE_CODE_CONFIG_DIR=/home/worker/.claude

# Routines (scheduled/reactive execution)
ROUTINES_ENABLED=true
ROUTINES_CRON_INTERVAL=60            # Tick interval in seconds
ROUTINES_MAX_CONCURRENT=3

# Skills system
SKILLS_ENABLED=true
SKILLS_MAX_TOKENS=4000                 # Max prompt budget per turn
SKILLS_CATALOG_URL=https://clawhub.dev # ClawHub registry URL
SKILLS_AUTO_DISCOVER=true              # Scan skill directories on startup

# Tinfoil private inference
TINFOIL_API_KEY=...                    # Required when LLM_BACKEND=tinfoil
TINFOIL_MODEL=kimi-k2-5               # Default model

# AWS Bedrock (native Converse API, requires --features bedrock)
# LLM_BACKEND=bedrock
# BEDROCK_REGION=us-east-1                    # AWS region
# BEDROCK_MODEL=anthropic.claude-opus-4-6-v1  # Required model ID
# BEDROCK_CROSS_REGION=us                     # Cross-region prefix (us/eu/apac/global)
# AWS_PROFILE=my-profile                      # Named profile (SSO/assume-role)

# Tunnel (public internet exposure for webhooks)
TUNNEL_URL=https://abc123.ngrok.io     # Static public URL (manual tunnel)
# Or use a managed tunnel provider:
TUNNEL_PROVIDER=none                   # none (default), cloudflare, tailscale, ngrok, custom
TUNNEL_CF_TOKEN=...                    # Required for TUNNEL_PROVIDER=cloudflare
TUNNEL_NGROK_TOKEN=...                 # Required for TUNNEL_PROVIDER=ngrok
# TUNNEL_NGROK_DOMAIN=...             # Custom domain (paid ngrok plan)
# TUNNEL_TS_FUNNEL=true               # Use tailscale funnel (public) vs serve (tailnet)
TUNNEL_CUSTOM_COMMAND=...              # Command with {host}/{port} for custom providers

# Observability backend
OBSERVABILITY_BACKEND=none             # none/noop (default) or log
```

### LLM Providers

Backends: `nearai` (default), `openai`, `anthropic`, `ollama`, `openai_compatible`, `tinfoil` — set via `LLM_BACKEND`. See [src/llm/CLAUDE.md](src/llm/CLAUDE.md) for per-provider auth and configuration details.

**AWS Bedrock** -- Uses the native Converse API via `aws-sdk-bedrockruntime`. Requires `--features bedrock` at build time (not included in default features due to heavy AWS SDK dependencies). Supports standard AWS auth methods: IAM credentials (`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`), SSO profiles (`AWS_PROFILE`), and instance roles. Configure with `BEDROCK_REGION` (default: `us-east-1`), `BEDROCK_MODEL` (required, e.g., `anthropic.claude-opus-4-6-v1`), and `BEDROCK_CROSS_REGION` (optional: `us`, `eu`, `apac`, `global` for cross-region inference profiles). The SDK credential chain resolves auth automatically from the environment.

## Database

Dual-backend persistence (PostgreSQL + libSQL/Turso). **All new persistence features must support both backends** — see [src/db/CLAUDE.md](src/db/CLAUDE.md) for schema, SQL dialect differences, adding operations, and libSQL limitations.

Implement every new operation in both `src/db/postgres.rs` and `src/db/libsql/mod.rs`. Test in isolation:
```bash
cargo check                                          # postgres (default)
cargo check --no-default-features --features libsql  # libsql only
cargo check --all-features                           # both
```

Database configuration: see Configuration section above.

## Safety Layer

All external tool output passes through `SafetyLayer`:
1. **Sanitizer** - Detects injection patterns, escapes dangerous content
2. **Validator** - Checks length, encoding, forbidden patterns
3. **Policy** - Rules with severity (Critical/High/Medium/Low) and actions (Block/Warn/Review/Sanitize)
4. **Leak Detector** - Scans for 15+ secret patterns (API keys, tokens, private keys, connection strings) at two points: tool output before it reaches the LLM, and LLM responses before they reach the user. Actions per pattern: Block (reject entirely), Redact (mask the secret), or Warn (flag but allow)

Tool outputs are wrapped before reaching LLM:
```xml
<tool_output name="search" sanitized="true">
[escaped content]
</tool_output>
```

### Shell Environment Scrubbing

The shell tool (`src/tools/builtin/shell.rs`) scrubs sensitive environment variables before executing commands, preventing secrets from leaking through `env`, `printenv`, or `$VAR` expansion. The sanitizer (`src/safety/sanitizer.rs`) also detects command injection patterns (chained commands, subshells, path traversal) and blocks or escapes them based on policy rules.

## Skills System

Skills are SKILL.md files that extend the agent's prompt with domain-specific instructions. Each skill is a YAML frontmatter block (metadata, activation criteria, required tools) followed by a markdown body that gets injected into the LLM context when the skill activates.

### Trust Model

| Trust Level | Source | Tool Access |
|-------------|--------|-------------|
| **Trusted** | User-placed in `~/.ironclaw/skills/` or workspace `skills/` | All tools available to the agent |
| **Installed** | Downloaded from ClawHub registry | Read-only tools only (no shell, file write, HTTP) |

### SKILL.md Format

```yaml
---
name: my-skill
version: 0.1.0
description: Does something useful
activation:
  patterns:
    - "deploy to.*production"
  keywords:
    - "deployment"
  max_context_tokens: 2000
metadata:
  openclaw:
    requires:
      bins: [docker, kubectl]
      env: [KUBECONFIG]
---

# Deployment Skill

Instructions for the agent when this skill activates...
```

### Selection Pipeline

1. **Gating** -- Check binary/env/config requirements; skip skills whose prerequisites are missing
2. **Scoring** -- Deterministic scoring against message content using keywords, tags, and regex patterns
3. **Budget** -- Select top-scoring skills that fit within `SKILLS_MAX_TOKENS` prompt budget
4. **Attenuation** -- Apply trust-based tool ceiling; installed skills lose access to dangerous tools

### Skill Tools

Four built-in tools for managing skills at runtime:
- **`skill_list`** -- List all discovered skills with trust level and status
- **`skill_search`** -- Search ClawHub registry for available skills
- **`skill_install`** -- Download and install a skill from ClawHub
- **`skill_remove`** -- Remove an installed skill

### Skill Directories

- `~/.ironclaw/skills/` -- User's global skills (trusted)
- `<workspace>/skills/` -- Per-workspace skills (trusted)
- `~/.ironclaw/installed_skills/` -- Registry-installed skills (installed trust)

### Testing Skills

- `skills/web-ui-test/` -- Manual test checklist for the web gateway UI via Claude for Chrome extension. Covers connection, chat, skills search/install/remove, and other tabs.

Skills configuration: see Configuration section above.

## Docker Sandbox

The `src/sandbox/` module provides Docker-based isolation for job execution with a network proxy that controls outbound access and injects credentials.

### Sandbox Policies

| Policy | Filesystem | Network | Use Case |
|--------|-----------|---------|----------|
| **ReadOnly** | Read-only workspace mount | Allowlisted domains only | Analysis, code review |
| **WorkspaceWrite** | Read-write workspace mount | Allowlisted domains only | Code generation, file edits |
| **FullAccess** | Full filesystem | Unrestricted | Trusted admin tasks |

### Network Proxy

Containers route all HTTP/HTTPS traffic through a host-side proxy (`src/sandbox/proxy/`):
- **Domain allowlist** -- Only allowlisted domains are reachable (default: package registries, docs sites, GitHub, common APIs)
- **Credential injection** -- The `CredentialResolver` trait injects auth headers into proxied requests so secrets never enter the container environment
- **CONNECT tunnel** -- HTTPS traffic uses CONNECT method; the proxy validates the target domain against the allowlist before establishing the tunnel
- **Policy decisions** -- The `NetworkPolicyDecider` trait allows custom logic for allow/deny/inject decisions per request

### Zero-Exposure Credential Model

Secrets (API keys, tokens) are stored encrypted on the host and injected into HTTP requests by the proxy at transit time. Container processes never have access to raw credential values, preventing exfiltration even if container code is compromised.

Sandbox configuration: see Configuration section above.

## Testing

Tests are in `mod tests {}` blocks at the bottom of each file. Run specific module tests:
```bash
cargo test safety::sanitizer::tests
cargo test tools::registry::tests
```

Key test patterns:
- Unit tests for pure functions
- Async tests with `#[tokio::test]`
- No mocks, prefer real implementations or stubs

## Current Limitations / TODOs

1. **Domain-specific tools** - `marketplace.rs`, `restaurant.rs`, `taskrabbit.rs`, `ecommerce.rs` return placeholder responses; need real API integrations
2. **Integration tests** - Need testcontainers setup for PostgreSQL
3. **MCP stdio transport** - Only HTTP transport implemented
4. **WIT bindgen integration** - Auto-extract tool description/schema from WASM modules (stubbed)
5. **Capability granting after tool build** - Built tools get empty capabilities; need UX for granting HTTP/secrets access
6. **Tool versioning workflow** - No version tracking or rollback for dynamically built tools
7. **Full channel status view** - Gateway status widget exists, but no per-channel connection dashboard
8. **Observability backends** - Only `log` and `noop` implemented; OpenTelemetry/Prometheus not yet supported

## Tool Architecture

**Keep tool-specific logic out of the main agent codebase.** The main agent provides generic infrastructure; tools are self-contained units that declare their requirements through `capabilities.json` files (API endpoints, credentials, rate limits, auth setup). Service-specific auth flows, CLI commands, and configuration do not belong in the main agent.

Tools can be built as **WASM** (sandboxed, credential-injected, single binary) or **MCP servers** (ecosystem of pre-built servers, any language, but no sandbox). Both are first-class via `ironclaw tool install`. Auth is declared in capabilities files with OAuth and manual token entry support.

See `src/tools/README.md` for full tool architecture, adding new tools (built-in Rust and WASM), auth JSON examples, and WASM vs MCP decision guide.

## Adding a New Channel

1. Create `src/channels/my_channel.rs`
2. Implement the `Channel` trait
3. Add config in `src/config/channels.rs`
4. Wire up in `src/app.rs` channel setup section

## Debugging

```bash
# Verbose logging
RUST_LOG=ironclaw=trace cargo run

# Just the agent module
RUST_LOG=ironclaw::agent=debug cargo run

# With HTTP request logging
RUST_LOG=ironclaw=debug,tower_http=debug cargo run
```

## Module Specifications

Some modules have a `README.md` that serves as the authoritative specification
for that module's behavior. When modifying code in a module that has a spec:

1. **Read the spec first** before making changes
2. **Code follows spec**: if the spec says X, the code must do X
3. **Update both sides**: if you change behavior, update the spec to match;
   if you're implementing a spec change, update the code to match
4. **Spec is the tiebreaker**: when code and spec disagree, the spec is correct
   (unless the spec is clearly outdated, in which case fix the spec first)

| Module | Spec File |
|--------|-----------|
| `src/setup/` | `src/setup/README.md` |
| `src/workspace/` | `src/workspace/README.md` |
| `src/tools/` | `src/tools/README.md` |
| `src/agent/` | `src/agent/CLAUDE.md` |
| `src/channels/web/` | `src/channels/web/CLAUDE.md` |
| `src/db/` | `src/db/CLAUDE.md` |
| `src/llm/` | `src/llm/CLAUDE.md` |
| `tests/e2e/` | `tests/e2e/CLAUDE.md` |

## Workspace & Memory System

OpenClaw-inspired persistent memory with a flexible filesystem-like structure. Principle: "Memory is database, not RAM" -- if you want to remember something, write it explicitly. Uses hybrid search combining FTS (keyword) + vector (semantic) via Reciprocal Rank Fusion.

Four memory tools for LLM use: `memory_search` (hybrid search -- call before answering questions about prior work), `memory_write`, `memory_read`, `memory_tree`. Identity files (AGENTS.md, SOUL.md, USER.md, IDENTITY.md) are injected into the LLM system prompt.

The heartbeat system runs proactive periodic execution (default: 30 minutes), reading `HEARTBEAT.md` and notifying via channel if findings are detected.

See `src/workspace/README.md` for full API documentation, filesystem structure, hybrid search details, chunking strategy, and heartbeat system.
