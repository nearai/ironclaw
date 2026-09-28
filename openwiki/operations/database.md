---
type: operational-reference
title: Persistence & Storage Backends
description: How IronClaw Reborn stores durable data through event sourcing, supports multiple storage backends (libSQL and PostgreSQL), and manages schema migrations safely.
tags: [persistence, storage, events, projections, database, migrations, data-durability]
verified:
  - by: openwiki/0.6.0
    at: 2026-09-28T08:16:43.565Z
sources:
  - id: openwiki-source-790146f7f4bc594715f8b5dd
    resource: repo://crates/events/ironclaw_event_log/README.md
  - id: openwiki-source-0442b291f39341f9d8645860
    resource: repo://crates/events/ironclaw_event_log/src/lib.rs
  - id: openwiki-source-76d4217cb9fe5244a9e71f42
    resource: repo://crates/events/ironclaw_event_log/src/runtime_event.rs
  - id: openwiki-source-de89fb85d3423e6af9502ff6
    resource: repo://crates/events/ironclaw_event_projections/README.md
  - id: openwiki-source-d408668254972e1be9ab35cd
    resource: repo://crates/events/ironclaw_event_projections/src/lib.rs
  - id: openwiki-source-f3c4c9e856cb9d71cfd0c677
    resource: repo://crates/events/ironclaw_event_store/README.md
  - id: openwiki-source-5ba4260196a2a94f59ddc751
    resource: repo://crates/events/ironclaw_event_store/src/durable_log.rs
  - id: openwiki-source-0475dce3750048ef679e2a8d
    resource: repo://crates/events/ironclaw_event_store/src/lib.rs
  - id: openwiki-source-165de698a2fed298bb3de487
    resource: repo://crates/substrates/ironclaw_filesystem/README.md
  - id: openwiki-source-7225d7c6674a28ba203674ea
    resource: repo://crates/substrates/ironclaw_libsql_runtime/README.md
  - id: openwiki-source-f385afd8edb5ea40b1ba6827
    resource: repo://docs/internal/reborn/contracts/events-projections.md
  - id: openwiki-source-880d9fa819e17df7686bb9d1
    resource: repo://docs/internal/reborn/contracts/events.md
  - id: openwiki-source-047f8e9aff565d82cd7cc833
    resource: repo://docs/internal/reborn/contracts/migration-compatibility.md
  - id: openwiki-source-3384390c929863f5e882e6b2
    resource: repo://docs/internal/reborn/contracts/turn-persistence.md
generated: { by: "openwiki/0.6.0", at: "2026-09-28T08:16:43.565Z" }
---

# Persistence & Storage Backends

IronClaw Reborn is built on **event sourcing**: the system stores immutable events as the primary source of truth, derives read models called projections from those events, and supports recovery and replay from the event log. This document explains the storage architecture, how data flows through the system, which backends are supported, and how to deploy schema changes safely.

## Architecture Overview

```mermaid
erDiagram
    EVENT_LOG ||--o{ PROJECTION : "computes"
    EVENT_LOG ||--o{ RUNTIME_EVENT : "contains"
    EVENT_LOG ||--o{ AUDIT_ENVELOPE : "records"
    PROJECTION ||--o{ RUN_STATUS : "produces"
    PROJECTION ||--o{ THREAD_TIMELINE : "produces"
    PROJECTION ||--o{ CAPABILITY_ACTIVITY : "produces"
    RUNTIME_EVENT }o--|| RESOURCE_SCOPE : "scoped_by"
    AUDIT_ENVELOPE }o--|| APPROVAL_REQUEST : "tracks"
    ROOT_FILESYSTEM ||--o{ EVENT_LOG : "persists"
    ROOT_FILESYSTEM ||--o{ LIBSQL_BACKEND : "uses"
    ROOT_FILESYSTEM ||--o{ POSTGRES_BACKEND : "uses"
```

**Event sourcing fundamentals:** Every meaningful state transition in the system (turn accepted, run started, capability invoked, approval resolved) emits an immutable event. The event is appended to a durable append-only log and carries scope metadata, timestamps, and redacted details about what happened. Projections replay the event log to compute read models on demand; if a projection is lost, it can be rebuilt from the events.

## Event Log Structure

### Event Classes

IronClaw records three types of events:

1. **Runtime events** (`RuntimeEvent` in `ironclaw_event_log`)
   - Dispatch lifecycle: `DispatchRequested`, `RuntimeSelected`, `DispatchSucceeded`, `DispatchFailed`
   - Capability activity: `CapabilityActivityRequested`, `CapabilityActivitySucceeded`, `CapabilityActivityFailed`
   - Model execution: `ModelStarted`, `ModelCompleted`, `ModelFailed`
   - Loop/process lifecycle: `LoopCompleted`, `LoopCancelled`, `LoopFailed`, `ProcessStarted`, `ProcessCompleted`, `ProcessFailed`, `ProcessKilled`
   - Hook execution: `HookDispatched`, `HookDecisionEmitted`, `HookFailed`
   - Recovery: `FailureRecovered` (with closed-vocabulary recovery stage, class, and disposition)

2. **Audit events** (`AuditEnvelope` in `ironclaw_event_log`)
   - Approval lifecycle: `ApprovalResolved` (with decision kind, actor, and timestamp)
   - Security decisions and authorization outcomes

3. **Security audit events** (control-plane audit for compliance and debugging)

### Event Redaction

All events are **redacted at construction time** to prevent leaking sensitive data. The system never stores:
- Raw prompts or assistant responses
- Tool input/output payloads  
- Secrets or authentication tokens
- Host paths or file system details
- Approval reason strings or lease contents
- Backend error details (collapsed to safe classification tokens like `Unclassified`)

Error kinds are constrained to short classification strings; unsafe detail is discarded. Error summaries are bounded and host-authored; unsafe summaries are collapsed to a fixed marker.

### Event Identity & Ordering

Each event carries:
- **Event ID** — unique identifier for the event record
- **Timestamp** — when the event was emitted
- **Correlation ID** — links causally related events
- **Resource scope** — tenant, user, agent, project, thread, optional process
- **Stream ordering** — per-`(tenant, user, agent)` stream with monotonic cursor

Ordering guarantees are explicit per stream:
- **Per-thread ordering** for thread and run events
- **Per-run ordering** for run progress
- **Global ordering only if explicitly provided** by the durable backend

## Storage Architecture

### Storage Abstraction Layers

<!-- openwiki: mermaid parse failed and this diagram was converted to a text fence so it does not break rendering. Fix the diagram source and restore the mermaid fence. Parser error: Heuristic: an unescaped angle bracket inside a label breaks rendering; rephrase the label. -->
```text
flowchart TD
    Producer["Runtime Producers<br/>Dispatcher, Process Manager,<br/>Approval Resolver"]
    Sink["DurableEventLog<br/>& DurableAuditLog Traits<br/>ironclaw_event_log"]
    EventStore["ironclaw_event_store<br/>Backend Selection & Fail-Closed"]
    Filesystem["ironclaw_filesystem<br/>RootFilesystem Trait<br/>Unified Backend Dispatch"]
    Backend["Storage Backends<br/>libSQL / PostgreSQL / JSONL"]
    
    Producer -->|produces events| Sink
    Sink -->|routes through| EventStore
    EventStore -->|configures| Filesystem
    Filesystem -->|dispatches to| Backend
```

**Key separation:** The system splits storage concerns across four layers:

1. **`ironclaw_event_log`** — vocabulary (event shapes, sinks, durable traits)
   - Owned by every producer; no storage driver or backend selection
   - Defines `EventSink`, `DurableEventLog`, and redaction constructors
   - Never links a database driver

2. **`ironclaw_event_store`** — backend selection and fail-closed policy
   - Only crate permitted to link database drivers (`libSQL`, PostgreSQL, Tokio)
   - Selects backend by profile (`Test`, `Standalone`, `Production`)
   - Validates fail-closed TLS/durability requirements before opening connections

3. **`ironclaw_filesystem`** — unified storage dispatch (RootFilesystem trait)
   - One virtual path space with scoped access and mount grants
   - Three production backends: libSQL (default), PostgreSQL, JSONL
   - Compare-and-swap floor for transactionality; all backends implement the same contract

4. **Concrete backends** — driver implementation
   - libSQL: SQLite with WAL, single-node durable, built-in with Reborn
   - PostgreSQL: multi-node, high-availability, TLS required for remote
   - JSONL: single-node append-only text logs (test/dev fallback only)

### Supported Backends

#### libSQL (Default)

- **When to use:** Single-node deployments, embedded/self-hosted, development
- **Durability:** SQLite WAL (write-ahead log); concurrent readers, single writer
- **Concurrency model:** `ironclaw_libsql_runtime` provides one reader pool (8 slots, query-only) and one writer lane per database
- **Configuration:**
  ```rust
  RebornEventStoreConfig::Libsql {
      path_or_url: "sqlite:///var/lib/ironclaw/events.db",
      auth_token: None,  // for remote Turso
  }
  ```
- **Production policy:** Must specify explicit local path or remote `https://` URL; cleartext `http://` rejected; must accept single-node durability explicitly

#### PostgreSQL

- **When to use:** High-availability deployments, multi-node clusters, hosted services
- **Durability:** ACID transactions, multi-node replication via WAL streaming
- **Concurrency model:** Standard PostgreSQL connection pooling; `deadpool-postgres` with configurable pool size
- **Configuration:**
  ```rust
  RebornEventStoreConfig::Postgres {
      url: SecretString::new("postgresql://user:pass@host:5432/ironclaw".to_string()),
      tls_options: PostgresPoolTlsOptions {
          ssl_mode_override: Some(RebornPostgresSslMode::Require),
          allow_remote_cleartext: false,
      },
  }
  // Or reuse existing pool:
  RebornEventStoreConfig::PostgresPool { pool }
  ```
- **Production policy:** Remote connections must use `sslmode=require`; cleartext `http://` rejected unless explicitly allowed (fail-closed); TLS options validated before opening pool
- **Pool management:** Default pool size is 8 connections; override with `pool_max_size` in config
- **Driver cone:** All Postgres driver imports isolated to `ironclaw_event_store`; no other crate links the driver

#### JSONL (Test/Development Only)

- **When to use:** Tests, local development, reference implementations
- **Durability:** Append-only text files; no transactionality
- **Configuration:**
  ```rust
  RebornEventStoreConfig::Jsonl {
      root: PathBuf::from("/var/lib/ironclaw/events"),
      accept_single_node_durable: true,
  }
  ```
- **Production policy:** Rejected in production unless `accept_single_node_durable` is explicitly set; no implicit fallback

#### In-Memory

- **When to use:** Unit tests, reference loops, demos
- **Durability:** Lost on process exit
- **Configuration:**
  ```rust
  RebornEventStoreConfig::InMemory
  ```
- **Production policy:** Rejected in production profiles

## Projections: Read Models from Events

Projections are **computed views** derived from the event log. They are not the source of truth and are completely rebuildable from events. The `ironclaw_event_projections` crate provides:

### Core Projection Types

1. **Thread Timeline** — chronological view of all events in a thread
   - Entry per dispatch, model call, process transition, capability activity
   - Includes timestamps, invocation IDs, provider info, duration, errors
   - Scoped: excludes events from other threads sharing the same stream

2. **Run Status Projection** — current execution state of a single run
   - Status (queued, running, blocked, completed, failed, cancelled)
   - Latest capability activity and checkpoint/gate references
   - Duration and error kind if terminal
   - Lease metadata if currently running

3. **Capability Activity Projection** — lifecycle of a single tool/capability invocation
   - Status (requested, running, succeeded, failed, blocked)
   - Parent run identity
   - Duration and sanitized error summary
   - Gate references if blocked waiting for approval/auth

### Projection Guarantees

- **Deterministic:** Same input events always produce the same projection
- **Side-effect free:** Projections never write state
- **Rebuildable:** A projection failure is observable but never mutating; lost projections are reconstructed on next access
- **Metadata-only:** Never leak raw tool input/output, host paths, secrets, or backend error details
- **Scoped:** Preserve full scope (tenant, user, agent, project, thread, process) to enforce read isolation
- **Bounded:** Replay caps at 100,000 events per call; hitting the cap returns `RebaseRequired` error so caller must re-snapshot

### Cursor & Replay

Projections use **cursor-based resume**, not byte offsets:

```mermaid
sequenceDiagram
    Client->>ProjectionService: snapshot(scope, limit=100)
    ProjectionService->>DurableEventLog: read_after_cursor(origin, 100)
    DurableEventLog-->>ProjectionService: events + next_cursor
    ProjectionService->>ProjectionService: replay events into state
    ProjectionService-->>Client: ProjectionSnapshot + next_cursor
    
    Client->>ProjectionService: updates(cursor, limit=100)
    ProjectionService->>DurableEventLog: read_after_cursor(cursor, 100)
    DurableEventLog-->>ProjectionService: new events + next_cursor
    ProjectionService->>ProjectionService: fold new events into state
    ProjectionService-->>Client: ProjectionReplay + next_cursor
```

- **Cursor** — monotonic position in the per-stream event log (not a byte offset)
- **Snapshot** — full rebuild from `origin` cursor
- **Update** — incremental fold of events after a previous cursor
- **Rebase gap** — if cursor is older than earliest retained entry, surfaces `ReplayGap` error; caller must request fresh snapshot
- **Scope binding** — cursor carries the scope it was minted under; replaying it in a different scope is rejected

## Turn Persistence & Run State Journal

Turns (user messages submitted to the agent) are durable admitted work. Before a turn executes, the system persists:

### Turn Records

- **Turns** — one per accepted inbound message
  - Scope, actor, accepted-message reference, source/reply binding references
  - Created timestamp

- **Turn Runs** — agent-turn view of one process execution
  - Bindings, status, resolved run profile, checkpoint/gate references, lease fields

- **Turn Active Locks** — one-active-process-per-thread concurrency ownership
  - Status, monotonic version, acquired/updated timestamps

- **Turn Checkpoints** — process suspension/resume records
  - Opaque checkpoint reference, metadata, bounded payload (redacted)

- **Turn Admission Reservations** — capacity tracking per tenant/actor/project/agent
  - Linked to each accepted run for telemetry and limit enforcement
  - Released exactly once when run reaches terminal state

### Lease & Heartbeat Mechanics

When a runner claims a queued turn:

1. **Lease minting** — atomically records runner ID, lease token, expiry time, claim count
2. **Heartbeat** — renews `last_heartbeat_at` and extends `lease_expires_at`
3. **Expiry handling** — expired running/cancel-requested leases emit recovery event and converge to stable state (queued, cancelled, or failed)
4. **Terminal release** — exactly once when run reaches final status

Heartbeat enforcement is durable: a crash during heartbeat window does not leak the lease; recovery converges on next cycle.

### Turn State Lifecycle

```mermaid
stateDiagram-v2
    [*] --> Accepted: submit_turn succeeds
    
    Accepted --> Queued: queued by admission policy
    
    Queued --> Running: claim_next_processes<br/>runner mints lease
    
    Running --> Blocked: loop checkpoint returned<br/>clear lease, write checkpoint
    
    Running --> Completed: loop completed<br/>terminal transition
    Running --> Failed: loop failed<br/>terminal transition
    Running --> Cancelled: cancel_run received<br/>with valid lease
    
    Blocked --> Running: resume received<br/>new lease minted
    
    Completed --> [*]: Turn terminal
    Failed --> [*]: Turn terminal
    Cancelled --> [*]: Turn terminal
    RecoveryRequired --> Completed: recovery converges
    RecoveryRequired --> Failed: recovery converges
    RecoveryRequired --> Cancelled: recovery converges
```

## Event Sourcing as Primary Store

### Immutable Events, Computed State

The event log is the **source of truth**:
- Every dispatch request, capability outcome, process transition, and approval decision emits an event
- Events are immutable and append-only; never update or delete existing events
- State (run status, capability activity, turn status) is always **computed** from events

### Recovery & Replay

If a process crashes:
1. Find the last checkpoint or unfinished work in the turn run journal
2. Replay the run's events to reconstruct state
3. Resume from the last clean checkpoint or restart from the most recent work boundary

If the entire system restarts:
1. Replay all events for a thread to reconstruct the full timeline
2. Rebuild all projections from events
3. Recover in-flight turns from turn admission reservations and leases

### Audit & Compliance

The immutable event log serves as audit trail:
- Every approval decision is recorded with decision kind, actor, and timestamp
- Capability activity is logged with scope, provider, resource usage
- Failures are recorded with sanitized error kind and recovery outcome
- All records carry correlation IDs for tracing causally related operations

## Storage Migrations

Migrations are SQL scripts that evolve the schema over time. IronClaw uses **Flyway** for PostgreSQL and **libSQL** native migration management for SQLite.

### Migration Structure

```
migrations/
├── V1__initial.sql                    # Base schema
├── V2__events_table.sql               # Event log foundation
├── V3__audit_schema.sql               # Audit records
├── V4__projections_indexes.sql        # Projection support
├── checksums.lock                     # Integrity manifest
```

Each migration is **immutable** once released. Flyway tracks which migrations have run and prevents re-execution.

### Migration Principles

1. **Reuse existing schemas where viable.** Do not replace a working production table unless the frozen contract cannot be satisfied with an adapter.

2. **Prefer adapters over new tables.** Add migrations only for missing columns, indexes, or constraints that are contractually required.

3. **PostgreSQL and libSQL compatibility.** Consider both backends when authoring SQL. Use dialect-specific conditionals when necessary; test both.

4. **Immutable releases.** Released migrations never change. A mistake requires a new migration (e.g., `V5__fix_previous_migration.sql`).

5. **Idempotent backfills.** Any data transformation must be safe to re-run. Use `INSERT ... ON CONFLICT` or `CREATE TABLE IF NOT EXISTS`.

6. **Multi-tenant isolation.** Test tenant, user, project, and agent scope boundaries after every schema or mapping change. Scope leaks are production failures.

7. **Rollback planning.** Document rollback steps and failure modes for any breaking change.

### Fail-Closed Migration Policy

- **Test profile:** All backends allowed (in-memory, JSONL, libSQL, PostgreSQL)
- **Production profile:** libSQL and PostgreSQL only; in-memory and JSONL rejected
- **Remote PostgreSQL:** Requires `sslmode=require` and explicit allow-remote-cleartext setting
- **Schema validation:** TLS and durability requirements validated before opening connections

### Common Migration Patterns

**Adding a column:**
```sql
ALTER TABLE runtime_events ADD COLUMN new_field TEXT;
```

**Adding an index for a read pattern:**
```sql
CREATE INDEX idx_runtime_events_scope ON runtime_events(tenant_id, user_id, agent_id);
```

**Backfilling data from an existing column:**
```sql
UPDATE turn_runs SET canonical_scope = scope_json WHERE canonical_scope IS NULL;
```

**Creating a new table for a new domain:**
```sql
CREATE TABLE IF NOT EXISTS new_records (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

## Operational Tasks

### Querying Event History

Use the `DurableEventLog` interface to read events:

```rust
// Get a scoped stream
let stream = EventStreamKey::from_scope(&resource_scope);

// Read events after a cursor
let replay = event_log.read_after_cursor(
    &stream,
    &read_scope,
    after_cursor,
    limit
).await?;

// Inspect events
for entry in replay.entries {
    println!("{:?}", entry.record);  // RuntimeEvent
    println!("cursor: {}", entry.cursor);
}
```

### Snapshot & Rebase

When projections fall behind or need a full rebuild:

```rust
// Request a full snapshot from origin
let snapshot = projection_service.snapshot(
    &projection_scope,
    100  // limit
).await?;

// Use the snapshot to rebuild UI state
for run in &snapshot.runs {
    println!("Run {:?}: {}", run.run_id, run.status);
}

// Get next cursor for resuming updates
let next = snapshot.next_cursor;
```

### Monitoring Storage Health

- **libSQL:** Monitor WAL file size; periodically checkpoint to compact
- **PostgreSQL:** Monitor table bloat, replication lag, and connection pool utilization; run `VACUUM ANALYZE` on index tables
- **JSONL:** Monitor file size and I/O latency; rotate logs when they exceed size thresholds

### Backups

- **libSQL:** Copy the database file and WAL; restore by replacing the file
- **PostgreSQL:** Use `pg_dump` or continuous WAL archiving (WAL-E, pgBackRest)
- **JSONL:** Copy the log directory; ensure atomic snapshots of entire `/events` tree

## Redaction & Security

### Event Redaction Checklist

Before storing an event, verify:
- ✓ No raw prompts or assistant responses
- ✓ No tool input/output payloads
- ✓ No secrets, tokens, or auth keys
- ✓ No raw host paths or file system details
- ✓ No raw provider error messages (use classification tokens)
- ✓ No approval reasons or lease contents
- ✓ Error kind constrained to short classification (e.g., `timeout`, `rate_limit`, `Unclassified`)
- ✓ Error summary bounded and host-authored; unsafe summaries collapsed to marker

### Projection Redaction

Read models must preserve redaction:
- Only expose metadata-safe lifecycle facts (status, provider, duration, capability ID)
- Never include raw inputs, outputs, or paths
- Sanitize error summaries when exposing errors to UI
- Use only closed-vocabulary fields for hook metadata

## Related Pages

- [Turn & Execution Data Flow](/openwiki/architecture/data-flow.md) — end-to-end trace from submission through authorization and emission
- [Development & Testing](/openwiki/development/testing.md) — testing storage backends and event sourcing

## Key Crates

- **`ironclaw_event_log`** — event vocabulary, redaction, sink/durable traits
  - [`repo://crates/events/ironclaw_event_log`](repo://crates/events/ironclaw_event_log)
- **`ironclaw_event_store`** — backend selection, fail-closed policy
  - [`repo://crates/events/ironclaw_event_store`](repo://crates/events/ironclaw_event_store)
- **`ironclaw_event_projections`** — projection service, read models
  - [`repo://crates/events/ironclaw_event_projections`](repo://crates/events/ironclaw_event_projections)
- **`ironclaw_filesystem`** — unified storage dispatch (RootFilesystem trait)
  - [`repo://crates/substrates/ironclaw_filesystem`](repo://crates/substrates/ironclaw_filesystem)
- **`ironclaw_libsql_runtime`** — libSQL connection admission (one writer lane per database)
  - [`repo://crates/substrates/ironclaw_libsql_runtime`](repo://crates/substrates/ironclaw_libsql_runtime)

## Design References

- **Events contract:** `docs/internal/reborn/contracts/events.md` — event shape, sinks, audit envelope
- **Projections contract:** `docs/internal/reborn/contracts/events-projections.md` — projection reducer, cursor semantics, reconnect/rebase
- **Turn persistence contract:** `docs/internal/reborn/contracts/turn-persistence.md` — turn run records, lease/checkpoint rules, idempotency
- **Migration compatibility:** `docs/internal/reborn/contracts/migration-compatibility.md` — when to reuse vs. create new schemas
- **Storage placement:** `docs/internal/reborn/contracts/storage-placement.md` — where data lives, mount grants, scope isolation
