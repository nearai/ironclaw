# libSQL Single-Writer Runtime and Recoverable Turn Journal

**Date:** 2026-07-29

## Problem

The production libSQL composition opens the same physical database through
multiple independent connection groups:

- `LibSqlRootFilesystem` owns an eight-connection pool.
- `LibSqlTriggerRepository` opens fresh connections for each operation.
- `ironclaw_event_store` reopens the configured path and creates another
  `LibSqlRootFilesystem` with another eight-connection pool.

SQLite WAL permits concurrent readers but still admits only one writer at a
time. These independent write-capable groups therefore compete for the same
database writer lock. A lock conflict is correctly mapped to
`FilesystemError::BackendBusy`, but the turn-state delta journal erases that
classification into `TurnError::Unavailable`, permanently latches itself
degraded, drains its accepted queue, and does not recover until the process is
restarted.

The observed incident is therefore writer-lock contention amplified into a
permanent application outage. We have not found evidence of a cyclic deadlock.

## Goals

1. Give each production libSQL database exactly one process-local write
   admission lane while retaining concurrent reads.
2. Make all production libSQL consumers share that lane.
3. Acquire SQLite write ownership before transactional read-modify-write work.
4. Preserve retryable contention classification across generic filesystem
   consumers.
5. Retain and retry the exact atomic turn-journal batch so a transient busy
   result cannot create a durable gap or poison the store until restart.
6. Leave PostgreSQL pooling and concurrency unchanged.
7. Add deterministic concurrency, recovery, atomicity, and composition tests.

## Non-goals

- Serializing PostgreSQL writes.
- Introducing a generic database connection abstraction.
- Coordinating writers across multiple IronClaw processes that independently
  open the same local SQLite file. A local libSQL deployment remains a
  single-process storage topology.
- Retrying operations that may have partially committed.
- Changing the public `RootFilesystem` API or backend-independent domain-store
  APIs.
- Solving every source of libSQL write amplification in this change. Repeated
  process-store index declaration is a follow-up optimization once the shared
  writer invariant is in place.

## Decision

### 1. Add a libSQL-only runtime substrate

Create a small `ironclaw_libsql_runtime` substrate crate. It owns:

- the `Arc<libsql::Database>`;
- an eight-connection read pool;
- a one-connection writer pool;
- connection initialization and bounded checkout policy.

Its public API exposes distinct read and writer lease types. Read leases expose
only the query API and their pooled connections enforce
`PRAGMA query_only = ON`; writer leases expose the full libSQL connection API
and are the sole write-admission tokens for that runtime. Holding a writer
lease for the full database operation prevents any other in-process filesystem
or trigger write from starting. Readers use the separate read pool and remain
concurrent with each other and with the current writer when WAL permits it.

The runtime contains only libSQL connection mechanics. It does not know about
filesystem paths, triggers, events, turns, or product policy.

`LibSqlRootFilesystem` and `LibSqlTriggerRepository` gain constructors that
accept `Arc<LibSqlRuntime>`. Their existing database constructors remain for
standalone and test compatibility, creating a private runtime internally.
Production composition must construct one runtime and pass the same `Arc` to
both adapters.

This new low-level crate avoids:

- making trigger-domain code depend on `ironclaw_filesystem`, which is
  explicitly forbidden by the architecture suite;
- passing an untyped `Arc<Mutex<()>>` through composition;
- moving libSQL-specific mechanics into a generic contract crate.

### 2. Reuse the production filesystem for event logs

Add a prebuilt-libSQL-filesystem event-store configuration, symmetric with the
existing prebuilt PostgreSQL pool path. Production composition passes the
already-migrated `Arc<LibSqlRootFilesystem>` to the event store instead of
reopening `path_or_url`.

The standalone `RebornEventStoreConfig::Libsql { ... }` path remains available
to the event-store crate's standalone callers. Production libSQL composition
does not accept a second event target: its input binds the primary runtime to
the target used for production policy validation, and it always constructs the
event store from the primary filesystem.

This removes the second production filesystem pool and guarantees event-log
writes use the same writer lane as all other `RootFilesystem` consumers.

### 3. Classify every libSQL operation as read or write

Every libSQL adapter method must deliberately request one of the runtime's two
lease types:

- Pure `SELECT`, listing, tailing, and metadata inspection use a read lease.
- DDL, migrations, inserts, updates, deletes, append operations, CAS, and
  transactional read-modify-write use a writer lease.

The writer lease is held until commit or rollback completes. No call may drop
the lease between its precondition read and its write.

Multi-statement mutations use one transaction and acquire the SQLite writer
lock at the start with `BEGIN IMMEDIATE`. In particular:

- filesystem batch append uses an explicit immediate transaction rather than a
  deferred transaction;
- filesystem multi-table delete becomes atomic;
- trigger migrations and multi-step trigger mutations stay on one writer
  lease and one transaction where applicable.

`busy_timeout` remains a secondary guard for external/process-level contention,
not the primary in-process admission mechanism.

### 4. Preserve generic retry semantics

`FilesystemError::BackendBusy` remains the backend-neutral signal that:

- the operation encountered transient database contention; and
- retrying the whole operation is safe because no partial side effect was
  committed.

SQLite `BUSY`/`LOCKED` extended codes and PostgreSQL serialization,
deadlock-victim, and lock-not-available SQLSTATEs continue to map to this
variant. File and other backends keep their own mappings. No generic layer
learns SQLite error codes.

Generic consumers may retry `BackendBusy` only at an operation boundary whose
contract is atomic and replay-safe. Other filesystem errors remain fatal for
the current operation.

### 5. Replace the turn journal's boolean latch with a health state

The turn-state delta journal uses three states:

- `Healthy`: accepts mutations normally.
- `RecoveringContention`: retains the current exact batch, rejects new mutation
  admission with a retryable unavailable result, and retries the retained batch
  in order.
- `FailedFatal`: a non-retryable append failure occurred; the existing
  fail-closed behavior applies.

The flusher must preserve `FilesystemError` until it classifies the append
result. On `BackendBusy` it must:

1. keep ownership of the exact `Vec<DeltaJournalRequest>`;
2. leave every acknowledgement unresolved;
3. mark the journal `RecoveringContention`;
4. retry that same atomic `append_batch` with jittered exponential backoff;
5. cap the backoff interval while continuing slow recovery probes;
6. acknowledge the batch only after one successful commit;
7. return to `Healthy` and process already-queued requests in original order.

The bounded pending-ack window remains the backpressure mechanism for
non-critical writes. Blocking new mutation admission while recovering prevents
unbounded new accepted work. Requests that won the existing admission race
before the health transition remain queued behind the retained batch and are
not dropped.

Reads continue to use the hot snapshot during contention recovery because it
contains accepted, ordered writes whose durable acknowledgements are still
pending. A fatal failure clears the divergent cache and falls back to the last
consistent durable point as today.

Critical callers may time out while a retained batch is still recovering. That
is an ambiguous-outcome boundary: the batch may commit later. Existing typed
idempotency keys and durable read-back must make a caller retry safe. The
journal must not clear the hot cache merely because the caller stopped waiting
for an acknowledgement that the flusher still owns.

### 6. Observability

Add structured metrics/log fields for:

- libSQL writer checkout wait duration;
- current writer queue depth where the pool exposes it;
- turn-journal transition into and out of contention recovery;
- contention retry attempt and backoff;
- time spent recovering;
- fatal journal failure, separately from retryable contention.

Do not include database URLs, local paths, credentials, payloads, or tenant
content.

## Tests

Implementation follows test-driven development. Each behavior begins with a
failing test that is observed before production code changes.

### `ironclaw_libsql_runtime`

- Holding a writer lease blocks a second writer lease.
- A read lease remains available while the writer lease is held.
- Dropping a writer lease admits exactly one waiting writer.
- A connection returned inside an open transaction is rejected/recreated.

### `ironclaw_filesystem`

- Concurrent writes through two filesystem handles sharing one runtime never
  overlap at the database writer boundary.
- Real external writer contention still surfaces as `BackendBusy`.
- `append_batch` is all-or-nothing and uses the immediate writer path.
- Multi-table delete cannot leave partial state when a later statement fails.
- Existing CAS-storm, pool-exhaustion, and migration tests remain green.

### `ironclaw_triggers` and composition

- Filesystem and trigger writes constructed from one runtime serialize against
  each other while reads remain available.
- Production libSQL composition passes one runtime to filesystem, triggers,
  and event logs and never accepts or reopens a second event target.
- Standalone constructors remain compatible.

### `ironclaw_turns`

- A fault-injected `BackendBusy` result leaves the exact batch retained.
- A later successful retry acknowledges the original requests once, restores
  healthy admission, and permits a subsequent mutation without reopening.
- Requests queued before recovery preserve order behind the retained batch.
- A fatal append error still fails closed and never appends a later delta behind
  a durable gap.
- Cancellation or caller timeout does not drop the retained batch.
- Recovery backoff is deterministic under paused Tokio time.

### Architecture and parity

- `cargo test -p ironclaw_architecture_tests`
- Focused libSQL runtime/filesystem/trigger/event-store/turn tests.
- PostgreSQL-focused contract tests that cover unchanged concurrent pool
  construction and generic `BackendBusy` mapping.
- Clippy on every changed production package with `-D warnings`.

No browser E2E behavior changes; the affected contract is below the HTTP and
channel adapters.

## Compatibility

- PostgreSQL continues to use its concurrent pool. It does not depend on or
  construct `LibSqlRuntime`.
- Backend-neutral code sees the same `RootFilesystem` contract and the same
  `FilesystemError::BackendBusy` variant.
- Existing standalone libSQL constructors keep source compatibility.
- No persistence schema or durable record format changes.
- No migration or data rewrite is required.

## Rollout

1. Deploy to QA with contention-recovery logs and writer-wait metrics enabled.
2. Exercise Telegram and Slack pairing plus concurrent chat/trigger/event-log
   writes.
3. Confirm there is one runtime-created writer pool and no production event
   store database reopen.
4. Confirm transient contention enters and exits recovery without a process
   restart or permanent HTTP 503s.
5. Promote to production after the QA soak.

## Rollback

The change is code-only and has no schema migration. Roll back to the preceding
application image if writer latency or recovery behavior regresses. Existing
database files remain compatible in both directions.

## Risks and mitigations

- **Writer head-of-line blocking:** expected for SQLite's one-writer model.
  Keep transactions short and measure writer wait and hold time.
- **Missed write call site:** central read/write lease helpers plus an exhaustive
  adapter audit and contention tests reduce this risk.
- **Ambiguous critical timeout:** retain the batch and rely on typed
  idempotency/read-back rather than risking a durable gap.
- **Multi-process access to one local file:** outside the process-local runtime
  guarantee; `busy_timeout` and `BackendBusy` recovery remain defensive, and
  deployment must keep one IronClaw process per local database.
- **Remote libSQL:** serialization is conservative but correct. A future
  backend-specific policy may increase remote write concurrency after measured
  evidence without changing generic store semantics.
