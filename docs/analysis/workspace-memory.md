# IronClaw Codebase Analysis — Workspace, Memory & Storage

> Updated: 2026-02-22 | Version: v0.9.0

## 1. Overview

IronClaw's persistence layer is built around a principle borrowed from OpenClaw: **"Memory is database, not RAM."** Every fact an agent wants to remember must be written explicitly. The system provides three interlocking layers:

1. **Database backends** — Two compile-time-selectable backends (`postgres` and `libsql`) unified behind a single `Database` trait. All persistence routes through this trait.
2. **Workspace filesystem** — A virtual, path-based document store modeled after a filesystem. Documents are stored in the database and indexed for full-text and semantic search. Agents use this as their persistent memory.
3. **Context and history** — Per-job in-memory state (`JobContext`, `Memory`) mirrored to the database via the `Store` / `Database` trait. Conversation turns, tool action records, and LLM call logs are all persisted here.

The relevant source locations are:

| Module | Path |
|--------|------|
| Workspace | `src/workspace/` |
| Database trait | `src/db/mod.rs` |
| PostgreSQL backend | `src/db/postgres.rs`, `src/history/store.rs` |
| libSQL backend | `src/db/libsql/` |
| libSQL schema | `src/db/libsql_migrations.rs` |
| PostgreSQL migrations | `migrations/` |
| Job context / state | `src/context/state.rs`, `src/context/memory.rs` |
| Context manager | `src/context/manager.rs` |
| Embeddings config | `src/config/embeddings.rs` |
| Database config | `src/config/database.rs` |

---

## 2. Database Backends

### 2.1 PostgreSQL Backend

The PostgreSQL backend is the default and is compiled in when the `postgres` Cargo feature is active (the default). It is implemented in two layers:

- **`Store`** (`src/history/store.rs`) — handles conversations, jobs, actions, LLM call records, sandbox jobs, routines, tool failures, and settings. Uses `deadpool-postgres` for connection pooling with configurable pool size (default: 10, controlled by `DATABASE_POOL_SIZE`).
- **`Repository`** (`src/workspace/repository.rs`) — handles workspace documents and memory chunks. Receives a `deadpool_postgres::Pool` cloned from the `Store`.
- **`PgBackend`** (`src/db/postgres.rs`) — wraps both `Store` and `Repository` to implement the unified `Database` trait.

Key capabilities specific to PostgreSQL:

- **pgvector** extension: The `memory_chunks` table stores embeddings as `VECTOR(1536)` (or another dimension). Vector similarity search uses the `<=>` cosine distance operator: `1 - (c.embedding <=> $3) as similarity`. The `pgvector` crate bridges Rust `Vec<f32>` to the Postgres `vector` type.
- **Full-text search**: `memory_chunks` carries a `content_tsv tsvector` column populated by a trigger. FTS queries use `plainto_tsquery('english', $3)` and `ts_rank_cd` for ranking.
- **PL/pgSQL functions**: The `list_workspace_files` function implements virtual directory listing by path prefix matching.
- **refinery** migration management: `Store::run_migrations()` calls `refinery::embed_migrations!("migrations")` and runs all pending versioned SQL files automatically on startup.

When to use: production deployments, multi-user scenarios, shared database, when pgvector's HNSW/IVFFlat indexes are needed for large memory corpora.

### 2.2 libSQL (Turso) Backend

The libSQL backend is compiled in with `--no-default-features --features libsql`. It is implemented as a modular set of files under `src/db/libsql/`:

```
src/db/libsql/
├── mod.rs             — LibSqlBackend struct, connect(), run_migrations()
├── conversations.rs   — ConversationStore impl
├── jobs.rs            — JobStore impl
├── sandbox.rs         — SandboxStore impl
├── routines.rs        — RoutineStore impl
├── settings.rs        — SettingsStore impl
├── tool_failures.rs   — ToolFailureStore impl
└── workspace.rs       — WorkspaceStore impl
```

`LibSqlBackend` operates in two modes:

- **Local embedded** (`new_local`): opens a file at the path given by `LIBSQL_PATH` (default: `~/.ironclaw/ironclaw.db`). No server process required; the database file is a single file on disk.
- **Remote replica** (`new_remote_replica`): opens a local replica file that syncs bidirectionally with a Turso cloud endpoint. Requires `LIBSQL_URL` (e.g., `libsql://xxx.turso.io`) and `LIBSQL_AUTH_TOKEN`.

The backend gets a fresh connection for each operation via `self.connect().await` to avoid shared mutable connection state across async tasks. This is the connection-per-operation model.

Type translations from PostgreSQL:

| PostgreSQL | libSQL |
|-----------|--------|
| `UUID` | `TEXT` (hex string) |
| `TIMESTAMPTZ` | `TEXT` (ISO-8601) |
| `JSONB` | `TEXT` (JSON encoded) |
| `BYTEA` | `BLOB` |
| `NUMERIC` | `TEXT` (rust_decimal precision) |
| `TEXT[]` | `TEXT` (JSON array) |
| `VECTOR(1536)` | `F32_BLOB(1536)` (libSQL native type) |
| `TSVECTOR` column + GIN index | FTS5 virtual table + sync triggers |
| `BIGSERIAL` | `INTEGER PRIMARY KEY AUTOINCREMENT` |
| PL/pgSQL functions | SQLite triggers |

Vector search in libSQL uses `vector_top_k('idx_memory_chunks_embedding', vector(?1), ?2)` against a `libsql_vector_idx` index, where the query vector is passed as a JSON array string.

When to use: personal/local deployments, single-machine setups, edge/embedded scenarios, zero-server dependency, or when combined with optional Turso cloud sync.

### 2.3 Migration System

**PostgreSQL**: Migrations are managed by the `refinery` crate. SQL files in `migrations/` follow the naming convention `V{N}__{description}.sql` (double underscore). They are embedded at compile time via `embed_migrations!("migrations")` and applied automatically on startup.

Migration files found in `migrations/`:

| File | Description |
|------|-------------|
| `V1__initial.sql` | Complete base schema: conversations, jobs, job actions, dynamic tools, LLM calls, estimation snapshots, workspace memory documents and chunks (with pgvector and tsvector), heartbeat state, secrets, WASM tools, tool capabilities, leak detection |
| `V2__wasm_secure_api.sql` | WASM secure API additions (tool capabilities, rate limits, secret usage audit, leak detection events) |
| `V3__tool_failures.sql` | `tool_failures` table for self-repair tracking |
| `V4__sandbox_columns.sql` | Sandbox job columns: `source`, `user_id`, `project_dir`, `job_mode` |
| `V5__claude_code.sql` | Claude Code mode column (`job_mode`) |
| `V6__routines.sql` | `routines` and `routine_runs` tables for scheduled/reactive execution |
| `V7__rename_events.sql` | Rename `sandbox_events` to `job_events` |
| `V8__settings.sql` | `settings` key-value table |
| `V9__flexible_embedding_dimension.sql` | Alter `memory_chunks.embedding` to support variable vector dimensions |

**libSQL**: A single consolidated schema in `src/db/libsql_migrations.rs` (`SCHEMA` constant, ~549 lines). All tables are created with `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS`, making the schema idempotent. Applied once via `LibSqlBackend::run_migrations()`. No incremental ALTER TABLE support; schema changes require a new consolidated schema version.

---

## 3. Database Schema

The schema below covers both backends. PostgreSQL uses native types; libSQL equivalents are noted where they differ.

### `conversations` table

Tracks multi-channel conversation sessions.

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID / TEXT | Primary key |
| `channel` | TEXT | Input channel (e.g., `tui`, `web`, `telegram`) |
| `user_id` | TEXT | User identifier |
| `thread_id` | TEXT (nullable) | External thread ID (e.g., Slack thread) |
| `started_at` | TIMESTAMPTZ / TEXT | Conversation start time |
| `last_activity` | TIMESTAMPTZ / TEXT | Last message timestamp; updated on each message |
| `metadata` | JSONB / TEXT | Flexible JSON metadata (e.g., `thread_type: "assistant"`) |

Indexes: `channel`, `user_id`, `last_activity`.

### `conversation_messages` table

Individual messages within a conversation.

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID / TEXT | Primary key |
| `conversation_id` | UUID / TEXT | FK to `conversations(id)` ON DELETE CASCADE |
| `role` | TEXT | `user`, `assistant`, or `system` |
| `content` | TEXT | Message content |
| `created_at` | TIMESTAMPTZ / TEXT | Message timestamp |

### `agent_jobs` table

Job metadata and status. Also used for sandbox container jobs (differentiated by `source = 'sandbox'`).

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID / TEXT | Primary key |
| `conversation_id` | UUID / TEXT (nullable) | FK to `conversations(id)` |
| `title` | TEXT | Short job title |
| `description` | TEXT | Full job description (or credential grants JSON for sandbox jobs) |
| `category` | TEXT (nullable) | Job category for routing |
| `status` | TEXT | State: `pending`, `in_progress`, `completed`, `submitted`, `accepted`, `failed`, `stuck`, `cancelled` |
| `source` | TEXT | `direct`, `sandbox`, `routine`, etc. |
| `user_id` | TEXT | Owner user ID |
| `project_dir` | TEXT (nullable) | Sandbox working directory |
| `job_mode` | TEXT | `worker` (default) or `claude_code` |
| `budget_amount` | NUMERIC / TEXT (nullable) | Budget ceiling |
| `budget_token` | TEXT (nullable) | Budget currency (e.g., `NEAR`, `USD`) |
| `bid_amount` | NUMERIC / TEXT (nullable) | Agent's bid |
| `estimated_cost` | NUMERIC / TEXT (nullable) | Pre-job cost estimate |
| `estimated_time_secs` | INTEGER (nullable) | Pre-job duration estimate |
| `estimated_value` | NUMERIC / TEXT (nullable) | Expected value delivered |
| `actual_cost` | NUMERIC / TEXT (nullable) | Accumulated actual cost |
| `actual_time_secs` | INTEGER (nullable) | Actual duration |
| `success` | BOOLEAN / INTEGER (nullable) | Final success flag |
| `failure_reason` | TEXT (nullable) | Human-readable failure description |
| `stuck_since` | TIMESTAMPTZ / TEXT (nullable) | When the job became stuck |
| `repair_attempts` | INTEGER | Number of self-repair attempts |
| `created_at` | TIMESTAMPTZ / TEXT | Job creation time |
| `started_at` | TIMESTAMPTZ / TEXT (nullable) | When execution began |
| `completed_at` | TIMESTAMPTZ / TEXT (nullable) | When execution ended |

### `job_actions` table

Event-sourced log of every tool call within a job.

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID / TEXT | Primary key |
| `job_id` | UUID / TEXT | FK to `agent_jobs(id)` ON DELETE CASCADE |
| `sequence_num` | INTEGER | Monotonic action sequence within the job |
| `tool_name` | TEXT | Name of the tool invoked |
| `input` | JSONB / TEXT | Tool parameters |
| `output_raw` | TEXT (nullable) | Raw tool output before sanitization |
| `output_sanitized` | JSONB / TEXT (nullable) | Output after safety layer processing |
| `sanitization_warnings` | JSONB / TEXT (nullable) | Warnings from the safety layer |
| `cost` | NUMERIC / TEXT (nullable) | Cost attributed to this action |
| `duration_ms` | INTEGER (nullable) | Wall-clock execution time |
| `success` | BOOLEAN / INTEGER | Whether the tool call succeeded |
| `error_message` | TEXT (nullable) | Error if not successful |
| `created_at` | TIMESTAMPTZ / TEXT | When the action executed |

### `routines` table

Scheduled (cron) and reactive (event, webhook) automation definitions.

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID / TEXT | Primary key |
| `name` | TEXT | Unique name within a user's scope |
| `description` | TEXT | Human-readable description |
| `user_id` | TEXT | Owner user ID |
| `enabled` | BOOLEAN / INTEGER | Whether the routine fires |
| `trigger_type` | TEXT | `cron`, `event`, or `webhook` |
| `trigger_config` | JSONB / TEXT | Type-specific trigger configuration |
| `action_type` | TEXT | `llm_task`, `shell`, etc. |
| `action_config` | JSONB / TEXT | Type-specific action configuration |
| `cooldown_secs` | INTEGER | Minimum seconds between runs (default: 300) |
| `max_concurrent` | INTEGER | Maximum simultaneous runs |
| `dedup_window_secs` | INTEGER (nullable) | Deduplication window |
| `notify_channel` | TEXT (nullable) | Channel to notify on completion |
| `notify_user` | TEXT | User to notify |
| `notify_on_success` | BOOLEAN / INTEGER | Notify on success |
| `notify_on_failure` | BOOLEAN / INTEGER | Notify on failure |
| `notify_on_attention` | BOOLEAN / INTEGER | Notify when attention needed |
| `state` | JSONB / TEXT | Runtime state for stateful routines |
| `last_run_at` | TIMESTAMPTZ / TEXT (nullable) | When the routine last ran |
| `next_fire_at` | TIMESTAMPTZ / TEXT (nullable) | When cron is next scheduled |
| `run_count` | INTEGER | Total lifetime runs |
| `consecutive_failures` | INTEGER | Consecutive failure streak |
| `created_at` / `updated_at` | TIMESTAMPTZ / TEXT | Audit timestamps |

### `memory_documents` table

The core of the workspace filesystem. Each row is a file identified by a virtual path.

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID / TEXT | Primary key |
| `user_id` | TEXT | Owner user ID |
| `agent_id` | UUID / TEXT (nullable) | Optional agent ID for multi-agent isolation |
| `path` | TEXT | Normalized virtual path (e.g., `context/vision.md`, `daily/2024-01-15.md`) |
| `content` | TEXT | Full document content (Markdown) |
| `created_at` | TIMESTAMPTZ / TEXT | Creation timestamp |
| `updated_at` | TIMESTAMPTZ / TEXT | Last modification timestamp; auto-updated by trigger |
| `metadata` | JSONB / TEXT | Flexible JSON metadata |

Unique constraint: `(user_id, agent_id, path)`. Indexes on `user_id`, `(user_id, path)`, `updated_at DESC`.

### `memory_chunks` table

Documents are split into overlapping chunks for indexing. Each chunk is a substring of a document.

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID / TEXT | Primary key |
| `document_id` | UUID / TEXT | FK to `memory_documents(id)` ON DELETE CASCADE |
| `chunk_index` | INTEGER | Position within the document (0-based) |
| `content` | TEXT | Chunk text (approximately 800 words) |
| `embedding` | VECTOR(1536) / F32_BLOB(1536) (nullable) | Embedding vector; NULL until generated |
| `created_at` | TIMESTAMPTZ / TEXT | Chunk creation timestamp |

In PostgreSQL: a `content_tsv tsvector` column carries the FTS index, updated by trigger. A pgvector index enables nearest-neighbor search on `embedding`.

In libSQL: a companion FTS5 virtual table `memory_chunks_fts` is kept in sync via INSERT/DELETE/UPDATE triggers. A `libsql_vector_idx` index on the `embedding` column enables vector search via `vector_top_k()`.

### `settings` table

Per-user runtime key-value settings store.

| Column | Type | Description |
|--------|------|-------------|
| `user_id` | TEXT | Owner user ID |
| `key` | TEXT | Setting key |
| `value` | JSONB / TEXT | JSON-encoded setting value |
| `updated_at` | TIMESTAMPTZ / TEXT | Last modification time |

Primary key: `(user_id, key)`. Upsert semantics on every write.

### `heartbeat_state` table

Tracks per-user heartbeat execution state.

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID / TEXT | Primary key |
| `user_id` | TEXT | Owner user ID |
| `agent_id` | UUID / TEXT (nullable) | Optional agent scope |
| `last_run` | TIMESTAMPTZ / TEXT (nullable) | Last heartbeat execution time |
| `next_run` | TIMESTAMPTZ / TEXT (nullable) | Scheduled next run |
| `interval_seconds` | INTEGER | Heartbeat interval (default: 1800) |
| `enabled` | BOOLEAN / INTEGER | Whether heartbeats are active |
| `consecutive_failures` | INTEGER | Consecutive failure count |
| `last_checks` | JSONB / TEXT | Results of the last checklist evaluation |

---

## 4. Workspace Filesystem (`workspace/`)

The workspace is a **virtual filesystem stored entirely in the database**. There is no real filesystem involvement — "directories" are inferred from path prefixes in the `memory_documents` table. Agents interact with the workspace through the `Workspace` struct (`src/workspace/mod.rs`).

### API

The core API surface exposed by `Workspace`:

```rust
workspace.read("context/vision.md")            // -> MemoryDocument
workspace.write("MEMORY.md", content)           // create or overwrite; re-indexes chunks
workspace.append("daily/2024-01-15.md", entry)  // append with newline separator
workspace.exists("SOUL.md")                     // -> bool
workspace.delete("old/file.md")                 // also deletes associated chunks
workspace.list("projects/")                     // -> Vec<WorkspaceEntry> (non-recursive)
workspace.list_all()                            // -> Vec<String> (all paths, flat)
workspace.search("query text", limit)           // hybrid FTS + vector search via RRF
```

Every `write` or `append` triggers `reindex_document`: old chunks are deleted, the content is split into new chunks, and embeddings are generated for each chunk if a provider is configured.

### Storage Backend Abstraction

`Workspace` holds a `WorkspaceStorage` enum that dispatches to either the PostgreSQL `Repository` (when compiled with the `postgres` feature) or any `Arc<dyn crate::db::Database>` (the generic path used by libSQL and any future backend). The two constructors are:

```rust
Workspace::new(user_id, pool)        // PostgreSQL: takes deadpool Pool directly
Workspace::new_with_db(user_id, db)  // Generic: takes Arc<dyn Database>
```

### Path Conventions

Paths are normalized: leading/trailing slashes stripped, double slashes collapsed. Well-known paths are defined in `workspace/document.rs` under `paths::`:

| Constant | Path | Purpose |
|----------|------|---------|
| `paths::README` | `README.md` | Root workspace index |
| `paths::MEMORY` | `MEMORY.md` | Long-term curated memory |
| `paths::IDENTITY` | `IDENTITY.md` | Agent name and nature |
| `paths::SOUL` | `SOUL.md` | Core values and principles |
| `paths::AGENTS` | `AGENTS.md` | Behavior instructions |
| `paths::USER` | `USER.md` | User context and preferences |
| `paths::HEARTBEAT` | `HEARTBEAT.md` | Periodic task checklist |
| `paths::DAILY_DIR` | `daily/` | Daily log directory |
| `paths::CONTEXT_DIR` | `context/` | Identity-related context documents |

Daily logs are keyed by date: `daily/2024-01-15.md`. They are created automatically and timestamped on append: `[15:32:01] entry text`.

### Identity Files

Four files compose the agent's identity, loaded in order by `Workspace::system_prompt()`:

| File | Header Injected | Purpose |
|------|----------------|---------|
| `AGENTS.md` | `## Agent Instructions` | Behavioral guidelines and tool usage rules |
| `SOUL.md` | `## Core Values` | Principles that govern agent behavior |
| `USER.md` | `## User Context` | User name, preferences, ongoing context |
| `IDENTITY.md` | `## Identity` | Agent name, nature, personality |

After identity files, the system prompt also appends today's and yesterday's daily logs under `## Today's Notes` / `## Yesterday's Notes`. All sections are joined with `\n\n---\n\n`.

The system prompt is assembled at session start and injected as the LLM system message. Files that do not exist or are empty are silently skipped.

### Seeding

On every boot, `Workspace::seed_if_empty()` checks for each identity file and creates it with a default template if missing. Existing files are never overwritten, preserving user edits. The method returns the count of files created. Seeded files: `README.md`, `MEMORY.md`, `IDENTITY.md`, `SOUL.md`, `AGENTS.md`, `USER.md`, `HEARTBEAT.md`.

### Multi-Agent Isolation

Each workspace operation accepts an optional `agent_id: Option<Uuid>`. Documents are scoped by `(user_id, agent_id, path)`. The `Workspace::with_agent(agent_id)` builder pins all operations to a specific agent scope, enabling multiple agents to share a database without interfering with each other's memory.

---

## 5. Semantic Memory System

### 5.1 Memory Chunks

When a document is written or updated, `Workspace::reindex_document()` performs the following steps:

1. Fetch the document content from the database.
2. Split it into overlapping chunks via `chunk_document()`.
3. Delete all existing `memory_chunks` rows for the document.
4. For each chunk, optionally call the embedding provider and insert a row into `memory_chunks`.

Chunks are represented by `MemoryChunk` (`src/workspace/document.rs`):

```rust
pub struct MemoryChunk {
    pub id: Uuid,
    pub document_id: Uuid,
    pub chunk_index: i32,      // 0-based position within document
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub created_at: DateTime<Utc>,
}
```

If the embedding provider is unavailable, chunks are inserted with `embedding = NULL` and are still searchable via FTS. The `backfill_embeddings()` method can generate embeddings for up to 100 unembedded chunks in one call, enabling gradual embedding after enabling a provider.

Memory tools exposed to the LLM (via `src/tools/builtin/memory.rs`): `memory_search`, `memory_write`, `memory_read`, `memory_tree`.

### 5.2 Embedding Pipeline

```
User input / document content
        │
        ▼
  chunk_document()          ← ~800 words per chunk, 15% overlap
        │
        ▼
  EmbeddingProvider::embed()   ← HTTP POST to embedding API
        │
        ▼
  Vec<f32> stored in DB     ← VECTOR(1536) or F32_BLOB(1536)
```

The `EmbeddingProvider` trait (`src/workspace/embeddings.rs`) defines:

```rust
pub trait EmbeddingProvider: Send + Sync {
    fn dimension(&self) -> usize;
    fn model_name(&self) -> &str;
    fn max_input_length(&self) -> usize;       // 32,000 chars for all API providers
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError>;
}
```

Supported providers:

| Provider | Struct | Default Model | Dimensions |
|----------|--------|--------------|-----------|
| OpenAI | `OpenAiEmbeddings` | `text-embedding-3-small` | 1536 |
| OpenAI (large) | `OpenAiEmbeddings::large()` | `text-embedding-3-large` | 3072 |
| OpenAI (legacy) | `OpenAiEmbeddings::ada_002()` | `text-embedding-ada-002` | 1536 |
| NEAR AI | `NearAiEmbeddings` | `text-embedding-3-small` | 1536 |
| Ollama (local) | `OllamaEmbeddings` | `nomic-embed-text` | 768 |
| Mock (tests) | `MockEmbeddings` | `mock-embedding` | configurable |

The OpenAI provider sends batches in a single POST to `https://api.openai.com/v1/embeddings`. Rate limit (HTTP 429) and auth errors (HTTP 401) are surfaced as typed `EmbeddingError` variants. The Ollama provider calls `{base_url}/api/embed` and validates the returned dimension against the configured dimension.

### 5.3 Document Chunker (`workspace/chunker.rs`)

The primary chunking function `chunk_document(content, config)` uses a word-based sliding window:

```
ChunkConfig {
    chunk_size:      800,   // target words per chunk
    overlap_percent: 0.15,  // 15% overlap between adjacent chunks
    min_chunk_size:  50,    // don't create trailing chunks smaller than this
}
```

Derived values:

- `overlap_size = chunk_size * overlap_percent` = 120 words
- `step_size = chunk_size - overlap_size` = 680 words

For content smaller than `chunk_size`, a single chunk containing the full content is returned. Trailing chunks smaller than `min_chunk_size` words are merged with the preceding chunk to avoid micro-chunks.

An alternative `chunk_by_paragraphs(content, config)` function (currently marked `#[allow(dead_code)]`) splits on double newlines first, then falls back to word-based chunking for paragraphs exceeding `chunk_size`. This respects semantic boundaries in Markdown documents.

### 5.4 Hybrid Search (`workspace/search.rs`)

Search runs two retrieval strategies in parallel and merges them with Reciprocal Rank Fusion.

**Full-text search (PostgreSQL)**:

```sql
SELECT c.id, c.document_id, c.content,
       ts_rank_cd(c.content_tsv, plainto_tsquery('english', $3)) as rank
FROM memory_chunks c
JOIN memory_documents d ON d.id = c.document_id
WHERE d.user_id = $1 AND d.agent_id IS NOT DISTINCT FROM $2
  AND c.content_tsv @@ plainto_tsquery('english', $3)
ORDER BY rank DESC
LIMIT $4
```

**Full-text search (libSQL)**:

```sql
SELECT c.id, c.document_id, c.content
FROM memory_chunks_fts fts
JOIN memory_chunks c ON c._rowid = fts.rowid
JOIN memory_documents d ON d.id = c.document_id
WHERE d.user_id = ?1 AND d.agent_id IS ?2
  AND memory_chunks_fts MATCH ?3
ORDER BY rank
LIMIT ?4
```

**Vector similarity search (PostgreSQL)**:

```sql
SELECT c.id, c.document_id, c.content,
       1 - (c.embedding <=> $3) as similarity
FROM memory_chunks c
JOIN memory_documents d ON d.id = c.document_id
WHERE d.user_id = $1 AND d.agent_id IS NOT DISTINCT FROM $2
  AND c.embedding IS NOT NULL
ORDER BY c.embedding <=> $3
LIMIT $4
```

**Vector similarity search (libSQL)**:

```sql
SELECT c.id, c.document_id, c.content
FROM vector_top_k('idx_memory_chunks_embedding', vector(?1), ?2) AS top_k
JOIN memory_chunks c ON c._rowid = top_k.id
JOIN memory_documents d ON d.id = c.document_id
WHERE d.user_id = ?3 AND d.agent_id IS ?4
```

**Reciprocal Rank Fusion (RRF)**:

The `reciprocal_rank_fusion(fts_results, vector_results, config)` function merges both ranked lists into a single score per chunk:

```
score(chunk) = Σ  1 / (k + rank_i)
               i
```

where `k = 60` (default) and `rank_i` is the 1-based rank of the chunk in retrieval method `i`. Each chunk appearing in both lists accumulates scores from both methods, which naturally boosts hybrid matches. Scores are normalized to `[0.0, 1.0]` by dividing by the maximum score.

Example with `k=60`:

- Rank 1 in FTS only: score = 1/61 ≈ 0.0164
- Rank 1 in vector only: score = 1/61 ≈ 0.0164
- Rank 1 in both FTS and vector: score = 1/61 + 1/61 ≈ 0.0328

After normalization, a chunk appearing first in both methods receives a score of 1.0.

`SearchConfig` controls behavior:

```rust
SearchConfig {
    limit: 10,              // final result count
    rrf_k: 60,             // RRF constant
    use_fts: true,          // enable full-text search
    use_vector: true,       // enable vector search (no-op if no embedding provided)
    min_score: 0.0,         // filter results below this normalized score
    pre_fusion_limit: 50,   // fetch up to 50 results from each method before fusion
}
```

If no embedding provider is configured, `Workspace::search()` passes `embedding = None` and the system falls back to FTS-only search. Vector search is silently skipped when the embedding is absent.

`SearchResult` carries metadata about which methods contributed:

```rust
pub struct SearchResult {
    pub document_id: Uuid,
    pub chunk_id: Uuid,
    pub content: String,
    pub score: f32,                 // normalized RRF score
    pub fts_rank: Option<u32>,      // rank in FTS results
    pub vector_rank: Option<u32>,   // rank in vector results
}
```

`result.is_hybrid()` is true when both `fts_rank` and `vector_rank` are `Some`.

### 5.5 Embeddings Config (`config/embeddings.rs`)

Configuration is resolved from environment variables with fallback to the `Settings` struct (loaded from the database `settings` table):

| Env Var | Default | Description |
|---------|---------|-------------|
| `EMBEDDING_ENABLED` | `false` | Enable vector embedding generation |
| `EMBEDDING_PROVIDER` | `openai` | Provider: `openai`, `nearai`, or `ollama` |
| `EMBEDDING_MODEL` | `text-embedding-3-small` | Model identifier |
| `EMBEDDING_DIMENSION` | Inferred from model | Vector dimensions |
| `OPENAI_API_KEY` | — | Required for OpenAI provider |
| `OLLAMA_BASE_URL` | `http://localhost:11434` | Ollama server URL |

Dimension inference from model name (fallback: 1536):

| Model | Dimensions |
|-------|-----------|
| `text-embedding-3-small` | 1536 |
| `text-embedding-3-large` | 3072 |
| `text-embedding-ada-002` | 1536 |
| `nomic-embed-text` | 768 |
| `mxbai-embed-large` | 1024 |
| `all-minilm` | 384 |

When `EMBEDDING_ENABLED=false` (the default), the workspace operates in FTS-only mode. Setting `OPENAI_API_KEY` does **not** implicitly enable embeddings; `EMBEDDING_ENABLED=true` must be set explicitly (this was a deliberate fix for issue #129).

---

## 6. Conversation History (`history/`)

Conversation turns and all job-related records are managed by `Store` (`src/history/store.rs`). The `Store` is a thin wrapper around a `deadpool_postgres::Pool`, exposing methods grouped by domain.

**Conversation turn storage**:

- `create_conversation(channel, user_id, thread_id)` — creates a new conversation row.
- `add_conversation_message(conversation_id, role, content)` — appends a message and bumps `last_activity`.
- `list_conversation_messages(conversation_id)` — retrieves all messages ordered by `created_at ASC`.
- `list_conversation_messages_paginated(conversation_id, before, limit)` — cursor-based pagination for the web UI; returns messages oldest-first with a `has_more` flag.

`ConversationMessage` carries `id`, `role`, `content`, `created_at`.

**History retrieval for context assembly**: The agent loop fetches recent messages from the database and reconstructs them into `ChatMessage` objects for the LLM call. Context compaction (`src/agent/compaction.rs`) summarizes old turns and replaces them with a summary message when the context window fills.

**Analytics** (`src/history/analytics.rs`): Aggregation queries on `agent_jobs` and `job_actions` for learning:

- `get_job_stats()` — total/completed/failed counts, success rate, average duration and cost.
- `get_tool_stats()` — per-tool call counts, success rate, average duration, total cost.
- `get_estimation_accuracy(category)` — average error rate between estimated and actual cost/time.
- `get_category_history(category, limit)` — historical estimation snapshots for EMA learning.

---

## 7. Context Builder (`context/`)

The `context/` module manages per-job execution state and in-memory conversation history. It is distinct from the workspace (long-term persistent memory) and the database history (durable log).

### In-Memory Job State

`JobContext` (`src/context/state.rs`) holds all runtime state for an executing job:

- `job_id`, `state` (enum `JobState`), `user_id`
- `title`, `description`, `category`
- Budget tracking: `budget`, `budget_token`, `bid_amount`, `estimated_cost`, `actual_cost`
- Token tracking: `total_tokens_used`, `max_tokens` (0 = unlimited)
- Timestamps: `created_at`, `started_at`, `completed_at`
- `transitions: Vec<StateTransition>` — capped at 200 entries
- `extra_env: Arc<HashMap<String, String>>` — injected credentials for child processes

State machine transitions are validated by `JobState::can_transition_to()`:

```
Pending -> InProgress -> Completed -> Submitted -> Accepted
                     └-> Failed
                     └-> Stuck -> InProgress (recovery)
                              └-> Failed
```

Terminal states: `Accepted`, `Failed`, `Cancelled`.

### In-Memory Conversation Memory

`Memory` (`src/context/memory.rs`) combines:

- `ConversationMemory` — a bounded ring buffer of `ChatMessage` objects (default max: 100). The system message (role `System`) is protected from eviction; older non-system messages are removed when the limit is reached.
- `Vec<ActionRecord>` — ordered log of every tool invocation in the job.

`ActionRecord` fields: `id`, `sequence`, `tool_name`, `input`, `output_raw`, `output_sanitized`, `sanitization_warnings`, `cost`, `duration`, `success`, `error`, `executed_at`.

### Context Assembly for LLM Calls

At each agent reasoning step, context is assembled in this order:

1. **System prompt**: identity files from workspace (`AGENTS.md`, `SOUL.md`, `USER.md`, `IDENTITY.md`) plus recent daily logs, assembled by `Workspace::system_prompt()`.
2. **Relevant memory chunks**: hybrid search results for the current user message, injected as system context.
3. **Conversation history**: recent turns from `ConversationMemory` or the database (`list_conversation_messages`).
4. **Tool definitions**: all registered tools from the `ToolRegistry`, serialized to JSON schema.
5. **Current job context**: job ID, status, budget remaining (if relevant).

### Context Manager

`ContextManager` (`src/context/manager.rs`) manages all active jobs via two `RwLock<HashMap<Uuid, _>>` maps:

- `contexts: RwLock<HashMap<Uuid, JobContext>>` — live job states.
- `memories: RwLock<HashMap<Uuid, Memory>>` — live job memories.

The write lock for both is held together during job creation to prevent TOCTOU races on the `max_jobs` check. Default capacity: 10 concurrent jobs.

---

## 8. Workspace Hygiene (`workspace/hygiene.rs`)

The hygiene system performs automatic cleanup of stale workspace documents. It is best-effort: all failures are logged as warnings and never propagate to the caller.

**What gets cleaned**: Documents in the `daily/` directory whose `updated_at` timestamp is older than `retention_days` (default: 30 days). Identity files (`AGENTS.md`, `SOUL.md`, `USER.md`, `IDENTITY.md`, `MEMORY.md`, etc.) are never touched.

**Cadence**: A state file at `~/.ironclaw/memory_hygiene_state.json` records the last run timestamp. The hygiene pass is skipped if fewer than `cadence_hours` (default: 12) have elapsed since the last run.

**Configuration** (`HygieneConfig`):

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `true` | Enable hygiene passes |
| `retention_days` | 30 | Days before daily logs are deleted |
| `cadence_hours` | 12 | Minimum hours between passes |
| `state_dir` | `~/.ironclaw/` | Directory for state file |

**`HygieneReport`** is returned from `run_if_due()`:

- `daily_logs_deleted: u32` — count of deleted documents.
- `skipped: bool` — true if the cadence has not elapsed or hygiene is disabled.

The `run_if_due(workspace, config)` function is called from the agent startup loop. The state directory and file are created automatically if missing.

---

## 9. Configuration Reference

| Env Var | Default | Description |
|---------|---------|-------------|
| `DATABASE_BACKEND` | `postgres` | `postgres` (or `pg`, `postgresql`) or `libsql` (or `turso`, `sqlite`) |
| `DATABASE_URL` | — | PostgreSQL connection string (required for postgres backend) |
| `DATABASE_POOL_SIZE` | `10` | PostgreSQL connection pool size |
| `LIBSQL_PATH` | `~/.ironclaw/ironclaw.db` | Path to local libSQL database file |
| `LIBSQL_URL` | — | Turso cloud endpoint URL (enables remote replica sync) |
| `LIBSQL_AUTH_TOKEN` | — | Required when `LIBSQL_URL` is set |
| `EMBEDDING_ENABLED` | `false` | Enable vector embedding generation |
| `EMBEDDING_PROVIDER` | `openai` | Embedding provider: `openai`, `nearai`, or `ollama` |
| `EMBEDDING_MODEL` | `text-embedding-3-small` | Embedding model identifier |
| `EMBEDDING_DIMENSION` | Inferred from model | Vector dimensions (1536 for small, 3072 for large) |
| `OPENAI_API_KEY` | — | OpenAI API key for embeddings (does not enable embeddings on its own) |
| `OLLAMA_BASE_URL` | `http://localhost:11434` | Ollama server URL |

### Backend Selection Summary

| Scenario | Recommended Backend | Feature Flag |
|----------|--------------------|-------------|
| Production, multi-user, shared DB | PostgreSQL | `postgres` (default) |
| Personal/local, zero-server | libSQL local | `--no-default-features --features libsql` |
| Personal + cloud sync | libSQL + Turso | `--no-default-features --features libsql` + `LIBSQL_URL` |
| Both available at runtime | Both | `--features postgres,libsql` |

### Embedding Dimension Quick Reference

| Model | Provider | Dimensions |
|-------|----------|-----------|
| `text-embedding-3-small` | OpenAI / NEAR AI | 1536 |
| `text-embedding-3-large` | OpenAI | 3072 |
| `text-embedding-ada-002` | OpenAI | 1536 |
| `nomic-embed-text` | Ollama | 768 |
| `mxbai-embed-large` | Ollama | 1024 |
| `all-minilm` | Ollama | 384 |
