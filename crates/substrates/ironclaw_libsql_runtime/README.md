# ironclaw_libsql_runtime

The shared libSQL connection-admission runtime. SQLite WAL permits concurrent
readers but admits only one writer, so this crate gives each physical database
one bounded reader pool (8 slots, opened `PRAGMA query_only = ON`) and exactly
one single-slot writer lane — every adapter writing the same file queues behind
the same admission point instead of forming a writer group of its own (the
defect #6863 fixed). It exists as its own crate because its three consumers sit
in three different families and must share one pool without any of them owning
it, and because it isolates the libSQL driver cone in a leaf with zero
workspace dependencies in either direction.

- **Family / layer:** `substrates` / `substrates` · **Package:**
  `ironclaw_libsql_runtime` · **Manifest:**
  `crates/substrates/ironclaw_libsql_runtime/Cargo.toml`
- **Use this when:** you need a libSQL connection. This is the only legal
  source of one — a second pool over the same database silently breaks the
  single-writer invariant.
- **Don't use this when:** you want to *store* something → use
  `ironclaw_filesystem` (the fabric sits above this runtime); you're writing
  SQL/schema/migrations → those belong to the backend crate that owns the
  records; you're on PostgreSQL → its concurrency model does not need this and
  must not inherit it.

## Public surface

- `LibSqlRuntime` — `open(path_or_url, auth_token)` (constructs the database
  and records target provenance), `new(Arc<libsql::Database>)` (caller-supplied
  handle, deliberately **without** provenance), `read()`, `write()`,
  `target_matches(path_or_url)` (proves what the runtime was opened for;
  always false for caller-supplied handles).
- `LibSqlReadConnectionLease` — exposes only `query`; the connection stays
  private and the pool's `query_only` pragma rejects write SQL regardless.
- `LibSqlWriteConnectionLease` — derefs to `libsql::Connection` for its
  lifetime (the lane's full capability) but never yields ownership; `discard`
  drops the connection outright so a cancelled transaction releases SQLite's
  writer lock immediately.
- `LibSqlLane` (`Read`/`Write`), `LibSqlCheckoutFailureReason`
  (`Timeout`/`Closed`/`RuntimeUnavailable`/`PostCreateHook`),
  `LibSqlRuntimeError` — a redacted, typed failure vocabulary so adapters
  classify retryable writer pressure vs broken infrastructure without parsing
  error text. `LIBSQL_READ_POOL_MAX_CONNECTIONS` (8).

No ports and no traits: a caller either holds the runtime or does not write.

## Depends on / consumed by

- **Depends on:** nothing in the workspace — the family's only crate with zero
  internal dependencies. External: `libsql`, `deadpool`, `thiserror`, `tokio`,
  `tracing`. A workspace dependency here would drag the driver cone into every
  dependent's dependents, which is the leakage this crate exists to bound.
- **Consumed by (measured 2026-08-05):** exactly `ironclaw_filesystem` (its
  libSQL backend), `ironclaw_triggers` (ADR 0003 hand-written SQL store), and
  `ironclaw_composition` (opens each physical database once and wires the
  shared runtime).

## Invariants

- **One writer per database.** Single-slot writer pool, plus rejection of
  reentrant writer acquisition from the same tokio task
  (`LibSqlRuntimeError::ReentrantWriter`).
- **Leases cannot be escalated** — the read lease exposes `query` only.
- **Bounded checkout:** 10s deadline, connect retry with backoff, connection
  recycling; checkout fails at the deadline rather than queueing without bound.
- **Target provenance:** `target_matches` refuses to vouch for a runtime it
  did not open itself.
- **Sole pool home** (PROPOSAL §11.2.6 "admission is singular"): the `deadpool`
  entry in `ADDITIONAL_DRIVER_ALLOWLISTS`
  (`crates/app/ironclaw_architecture_tests/tests/reborn_persistence_driver_boundary.rs`)
  is exactly `{ironclaw_filesystem, ironclaw_libsql_runtime}`, and the `libsql`
  driver allowlist is a shrink-only equality — no new crate can link either
  without failing `only_chartered_crates_link_the_other_persistence_drivers`.
- **Stays below every adapter:** the `BoundaryRule` for
  `ironclaw_libsql_runtime` in `reborn_dependency_boundaries.rs`
  (`reborn_crate_dependency_boundaries_hold`) forbids seventeen upward edges by
  name.

## Tests

```bash
cargo test -p ironclaw_libsql_runtime
cargo test -p ironclaw_architecture_tests   # driver allowlists + boundary rules
```

## See also

Family boundary: [`crates/substrates/AGENTS.md`](../AGENTS.md) · design record:
`docs/reborn/target-architecture/PROPOSAL.md` §6.2.6 and §11.2.6 · relationship
to the fabric: `ironclaw_filesystem` answers "where do these bytes go"; this
crate answers "which connection, on which lane, may run this statement now".
