# Database Module

Dual-backend persistence layer. **All new persistence features must support both backends.**

## Quick Reference

```bash
# Default build (PostgreSQL)
cargo build

# libSQL/Turso build
cargo build --no-default-features --features libsql

# Both backends
cargo build --features "postgres,libsql"

# Test each backend in isolation
cargo check                                           # postgres (default)
cargo check --no-default-features --features libsql   # libsql only
cargo check --all-features                            # both
```

## Files

| File | Role |
|------|------|
| `mod.rs` | `Database` supertrait + 7 sub-traits (~78 async methods total) — add new ops here first |
| `postgres.rs` | PostgreSQL backend — delegates to `Store` + `Repository` in `history/` |
| `libsql/mod.rs` | libSQL/Turso backend struct, connection helpers, row parsing utilities |
| `libsql/conversations.rs` | `ConversationStore` impl |
| `libsql/jobs.rs` | `JobStore` impl |
| `libsql/sandbox.rs` | `SandboxStore` impl |
| `libsql/routines.rs` | `RoutineStore` impl |
| `libsql/settings.rs` | `SettingsStore` impl |
| `libsql/tool_failures.rs` | `ToolFailureStore` impl |
| `libsql/workspace.rs` | `WorkspaceStore` impl (FTS5 + vector search) |
| `libsql_migrations.rs` | Consolidated libSQL schema (CREATE IF NOT EXISTS, no ALTER TABLE) |
| `tls.rs` | TLS connector factory for PostgreSQL (`rustls` + system root certs) |

PostgreSQL schema: `migrations/V1__initial.sql` through `V9__flexible_embedding_dimension.sql` (managed by `refinery`). V1 is the base schema; later migrations add tables, columns, and rename `claude_code_events` → `job_events`.

## Trait Structure

The `Database` supertrait is composed of seven sub-traits. Leaf consumers can depend on the narrowest sub-trait they need rather than the full `Database`:

| Sub-trait | Methods | Covers |
|-----------|---------|--------|
| `ConversationStore` | 12 | Conversations, messages |
| `JobStore` | 13 | Agent jobs, actions, LLM calls, estimation |
| `SandboxStore` | 13 | Sandbox jobs, job events |
| `RoutineStore` | 15 | Routines, routine runs |
| `ToolFailureStore` | 4 | Self-repair tracking |
| `SettingsStore` | 8 | Per-user key-value settings |
| `WorkspaceStore` | 13 | Memory documents, chunks, hybrid search |

`Database` adds `run_migrations()` and combines all sub-traits.

## Adding a New Persistence Operation

1. Decide which sub-trait the method belongs to, or create a new sub-trait
2. Add the async method signature to that sub-trait in `mod.rs`
3. Implement in `postgres.rs` (delegate to `Store` or `Repository`)
4. Implement in `libsql/<module>.rs` (SQLite-dialect SQL, use `self.connect().await?` per operation)
5. Add migration if needed:
   - PostgreSQL: new `migrations/VN__description.sql`
   - libSQL: add entry to `INCREMENTAL_MIGRATIONS` in `libsql_migrations.rs`
   - **Version numbering**: always base your migration number on what is already in `staging`/`main`, not your local branch. Migrations that have reached staging may already be deployed to production. Your new migration must come *after* the highest version on staging. Check with `git ls-tree origin/staging migrations/` (PG) and grep `INCREMENTAL_MIGRATIONS` on staging (libSQL). Never reuse or insert before an existing version number.

## SQL Dialect Differences

| Feature | PostgreSQL | libSQL |
|---------|-----------|--------|
| UUIDs | `UUID` type | `TEXT` |
| Timestamps | `TIMESTAMPTZ` | `TEXT` (ISO-8601 RFC 3339 with ms precision) |
| JSON | `JSONB` | `TEXT` |
| Numeric/Decimal | `NUMERIC` | `TEXT` (preserves `rust_decimal` precision) |
| Arrays | `TEXT[]` | `TEXT` (JSON-encoded array) |
| Booleans | `BOOLEAN` | `INTEGER` (0/1) |
| Vector embeddings | `VECTOR` (any dim, V9 removed fixed 1536) | `F32_BLOB(N)` via `libsql_vector_idx` (dimension set dynamically by `ensure_vector_index`) |
| Full-text search | `tsvector` + `ts_rank_cd` | FTS5 virtual table + sync triggers |
| JSON path update | `jsonb_set(col, '{key}', val)` | `json_patch(col, '{"key": val}')` |
| PL/pgSQL | Functions | Triggers (no stored procs in SQLite) |
| Connection model | `deadpool-postgres` connection pool | New connection per operation (`self.connect()`) |
| Concurrency | Pool-based, fully concurrent | WAL mode + 5 s busy timeout; write serialized |
| Auto-timestamp | `DEFAULT NOW()` | `DEFAULT (datetime('now'))` |
| Timestamp parsing | Native type | Multi-format fallback in `parse_timestamp()` |

**JSON merge patch gotcha:** libSQL uses RFC 7396 JSON Merge Patch (`json_patch`) for metadata updates. This replaces top-level keys entirely — it **cannot** do partial nested updates. PostgreSQL uses `jsonb_set` which is path-targeted. Don't rely on partial nested metadata updates if you need libSQL compat.

**Boolean storage:** libSQL stores booleans as integers. When reading, use `get_i64(row, idx) != 0`; when writing, pass `1i64`/`0i64`. Never pass a Rust `bool` directly.

**Timestamp write format:** Always write timestamps with `fmt_ts(dt)` (RFC 3339, millisecond precision). Read with `get_ts()` / `get_opt_ts()` which handle legacy naive formats too.

**Vector dimension:** PostgreSQL V9 migration changed the column to unbounded `vector` (removing the HNSW index). libSQL dynamically creates `F32_BLOB(N)` with the correct dimension via `ensure_vector_index()` during `run_migrations()`, reading `EMBEDDING_DIMENSION` / `EMBEDDING_MODEL` from env vars.

**Connection per operation:** `LibSqlBackend::connect()` creates a fresh connection for every operation, sets `PRAGMA busy_timeout = 5000`, and closes it when the `Connection` is dropped. This is intentional — the libSQL SDK does not offer a pool. Avoid holding connections open across `await` points.

## Schema: Key Tables

**Core:**
- `conversations` — multi-channel conversation tracking
- `conversation_messages` — individual messages within a conversation
- `agent_jobs` — job metadata and status
- `job_actions` — event-sourced tool executions
- `job_events` — sandbox job streaming events (renamed from `claude_code_events` in V7)
- `dynamic_tools` — agent-built tools
- `llm_calls` — cost/token tracking
- `estimation_snapshots` — learning data
- `repair_attempts` — self-repair action log (not exposed via Database trait yet)

**Workspace/Memory:**
- `memory_documents` — flexible path-based files
- `memory_chunks` — chunked content with FTS + vector indexes
- `memory_chunks_fts` — FTS5 virtual table (libSQL) / `tsvector` column (PostgreSQL)
- `heartbeat_state` — periodic execution tracking

**Security/Extensions:**
- `secrets` — AES-256-GCM encrypted credentials
- `wasm_tools` — installed WASM tool binaries
- `tool_capabilities` — per-tool HTTP allowlist, secret access, rate limits
- `leak_detection_patterns` — secret regex patterns (seed data in both backends)
- `leak_detection_events` — audit log of detected leaks
- `secret_usage_log` — per-request credential injection audit trail
- `tool_rate_limit_state` — sliding window rate limit counters

**Other:**
- `routines`, `routine_runs` — scheduled/reactive execution
- `settings` — per-user key-value
- `tool_failures` — broken tool tracking for self-repair
- `_migrations` — libSQL-only internal migration version tracking

## libSQL Current Limitations

- **Secrets store** — still requires `PostgresSecretsStore`; `LibSqlSecretsStore` exists but is not plumbed through the main startup path
- **Settings reload** — `Config::from_db` skipped (requires `Store`)
- **No incremental migrations** — schema is idempotent CREATE IF NOT EXISTS; no ALTER TABLE support; column additions require a new versioned approach
- **No encryption at rest** — only secrets (API tokens) are AES-256-GCM encrypted; all other data is plaintext SQLite
- **Hybrid search** — both FTS5 and vector search (`libsql_vector_idx`) are implemented; `ensure_vector_index()` dynamically creates the index with the correct `F32_BLOB(N)` dimension from env vars during `run_migrations()`
- **Write serialization** — WAL mode allows concurrent readers but only one writer at a time; busy timeout is 5 s, which may cause timeouts under high write concurrency

## Running Locally with libSQL

```bash
# Use local SQLite file (default)
DATABASE_BACKEND=libsql LIBSQL_PATH=~/.ironclaw/test.db cargo run

# Use Turso cloud (embedded replica syncs local file to cloud)
DATABASE_BACKEND=libsql LIBSQL_URL=libsql://xxx.turso.io LIBSQL_AUTH_TOKEN=xxx cargo run

# In-memory (tests only — data is lost when the process exits)
# Use LibSqlBackend::new_memory() directly in test code
```

## Testing the libSQL Backend

Use `LibSqlBackend::new_memory()` in unit tests — no files, no cleanup required:

```rust
#[tokio::test]
async fn test_my_feature() {
    let backend = LibSqlBackend::new_memory().await.unwrap();
    backend.run_migrations().await.unwrap();
    // backend implements Database — call any trait method
}
```

For concurrency tests that require multiple connections sharing state, use `LibSqlBackend::new_local(&tmp_path)` with a `tempfile::tempdir()`. In-memory databases do not share state between connections.

## Sharing the libSQL Database Handle

`LibSqlBackend::shared_db()` returns an `Arc<LibSqlDatabase>` for passing to satellite stores (e.g., `LibSqlSecretsStore`, `LibSqlWasmToolStore`) that need their own connections per-operation but should share the same underlying database file. These stores call `.connect()` on the shared handle themselves. This is the correct pattern — do not pass a live `Connection` to satellite stores.

## Pattern: Fix the Pattern, Not the Instance

When fixing a bug in one backend's SQL, always grep for the same pattern in the other backend. A fix to `postgres.rs` that doesn't also fix the libSQL module (e.g., `libsql/jobs.rs`) is half a fix. The same applies to satellite types like `LibSqlSecretsStore` or `LibSqlWasmToolStore`.
