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
- **Sandbox execution**: Docker container isolation with orchestrator/worker pattern
- **Claude Code mode**: Delegate jobs to Claude CLI inside containers
- **Routines**: Scheduled (cron) and reactive (event, webhook) task execution
- **Web gateway**: Browser UI with SSE/WebSocket real-time streaming
- **Extension management**: Install, auth, activate MCP/WASM extensions
- **Extensible tools**: Built-in tools, WASM sandbox, MCP client, dynamic builder
- **Persistent memory**: Workspace with hybrid search (FTS + vector via RRF)
- **Prompt injection defense**: Sanitizer, validator, policy rules, leak detection
- **Heartbeat system**: Proactive periodic execution with checklist

## Build & Test

```bash
# Format code
cargo fmt

# Lint (address warnings before committing)
cargo clippy --all --benches --tests --examples --all-features

# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with logging
RUST_LOG=ironclaw=debug cargo run
```

## Project Structure

```
src/
├── lib.rs              # Library root, module declarations
├── main.rs             # Entry point, CLI args, startup
├── config.rs           # Configuration from env vars
├── error.rs            # Error types (thiserror)
│
├── agent/              # Core agent logic
│   ├── agent_loop.rs   # Main Agent struct, message handling loop
│   ├── router.rs       # MessageIntent classification
│   ├── scheduler.rs    # Parallel job scheduling
│   ├── worker.rs       # Per-job execution with LLM reasoning
│   ├── self_repair.rs  # Stuck job detection and recovery
│   ├── heartbeat.rs    # Proactive periodic execution
│   ├── session.rs      # Session/thread/turn model with state machine
│   ├── session_manager.rs # Thread/session lifecycle management
│   ├── compaction.rs   # Context window management with turn summarization
│   ├── context_monitor.rs # Memory pressure detection
│   ├── undo.rs         # Turn-based undo/redo with checkpoints
│   ├── submission.rs   # Submission parsing (undo, redo, compact, clear, etc.)
│   ├── task.rs         # Sub-task execution framework
│   ├── routine.rs      # Routine types (Trigger, Action, Guardrails)
│   └── routine_engine.rs # Routine execution (cron ticker, event matcher)
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
│   ├── repl.rs         # Simple REPL (for testing)
│   ├── web/            # Web gateway (browser UI)
│   │   ├── mod.rs      # Gateway builder, startup
│   │   ├── server.rs   # Axum router, 40+ API endpoints
│   │   ├── sse.rs      # SSE broadcast manager
│   │   ├── ws.rs       # WebSocket gateway + connection tracking
│   │   ├── types.rs    # Request/response types, SseEvent enum
│   │   ├── auth.rs     # Bearer token auth middleware
│   │   ├── log_layer.rs # Tracing layer for log streaming
│   │   └── static/     # HTML, CSS, JS (single-page app)
│   └── wasm/           # WASM channel runtime
│       ├── mod.rs
│       ├── bundled.rs  # Bundled channel discovery
│       └── wrapper.rs  # Channel trait wrapper for WASM modules
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
│   └── leak_detector.rs # Secret detection (API keys, tokens, etc.)
│
├── llm/                # LLM integration (NEAR AI only)
│   ├── provider.rs     # LlmProvider trait, message types
│   ├── nearai.rs       # NEAR AI chat-api implementation
│   ├── reasoning.rs    # Planning, tool selection, evaluation
│   └── session.rs      # Session token management with auto-renewal
│
├── tools/              # Extensible tool system
│   ├── tool.rs         # Tool trait, ToolOutput, ToolError
│   ├── registry.rs     # ToolRegistry for discovery
│   ├── sandbox.rs      # Process-based sandbox (stub, superseded by wasm/)
│   ├── builtin/        # Built-in tools
│   │   ├── echo.rs, time.rs, json.rs, http.rs
│   │   ├── file.rs     # ReadFile, WriteFile, ListDir, ApplyPatch
│   │   ├── shell.rs    # Shell command execution
│   │   ├── memory.rs   # Memory tools (search, write, read, tree)
│   │   ├── job.rs      # CreateJob, ListJobs, JobStatus, CancelJob
│   │   ├── routine.rs  # routine_create/list/update/delete/history
│   │   ├── extension_tools.rs # Extension install/auth/activate/remove
│   │   └── marketplace.rs, ecommerce.rs, taskrabbit.rs, restaurant.rs (stubs)
│   ├── builder/        # Dynamic tool building
│   │   ├── core.rs     # BuildRequirement, SoftwareType, Language
│   │   ├── templates.rs # Project scaffolding
│   │   ├── testing.rs  # Test harness integration
│   │   └── validation.rs # WASM validation
│   ├── mcp/            # Model Context Protocol
│   │   ├── client.rs   # MCP client over HTTP
│   │   └── protocol.rs # JSON-RPC types
│   └── wasm/           # Full WASM sandbox (wasmtime)
│       ├── runtime.rs  # Module compilation and caching
│       ├── wrapper.rs  # Tool trait wrapper for WASM modules
│       ├── host.rs     # Host functions (logging, time, workspace)
│       ├── limits.rs   # Fuel metering and memory limiting
│       ├── allowlist.rs # Network endpoint allowlisting
│       ├── credential_injector.rs # Safe credential injection
│       ├── loader.rs   # WASM tool discovery from filesystem
│       ├── rate_limiter.rs # Per-tool rate limiting
│       └── storage.rs  # Linear memory persistence
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
├── secrets/            # Secrets management
│   ├── crypto.rs       # AES-256-GCM encryption
│   ├── store.rs        # Secret storage
│   └── types.rs        # Credential types
│
└── history/            # Persistence
    ├── store.rs        # PostgreSQL repositories
    └── analytics.rs    # Aggregation queries (JobStats, ToolStats)
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
- `Channel` - Add new input sources
- `Tool` - Add new capabilities
- `LlmProvider` - Add new LLM backends
- `SuccessEvaluator` - Custom evaluation logic
- `EmbeddingProvider` - Add embedding backends (workspace search)

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

## Configuration

Environment variables (see `.env.example`):
```bash
DATABASE_URL=postgres://user:pass@localhost/ironclaw

# NEAR AI (required)
NEARAI_SESSION_TOKEN=sess_...
NEARAI_MODEL=claude-3-5-sonnet-20241022
NEARAI_BASE_URL=https://private.near.ai

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

# Claude Code mode (runs inside sandbox containers)
CLAUDE_CODE_ENABLED=false
CLAUDE_CODE_MODEL=claude-sonnet-4-20250514
CLAUDE_CODE_MAX_TURNS=50
CLAUDE_CODE_CONFIG_DIR=/home/worker/.claude

# Routines (scheduled/reactive execution)
ROUTINES_ENABLED=true
ROUTINES_CRON_INTERVAL=60            # Tick interval in seconds
ROUTINES_MAX_CONCURRENT=3
```

### NEAR AI Provider

Uses the NEAR AI chat-api (`https://api.near.ai/v1/responses`) which provides:
- Unified access to multiple models (OpenAI, Anthropic, etc.)
- User authentication via session tokens
- Usage tracking and billing through NEAR AI

Session tokens have the format `sess_xxx` (37 characters). They are authenticated against the NEAR AI auth service.

## Database

Single migration in `migrations/V1__initial.sql`. Tables:

**Core:**
- `conversations` - Multi-channel conversation tracking
- `agent_jobs` - Job metadata and status
- `job_actions` - Event-sourced tool executions
- `dynamic_tools` - Agent-built tools
- `llm_calls` - Cost tracking
- `estimation_snapshots` - Learning data

**Workspace/Memory:**
- `memory_documents` - Flexible path-based files (e.g., "context/vision.md", "daily/2024-01-15.md")
- `memory_chunks` - Chunked content with FTS (tsvector) and vector (pgvector) indexes
- `heartbeat_state` - Periodic execution tracking

Requires pgvector extension: `CREATE EXTENSION IF NOT EXISTS vector;`

Run migrations: `refinery migrate -c refinery.toml`

## Safety Layer

All external tool output passes through `SafetyLayer`:
1. **Sanitizer** - Detects injection patterns, escapes dangerous content
2. **Validator** - Checks length, encoding, forbidden patterns
3. **Policy** - Rules with severity (Critical/High/Medium/Low) and actions (Block/Warn/Review/Sanitize)

Tool outputs are wrapped before reaching LLM:
```xml
<tool_output name="search" sanitized="true">
[escaped content]
</tool_output>
```

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
7. **Webhook trigger endpoint** - Routines webhook trigger not yet exposed in web gateway
8. **Full channel status view** - Gateway status widget exists, but no per-channel connection dashboard

### Completed

- ✅ **Workspace integration** - Memory tools registered, workspace passed to Agent and heartbeat
- ✅ **WASM sandboxing** - Full implementation in `tools/wasm/` with fuel metering, memory limits, capabilities
- ✅ **Dynamic tool building** - `tools/builder/` has LlmSoftwareBuilder with iterative build loop
- ✅ **HTTP webhook security** - Secret validation implemented, proper error handling (no panics)
- ✅ **Embeddings integration** - OpenAI and NEAR AI providers wired to workspace for semantic search
- ✅ **Workspace system prompt** - Identity files (AGENTS.md, SOUL.md, USER.md, IDENTITY.md) injected into LLM context
- ✅ **Heartbeat notifications** - Route through channel manager (broadcast API) instead of logging-only
- ✅ **Auto-context compaction** - Triggers automatically when context exceeds threshold
- ✅ **Embedding backfill** - Runs on startup when embeddings provider is enabled
- ✅ **Clippy clean** - All warnings addressed via config struct refactoring
- ✅ **Tool approval enforcement** - Tools with `requires_approval()` (shell, http, file write/patch, build_software) now gate execution, track auto-approved tools per session
- ✅ **Tool definition refresh** - Tool definitions refreshed each iteration so newly built tools become visible in same session
- ✅ **Worker tool call handling** - Uses `respond_with_tools()` to properly execute tool calls when `select_tools()` returns empty
- ✅ **Gateway control plane** - Web gateway with 40+ API endpoints, SSE/WebSocket
- ✅ **Web Control UI** - Browser-based dashboard with chat, memory, jobs, logs, extensions, routines
- ✅ **Slack/Telegram channels** - Implemented as WASM tools
- ✅ **Docker sandbox** - Orchestrator/worker containers with per-job auth
- ✅ **Claude Code mode** - Delegate jobs to Claude CLI inside containers
- ✅ **Routines system** - Cron, event, webhook, and manual triggers with guardrails
- ✅ **Extension management** - Install, auth, activate MCP/WASM extensions via CLI and web UI

## Adding a New Tool

### Built-in Tools (Rust)

1. Create `src/tools/builtin/my_tool.rs`
2. Implement the `Tool` trait
3. Add `mod my_tool;` and `pub use` in `src/tools/builtin/mod.rs`
4. Register in `ToolRegistry::register_builtin_tools()` in `registry.rs`
5. Add tests

### WASM Tools (Recommended)

WASM tools are the preferred way to add new capabilities. They run in a sandboxed environment with explicit capabilities.

1. Create a new crate in `tools-src/<name>/`
2. Implement the WIT interface (`wit/tool.wit`)
3. Create `<name>.capabilities.json` declaring required permissions
4. Build with `cargo build --target wasm32-wasip2 --release`
5. Install with `ironclaw tool install path/to/tool.wasm`

See `tools-src/` for examples.

## Tool Architecture Principles

**CRITICAL: Keep tool-specific logic out of the main agent codebase.**

The main agent provides generic infrastructure; tools are self-contained units that declare their requirements through capabilities files.

### What Goes in Tools (capabilities.json)

- API endpoints the tool needs (HTTP allowlist)
- Credentials required (secret names, injection locations)
- Rate limits and timeouts
- Auth setup instructions (see below)
- Workspace paths the tool can read

### What Does NOT Go in Main Agent

- Service-specific auth flows (OAuth for Notion, Slack, etc.)
- Service-specific CLI commands (`auth notion`, `auth slack`)
- Service-specific configuration handling
- Hardcoded API URLs or token formats

### Tool Authentication

Tools declare their auth requirements in `<tool>.capabilities.json` under the `auth` section. Two methods are supported:

#### OAuth (Browser-based login)

For services that support OAuth, users just click through browser login:

```json
{
  "auth": {
    "secret_name": "notion_api_token",
    "display_name": "Notion",
    "oauth": {
      "authorization_url": "https://api.notion.com/v1/oauth/authorize",
      "token_url": "https://api.notion.com/v1/oauth/token",
      "client_id_env": "NOTION_OAUTH_CLIENT_ID",
      "client_secret_env": "NOTION_OAUTH_CLIENT_SECRET",
      "scopes": [],
      "use_pkce": false,
      "extra_params": { "owner": "user" }
    },
    "env_var": "NOTION_TOKEN"
  }
}
```

To enable OAuth for a tool:
1. Register a public OAuth app with the service (e.g., notion.so/my-integrations)
2. Configure redirect URIs: `http://localhost:9876/callback` through `http://localhost:9886/callback`
3. Set environment variables for client_id and client_secret

#### Manual Token Entry (Fallback)

For services without OAuth or when OAuth isn't configured:

```json
{
  "auth": {
    "secret_name": "openai_api_key",
    "display_name": "OpenAI",
    "instructions": "Get your API key from platform.openai.com/api-keys",
    "setup_url": "https://platform.openai.com/api-keys",
    "token_hint": "Starts with 'sk-'",
    "env_var": "OPENAI_API_KEY"
  }
}
```

#### Auth Flow Priority

When running `ironclaw tool auth <tool>`:

1. Check `env_var` - if set in environment, use it directly
2. Check `oauth` - if configured, open browser for OAuth flow
3. Fall back to `instructions` + manual token entry

The agent reads auth config from the tool's capabilities file and provides the appropriate flow. No service-specific code in the main agent.

### WASM Tools vs MCP Servers: When to Use Which

Both are first-class in the extension system (`ironclaw tool install` handles both), but they have different strengths.

**WASM Tools (IronClaw native)**

- Sandboxed: fuel metering, memory limits, no access except what's allowlisted
- Credentials injected by host runtime, tool code never sees the actual token
- Output scanned for secret leakage before returning to the LLM
- Auth (OAuth/manual) declared in `capabilities.json`, agent handles the flow
- Single binary, no process management, works offline
- Cost: must build yourself in Rust, no ecosystem, synchronous only

**MCP Servers (Model Context Protocol)**

- Growing ecosystem of pre-built servers (GitHub, Notion, Postgres, etc.)
- Any language (TypeScript/Python most common)
- Can do websockets, streaming, background polling
- Cost: external process with full system access (no sandbox), manages own credentials, IronClaw can't prevent leaks

**Decision guide:**

| Scenario | Use |
|----------|-----|
| Good MCP server already exists | **MCP** |
| Handles sensitive credentials (email send, banking) | **WASM** |
| Quick prototype or one-off integration | **MCP** |
| Core capability you'll maintain long-term | **WASM** |
| Needs background connections (websockets, polling) | **MCP** |
| Multiple tools share one OAuth token (e.g., Google suite) | **WASM** |

The LLM-facing interface is identical for both (tool name, schema, execute), so swapping between them is transparent to the agent.

## Adding a New Channel

1. Create `src/channels/my_channel.rs`
2. Implement the `Channel` trait
3. Add config in `src/config.rs`
4. Wire up in `main.rs` channel setup section

## Debugging

```bash
# Verbose logging
RUST_LOG=ironclaw=trace cargo run

# Just the agent module
RUST_LOG=ironclaw::agent=debug cargo run

# With HTTP request logging
RUST_LOG=ironclaw=debug,tower_http=debug cargo run
```

## Code Style

- Use `crate::` imports, not `super::`
- No `pub use` re-exports unless exposing to downstream consumers
- Prefer strong types over strings (enums, newtypes)
- Keep functions focused, extract helpers when logic is reused
- Comments for non-obvious logic only

## Review & Fix Discipline

Hard-won lessons from code review -- follow these when fixing bugs or addressing review feedback.

### Fix the pattern, not just the instance
When a reviewer flags a bug (e.g., TOCTOU race in INSERT + SELECT-back), search the entire codebase for all instances of that same pattern. A fix in `SecretsStore::create()` that doesn't also fix `WasmToolStore::store()` is half a fix.

### Propagate architectural fixes to satellite types
If a core type changes its concurrency model (e.g., `LibSqlBackend` switches to connection-per-operation), every type that was handed a resource from the old model (e.g., `LibSqlSecretsStore`, `LibSqlWasmToolStore` holding a single `Connection`) must also be updated. Grep for the old type across the codebase.

### Schema translation is more than DDL
When translating a database schema between backends (PostgreSQL to libSQL, etc.), check for:
- **Indexes** -- diff `CREATE INDEX` statements between the two schemas
- **Seed data** -- check for `INSERT INTO` in migrations (e.g., `leak_detection_patterns`)
- **Semantic differences** -- document where SQL functions behave differently (e.g., `json_patch` vs `jsonb_set`)

### Feature flag testing
When adding feature-gated code, test compilation with each feature in isolation:
```bash
cargo check                                          # default features
cargo check --no-default-features --features libsql  # libsql only
cargo check --all-features                           # all features
```
Dead code behind the wrong `#[cfg]` gate will only show up when building with a single feature.

### Mechanical verification before committing
Run these checks on changed files before committing:
- `grep -rnE '\.unwrap\(|\.expect\(' <files>` -- no panics in production
- `grep -rn 'super::' <files>` -- use `crate::` imports
- If you fixed a pattern bug, `grep` for other instances of that pattern across `src/`

## Workspace & Memory System

Inspired by [OpenClaw](https://github.com/openclaw/openclaw), the workspace provides persistent memory for agents with a flexible filesystem-like structure.

### Key Principles

1. **"Memory is database, not RAM"** - If you want to remember something, write it explicitly
2. **Flexible structure** - Create any directory/file hierarchy you need
3. **Self-documenting** - Use README.md files to describe directory structure
4. **Hybrid search** - Combines FTS (keyword) + vector (semantic) via Reciprocal Rank Fusion

### Filesystem Structure

```
workspace/
├── README.md              <- Root runbook/index
├── MEMORY.md              <- Long-term curated memory
├── HEARTBEAT.md           <- Periodic checklist
├── IDENTITY.md            <- Agent name, nature, vibe
├── SOUL.md                <- Core values
├── AGENTS.md              <- Behavior instructions
├── USER.md                <- User context
├── context/               <- Identity-related docs
│   ├── vision.md
│   └── priorities.md
├── daily/                 <- Daily logs
│   ├── 2024-01-15.md
│   └── 2024-01-16.md
├── projects/              <- Arbitrary structure
│   └── alpha/
│       ├── README.md
│       └── notes.md
└── ...
```

### Using the Workspace

```rust
use crate::workspace::{Workspace, OpenAiEmbeddings, paths};

// Create workspace for a user
let workspace = Workspace::new("user_123", pool)
    .with_embeddings(Arc::new(OpenAiEmbeddings::new(api_key)));

// Read/write any path
let doc = workspace.read("projects/alpha/notes.md").await?;
workspace.write("context/priorities.md", "# Priorities\n\n1. Feature X").await?;
workspace.append("daily/2024-01-15.md", "Completed task X").await?;

// Convenience methods for well-known files
workspace.append_memory("User prefers dark mode").await?;
workspace.append_daily_log("Session note").await?;

// List directory contents
let entries = workspace.list("projects/").await?;

// Search (hybrid FTS + vector)
let results = workspace.search("dark mode preference", 5).await?;

// Get system prompt from identity files
let prompt = workspace.system_prompt().await?;
```

### Memory Tools

Four tools for LLM use:

- **`memory_search`** - Hybrid search, MUST be called before answering questions about prior work
- **`memory_write`** - Write to any path (memory, daily_log, or custom paths)
- **`memory_read`** - Read any file by path
- **`memory_tree`** - View workspace structure as a tree (depth parameter, default 1)

### Hybrid Search (RRF)

Combines full-text search (PostgreSQL `ts_rank_cd`) and vector similarity (pgvector cosine) using Reciprocal Rank Fusion:

```
score(d) = Σ 1/(k + rank(d)) for each method where d appears
```

Default k=60. Results from both methods are combined, with documents appearing in both getting boosted scores.

### Heartbeat System

Proactive periodic execution (default: 30 minutes):

1. Reads `HEARTBEAT.md` checklist
2. Runs agent turn with checklist prompt
3. If findings, notifies via channel
4. If nothing, agent replies "HEARTBEAT_OK" (no notification)

```rust
use crate::agent::{HeartbeatConfig, spawn_heartbeat};

let config = HeartbeatConfig::default()
    .with_interval(Duration::from_secs(60 * 30))
    .with_notify("user_123", "telegram");

spawn_heartbeat(config, workspace, llm, response_tx);
```

### Chunking Strategy

Documents are chunked for search indexing:
- Default: 800 words per chunk (roughly 800 tokens for English)
- 15% overlap between chunks for context preservation
- Minimum chunk size: 50 words (tiny trailing chunks merge with previous)
