# NEAR Agent Development Guide

## Project Overview

LLM-powered autonomous agent for the NEAR AI marketplace. Handles multi-channel input (CLI, HTTP, Slack, Telegram), parallel job execution, extensible tools (including MCP), prompt injection defense, and self-repair for stuck jobs.

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
RUST_LOG=near_agent=debug cargo run
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
│   └── heartbeat.rs    # Proactive periodic execution (OpenClaw-inspired)
│
├── channels/           # Multi-channel input
│   ├── channel.rs      # Channel trait, IncomingMessage, OutgoingResponse
│   ├── manager.rs      # ChannelManager merges streams
│   ├── cli.rs          # Interactive CLI (stdin)
│   ├── http.rs         # HTTP webhook (axum)
│   ├── slack.rs        # Stub
│   └── telegram.rs     # Stub
│
├── safety/             # Prompt injection defense
│   ├── sanitizer.rs    # Pattern detection, content escaping
│   ├── validator.rs    # Input validation (length, encoding, patterns)
│   └── policy.rs       # PolicyRule system with severity/actions
│
├── llm/                # LLM integration
│   ├── provider.rs     # LlmProvider trait, message types
│   ├── nearai.rs       # NEAR AI chat-api (default, unified interface)
│   ├── openai.rs       # OpenAI API implementation
│   ├── anthropic.rs    # Anthropic API implementation
│   └── reasoning.rs    # Planning, tool selection, evaluation
│
├── tools/              # Extensible tool system
│   ├── tool.rs         # Tool trait, ToolOutput, ToolError
│   ├── registry.rs     # ToolRegistry for discovery
│   ├── builder.rs      # Dynamic tool creation (stub)
│   ├── sandbox.rs      # Sandboxed execution (stub)
│   ├── builtin/        # Built-in tools
│   │   ├── echo.rs, time.rs, json.rs, http.rs
│   │   ├── marketplace.rs, ecommerce.rs
│   │   ├── taskrabbit.rs, restaurant.rs
│   │   └── memory.rs   # Memory tools (search, write, read)
│   └── mcp/            # Model Context Protocol
│       ├── client.rs   # MCP client over HTTP
│       └── protocol.rs # JSON-RPC types
│
├── workspace/          # Persistent memory system (OpenClaw-inspired)
│   ├── mod.rs          # Workspace struct, memory operations
│   ├── document.rs     # DocType enum, MemoryDocument, MemoryChunk
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
│   ├── success.rs      # SuccessEvaluator trait, RuleBasedEvaluator
│   └── metrics.rs      # MetricsCollector, QualityMetrics
│
└── history/            # Persistence
    ├── store.rs        # PostgreSQL repositories
    └── analytics.rs    # Aggregation queries for learning
```

## Key Patterns

### Error Handling
- Use `thiserror` for error types in `error.rs`
- Never use `.unwrap()` in production code (tests are fine)
- Map errors with context: `.map_err(|e| SomeError::Variant { reason: e.to_string() })?`

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
DATABASE_URL=postgres://user:pass@localhost/near_agent

# LLM Provider (default: nearai)
LLM_PROVIDER=nearai  # Options: nearai, openai, anthropic

# NEAR AI (recommended - unified API with user auth)
NEARAI_SESSION_TOKEN=sess_...
NEARAI_MODEL=claude-3-5-sonnet-20241022
NEARAI_BASE_URL=https://api.near.ai

# OpenAI (alternative)
OPENAI_API_KEY=sk-...
OPENAI_MODEL=gpt-4-turbo

# Anthropic (alternative)
ANTHROPIC_API_KEY=sk-ant-...
ANTHROPIC_MODEL=claude-3-opus-20240229

# Agent settings
AGENT_NAME=near-agent
MAX_PARALLEL_JOBS=5
```

### NEAR AI Provider

The default provider uses the NEAR AI chat-api (`https://api.near.ai/v1/responses`) which provides:
- Unified access to multiple models (OpenAI, Anthropic, etc.)
- User authentication via session tokens
- Usage tracking and billing through NEAR AI

Session tokens have the format `sess_xxx` (37 characters). They are authenticated against the NEAR AI auth service.

## Database

Migrations in `migrations/`. Tables:

**V1 (initial):**
- `conversations` - Multi-channel conversation tracking
- `agent_jobs` - Job metadata and status
- `job_actions` - Event-sourced tool executions
- `dynamic_tools` - Agent-built tools
- `llm_calls` - Cost tracking
- `estimation_snapshots` - Learning data

**V2 (workspace/memory):**
- `memory_documents` - Full documents (MEMORY.md, daily logs, identity files)
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

1. **Slack/Telegram channels** - Stubs only, need implementation
2. **Tool sandboxing** - `sandbox.rs` is a stub, needs WASM integration
3. **Dynamic tool building** - `builder.rs` placeholder, needs LLM code generation
4. **Database integration** - Store is created but not fully wired into agent loop
5. **Integration tests** - Need testcontainers setup for PostgreSQL
6. **MCP stdio transport** - Only HTTP transport implemented
7. **Workspace integration** - Memory tools need to be registered and workspace passed to workers
8. **Embedding backfill** - Background job to generate embeddings for chunks missing them
9. **Context compaction** - Auto-trigger memory preservation before context window fills

## Adding a New Tool

1. Create `src/tools/builtin/my_tool.rs`
2. Implement the `Tool` trait
3. Add `mod my_tool;` and `pub use` in `src/tools/builtin/mod.rs`
4. Register in `ToolRegistry::register_builtin_tools()` in `registry.rs`
5. Add tests

## Adding a New Channel

1. Create `src/channels/my_channel.rs`
2. Implement the `Channel` trait
3. Add config in `src/config.rs`
4. Wire up in `main.rs` channel setup section

## Debugging

```bash
# Verbose logging
RUST_LOG=near_agent=trace cargo run

# Just the agent module
RUST_LOG=near_agent::agent=debug cargo run

# With HTTP request logging
RUST_LOG=near_agent=debug,tower_http=debug cargo run
```

## Code Style

- Use `crate::` imports, not `super::`
- No `pub use` re-exports unless exposing to downstream consumers
- Prefer strong types over strings (enums, newtypes)
- Keep functions focused, extract helpers when logic is reused
- Comments for non-obvious logic only

## Workspace & Memory System

Inspired by [OpenClaw](https://github.com/openclaw/openclaw), the workspace provides persistent memory for agents.

### Key Principles

1. **"Memory is files, not RAM"** - If you want to remember something, write it explicitly
2. **Two-tier memory** - Daily logs (raw) + curated MEMORY.md (distilled wisdom)
3. **Hybrid search** - Combines FTS (keyword) + vector (semantic) via Reciprocal Rank Fusion

### Document Types

| Type | Purpose | Singleton |
|------|---------|-----------|
| `Memory` | Long-term curated facts (MEMORY.md) | Yes |
| `DailyLog` | Append-only daily notes (keyed by date) | No |
| `Identity` | Agent name, nature, vibe | Yes |
| `Soul` | Core values and principles | Yes |
| `Agents` | Behavior instructions | Yes |
| `User` | User context (name, preferences) | Yes |
| `Heartbeat` | Periodic checklist | Yes |

### Using the Workspace

```rust
use crate::workspace::{Workspace, DocType, OpenAiEmbeddings};

// Create workspace for a user
let workspace = Workspace::new("user_123", pool)
    .with_embeddings(Arc::new(OpenAiEmbeddings::new(api_key)));

// Write to memory
workspace.append_memory("User prefers dark mode").await?;
workspace.append_daily_log("Completed task X").await?;

// Search (hybrid FTS + vector)
let results = workspace.search("dark mode preference", 5).await?;

// Get system prompt from identity files
let prompt = workspace.system_prompt().await?;
```

### Memory Tools

Three tools for LLM use:

- **`memory_search`** - Hybrid search, MUST be called before answering questions about prior work
- **`memory_write`** - Write to memory or daily_log target
- **`memory_read`** - Read specific document by type

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
