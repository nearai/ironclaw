---
paths:
  - "src/db/**"
  - "src/history/**"
  - "migrations/**"
---
# Database Rules

## Status & Direction

The repo is migrating off per-crate `Store`/`Repository` traits onto a
single universal `RootFilesystem` mount table (`crates/ironclaw_filesystem/`).
Under the new model, every persistence concern is a mount path
(`/system/secrets`, `/system/processes`, `/engine/threads`, …) backed by
exactly one `RootFilesystem` implementation — typed stores become thin
wrappers around `ScopedFilesystem` and own no backend dispatch of their
own. See `crates/ironclaw_filesystem/CLAUDE.md` and the
`2026-05-14-universal-fs-dispatch.md` plan/ADR.

**New persistence features go on `ScopedFilesystem`, not into `src/db/`.**
The rules below cover the *legacy* per-crate dual-backend pattern that
still exists in `src/db/`, `src/history/`, and `migrations/`. Touch them
only when fixing or extending code that already lives there; do not add
new sub-traits or per-domain backends.

This file is `paths`-scoped to those legacy directories so the rule
loads when (and only when) you're inside them. New code under
`crates/ironclaw_filesystem/`, consumer crates routing through it, or
any new mount-backed store should follow the unified-surface contract
in `crates/ironclaw_filesystem/CLAUDE.md` instead.

---

## Legacy: Dual-Backend Per-Crate Pattern

Dual-backend persistence: PostgreSQL + libSQL/Turso. **All new persistence features must support both backends.** *(Applies only inside the legacy directories scoped above. For new crates, mount through `RootFilesystem` and let the wiring layer pick the backend.)*

See `src/db/CLAUDE.md` for full schema, dialect differences, and libSQL limitations.

## Adding a New Operation

1. Decide which sub-trait it belongs to (`ConversationStore`, `JobStore`, `SandboxStore`, `RoutineStore`, `ToolFailureStore`, `SettingsStore`, `WorkspaceStore`) or create a new one
2. Add the async method signature to that sub-trait in `src/db/mod.rs`
3. Implement in `src/db/postgres.rs` (delegate to `Store`/`Repository`)
4. Implement in `src/db/libsql/<module>.rs` (use `self.connect().await?` per operation)
5. Add migration if needed:
   - PostgreSQL: new `migrations/VN__description.sql`
   - libSQL: add entry to `INCREMENTAL_MIGRATIONS` in `libsql_migrations.rs`
   - **Version numbering**: always number after the highest version on `staging`/`main` — those migrations may already be in production. Check with `git ls-tree origin/staging migrations/` and staging's `INCREMENTAL_MIGRATIONS`. Never reuse or insert before an existing version.
6. Test feature isolation:
   ```bash
   cargo check                                          # postgres (default)
   cargo check --no-default-features --features libsql  # libsql only
   cargo check --all-features                           # both
   ```

## Independent per-record reads must be concurrent, never a serial loop

When a caller fetches N *independent* records by key (no shared transaction,
no read-after-write ordering between them) — e.g. loading a fixed set of
identity/config files, or hydrating several entities by id — issue the reads
CONCURRENTLY (`futures::future::try_join_all` / `join_all`), never in a serial
`for path in … { store.get(path).await? }` loop.

Each `get_document_by_path`-style call is one self-contained query on its own
pooled connection (postgres: `self.conn().await?` per call; libsql:
`self.connect().await?` per call) and returns `DocumentNotFound` on a miss —
so a serial loop pays one full round-trip *per record, including misses*.
Against the hosted cross-region Postgres backend (~100-200 ms/RTT) that turned
5-7 cold identity reads into multiple seconds of pre-provider latency on every
cold turn; concurrent dispatch collapses them to ~1 RTT. The pattern is
backend-agnostic and safe on all three backends (postgres pool=30 ≫ N, libsql
per-call connection, in-memory trivially independent — in-memory just saves
~0 wall-clock).

Preserve deterministic output order (`try_join_all` keeps input order). Guard
the invariant with a `ConcurrencyProbeDb`-style test that instruments the
`Database` boundary and asserts max-in-flight ≥ 2 — see
`tests/reborn_identity_parallel_read.rs` and
`src/workspace/reborn_identity_context.rs::load_identity_candidates`.

## SQL Dialect Translation Checklist

When writing SQL for both backends, translate these types:

| PostgreSQL | libSQL |
|-----------|--------|
| `UUID` | `TEXT` |
| `TIMESTAMPTZ` | `TEXT` (ISO-8601, write with `fmt_ts()`, read with `get_ts()`) |
| `JSONB` | `TEXT` (JSON string) |
| `BOOLEAN` | `INTEGER` (0/1 -- use `get_i64(row, idx) != 0` to read) |
| `NUMERIC` | `TEXT` (preserves `rust_decimal` precision) |
| `TEXT[]` | `TEXT` (JSON-encoded array) |
| `VECTOR` | `BLOB` (flexible dimensions; vector index dropped, brute-force search fallback) |
| `jsonb_set(col, '{key}', val)` | `json_patch(col, '{"key": val}')` -- replaces top-level keys entirely, cannot do partial nested updates |
| `DEFAULT NOW()` | `DEFAULT (datetime('now'))` |
| `tsvector` + `ts_rank_cd` | FTS5 virtual table + sync triggers |

## Schema Translation Beyond DDL

Don't just translate `CREATE TABLE`. Also check:
- **Indexes** -- diff `CREATE INDEX` statements between backends
- **Seed data** -- check for `INSERT INTO` in migrations (e.g., `leak_detection_patterns`)
- **Triggers** -- PostgreSQL functions vs SQLite triggers (no stored procs in SQLite)

## Transaction Safety

Multi-step operations (INSERT+INSERT, UPDATE+DELETE, read-modify-write) MUST be wrapped in a transaction. Ask: "If this crashes between step N and N+1, is the database consistent?" If not, wrap in a transaction. Applies to both backends.

## libSQL Connection Model

`LibSqlBackend::connect()` creates a fresh connection per operation with `PRAGMA busy_timeout = 5000`. This is intentional -- no pool exists. Never hold connections open across `await` points. Satellite stores (`LibSqlSecretsStore`, `LibSqlWasmToolStore`) receive `Arc<LibSqlDatabase>` via `shared_db()` and call `.connect()` themselves -- never pass a live `Connection`.

## Never Delete LLM Output Data

All LLM execution data — thread messages, steps, events, tool call parameters and results — must **never** be deleted from the database. This is the most valuable data in the system. No `DELETE` statements, no `DROP`, no truncation of LLM-generated content. In-memory caches (HashMaps in `HybridStore`) may evict entries for memory pressure, but database rows are permanent. Load methods must fall back to the database on a cache miss.

## Fix the Pattern, Not the Instance

When fixing a bug in one backend's SQL, always grep for the same pattern in the other. A fix to `postgres.rs` that doesn't also fix `libsql/jobs.rs` is half a fix. Same applies to satellite stores.
