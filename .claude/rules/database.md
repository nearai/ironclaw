---
paths:
  - "src/db/**"
  - "src/history/**"
  - "migrations/**"
---
# Database Rules

Dual-backend persistence: PostgreSQL + libSQL/Turso. **All new persistence features must support both backends.**

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

## Fix the Pattern, Not the Instance

When fixing a bug in one backend's SQL, always grep for the same pattern in the other. A fix to `postgres.rs` that doesn't also fix `libsql/jobs.rs` is half a fix. Same applies to satellite stores.
