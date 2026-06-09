# Subagent durability sub-spec (WU-B)

Date: 2026-06-08.
Status: doc-only PR — blocks WU-C.
Source plan: `docs/plans/2026-06-06-subagent-compaction-impl.md` (WU-B section).
Parent design: `docs/reborn/2026-06-04-subagent-compaction-design.md`.
Prior shipped work: PR #4538 (WU-A — `PostCapabilityStage` + proactive compaction).

---

## Overview

This sub-spec closes the durability gaps the WU-A implementation plan deferred to a written design. WU-C cannot open until this lands.

Scope: four in-memory stores (`gate_resolution`, `goal`, `tombstone`, `capability_result`) plus one new write-ahead log (`settlement_event_log`) and one new idempotency table (`idempotency_ledger`). Each gets a typed trait, a libSQL backend, a PostgreSQL backend, and scope-column wiring per `_contract-freeze-index.md` §2 + §8.

Plus: introduce the `CapabilityResultStore` trait (does not exist today). Introduce the `SubagentRestartReconciler` trait (only a stub enum member today). Define the migration / rollback / re-flip behavior under the `subagent.background_enabled` feature toggle. Define the dual-backend parity test (#4431 follow-on).

### Decisions ratified up front

| # | Decision | Rationale anchor |
|---|---|---|
| 1 | All four stores + ledger + log live in `crates/ironclaw_reborn_event_store/`. No new `ironclaw_reborn_persistence` crate. | Reviewer 1 V1 — every new active Reborn crate needs a `BoundaryRule`; pivoting to `event_store` reuses the existing rule. `events.md` §2 makes it canonical owner. |
| 2 | `CapabilityResultStore` trait lives in `ironclaw_reborn_event_store`, NOT `ironclaw_loop_support`. | Reviewer 1 R2 — `loop_support` is adapter glue, not persistence. |
| 3 | Goal + tombstone stores use `ScopedFilesystem` (typed `FilesystemSubagentGoalStore` + new `FilesystemSubagentTombstoneStore`). | `.claude/rules/database.md` direction-of-travel for file-shaped, point-key/value access. Goal store already implements this. |
| 4 | Gate resolution + capability result store + settlement event log + idempotency ledger use **typed libSQL/PostgreSQL repositories**, NOT ScopedFilesystem. | `_contract-freeze-index.md` §2 storage model: high write rate, transactional multi-table consistency, scoped index scans. |
| 5 | All durable tables carry `tenant_id TEXT NOT NULL`, `user_id TEXT NOT NULL`, `agent_id TEXT NULL`. Primary lookup index leads with `(tenant_id, user_id, agent_id, …)`. | `_contract-freeze-index.md` §2 + §8 — cross-tenant scan isolation; `TurnScope` projection parity. |
| 6 | Settlement is first-writer-wins everywhere: `INSERT OR IGNORE` (libSQL) / `ON CONFLICT DO NOTHING` (PostgreSQL). | Plan Part 1 soft corrections — match in-memory `gate_resolution.rs` skip-if-set semantic. |
| 7 | Tombstone store gets one behavior correction: in-memory store moves from last-writer-wins to first-writer-wins to match durable backend. | Same as #6 — keep contract uniform across in-memory and durable paths. |
| 8 | In-flight RAM state at deploy → accept loss. Feature toggle (`subagent.background_enabled`, default false) gates user impact. | Plan WU-B "Migration of in-flight RAM state at deploy" — explicit recommendation. |
| 9 | Rollback (toggle OFF after ON) → leave durable rows in place. No GC in WU-C. `SubagentRestartReconciler` runs as no-op. | Plan Cross-cutting + LLM-data-never-deleted invariant from `CLAUDE.md`. |
| 10 | Re-flip (OFF → ON → OFF → ON) → idempotency ledger blocks double-delivery via `(run_id, child_run_id, terminal_kind)` UNIQUE constraint. | Plan WU-B "Concurrent settlement" + plan Part 1 Soft corrections. |
| 11 | Idempotency ledger is **two-phase** (`delivered_at` NULL = pencil, NOT NULL = sealed). Pencil insert claims ownership; gate-store write completes delivery; seal UPDATE marks final. Pencil rows surviving a crash become `retryable` on next boot. | D1 fix. Resolves "crash between ledger insert and gate-store write silently strands the parent loop" bug surfaced by multi-agent review. Matches `IdempotencyLedger::begin_or_replay` precedent. |
| 12 | Reconciler handles **orphan settlement log rows** (gate cleaned up before delivery) by writing a tombstone + sealing the ledger row in one pass. Counts as `skipped_orphan`, not `failed`. | D9 fix. Resolves "every boot counts cleaned-up gates as `failed` forever" bug. Preserves settlement log append-only invariant. |
| 13 | `ReplayReport` has five operator-meaningful counters: `redelivered`, `skipped_idempotent`, `retryable`, `skipped_orphan`, `failed`. Only `failed > 0` is actionable. | D1+D9. Eliminates "what does `skipped` actually mean here" alert ambiguity. |
| 14 | Reconciler replay algorithm is **batch-phased**: Phase 0 (bound input via LEFT JOIN), Phase 1 (preflight batch read), Phase 2 (multi-row ledger writes), Phase 3 (parallel capability loads via `join_all`), Phase 4 (per-row deliver+seal). Phases 0–3 issue O(1) DB calls regardless of pending-row count. | D4 fix. Resolves N+1 query problem surfaced by review. Phase 0 LEFT JOIN bounds replay input by outstanding work, not historical log size — addresses long-term concern (settlement log growth). |
| 15 | Reconciler runs in a **background `tokio::spawn` task**, not synchronously at boot. Foreground traffic (incl. blocking subagent calls) is NEVER blocked by replay. Background-mode spawn admission is gated **per-scope** by `ReplayState[scope].completed_at` — rejected with `SubagentSpawnError::ReplayInProgress { try_again_after_ms }` until complete. | D5 fix. Preserves <100 ms foreground cold start regardless of replay backlog. Multi-tenant safe: tenant A's recovery does not gate tenant B's admissions. Background-mode default is OFF through WU-G so user impact is bounded. |
| 16 | Reconciler uses a **dedicated `replay_pool`** (default 4 DB connections, configurable via `RebornEventStoreConfig.replay_pool_size`), separate from the main runtime connection pool. Prevents replay from starving foreground writes during a recovery storm. | D5 fix. Operationally observable as a distinct metric. Sizing controlled by operator. |
| 17 | **Active-scope enumeration is eager at boot** via a runs-table query for non-terminal runs. Bounded by active-runs count, not historical user count. Lazy per-scope replay on first traffic is a deferred optimization (would add cold-foreground latency for first-touch tenants after restart). | D5 design choice. Eager wins for foreground SLA at typical scale. |
| 18 | **Per-replay observability** is a contract, not optional. Required metrics (`replay_duration_seconds`, `replay_pending_rows`, `replay_outcomes_total{outcome=…}`, `pencil_age_seconds`, `replay_in_progress`), required alerts (`failed > 0`, `pencil_age_seconds > 60`, `replay_duration_seconds{P95} > 30`), tracing spans (one per scope + one per phase). Prerequisite for WU-G E2E + WU-F WebUI integration. | D5 long-term concern (operator clarity). See §5.7 for full contract. |
| 19 | **HA replication is supported but redundant.** Each replica boot runs its own replay independently. Correctness holds (Phase 2b `INSERT OR IGNORE` arbitrates, seal UPDATE is single-winner). Cost is N× DB load at boot. Active-active leader election is a cross-cutting follow-up, NOT WU-C scope. | Long-term posture. Spec is HA-safe today, HA-redundant. Optimizations are additive. |
| 20 | **Capacity cap uses a durable `subagent_gate_capacity_counter` table** (replaces per-spawn `SELECT COUNT(*)`). One transaction per spawn — no extra round-trip. Race-safe via `SELECT FOR UPDATE` (PostgreSQL) or `BEGIN IMMEDIATE` (libSQL). Counter is decremented symmetrically on delivery + delete. | D6-A. Removes hot-path COUNT scan. Scales linearly with spawn rate without lock contention beyond per-scope serialization. |
| 21 | **Capacity counter is sharded into K rows per scope** (default K = `CAPACITY_COUNTER_BUCKETS = 16`, operator-tunable). Spawn writes to `bucket = hash(child_run_id) % K`. Cap check is `SUM(undelivered) FROM counter WHERE scope`. `subagent_gate_awaited_children.counter_bucket` stores the bucket-of-record. | E.A. Lifts per-scope spawn throughput from ~100/sec (single-row lock) to ~1600/sec at K=16. Drift bound ≤ K-1 rows under maximum concurrency. Scales mega-tenant workloads (10k+ concurrent spawns under one scope). |
| 22 | **`CapabilityResultStore` trait accepts pre-serialized `Vec<u8>` payload**, not `serde_json::Value`. Executor serializes once via `serde_json::to_vec`; `byte_len = bytes.len() as u64` is derived for free; bytes are moved (not cloned) into the store. `read()` returns `Vec<u8>` symmetrically; caller deserializes lazily. | D8-A. Eliminates 2× serialization + Value clone per capability call. ~50% CPU reduction on capability writes at production scale. Trait shape reflects what actually crosses the boundary (bytes, not a tree). Composes with future streaming variants. |
| 23 | **Reconciler replay jitter (0..`RECONCILER_REPLAY_JITTER_MS`, default 5000 ms)** before launching background replay task. Spreads fleet-wide reconciler load over a wider window during rolling deploys. Jitter applies once per process boot; does NOT block foreground traffic. | A.A. Mitigates the stampede where N replicas all start replay simultaneously, caps DB conn count at ~`N×replay_pool/K` instead of `N×replay_pool`. Zero architectural cost. |
| 24 | **HA per-scope leader election via Postgres advisory locks is the long-term answer**, NOT WU-C scope. Documented in §5.10. Without leader election, all replicas replay redundantly but correctly — A.A jitter solves the operational pain at deploy time. libSQL deployments are typically single-node so the redundancy concern does not materialize. | A.B. Future direction documented; promotion trigger is `replay_duration_seconds{P95} > 30` at fleet-rollout time. Composes cleanly with D1 two-phase ledger (lock auto-releases on transaction end). |

The rest of this document fills in mechanics for each store.

---

## Section 1 — Gate resolution store

### 1.1 Current in-memory shape

`BoundedSubagentGateResolutionStore` (defined in `crates/ironclaw_reborn/src/subagent/gate_resolution.rs`) wraps a `parking_lot::Mutex<GateResolutionInner>`. The three denormalized maps inside `GateResolutionInner` are:

| Field | Key type | Value type | Purpose |
|---|---|---|---|
| `by_gate` | `GateRef` | `Vec<AwaitedChildState>` | Primary record store: all awaited-child states indexed by gate. Each `AwaitedChildState` embeds an `AwaitedChildSetRecord` plus per-child lifecycle flags (`terminal_status`, `terminal_event`, `terminal_result_written`, `terminal_byte_len`, `descendant_reservation_release_claimed`, `descendant_reservation_released`, `delivery_claimed`, `delivered_to_parent`). |
| `gates_by_child` | `TurnRunId` | `Vec<GateRef>` | Reverse index: all gates a given child run participates in. Used by `record_child_terminal` and `claim_next_terminal_state_for_child` to fan out terminal signals to every gate that references a child. |
| `deliverable_by_child` | `TurnRunId` | `VecDeque<GateRef>` | Delivery queue: gates for which a child has a claimable terminal state. Maintained alongside every write to `by_gate` so the O(1) claim path (`claim_deliverable_state_for_child`) never scans `by_gate`. |

The `total_states: usize` field is a cached count across all gate keys (O(1) capacity enforcement at `MAX_GATE_RECORDS = 4096`). It is not a fourth map. In the durable backend (§1.3 / §1.4) this O(1) accounting is preserved via a sidecar **`subagent_gate_capacity_counter`** table — one row per `(tenant_id, user_id, agent_id)` scope — updated transactionally with every INSERT / DELETE in the primary tables. The counter is the source of truth for cap-check on the spawn hot path; `SELECT COUNT(*)` on the primary table is NOT used.

`AwaitedChildSetRecord` (in `crates/ironclaw_loop_support/src/subagent_spawn_port.rs`) carries the key scope fields: `child_scope: TurnScope`, `parent_run_context: LoopRunContext`. `TurnScope` (in `crates/ironclaw_turns/src/scope.rs`) contains `tenant_id: TenantId`, `agent_id: Option<AgentId>`, `project_id: Option<ProjectId>`, and `thread_id: ThreadId`. The owning `user_id` is carried through `AwaitedChildTerminalEvent.owner_user_id: Option<UserId>` and indirectly through `TurnScope::thread_owner`.

First-writer-wins semantics are enforced at `record_awaited_child` (dedup by `gate_ref + child_run_id` before insert) and at `record_child_terminal` (skips re-recording if `terminal_status.is_some()`). The durable backend must replicate this with `INSERT OR IGNORE` / `ON CONFLICT DO NOTHING`.

### 1.2 Backend choice + rationale

**Choice: typed repository (SQL) inside `crates/ironclaw_reborn_event_store/`.**

Rationale against `ScopedFilesystem`:

- `_contract-freeze-index.md` §2 — "Storage model: hybrid: file-shaped content uses filesystem surfaces; **structured/query-heavy/security/control-plane state uses typed repositories**." Gate resolution is control-plane state: it gates parent-loop resumption and participates in descent-reservation accounting. It requires atomic cross-map consistency (all three maps are updated under one `parking_lot::Mutex` lock today), not sequential file writes.
- `storage-placement.md` §5.3 — "Structured control-plane state: source of truth is a typed repository owned by the domain; optional file-shaped projections may exist for diagnostics." Gate records are not file-shaped documents; they carry structured foreign-key relationships to `run_id` and `gate_ref`, need index-backed scoped queries (`tenant_id`, `child_run_id`, `gate_ref`), and need transactional multi-row upserts to preserve `INSERT OR IGNORE` first-writer-wins semantics.
- `.claude/rules/database.md` direction — "New persistence features go on `ScopedFilesystem`" applies to the legacy `src/db/` dissolution path. That rules file is scoped to `src/db/**`, `src/history/**`, `migrations/**`. The Reborn crate ecosystem under `crates/` is not in that scope. For Reborn persistence `ironclaw_reborn_event_store` is the established canonical owner (per `events.md` §2 and `crates/ironclaw_reborn_event_store/src/lib.rs` module doc). WU-C plan also explicitly designates `ironclaw_reborn_event_store` as the owner after ruling out both `ironclaw_reborn_persistence` (no boundary rule) and filesystem-only models.
- `ScopedFilesystem` cannot atomically update three logically related maps in a single transaction. The claim-then-deliver lifecycle across `by_gate`, `deliverable_by_child`, and `gates_by_child` must be atomic under restart recovery — a file-per-key approach cannot provide this.

**File locations (typed-repo path):**

- `crates/ironclaw_reborn_event_store/src/libsql/gate_resolution.rs` — libSQL repository
- `crates/ironclaw_reborn_event_store/src/postgres/gate_resolution.rs` — PostgreSQL repository
- The repository trait (`DurableSubagentGateResolutionStore`) lives in `crates/ironclaw_reborn_event_store/src/lib.rs` alongside the existing `DurableEventLog` / `DurableAuditLog` surface.

### 1.3 libSQL schema

```sql
-- Primary record table (replaces GateResolutionInner.by_gate)
CREATE TABLE IF NOT EXISTS subagent_gate_awaited_children (
    tenant_id               TEXT NOT NULL,
    user_id                 TEXT NOT NULL,
    agent_id                TEXT,           -- NULL for non-agent runs
    gate_ref                TEXT NOT NULL,
    parent_run_id           TEXT NOT NULL,
    tree_root_run_id        TEXT NOT NULL,
    child_run_id            TEXT NOT NULL,
    child_thread_id         TEXT NOT NULL,
    child_scope_json        TEXT NOT NULL,  -- JSON-encoded TurnScope (child_scope)
    parent_run_context_json TEXT NOT NULL,  -- JSON-encoded LoopRunContext
    source_binding_ref      TEXT NOT NULL,
    reply_target_binding_ref TEXT NOT NULL,
    subagent_kind           TEXT NOT NULL,
    spawn_capability_id     TEXT NOT NULL,
    result_ref              TEXT NOT NULL,
    spawn_mode              TEXT NOT NULL,  -- "blocking" | "background"
    counter_bucket          INTEGER NOT NULL,  -- bucket index used for capacity counter increment at INSERT time
    -- lifecycle flags (updated in-place by settlement/delivery ops)
    terminal_status         TEXT,           -- NULL until terminal; e.g. "completed" | "failed"
    terminal_event_json     TEXT,           -- JSON-encoded AwaitedChildTerminalEvent; NULL until terminal
    terminal_result_written INTEGER NOT NULL DEFAULT 0,  -- BOOLEAN (0/1)
    terminal_byte_len       INTEGER NOT NULL DEFAULT 0,
    descendant_reservation_release_claimed INTEGER NOT NULL DEFAULT 0,
    descendant_reservation_released        INTEGER NOT NULL DEFAULT 0,
    delivery_claimed        INTEGER NOT NULL DEFAULT 0,
    delivered_to_parent     INTEGER NOT NULL DEFAULT 0,
    created_at              TEXT NOT NULL DEFAULT (datetime('now')),
    settled_at              TEXT,           -- set when terminal_status first written
    PRIMARY KEY (gate_ref, child_run_id)
);

CREATE INDEX IF NOT EXISTS idx_sgac_tenant_user_agent
    ON subagent_gate_awaited_children (tenant_id, user_id, agent_id);
CREATE INDEX IF NOT EXISTS idx_sgac_child_run_id
    ON subagent_gate_awaited_children (child_run_id);
CREATE INDEX IF NOT EXISTS idx_sgac_parent_run_id
    ON subagent_gate_awaited_children (parent_run_id);
CREATE INDEX IF NOT EXISTS idx_sgac_undelivered_terminal
    ON subagent_gate_awaited_children (tenant_id, user_id, agent_id, delivered_to_parent, terminal_status)
    WHERE delivered_to_parent = 0;

-- Reverse-index table (replaces GateResolutionInner.gates_by_child)
CREATE TABLE IF NOT EXISTS subagent_gate_child_index (
    tenant_id    TEXT NOT NULL,
    child_run_id TEXT NOT NULL,
    gate_ref     TEXT NOT NULL,
    PRIMARY KEY (child_run_id, gate_ref)
);
CREATE INDEX IF NOT EXISTS idx_sgci_tenant_child
    ON subagent_gate_child_index (tenant_id, child_run_id);

-- Deliverable queue table (replaces GateResolutionInner.deliverable_by_child)
CREATE TABLE IF NOT EXISTS subagent_gate_deliverable_queue (
    tenant_id    TEXT NOT NULL,
    child_run_id TEXT NOT NULL,
    gate_ref     TEXT NOT NULL,
    queued_at    TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (child_run_id, gate_ref)
);
CREATE INDEX IF NOT EXISTS idx_sgdq_tenant_child
    ON subagent_gate_deliverable_queue (tenant_id, child_run_id);

-- Capacity counter (replaces per-spawn SELECT COUNT(*) on hot path)
-- Sharded by bucket for write throughput on hot scopes:
--   bucket = hash(child_run_id) % CAPACITY_COUNTER_BUCKETS  (default K=16)
-- Cap check reads SUM(undelivered) across all buckets for a scope.
CREATE TABLE IF NOT EXISTS subagent_gate_capacity_counter (
    tenant_id    TEXT     NOT NULL,
    user_id      TEXT     NOT NULL,
    agent_id     TEXT,             -- NULL for non-agent runs
    bucket       INTEGER  NOT NULL,  -- 0 .. CAPACITY_COUNTER_BUCKETS-1
    undelivered  INTEGER  NOT NULL DEFAULT 0
        CHECK (undelivered >= 0),
    PRIMARY KEY (tenant_id, user_id, agent_id, bucket)
);
-- Range-scan-friendly index for SUM(undelivered) WHERE (scope) lookup:
CREATE INDEX IF NOT EXISTS idx_sgcc_scope
    ON subagent_gate_capacity_counter (tenant_id, user_id, agent_id);
```

All inserts use `INSERT OR IGNORE` keyed on the `PRIMARY KEY` for first-writer-wins.

### 1.4 PostgreSQL schema

Same logical shape, dialect-correct:

```sql
CREATE TABLE IF NOT EXISTS subagent_gate_awaited_children (
    tenant_id               TEXT        NOT NULL,
    user_id                 TEXT        NOT NULL,
    agent_id                TEXT,
    gate_ref                TEXT        NOT NULL,
    parent_run_id           TEXT        NOT NULL,
    tree_root_run_id        TEXT        NOT NULL,
    child_run_id            TEXT        NOT NULL,
    child_thread_id         TEXT        NOT NULL,
    child_scope_json        JSONB       NOT NULL,
    parent_run_context_json JSONB       NOT NULL,
    source_binding_ref      TEXT        NOT NULL,
    reply_target_binding_ref TEXT       NOT NULL,
    subagent_kind           TEXT        NOT NULL,
    spawn_capability_id     TEXT        NOT NULL,
    result_ref              TEXT        NOT NULL,
    spawn_mode              TEXT        NOT NULL,
    counter_bucket          SMALLINT    NOT NULL,  -- bucket index used for capacity counter increment at INSERT time
    terminal_status         TEXT,
    terminal_event_json     JSONB,
    terminal_result_written BOOLEAN     NOT NULL DEFAULT FALSE,
    terminal_byte_len       BIGINT      NOT NULL DEFAULT 0,
    descendant_reservation_release_claimed BOOLEAN NOT NULL DEFAULT FALSE,
    descendant_reservation_released        BOOLEAN NOT NULL DEFAULT FALSE,
    delivery_claimed        BOOLEAN     NOT NULL DEFAULT FALSE,
    delivered_to_parent     BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    settled_at              TIMESTAMPTZ,
    PRIMARY KEY (gate_ref, child_run_id)
);

CREATE INDEX IF NOT EXISTS idx_sgac_tenant_user_agent
    ON subagent_gate_awaited_children (tenant_id, user_id, agent_id);
CREATE INDEX IF NOT EXISTS idx_sgac_child_run_id
    ON subagent_gate_awaited_children (child_run_id);
CREATE INDEX IF NOT EXISTS idx_sgac_parent_run_id
    ON subagent_gate_awaited_children (parent_run_id);
CREATE INDEX IF NOT EXISTS idx_sgac_undelivered_terminal
    ON subagent_gate_awaited_children (tenant_id, user_id, agent_id, delivered_to_parent, terminal_status)
    WHERE delivered_to_parent = FALSE;

CREATE TABLE IF NOT EXISTS subagent_gate_child_index (
    tenant_id    TEXT NOT NULL,
    child_run_id TEXT NOT NULL,
    gate_ref     TEXT NOT NULL,
    PRIMARY KEY (child_run_id, gate_ref)
);
CREATE INDEX IF NOT EXISTS idx_sgci_tenant_child
    ON subagent_gate_child_index (tenant_id, child_run_id);

CREATE TABLE IF NOT EXISTS subagent_gate_deliverable_queue (
    tenant_id    TEXT        NOT NULL,
    child_run_id TEXT        NOT NULL,
    gate_ref     TEXT        NOT NULL,
    queued_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (child_run_id, gate_ref)
);
CREATE INDEX IF NOT EXISTS idx_sgdq_tenant_child
    ON subagent_gate_deliverable_queue (tenant_id, child_run_id);

-- Capacity counter (replaces per-spawn SELECT COUNT(*) on hot path)
-- Sharded by bucket for write throughput on hot scopes:
--   bucket = hash(child_run_id) % CAPACITY_COUNTER_BUCKETS  (default K=16)
-- Cap check reads SUM(undelivered) across all buckets for a scope.
CREATE TABLE IF NOT EXISTS subagent_gate_capacity_counter (
    tenant_id    TEXT     NOT NULL,
    user_id      TEXT     NOT NULL,
    agent_id     TEXT,
    bucket       SMALLINT NOT NULL,
    undelivered  INTEGER  NOT NULL DEFAULT 0
        CHECK (undelivered >= 0)
);
-- PK with NULL agent_id requires COALESCE-based unique index in PostgreSQL:
CREATE UNIQUE INDEX IF NOT EXISTS idx_sgcc_pk
    ON subagent_gate_capacity_counter
       (tenant_id, user_id, COALESCE(agent_id, '__non_agent__'), bucket);
-- Range-scan index for SUM lookup:
CREATE INDEX IF NOT EXISTS idx_sgcc_scope
    ON subagent_gate_capacity_counter (tenant_id, user_id, agent_id);
```

All inserts use `ON CONFLICT DO NOTHING`.

### 1.5 Settlement event log

The `SubagentRestartReconciler` (currently a stub enum member in `crates/ironclaw_reborn/src/production_readiness.rs` under `RebornLoopProductionComponent::SubagentRestartReconciler`) needs a replay log to reconstruct settled terminal states after a process restart. This table records every terminal settlement event so the reconciler can re-drive delivery for any gate not yet marked `delivered_to_parent = true`.

**libSQL:**

```sql
CREATE TABLE IF NOT EXISTS subagent_gate_settlement_log (
    id              INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    tenant_id       TEXT    NOT NULL,
    user_id         TEXT    NOT NULL,
    agent_id        TEXT,
    gate_ref        TEXT    NOT NULL,
    child_run_id    TEXT    NOT NULL,
    result_ref      TEXT    NOT NULL,
    parent_run_id   TEXT    NOT NULL,
    terminal_status TEXT    NOT NULL,   -- "completed" | "failed" | "cancelled"
    terminal_kind   TEXT    NOT NULL,   -- TurnEventKind serialized
    event_cursor    INTEGER NOT NULL,
    terminal_byte_len INTEGER NOT NULL DEFAULT 0,
    sanitized_reason TEXT,              -- redacted; NULL is valid
    owner_user_id   TEXT,
    settled_at      TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_sgsl_tenant_user_agent
    ON subagent_gate_settlement_log (tenant_id, user_id, agent_id);
CREATE INDEX IF NOT EXISTS idx_sgsl_parent_run_id
    ON subagent_gate_settlement_log (parent_run_id);
CREATE INDEX IF NOT EXISTS idx_sgsl_child_run_id
    ON subagent_gate_settlement_log (child_run_id);
```

**PostgreSQL:**

```sql
CREATE TABLE IF NOT EXISTS subagent_gate_settlement_log (
    id              BIGSERIAL   NOT NULL PRIMARY KEY,
    tenant_id       TEXT        NOT NULL,
    user_id         TEXT        NOT NULL,
    agent_id        TEXT,
    gate_ref        TEXT        NOT NULL,
    child_run_id    TEXT        NOT NULL,
    result_ref      TEXT        NOT NULL,
    parent_run_id   TEXT        NOT NULL,
    terminal_status TEXT        NOT NULL,
    terminal_kind   TEXT        NOT NULL,
    event_cursor    BIGINT      NOT NULL,
    terminal_byte_len BIGINT    NOT NULL DEFAULT 0,
    sanitized_reason TEXT,
    owner_user_id   TEXT,
    settled_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_sgsl_tenant_user_agent
    ON subagent_gate_settlement_log (tenant_id, user_id, agent_id);
CREATE INDEX IF NOT EXISTS idx_sgsl_parent_run_id
    ON subagent_gate_settlement_log (parent_run_id);
CREATE INDEX IF NOT EXISTS idx_sgsl_child_run_id
    ON subagent_gate_settlement_log (child_run_id);
```

**`sanitized_reason` contract.** This column persists a short, sanitized failure reason for operator debugging. Source: the `LoopFailureKind` discriminator OR a fixed-length truncated prefix of the failure message (max 256 chars), with non-ASCII characters stripped. Sanitization MUST occur at the settlement write site (`SubagentCompletionObserver`), not at the log boundary. Raw LLM output, user task descriptions, and unbounded error strings MUST NOT be written here. When in doubt, write NULL — the log row is still actionable from the `terminal_status` + `terminal_kind` columns alone. Prefer a future migration to a constrained `failure_code TEXT` (enum-backed) column if redaction proves error-prone.

The log is **append-only**. Replay reads rows for active parent runs and re-drives `record_child_terminal` on the primary `subagent_gate_awaited_children` table; the `terminal_status IS NULL` guard there makes replay idempotent.

### 1.6 Atomic migration

**Scope-predicate convention.** Every UPDATE and DELETE in this section MUST scope by the caller's full `TurnScope`. The `agent_id` predicate is **conditional**:

- When the caller's `TurnScope.agent_id` is `Some(id)`: emit `agent_id = ?` (and bind `id`).
- When the caller's `TurnScope.agent_id` is `None`: emit `agent_id IS NULL`.

NEVER unconditionally emit `(agent_id = ? OR agent_id IS NULL)` — that pattern lets agent-scoped callers reach system-level (non-agent) rows under the same `(tenant_id, user_id)`, breaking cross-agent isolation. The pseudocode below uses a placeholder `<agent_predicate>` which the application binds per the rule above.

The insert/settle/delete paths must move all three tables under one transaction so a partial crash can't leave the indexes pointing at a missing primary row (or vice versa).

```sql
BEGIN;
  -- 1. Pick a bucket for this spawn (deterministic from child_run_id).
  --    Pseudocode in application layer:
  --        bucket = (hash(child_run_id) % CAPACITY_COUNTER_BUCKETS) as i32
  -- 2. Initialize this bucket row if missing (INSERT OR IGNORE).
  INSERT OR IGNORE INTO subagent_gate_capacity_counter
    (tenant_id, user_id, agent_id, bucket, undelivered)
    VALUES (?, ?, ?, ?, 0);

  -- 3. Cap check: SUM across all buckets for this scope.
  --    libSQL uses transaction-level isolation (BEGIN IMMEDIATE) to
  --    serialize per-scope; PostgreSQL adds SELECT ... FOR UPDATE on
  --    each bucket row that will be written (see §1.4 note below).
  SELECT COALESCE(SUM(undelivered), 0) FROM subagent_gate_capacity_counter
   WHERE tenant_id = ? AND user_id = ? AND <agent_predicate>;
  -- If SUM >= 4096: ROLLBACK + return CapacityExceeded.
  -- Otherwise:

  INSERT OR IGNORE INTO subagent_gate_awaited_children (...) VALUES (...);
  INSERT OR IGNORE INTO subagent_gate_child_index (tenant_id, child_run_id, gate_ref) VALUES (?, ?, ?);
  -- only if terminal status is being inserted directly (background re-hydration):
  INSERT OR IGNORE INTO subagent_gate_deliverable_queue (tenant_id, child_run_id, gate_ref) VALUES (?, ?, ?);

  -- 4. Increment this specific bucket only (no cross-bucket contention).
  UPDATE subagent_gate_capacity_counter
     SET undelivered = undelivered + 1
   WHERE tenant_id = ? AND user_id = ? AND <agent_predicate> AND bucket = ?;
COMMIT;
```

**Bucketed counter under PostgreSQL.** Cap check is `SELECT COALESCE(SUM(undelivered), 0) FROM ... WHERE scope`. To prevent TOCTOU drift, the transaction takes a row-level lock on the bucket being incremented via `SELECT undelivered FROM subagent_gate_capacity_counter WHERE scope = ? AND bucket = ? FOR UPDATE` immediately before the SUM read. Drift bound: at most `CAPACITY_COUNTER_BUCKETS - 1` rows over cap under maximum concurrency (each bucket's transaction holds its own lock but reads others without lock). For default K=16, drift ≤ 15 rows on a 4096 cap — negligible relative to fan-out scale, and bounded.

**Bucketed counter under libSQL.** Use `BEGIN IMMEDIATE` for transaction-level serialization. Drift bound is 0 because the entire counter table is logically locked during the transaction. Throughput per scope is correspondingly lower; libSQL deployments are single-node so this is acceptable.

**Why bucketed.** A mega-tenant running 10k+ concurrent spawns under one scope would otherwise serialize on a single counter row. With K=16 buckets, write contention drops by 16× — practical throughput per scope rises from ~100/sec to ~1600/sec on PostgreSQL. Cross-bucket reads (the SUM) are cheap because the partial index `idx_sgcc_scope` covers them. K is the `CAPACITY_COUNTER_BUCKETS` constant in `ironclaw_reborn_event_store`, default 16, operator-tunable per deployment via `RebornEventStoreConfig`.

```sql
-- Settlement path (record_child_terminal equivalent):
BEGIN;
  UPDATE subagent_gate_awaited_children
     SET terminal_status = ?, terminal_event_json = ?, settled_at = datetime('now')
   WHERE gate_ref = ? AND child_run_id = ? AND terminal_status IS NULL
     AND tenant_id = ? AND user_id = ? AND <agent_predicate>;
  -- log row only if the UPDATE above touched a row:
  INSERT INTO subagent_gate_settlement_log (...) VALUES (...);
  INSERT OR IGNORE INTO subagent_gate_deliverable_queue (tenant_id, child_run_id, gate_ref) VALUES (?, ?, ?);
COMMIT;
```

Terminal settlement does NOT touch the capacity counter — the row remains until either delivery completes (sets `delivered_to_parent = 1`, see below) or the gate is explicitly deleted.

> **Scope predicate is mandatory.** Every UPDATE/DELETE in this section MUST include the full `(tenant_id, user_id, agent_id)` scope predicate matching §8.2. Reviewer-mandated.

**Post-result-write flag update path (separate transaction):**

After the capability result store write completes, the executor issues a single-row UPDATE to flip the `terminal_result_written` flag and record `terminal_byte_len`. This is intentionally separate from the settlement transaction so the capability write can be retried without re-running settlement.

```sql
UPDATE subagent_gate_awaited_children
   SET terminal_result_written = 1,
       terminal_byte_len       = ?
 WHERE gate_ref = ? AND child_run_id = ? AND terminal_result_written = 0
   AND tenant_id = ? AND user_id = ? AND <agent_predicate>;
```

PostgreSQL substitutes `terminal_result_written = TRUE` / `= FALSE`. The reconciler treats `terminal_status IS NOT NULL AND terminal_result_written = 0` as "settled but capability result write pending" — it loads the result from the capability result store and flips the flag itself if it finds a written result.

Settlement log rows are **not** deleted on gate cleanup — they remain the replay source of truth for `SubagentRestartReconciler`. PostgreSQL uses `ON CONFLICT DO NOTHING` in place of `INSERT OR IGNORE`.

**Delivery-claim path** flips `delivery_claimed` / `delivered_to_parent` and decrements the capacity counter atomically:

```sql
BEGIN;
  UPDATE subagent_gate_awaited_children
     SET delivery_claimed = 1,
         delivered_to_parent = 1
   WHERE gate_ref = ? AND child_run_id = ?
     AND tenant_id = ? AND user_id = ? AND <agent_predicate>
     AND delivered_to_parent = 0;
  -- Decrement THIS spawn's bucket (counter_bucket recorded at INSERT).
  UPDATE subagent_gate_capacity_counter
     SET undelivered = GREATEST(undelivered - 1, 0)
   WHERE tenant_id = ? AND user_id = ? AND <agent_predicate>
     AND bucket = ?;
  -- Delete the SPECIFIC child's queue entry — NOT all queue entries for the gate.
  DELETE FROM subagent_gate_deliverable_queue
   WHERE gate_ref = ? AND child_run_id = ? AND tenant_id = ?;
COMMIT;
```

PostgreSQL substitutes `delivered_to_parent = TRUE` and `GREATEST` works as-is. The decrement is `GREATEST(undelivered - 1, 0)` so that even under partial-failure replay the counter never goes negative — a safety net for the rare case where a flip is replayed.

**Delete path:**

```sql
BEGIN;
  -- (a) Count rows to delete, grouped by counter_bucket.
  SELECT counter_bucket, COUNT(*) FROM subagent_gate_awaited_children
   WHERE gate_ref = ?
     AND tenant_id = ? AND user_id = ? AND <agent_predicate>
     AND delivered_to_parent = 0
   GROUP BY counter_bucket;
  -- Result: (bucket, N) tuples. Application iterates.

  DELETE FROM subagent_gate_deliverable_queue
   WHERE gate_ref = ? AND tenant_id = ? AND user_id = ?;
  DELETE FROM subagent_gate_child_index
   WHERE gate_ref = ? AND tenant_id = ? AND user_id = ?;
  DELETE FROM subagent_gate_awaited_children
   WHERE gate_ref = ? AND tenant_id = ? AND user_id = ? AND <agent_predicate>;

  -- (b) Decrement each touched bucket by its row count. App-side loop:
  --   for (bucket, n) in result:
  --     UPDATE subagent_gate_capacity_counter
  --        SET undelivered = GREATEST(undelivered - n, 0)
  --      WHERE tenant_id = ? AND user_id = ? AND <agent_predicate>
  --        AND bucket = ?;
COMMIT;
```

The delete path's GROUP BY scan is bounded by rows under one `gate_ref` (typically <20). Bucket-decrement is up to K UPDATE statements (one per distinct bucket touched). At K=16, worst case is 16 small UPDATE statements — still O(1) per gate cleanup. The schema mandates `subagent_gate_awaited_children.counter_bucket INTEGER NOT NULL` so the bucket lookup is a column read, not a recomputation.

### 1.7 Risks / open questions

- **`child_scope_json` / `parent_run_context_json` size + sensitivity (audit required).** `LoopRunContext` is a complex struct (strategy configuration included). Two concerns:

  - *Size*: storing as JSON/JSONB blob sidesteps schema normalization but blocks queries against context fields. If the reconciler ever needs to query by `parent_run_context.scope.agent_id`, those columns must be promoted to first-class SQL columns. For now, top-level indexed columns (`tenant_id`, `user_id`, `agent_id`, `parent_run_id`) cover the scoped-scan needs.

  - *Sensitivity*: WU-C MUST audit `LoopRunContext` fields before implementation and confirm none of them carry credentials, API keys, LLM provider tokens, or other sensitive material. If any sensitive field is found, the write-site MUST strip it before serialization. If no sensitive field exists, WU-C MUST add a compile-time lint or test asserting `LoopRunContext` remains credential-free (any future field addition must re-verify). Persisting plaintext credentials in a durable, replicated table is unacceptable. This is a closing-checklist gate.
- **`user_id` derivation.** `TurnScope` does not carry an explicit `user_id` directly; it surfaces the owner through `TurnThreadOwner::ExplicitUser.owner_user_id` or falls back to the system sentinel. The durable schema uses `user_id TEXT NOT NULL` — the insert path resolves `TurnScope::explicit_owner_user_id()` and writes the sentinel (`ironclaw_host_api::SYSTEM_RESERVED_ID`) when the owner is `ActorFallback` or `Ownerless`.
- **Settlement log deduplication (resolved).** The settlement log is append-only. Duplicate rows on the same `(gate_ref, child_run_id, terminal_kind)` are *possible* under replay-storm conditions but are **benign**: the idempotency ledger's UNIQUE constraint on `(run_id, child_run_id, terminal_kind)` (§5.4 / §5.5) ensures at most one pencil-receipt insert succeeds; the gate-store record_background_settlement is idempotent on its own primary key; the seal UPDATE is row-level idempotent (`delivered_at IS NULL` guard). Phase 0's LEFT JOIN against the ledger filters out already-sealed rows so duplicate log entries that map to a sealed ledger row are never re-processed. There is no need for a `MIN(id)` ordering choice or a settlement-log UNIQUE constraint at this layer. A future log-rotation / TTL job MAY de-duplicate physically for storage hygiene but that is operational, not correctness-load-bearing.
- **`deliverable_by_child` as queue table vs. computed view.** Queue table matches in-memory semantics exactly but risks queue/primary-table skew on partial failure. Computed view is always consistent but adds a join on every claim call. Decision recorded in WU-C PR description; recommendation: queue table (matches the lock-free O(1) in-memory contract).
- **Capacity cap (D6-A + E.A).** `MAX_GATE_RECORDS = 4096` per scope is enforced via the `subagent_gate_capacity_counter` table, **sharded into `CAPACITY_COUNTER_BUCKETS = 16` rows per `(tenant_id, user_id, agent_id)` scope** (operator-tunable per deployment). Spawn picks a bucket via `hash(child_run_id) % K` and increments only that bucket's row. Cap check reads `SUM(undelivered) FROM counter WHERE scope` — cheap with the partial index. This sharding lifts the per-scope spawn throughput ceiling from ~100/sec (single-row lock contention) to ~1600/sec on PostgreSQL at K=16; libSQL deployments retain serialized `BEGIN IMMEDIATE` semantics regardless of K. Drift bound under PostgreSQL with concurrent fan-out is `K - 1` rows over cap (15 at K=16) — bounded and well below the cap itself. The bucket-of-record is stored on `subagent_gate_awaited_children.counter_bucket` at INSERT time so cleanup + delivery paths decrement the correct bucket without rehashing. K is a one-line config knob; raising to K=64 doubles throughput at the cost of slightly more bucket scan on the cap-check SUM (still well under 1 ms). This design scales to mega-tenant deployments (10k+ concurrent spawns under one scope) without falling back to soft caps or background admission control.
- **`agent_id` nullable contract.** Every scoped query must include `agent_id` in the predicate (`agent_id = ?` OR `agent_id IS NULL`) — never omit it — to avoid cross-agent leakage in multi-agent tenants.

---

## Section 2 — Goal store

### 2.1 Current in-memory shape

**Symbol:** `InMemoryBoundedSubagentGoalStore` (`crates/ironclaw_reborn/src/subagent/goal_store.rs`).

**Trait:** `SubagentGoalStore` (three async methods: `put_goal`, `get_goal`, `delete_goal`). Also implements `ironclaw_loop_support::SubagentSpawnGoalStore` (two-method subset: `put_goal`, `delete_goal`). The spawn port calls through the narrower trait; the reconciler will call through the full trait.

**Key shape:** `(scope: &TurnScope, run_id: TurnRunId)`. `TurnScope` carries `tenant_id` (always present), `agent_id` (nullable), `project_id` (nullable), and `thread_id` (always present).

**Value:** `SubagentGoal { task: String, handoff: Option<String> }` — JSON-serialized, capped at `MAX_GOAL_BYTES = 64 KiB`.

**Internal data structure:** `GoalStoreInner { goals: HashMap<GoalKey, SubagentGoal>, insertion_order: VecDeque<GoalKey> }` behind a `std::sync::Mutex`. Bounded at `MAX_GOAL_ENTRIES = 4096`. Eviction: LRU-by-insertion. Write semantics: `DuplicateKey` error on second `put_goal` for the same `(scope, run_id)` — first-writer-wins.

**Existing production path:** `FilesystemSubagentGoalStore<F>` already exists in the same file, behind `#[cfg(feature = "filesystem-goal-store")]`. Each goal is a JSON file at a `ScopedPath` under `/turns/subagent-goals/`. Composition (`crates/ironclaw_reborn_composition/src/runtime.rs`) already selects `FilesystemSubagentGoalStore` when `libsql` or `postgres` feature is enabled. **The goal store already has a durable production backend; WU-C must document the schema and verify the production-readiness wiring is marked correctly.**

### 2.2 Backend choice + rationale

**Choice: `ScopedFilesystem` (already implemented as `FilesystemSubagentGoalStore`).**

- `.claude/rules/database.md`: "New persistence features go on `ScopedFilesystem`, not into `src/db/`."
- Each goal is an independent JSON document addressed by a unique `(scope, run_id)` path — file-shaped, key-value access.
- No cross-row queries, joins, or aggregations.
- `ScopedFilesystem` with a `LibSqlRootFilesystem` or `PostgresRootFilesystem` backend gives both durable backends through one path without a new SQL schema.
- `ironclaw_filesystem/CLAUDE.md` invariant 7 satisfied: scope keys appear in the path prefix.

A typed SQL repository would be wrong: this is not query-heavy structured state.

### 2.3 libSQL / PostgreSQL — file layout

`FilesystemSubagentGoalStore` has no SQL schema of its own. The backing table is owned by `LibSqlRootFilesystem` / `PostgresRootFilesystem`, which store every `Entry` as a row in the universal blob table. The consumer-visible layout:

```
/turns/subagent-goals/
  [agents/<agent_id>/]
  [projects/<project_id>/]
  threads/<thread_id>/
  <run_id_uuid>.json
```

- `agent_id` and `project_id` path segments are optional (inserted only when present in `TurnScope`).
- `tenant_id` / `user_id` isolation provided by `ScopedFilesystem`'s `MountView` (per-tenant mount); they never appear in the `ScopedPath` itself per invariant 7.
- CAS semantics: `put_goal` uses `CasExpectation::Absent` → `FilesystemError::VersionMismatch` on duplicate → mapped to `SubagentGoalStoreError::DuplicateKey`. First-writer-wins.
- `delete_goal` treats `FilesystemError::NotFound` as success (idempotent delete).

No per-store `CREATE TABLE` required for goal store. The `LibSqlRootFilesystem` / `PostgresRootFilesystem` universal table is already present.

If `list_dir` on `/turns/subagent-goals/agents/<agent_id>/` is ever needed for reconciler replay, verify `PostgresRootFilesystem` has a path-prefix index; add one during WU-C if missing.

### 2.4 Risks / open questions

- **Production-readiness wiring gap.** Composition selects `FilesystemSubagentGoalStore` correctly when the db feature is enabled, but `RebornLoopComponentGraphReadiness.subagent_goal_store` must be set to `RebornComponentReadiness::production_verified(Required)` (not `non_durable`) when `FilesystemSubagentGoalStore` is in use. WU-C must close this. WU-C MUST also add the symmetric positive test `production_readiness_accepts_filesystem_subagent_goal_store` asserting that `graph.subagent_goal_store = RebornComponentReadiness::production_verified(Required)` yields `RebornLoopProductionStatus::Ready`.
- **`MAX_GOAL_ENTRIES` eviction not replicated in filesystem store.** In-memory silently evicts oldest at 4096. Filesystem store has no cap — old goals accumulate until explicitly deleted. Lifecycle cleanup of stale goals (runs that completed or were cancelled without a `delete_goal` call) is the reconciler's responsibility.
- **Restart-resume correctness.** `get_goal` is called during prompt assembly for a restarted subagent run. `FilesystemSubagentGoalStore::get_goal` with libSQL or PostgreSQL backend is durable as soon as `put` returns `Ok`.
- **Duplicate-key vs restart.** `SubagentCompletionObserver` doesn't retry `put_goal`; `SubagentRestartReconciler` replay might. Recommendation: reconciler skips `put_goal` if `get_goal` succeeds — goal already present means the original write committed.

---

## Section 3 — Tombstone store

### 3.1 Current in-memory shape

**Symbol:** `BoundedSubagentResultTombstoneStore` (`crates/ironclaw_reborn/src/subagent/tombstone_store.rs`).

**Trait:** `SubagentResultTombstoneStore` — two async methods:
- `write_tombstone(&self, tombstone: SubagentResultTombstone) -> Result<(), TombstoneStoreError>`
- `read_tombstone(&self, child_run_id: TurnRunId) -> Result<Option<SubagentResultTombstone>, TombstoneStoreError>`

**Key shape:** `child_run_id: TurnRunId`. Note: **no `TurnScope` in the key.** The in-memory store keys on global-UUID uniqueness.

**Value:** `SubagentResultTombstone { child_run_id: TurnRunId, terminal_status: TurnStatus, disposition: SubagentResultDisposition }`. `SubagentResultDisposition` variants:
- `DiscardedByParentCancel` (in-memory store today)
- `DiscardedParentGone` (WU-C addition — emitted by `SubagentRestartReconciler` when an orphan row is detected during replay; see §5.3 Phase 2a)

WU-D may add more (e.g. `Delivered`, `SettledByBackground`); WU-C must accept additional variants without breaking serde round-trip.

**Internal data structure:** `TombstoneInner { by_child: HashMap<TurnRunId, SubagentResultTombstone>, insertion_order: VecDeque<TurnRunId> }` behind `std::sync::Mutex`. Bounded at `MAX_TOMBSTONE_RECORDS = 4096`. Write semantics: `write_tombstone` is **last-writer-wins** in memory today. This must be corrected to first-writer-wins to match the durable backend — see §3.6.

**Current wiring:** `BoundedSubagentResultTombstoneStore` is instantiated but **not currently passed into `DefaultPlannedRuntimeParts` or `SubagentCompletionObserver`**. The tombstone write site (`mark_child_deliveries` path) does not call into any tombstone store today. The store exists, the trait exists, the production-readiness component exists — the wiring is the missing piece.

### 3.2 Why it's in scope

`production_readiness.rs` lists `SubagentResultTombstoneStore` as `RebornLoopProductionComponent::SubagentResultTombstoneStore` and validates it against `RebornComponentGraphReadiness.subagent_result_tombstone_store`. The test `production_readiness_rejects_non_durable_subagent_tombstone_store` confirms that `RebornComponentReadiness::non_durable(Required)` yields `RebornLoopProductionStatus::NotReady`. The only available implementation today is `NonDurable` → production blocks.

**Idempotency role:** prevents re-delivery of a settled child result after a parent-process restart. Without a durable tombstone, a restart can deliver the same result twice → double writes to the capability result store and double-resumes of the parent gate.

### 3.3 Backend choice + rationale

**Choice: `ScopedFilesystem` — new `FilesystemSubagentTombstoneStore<F>`** mirroring `FilesystemSubagentGoalStore`.

- Access pattern matches goal store: point-write keyed by `child_run_id`, point-read by `child_run_id`, no cross-row queries.
- `.claude/rules/database.md`: "New persistence features go on `ScopedFilesystem`."
- `ScopedFilesystem` idempotency via `CasExpectation::Absent` gives first-writer-wins semantics.
- Path-based `tenant_id` / `user_id` isolation: include scope axes in the path prefix. Unlike the in-memory store, the durable store **must** include scope columns in the path because `TurnRunId` UUIDs are unique in practice but not cryptographically scoped per-tenant.

### 3.4 libSQL / PostgreSQL — `ScopedPath` layout

`FilesystemSubagentTombstoneStore<F>` stores each tombstone as a JSON entry at:

```
/turns/subagent-tombstones/
  [agents/<agent_id>/]
  threads/<thread_id>/
  <child_run_id_uuid>.json
```

- `tenant_id` / `user_id` isolation via `ScopedFilesystem` `MountView`.
- `agent_id` is nullable — include when present in scope.
- `thread_id` included for scoping + future `list_dir` enumeration during reconciler replay.

**Idempotency via `CasExpectation::Absent`:**
```
put(&resource_scope, &tombstone_path, entry, CasExpectation::Absent)
```
- First write → `Ok(_)`.
- Second write for same `child_run_id` → `FilesystemError::VersionMismatch` → map to `Ok(())` (already recorded; idempotent). This is `INSERT OR IGNORE` semantics at filesystem layer.
- Do NOT use `CasExpectation::Any` — that allows silent overwrite and clobbers the original terminal status.

No per-store `CREATE TABLE` required for the `ScopedFilesystem` path.

### 3.5 Fallback typed-repo schema (if ever promoted)

If the tombstone store is ever promoted to a typed repository for query support:

```sql
-- libsql
CREATE TABLE IF NOT EXISTS subagent_result_tombstones (
    child_run_id      TEXT NOT NULL,
    tenant_id         TEXT NOT NULL,
    user_id           TEXT NOT NULL,
    agent_id          TEXT,
    thread_id         TEXT NOT NULL,
    terminal_status   TEXT NOT NULL,
    disposition       TEXT NOT NULL,
    recorded_at       TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (child_run_id)
);
CREATE INDEX IF NOT EXISTS idx_tombstones_scope
    ON subagent_result_tombstones (tenant_id, user_id, agent_id, thread_id);
```

```sql
-- postgres
CREATE TABLE IF NOT EXISTS subagent_result_tombstones (
    child_run_id      TEXT NOT NULL,
    tenant_id         TEXT NOT NULL,
    user_id           TEXT NOT NULL,
    agent_id          TEXT,
    thread_id         TEXT NOT NULL,
    terminal_status   TEXT NOT NULL,
    disposition       TEXT NOT NULL,
    recorded_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (child_run_id)
);
CREATE INDEX IF NOT EXISTS idx_tombstones_scope
    ON subagent_result_tombstones (tenant_id, user_id, agent_id, thread_id);
```

Write idempotency: libSQL `INSERT OR IGNORE`, PostgreSQL `ON CONFLICT (child_run_id) DO NOTHING`.

### 3.6 Relationship to idempotency ledger + first-writer-wins correction

The tombstone store and the idempotency ledger (§5) are **distinct concerns at different granularity levels**:

| Concern | Tombstone store | Idempotency ledger |
|---|---|---|
| Key | `child_run_id` | `(run_id, child_run_id, terminal_kind)` |
| Owner | `SubagentResultTombstoneStore` trait | Separate ledger table (§5) |
| Written by | `SubagentCompletionObserver` (hot path, every child settlement) | `SubagentRestartReconciler::replay` (boot-time only) |
| Read by | `SubagentRestartReconciler` (skip re-delivery) | `SubagentRestartReconciler` (skip re-replay) |
| Semantic | "This child's terminal result was already delivered." | "This `(run_id, child_run_id, terminal_kind)` event has been fully replayed through the reconciler." |
| Idempotency level | Delivery-side | Replay-side |

**Do not unify them.** Tombstone is in the hot path on every settlement; ledger is touched only by the reconciler at boot. Merging would couple the hot path to reconciler bookkeeping.

**First-writer-wins correction:** The in-memory `BoundedSubagentResultTombstoneStore::write_tombstone` is currently last-writer-wins (calls `HashMap::insert` unconditionally). The durable `FilesystemSubagentTombstoneStore` uses `CasExpectation::Absent` (first-writer-wins). To keep contract uniform: the in-memory store must be corrected to return `Ok(())` on duplicate key without overwriting. Behavioral correction, same PR as durable wire-up. Same change cited in WU-A plan Part 1 soft corrections.

WU-C MUST add the test `write_tombstone_preserves_first_writer_when_second_write_has_different_disposition` to `crates/ironclaw_reborn/src/subagent/tombstone_store.rs` tests module. The test:
1. Writes tombstone A (`terminal_status = Cancelled`) for some `child_run_id`.
2. Writes tombstone B (`terminal_status = Completed`) for the same `child_run_id` — must return `Ok(())` (idempotent).
3. Asserts `read_tombstone` returns tombstone A (first-writer-wins, not B).

The existing `tombstone_store_is_idempotent_by_child_run` test writes identical payloads twice and therefore cannot distinguish first-writer-wins from last-writer-wins — it passes either way. The new test is required to guard the behavioral correction.

### 3.7 Risks / open questions

- **Scope on `write_tombstone` trait signature (resolved in this spec).** The current in-memory `write_tombstone(&self, tombstone: SubagentResultTombstone) -> Result<...>` signature must change to `write_tombstone(&self, scope: &TurnScope, tombstone: SubagentResultTombstone) -> Result<...>` to enable the `ScopedPath` layout in §3.4. This matches the `SubagentGoalStore::put_goal(&self, scope: &TurnScope, ...)` pattern. **WU-C MUST land the trait signature change before implementing `FilesystemSubagentTombstoneStore`.**
- **Wiring gap is total.** `BoundedSubagentResultTombstoneStore` is never passed to `SubagentCompletionObserver` or `DefaultPlannedRuntimeParts` today. WU-C must (a) add `subagent_result_tombstone_store` field to `DefaultPlannedRuntimeParts`, (b) inject into `SubagentCompletionObserver` (or a helper called from `mark_child_deliveries`), (c) add the tombstone write call in the observer after successful gate delivery.
- **`SubagentResultDisposition` variant additions.** WU-C MUST add `DiscardedParentGone` (used by §5.3 Phase 2a reconciler orphan-cleanup). WU-D will add `Delivered` and/or `SettledByBackground`. The TEXT-column / JSON-string serialization handles new variants forward-compatibly — older code that doesn't recognize a variant must round-trip the raw string, not error.
- **Eviction creates a gap.** `MAX_TOMBSTONE_RECORDS = 4096` means in-memory silently loses old tombstones under pressure. Durable store eliminates this gap by design (no eviction). The constant can be removed from the durable implementation; in-memory retains it as a safety valve for local-dev.
- **Production-readiness classification.** Once `FilesystemSubagentTombstoneStore` is wired, `subagent_result_tombstone_store` field must be set to `production_verified(Required)`. Add a symmetric positive test that the verified composition reports `RebornLoopProductionStatus::Ready`. Name the test `production_readiness_accepts_filesystem_subagent_tombstone_store`. It must assert that setting `graph.subagent_result_tombstone_store = RebornComponentReadiness::production_verified(Required)` (with all other required fields likewise verified) yields `RebornLoopProductionStatus::Ready`.

---

## Section 4 — Capability result store + `CapabilityResultStore` trait

### 4.1 Current shape — how results land today

A capability result flows through four distinct layers before it rests in memory.

**Layer 1 — `CapabilityResultWrite` assembled at the call site.**
When a capability finishes, the executor packages the result into a `CapabilityResultWrite<'_>` value (`crates/ironclaw_loop_support/src/capability_port.rs`). The struct carries: `run_context: &LoopRunContext`, `input_ref: &CapabilityInputRef`, `invocation_id: InvocationId`, `capability_id: &CapabilityId`, `output: serde_json::Value`, and `display_preview: Option<CapabilityDisplayOutputPreview>`.

**Layer 2 — `LoopCapabilityResultWriter` trait routes the write.**
The trait (also in `crates/ironclaw_loop_support/src/capability_port.rs`) declares three methods: `write_capability_result`, `update_capability_result`, `delete_capability_result`. WU-A widened `write_capability_result`'s return from `Result<LoopResultRef, AgentLoopHostError>` to `Result<(LoopResultRef, u64), AgentLoopHostError>` so the already-computed `byte_len` surfaces to the caller without re-serializing.

**Layer 3 — `ProductLiveCapabilityIo` is the production-composition impl.**
Found in `crates/ironclaw_reborn_composition/src/product_live_adapters.rs`. Its `write_capability_result` method:
1. Calls `serialized_json_len(&output, "capability result")` → `byte_len: usize`.
2. Mints a `LoopResultRef` with key `"result:{run_id}.{uuid}"` via `LoopResultRef::new(...)`.
3. Acquires the `Mutex<HashMap<String, StagedCapabilityResult>>` guard; calls `ensure_staging_capacity` (cap: 1 024 entries, 4 MiB total).
4. Inserts a `StagedCapabilityResult { run_id, output, byte_len }` keyed by the ref string.
5. Calls `self.display_previews.record_result_with_preview(...)`.
6. Returns `(result_ref, byte_len as u64)`.

**Layer 4 — `StagedCapabilityResult` lives entirely in a `Mutex<HashMap>`.**
A private struct in `product_live_adapters.rs`. Three fields: `run_id: String`, `output: serde_json::Value`, `byte_len: usize`. No persistence path. Held in `ProductLiveCapabilityIo.results`. Never written to a database. Ref strings expire when the `ProductLiveCapabilityIo` is dropped.

A second, simpler in-memory impl exists in `crates/ironclaw_reborn_composition/src/runtime/local_dev.rs` (`LocalDevCapabilityIo`). Uses a `BoundedRing` instead of a plain `HashMap`. In-process only.

### 4.2 Why a trait is needed

Plan WU-C section: "`crates/ironclaw_loop_support/src/capability_port.rs` — do NOT introduce `CapabilityResultStore` trait here (Reviewer 1 R2: loop_support is adapter glue, not persistence). Introduce in `ironclaw_reborn_event_store`."

Soundness eval: "Doc treats the durable swap as drop-in; reality requires introducing the trait first." Without a trait, durable swap would require conditional compilation or hard-coded `if local_dev { HashMap } else { SQL }` branches scattered through `product_live_adapters.rs`. A trait gives:

- Single injection point: `ProductLiveCapabilityIo` (and `LocalDevCapabilityIo`) each take an `Arc<dyn CapabilityResultStore>` at construction.
- Swappable backends: in-memory for `local_dev`, libSQL or PostgreSQL typed-repo for production.
- Testability: mock impls in executor tests return canned `LoopResultRef`s without touching a database.
- `SubagentRestartReconciler` replay + `PostCapabilityStage::drain_settled` paths both need a read interface — they cannot call `ProductLiveCapabilityIo.results.lock()` directly (different crates, no private-field visibility).
- Parent-resume after restart requires hydrating `LoopResultRef`s from a durable store; in-memory `HashMap` is empty after a process restart.

`CapabilityResultStore` does not exist today. Confirmed via codegraph: zero results for `CapabilityResultStore` in the indexed tree.

### 4.3 Trait shape

```rust
use async_trait::async_trait;
use ironclaw_turns::{TurnRunId, TurnScope};
use ironclaw_turns::run_profile::host::LoopResultRef;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CapabilityResultStoreError {
    #[error("capability result backend unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("capability result deserialization failed: {reason}")]
    Deserialization { reason: String },
    #[error("capability result ref is not valid: {reason}")]
    InvalidRef { reason: String },
    #[error("capability result store capacity exceeded: {reason}")]
    CapacityExceeded { reason: String },
    #[error("capability result store I/O error: {reason}")]
    Io { reason: String },
}

#[async_trait]
pub trait CapabilityResultStore: Send + Sync {
    /// Persist pre-serialized capability result bytes.
    ///
    /// Caller MUST pass JSON bytes (typically from `serde_json::to_vec(&value)`).
    /// Returns `(result_ref, byte_len)`. `byte_len` is `payload.len() as u64` —
    /// no second serialization happens.
    ///
    /// Idempotent under retries keyed on (scope, run_id, capability_id) within
    /// the same invocation.
    async fn write(
        &self,
        scope: &TurnScope,
        run_id: &TurnRunId,
        capability_id: &str,
        payload: Vec<u8>,
    ) -> Result<(LoopResultRef, u64), CapabilityResultStoreError>;

    /// Fetch previously written payload bytes by opaque ref. Returns None if
    /// the ref does not exist (tombstoned or GC'd). Caller deserializes
    /// lazily — store does not parse JSON on the read path.
    async fn read(
        &self,
        scope: &TurnScope,
        result_ref: &LoopResultRef,
    ) -> Result<Option<Vec<u8>>, CapabilityResultStoreError>;

    /// All result refs written for a given `run_id`, ordered by insertion
    /// time ascending. Used by `SubagentRestartReconciler` (replay) and
    /// `PostCapabilityStage::drain_settled`.
    async fn list_by_run(
        &self,
        scope: &TurnScope,
        run_id: &TurnRunId,
    ) -> Result<Vec<LoopResultRef>, CapabilityResultStoreError>;

    /// Mark a result as deleted. Soft-delete via `tombstoned_at` column to
    /// preserve "LLM data is never deleted" invariant. Hard-deletes reserved
    /// for explicit GC. Idempotent.
    async fn tombstone(
        &self,
        scope: &TurnScope,
        result_ref: &LoopResultRef,
    ) -> Result<(), CapabilityResultStoreError>;
}
```

All methods async. `thiserror` error type with distinct variants for each failure class so callers can pattern-match without string parsing. `list_by_run` is required — both `SubagentRestartReconciler` and `PostCapabilityStage::drain_settled` need to enumerate all results for a given run without knowing the individual ref strings. `tombstone` soft-deletes consistent with project-wide "LLM data is never deleted" invariant.

**Why `Vec<u8>` not `serde_json::Value`.** At multi-tenant scale with megabyte-scale capability payloads (HTML extraction, API responses), passing `Value` forces (a) the caller to serialize for byte counting, (b) a Value clone to pass ownership to the store, (c) the store to serialize again for INSERT — two full serializations + one tree clone per call. With `Vec<u8>`: executor calls `serde_json::to_vec(&output)` once, `byte_len = bytes.len() as u64` derived for free, `bytes` is moved into the store (no clone), store INSERTs the bytes directly into the BLOB/JSONB column. Single serialization, single allocation, zero clones. The trait shape reflects what actually crosses the boundary — bytes, not a tree. On the `read` path: store returns the BLOB/JSONB bytes directly; caller deserializes via `serde_json::from_slice(&bytes)?` lazily, only when the Value is actually needed (e.g., for prompt assembly or compaction). The error variant `Deserialization` covers caller-side parsing failures from the read path; the store itself never parses JSON.

### 4.4 Crate placement — `ironclaw_reborn_event_store`

**Owner:** `crates/ironclaw_reborn_event_store/src/capability_result_store.rs` (new file), trait + error type exported from the crate's `lib.rs`.

Rationale:

1. **Not `ironclaw_loop_support`.** "loop_support is adapter glue, not persistence" (Reviewer 1 R2). `LoopCapabilityResultWriter` there is a routing trait — it mediates the call from executor to whatever destination is wired. Adding persistence ownership conflates routing and storage. Boundary-test rule separates these.
2. **Not a new `ironclaw_reborn_persistence` crate.** Reviewer 1 V1 + Reviewer 4 G2 ruled this out: requires a `BoundaryRule` entry, contradicts `database.md` direction. No new crate without a boundary rule.
3. **`ironclaw_reborn_event_store` is the canonical Reborn durable backend selection point.** Already owns `DurableEventLog`, `DurableAuditLog`. Already has `InMemory`, `Jsonl`, `Postgres`, `Libsql` config variants in `RebornEventStoreConfig`. Existing boundary rule covers it. `build_reborn_event_stores` factory matches the pattern.
4. **The existing boundary rule covers it** — no new `BoundaryRule` entry needed.
5. **`ironclaw_reborn_composition` remains the wiring layer.** `product_live_adapters.rs` imports `CapabilityResultStore` from `ironclaw_reborn_event_store` and passes the concrete impl into `ProductLiveCapabilityIo::new`. Composition already depends on `ironclaw_reborn_event_store`.

### 4.5 Backend choice + rationale

**Choice: typed-repo (libSQL + PostgreSQL), option (b) per `_contract-freeze-index.md` §2.**

Capability results are the wrong shape for `ScopedFilesystem`:

- **Write rate.** Every capability call (HTTP, web_fetch, spawn_subagent, tool dispatch) produces one write. Background fan-out of 16 subagents × 4 capabilities = 64 writes in seconds. `ScopedFilesystem` serializes through a single async I/O path with no indexes; scan-by-run requires a directory listing + per-file open.
- **Large payloads.** Capability results can be megabyte-scale JSON (HTML extraction, API response bodies). `ScopedFilesystem` deserializes the full file to return payload. JSONB column allows partial reads via column projection.
- **Query-by-run is structural, not file-shaped.** `list_by_run` needs a `WHERE run_id = $1 ORDER BY created_at` scan with an index. No equivalent in `ScopedFilesystem` without a separate index file (its own CAS logic; hot spot).
- **Atomic tombstone.** Setting `tombstoned_at` while reading `byte_len` is a single `UPDATE ... WHERE result_ref = $1` in SQL.
- **`ironclaw_reborn_event_store` already has libSQL and PostgreSQL typed-repo backends** for `DurableEventLog`. The `capability_results` table follows the same module shape: `crates/ironclaw_reborn_event_store/src/libsql/capability_result_repo.rs` and `.../postgres/capability_result_repo.rs`. No new dependency; existing feature flags gate respective backends.

**In-memory impl** (`InMemoryCapabilityResultStore`) retained as `local_dev` fallback — same role as `InMemoryDurableEventLog`. Wraps `Mutex<HashMap<String, Vec<u8>>>` (no `BoundedRing` — production-readiness check gates its use to `LocalDevTest` mode).

**Implementation note.** Both libSQL and PostgreSQL backends use a single statement: `INSERT INTO capability_results (..., payload, byte_len) VALUES (..., ?, ?)` with `byte_len = payload.len() as u64`. No `serde_json` call inside the backend. PostgreSQL's JSONB column accepts a `bytea`/`Vec<u8>` parameter via `payload::jsonb` cast: the Postgres driver converts the bytes to JSONB representation server-side. libSQL stores raw bytes in BLOB. The in-memory `InMemoryCapabilityResultStore` holds `Mutex<HashMap<String, Vec<u8>>>` — keys are the opaque ref strings, values are the serialized bytes. Round-trip parity is byte-exact across all three impls: bytes in == bytes out.

### 4.6 libSQL schema

```sql
CREATE TABLE IF NOT EXISTS capability_results (
    tenant_id     TEXT NOT NULL,
    user_id       TEXT NOT NULL,
    agent_id      TEXT,                         -- nullable: non-agent runs
    run_id        TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    result_ref    TEXT NOT NULL PRIMARY KEY,     -- opaque ref minted by write()
    byte_len      INTEGER NOT NULL,             -- serialized byte count
    payload       BLOB NOT NULL                 -- raw JSON bytes (Vec<u8> from serde_json::to_vec)
        CHECK (length(payload) <= 8388608),     -- 8 MiB hard cap
    tombstoned_at TEXT,                         -- ISO-8601; NULL = live
    created_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_capability_results_run
    ON capability_results (tenant_id, user_id, run_id, created_at);
CREATE INDEX IF NOT EXISTS idx_capability_results_cap
    ON capability_results (tenant_id, user_id, agent_id, capability_id);
```

`payload` is `BLOB` (libSQL has no native JSONB). Application deserializes with `serde_json::from_slice`. `tombstoned_at` nullable TEXT for causal ordering on GC audits.

Write idempotency: `INSERT OR IGNORE` keyed on `result_ref`. Since the ref is UUID-qualified, collisions only arise from retried writes of the exact same call → first-writer-wins matching in-memory `HashMap::insert`.

### 4.7 PostgreSQL schema

```sql
CREATE TABLE IF NOT EXISTS capability_results (
    tenant_id     TEXT        NOT NULL,
    user_id       TEXT        NOT NULL,
    agent_id      TEXT,
    run_id        TEXT        NOT NULL,
    capability_id TEXT        NOT NULL,
    result_ref    TEXT        NOT NULL PRIMARY KEY,
    byte_len      BIGINT      NOT NULL,
    payload       JSONB       NOT NULL          -- written from Vec<u8> via JSONB cast; bytes stored canonically
        CHECK (octet_length(payload::text) <= 8388608),  -- 8 MiB hard cap
    tombstoned_at TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_capability_results_run
    ON capability_results (tenant_id, user_id, run_id, created_at);
CREATE INDEX IF NOT EXISTS idx_capability_results_cap
    ON capability_results (tenant_id, user_id, agent_id, capability_id);
```

PostgreSQL uses `JSONB` (supports `@>` containment + future partial reads), `BIGINT` for `byte_len` (Rust `u64` via `i64` cast with range check at boundary), `TIMESTAMPTZ` for time columns. Idempotency: `INSERT INTO ... ON CONFLICT (result_ref) DO NOTHING`.

Both schemas match `_contract-freeze-index.md` §2: every table carries `(tenant_id, user_id, agent_id)`; run-scan index leads with `(tenant_id, user_id, run_id)` to prevent cross-tenant scan leakage.

### 4.8 Wire-up plan

**`ProductLiveCapabilityIo` becomes backend-agnostic.**

```rust
pub struct ProductLiveCapabilityIo {
    inputs: Mutex<HashMap<String, StagedCapabilityInput>>,
    result_store: Arc<dyn CapabilityResultStore>,       // NEW — replaces `results` HashMap
    display_previews: Arc<CapabilityDisplayPreviewStore>,
}
```

`write_capability_result`:

```rust
async fn write_capability_result(
    &self,
    write: CapabilityResultWrite<'_>,
) -> Result<(LoopResultRef, u64), AgentLoopHostError> {
    // Single serialization pass. byte_len derived for free from the bytes.
    let bytes = serde_json::to_vec(&write.output)
        .map_err(|e| AgentLoopHostError::serialization("capability result", e))?;

    let (result_ref, byte_len) = self
        .result_store
        .write(
            &scope_from_run_context(write.run_context),
            &write.run_context.run_id.into(),
            write.capability_id.as_str(),
            bytes,
        )
        .await
        .map_err(map_store_error)?;

    self.display_previews.record_result_with_preview(...);

    Ok((result_ref, byte_len))
}
```

**`scope_from_run_context` helper.** Defined in `crates/ironclaw_reborn_composition`. Maps `LoopRunContext` → `TurnScope`. Field mapping:

- `TurnScope.tenant_id`        ← `run_context.tenant_id`
- `TurnScope.agent_id`         ← `run_context.scope.agent_id` (Option<AgentId>)
- `TurnScope.project_id`       ← `run_context.scope.project_id`
- `TurnScope.thread_id`        ← `run_context.thread_id`
- `TurnScope.thread_owner`     ← `run_context.thread_owner`

`user_id` is NOT a field on `TurnScope` directly — it is resolved at query-bind time via `TurnScope::explicit_owner_user_id()`, falling back to the `SYSTEM_RESERVED_ID` sentinel for `Ownerless` / `ActorFallback` runs (see §8.4). The helper MUST NOT drop or substitute `agent_id` — bypass of this rule reintroduces the cross-agent IDOR class.

**Why this shape.** The executor serializes `write.output` exactly once via `serde_json::to_vec`. The resulting bytes are moved (not cloned) into the store, which INSERTs them directly into the BLOB/JSONB column. `byte_len` is `bytes.len() as u64` — derived from the same bytes, no extra work. Result: one serialization, one allocation, zero tree-walks per capability call. At the scale of 100s of calls per second per node with megabyte-scale payloads, this saves ~50% of capability-write CPU vs the prior double-serialize approach.

**`LocalDevCapabilityIo`** also switches to an injected store with `InMemoryCapabilityResultStore` supplied at construction. The `LocalDevCapabilityIo.results` `BoundedRing` field is removed. Unifies local-dev and production paths.

**`local_dev.rs` construction** wires `InMemoryCapabilityResultStore`:

```rust
let result_store = Arc::new(InMemoryCapabilityResultStore::default());
let capability_io = LocalDevCapabilityIo::new(result_store, display_previews);
```

**Production composition** selects backend via `RebornEventStoreConfig`:

```rust
pub async fn build_reborn_capability_result_store(
    profile: RebornProfile,
    config: &RebornEventStoreConfig,
) -> Result<Arc<dyn CapabilityResultStore>, RebornEventStoreError> {
    match config {
        RebornEventStoreConfig::InMemory => {
            if profile == RebornProfile::Production {
                return Err(RebornEventStoreError::ProductionInMemoryDisabled);
            }
            Ok(Arc::new(InMemoryCapabilityResultStore::default()))
        }
        RebornEventStoreConfig::Libsql { path_or_url, auth_token } => {
            // libsql_backed::build_capability_store(...)
        }
        RebornEventStoreConfig::Postgres { url } => {
            // postgres_backed::build_capability_store(...)
        }
        RebornEventStoreConfig::Jsonl { .. } => {
            // Jsonl is file-event-log only; capability results fall back to
            // InMemory with a non_durable readiness flag.
            Ok(Arc::new(InMemoryCapabilityResultStore::default()))
        }
    }
}
```

**`production_readiness.rs` wire-up** mirrors the existing `subagent_result_tombstone_store` field pattern. Add `capability_result_store: RebornComponentReadiness` to `RebornLoopComponentGraphReadiness`. Production: `production_verified(Required)`. Local-dev: `non_durable(Required)` → yields `LocalDevDegraded` (warning, not blocker) in `LocalDevTest` mode.

Existing `production_readiness_rejects_in_memory_checkpoint_store` test in `crates/ironclaw_reborn/tests/production_readiness.rs` is the template for a new `production_readiness_rejects_in_memory_capability_result_store` test. Add the symmetric positive test `production_readiness_accepts_production_verified_capability_result_store` asserting that `graph.capability_result_store = RebornComponentReadiness::production_verified(Required)` yields `RebornLoopProductionStatus::Ready`.

`SubagentRestartReconciler` field (`subagent_restart_reconciler`) is already declared. WU-C flips its `RebornComponentRequirement` from `Optional` to `Required` in the production-verified constructor once a concrete impl exists.

### 4.9 Risks / open questions

- **Payload size cap (MUST).** Per-result cap is **8 MiB** enforced at the SQL CHECK constraint AND at the application layer before INSERT. Implementations MUST surface `CapabilityResultStoreError::CapacityExceeded` when a write would exceed the cap. The cross-result aggregate limit that the in-memory impl carried (`ensure_staging_capacity`) is removed — backpressure for total storage growth is owned by `PostCapabilityStage` compaction, not the result store.
- **Serialization discipline (D8-A).** The trait MUST take `Vec<u8>`. Implementations MUST NOT accept `serde_json::Value` and serialize internally — that re-introduces the double-serialization regression this fix addresses. The `read` path returns `Vec<u8>` for the same reason: callers deserialize lazily, only when a `Value` is actually needed. Backend implementations parse JSON ONLY on integrity-check paths (e.g., a startup self-test) and NEVER on the hot read/write paths. Future streaming variants (e.g. an `async-trait` returning a `BoxStream<Item = Bytes>`) compose cleanly with this byte-oriented trait shape; a Value-based trait would block that evolution.
- **GC policy.** Tombstoned rows accumulate indefinitely unless GC runs. Background GC outside WU-C scope — delete rows where `tombstoned_at < NOW() - interval '7 days'` (or configurable). Until GC lands, disk usage grows proportionally to run volume.
- **Backward-compat for in-flight refs at deploy.** Active runs have refs in old in-memory `HashMap` inside running process. On process restart those refs are lost. Plan mitigation: "accept loss — feature toggle gates user impact." Background mode defaults `false` through WU-G; no parent loop is actively draining background results in production at deploy time. Blocking capability results are consumed before executor returns to loop, so never re-read after restart. The only at-risk refs are between capability call finish and turn transcript commit — milliseconds window.
- **Ref durability vs. ref opacity.** `LoopResultRef` is opaque per `ironclaw_turns/CLAUDE.md`. Durable store keyed on `result_ref TEXT` preserves opacity — store never interprets ref's internal structure. Format `"result:{run_id}.{uuid}"` is sufficient as a unique store key; no schema migration needed when format changes.
- **Dual-backend parity.** Covered in §7.

---

## Section 5 — `SubagentRestartReconciler` + idempotency ledger

### 5.1 Current state

`SubagentRestartReconciler` exists today solely as a variant of `RebornLoopProductionComponent` in `crates/ironclaw_reborn/src/production_readiness.rs`:

```rust
pub enum RebornLoopProductionComponent {
    // ...
    SubagentRestartReconciler,
    // ...
}
```

Wired into the readiness check via `RebornLoopComponentGraphReadiness.subagent_restart_reconciler: RebornComponentReadiness`. The `production_verified()` constructor already declares this field as `RebornComponentReadiness::production_verified(required)` — meaning in production mode the check fails closed the moment it sees a non-`ProductionVerified` safety class. No trait, no concrete implementation, no boot-replay logic exists anywhere.

No analogous boot-replay code exists elsewhere in the Reborn tree. Closest existing precedent: `IdempotencyLedger::begin_or_replay` in `crates/ironclaw_product_workflow/src/ledger.rs` — handles inbound-message deduplication at the workflow boundary (backed by `InMemoryIdempotencyLedger`, `FilesystemIdempotencyLedger`, and three concrete impls `RebornFilesystemIdempotencyLedger`, `RebornLibSqlIdempotencyLedger`, `RebornPostgresIdempotencyLedger` in `crates/ironclaw_product_workflow_storage`). Drives from `ActionFingerprintKey`, returns `IdempotencyDecision::Replay` carrying the prior settled `ProductInboundAction`. Subagent restart reconciler is structurally analogous but operates on capability result store + settlement event log, not product workflow inbound actions.

Second precedent: `BoundedSubagentResultTombstoneStore` (§3). In-memory only, bounded 4096, evicting by insertion order. `write_tombstone`/`read_tombstone` methods keyed by `child_run_id: TurnRunId`. When the durable backend is introduced the tombstone store is one of four stores the reconciler must consult to avoid replaying results that were already discarded before the crash.

### 5.2 Reconciler trait shape

Trait + associated types live in `crates/ironclaw_reborn_event_store` (canonical owner per `events.md` §2 + existing `BoundaryRule`).

```rust
// crates/ironclaw_reborn_event_store/src/reconciler.rs

use async_trait::async_trait;
use ironclaw_turns::TurnScope;

/// One call per boot: re-deliver any settled background-subagent results that
/// were written to the durable settlement log before the crash but never
/// acknowledged by the parent's mailbox / gate store.
#[async_trait]
pub trait SubagentRestartReconciler: Send + Sync {
    async fn replay(
        &self,
        scope: &TurnScope,
    ) -> Result<ReplayReport, ReconcilerError>;
}

/// Summary returned by a completed replay pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayReport {
    /// Results successfully re-delivered to the parent gate store this pass.
    pub redelivered: u32,
    /// Results skipped because an idempotency ledger row was already sealed
    /// (delivered_at IS NOT NULL) by a previous pass or another node.
    pub skipped_idempotent: u32,
    /// Pencil-receipt rows found (delivered_at IS NULL) from a previous
    /// reconciler that crashed between ledger insert and gate-store write.
    /// The current pass re-attempts delivery for these.
    pub retryable: u32,
    /// Settlement-log entries whose gate was cleaned up before delivery
    /// completed (parent cancelled, gate row removed). The reconciler
    /// tombstones the child and seals the ledger row — no further replay
    /// will attempt redelivery for these entries.
    pub skipped_orphan: u32,
    /// Real delivery failures (backend error, missing capability result,
    /// tombstoned-result race). Each failure is logged at `warn!` level
    /// and the reconciler continues. `failed > 0` is operator-actionable.
    pub failed: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum ReconcilerError {
    #[error("settlement event log read failed: {reason}")]
    SettlementLogRead { reason: String },
    #[error("idempotency ledger read failed: {reason}")]
    LedgerRead { reason: String },
    #[error("reconciler backend unavailable: {reason}")]
    Backend { reason: String },
}
```

`TurnScope` (from `crates/ironclaw_turns/src/scope.rs`) carries `tenant_id`, `agent_id`, `project_id`, `thread_id` — already threaded through every Reborn runtime call site. Using it directly avoids introducing a new `Scope` wrapper.

### 5.3 Replay algorithm

```
fn replay(scope: &TurnScope) -> ReplayReport:
  // ── Phase 0 — input bounding (one query) ──
  // Read ONLY settlement-log rows whose ledger row is missing or pencil
  // (NULL delivered_at). Bounded by outstanding work, not historical log size.
  // PostgreSQL example:
  //   SELECT s.* FROM subagent_gate_settlement_log s
  //     LEFT JOIN subagent_idempotency_ledger l
  //       ON  s.parent_run_id = l.run_id
  //       AND s.child_run_id  = l.child_run_id
  //       AND s.terminal_kind = l.terminal_kind
  //   WHERE s.tenant_id = $1 AND s.user_id = $2
  //     AND s.terminal_kind IN ('Completed', 'Failed', 'Cancelled')
  //     AND s.child_run_id IS NOT NULL
  //     AND (l.delivered_at IS NULL OR l.run_id IS NULL);
  pending = settlement_event_log.read_pending_for_scope(scope)

  if pending is empty:
    return ReplayReport::zero()

  redelivered = 0; skipped_idempotent = 0
  retryable = 0; skipped_orphan = 0; failed = 0

  // ── Phase 1 — preflight (two batched reads) ──
  // Partition `pending` rows into live / orphan / explicit-tombstoned.
  live_gate_refs = gate_store.gates_exist_batch(
                     scope,
                     pending.iter().map(|r| r.gate_ref).collect())
  // Returns Set<GateRef> of refs that still exist.

  tombstoned_child_ids = tombstone_store.read_tombstones_batch(
                          scope,
                          pending.iter().map(|r| r.child_run_id).collect())
  // Returns Set<TurnRunId> of children with explicit tombstones.

  (live_rows, orphan_rows) = pending.partition(|r| live_gate_refs.contains(&r.gate_ref))
  (live_rows, tombstoned_rows) = live_rows.partition(
                                   |r| !tombstoned_child_ids.contains(&r.child_run_id))

  // ── Phase 2a — orphan + tombstoned cleanup (batched) ──
  // Build all tombstones for orphan rows, write in ONE batched call.
  // The trait MUST provide write_tombstones_batch; per-row write_tombstone
  // calls in a for-loop reintroduce N+1 latency in the recovery hot path.
  orphan_tombstones = orphan_rows.iter().map(|row|
    SubagentResultTombstone {
      child_run_id: row.child_run_id,
      terminal_status: row.terminal_status,
      disposition: SubagentResultDisposition::DiscardedParentGone,
    }
  ).collect()
  tombstone_store.write_tombstones_batch(scope, orphan_tombstones)
  // PostgreSQL multi-row upsert (single round-trip):
  //   INSERT INTO subagent_idempotency_ledger
  //     (tenant_id, user_id, agent_id, run_id, child_run_id, terminal_kind,
  //      delivery_node, delivered_at)
  //   VALUES (?,?,?,?,?,?,?,NOW()), …
  //   ON CONFLICT (run_id, child_run_id, terminal_kind) DO UPDATE
  //     SET delivered_at = COALESCE(subagent_idempotency_ledger.delivered_at, NOW());
  idempotency_ledger.upsert_sealed_batch(
    scope, orphan_rows ++ tombstoned_rows, delivery_node=self.node_id)
  skipped_orphan = orphan_rows.len() + tombstoned_rows.len()

  // ── Phase 2b — pencil claim (one multi-row write) ──
  // INSERT OR IGNORE / ON CONFLICT DO NOTHING. Leaves delivered_at NULL.
  inserted_keys = idempotency_ledger.insert_pencil_batch(
                    scope, live_rows, delivery_node=self.node_id)
  // Returns Set<LedgerKey> of rows actually inserted (not pre-existing).

  // Partition: freshly-claimed vs pre-existing ledger row.
  (freshly_claimed, pre_existing) = live_rows.partition(
                                     |r| inserted_keys.contains(&r.key()))

  // For pre-existing rows, read the ledger to see if sealed or pencil.
  // One batched SELECT keyed on the pre_existing row keys.
  pre_existing_states = idempotency_ledger.read_batch(
                          scope, pre_existing.iter().map(|r| r.key()))

  to_attempt = freshly_claimed
  for (row, state) in pre_existing.zip(pre_existing_states):
    if state.delivered_at is Some:
      skipped_idempotent += 1   // already sealed
    else:
      retryable += 1            // pencil from prior crash
      to_attempt.push(row)

  // ── Phase 3 — parallel capability loads (bounded by replay_pool_size) ──
  // Use buffer_unordered to cap concurrent in-flight loads at the durable
  // backend's connection-pool size. Megabyte-scale payloads × thousands of
  // pending rows over a 4-conn pool would otherwise starve foreground
  // writes. Implementations MUST use buffer_unordered, NOT join_all.
  load_results = futures::stream::iter(
                   to_attempt.iter().map(|r|
                     capability_result_store.load(scope, &r.result_ref)
                   )
                 )
                 .buffer_unordered(replay_pool_size)
                 .collect::<Vec<_>>()
                 .await

  // ── Phase 4 — per-row deliver + seal (sequential per row, parallel-OK) ──
  // Per-row because each row delivers to a different parent's mailbox.
  // The seal UPDATE is the single point of truth for "is this final?"
  for (row, load_result) in to_attempt.zip(load_results):
    match load_result:
      Err(_) | Ok(None) =>
        debug!("reconciler: capability result missing for child_run_id={}",
               row.child_run_id)
        failed += 1
        // Leave pencil receipt; next boot will retry.
        continue
      Ok(Some(payload)) =>
        match gate_store.record_background_settlement(
                scope, row.parent_run_id, row.child_run_id, payload):
          Ok(_) =>
            // SEAL — single-row UPDATE keyed on the ledger PK.
            // Sets delivered_at = NOW() WHERE delivered_at IS NULL.
            idempotency_ledger.seal(scope, row.key())
            redelivered += 1
          Err(e) =>
            warn!("reconciler: gate-store re-delivery failed: {e}")
            failed += 1
            // Leave pencil receipt; next boot will retry.

  return ReplayReport { redelivered, skipped_idempotent, retryable,
                        skipped_orphan, failed }
```

**Performance shape (D4).** The replay algorithm is phase-batched: each phase issues O(1) DB calls regardless of `len(pending)`. Phase 0 bounds input to outstanding work via a LEFT JOIN against the ledger — historical settled log rows never enter the algorithm. Phase 1 batches both preflight reads. Phase 2 batches both ledger writes (orphan-seal + pencil-claim). Phase 3 parallelizes capability loads via `join_all`. Only Phase 4 (deliver + seal) is per-row, and only because each row's gate-store target differs — within Phase 4 the work can be further sharded across a `tokio::JoinSet` if profiling demands it. Net cost is dominated by Phase 4's per-row delivery (~5–30 ms per row depending on backend latency) rather than the historical N+1 round-trip cost. Implementations MUST cap Phase 3 concurrency at the durable backend's connection-pool size; running `join_all` over thousands of futures with a 4-connection pool will starve foreground writes. See §5.6 for the dedicated replay pool guidance. Phase 2a's tombstone writes MUST use `write_tombstones_batch` — a single round-trip for all orphans — not a per-row `write_tombstone` loop. The trait `SubagentResultTombstoneStore` is extended in WU-C with a `write_tombstones_batch(&self, scope: &TurnScope, tombstones: Vec<SubagentResultTombstone>) -> Result<(), TombstoneStoreError>` method.

**Concurrency safety.** Two-phase ledger semantics from D1 hold under batching: Phase 2b's multi-row `INSERT OR IGNORE` is row-level idempotent — racing nodes either insert a pencil row or observe an existing one; only one node's seal UPDATE will succeed (the `delivered_at IS NULL` guard arbitrates). The gate-store write in Phase 4 is independently idempotent on its own primary key (per §1). Together: at most one delivery per `(run_id, child_run_id, terminal_kind)` tuple, regardless of replica count, crash count, or fan-out scale.

### 5.4 Idempotency ledger schema (libSQL)

```sql
-- crates/ironclaw_reborn_event_store/src/libsql/migrations/
-- 0005_subagent_idempotency_ledger.sql

CREATE TABLE IF NOT EXISTS subagent_idempotency_ledger (
    tenant_id          TEXT NOT NULL,
    user_id            TEXT NOT NULL,
    agent_id           TEXT,                       -- NULL for non-agent runs
    run_id             TEXT NOT NULL,              -- parent run (UUID string)
    child_run_id       TEXT NOT NULL,              -- child TurnRunId (UUID string)
    terminal_kind      TEXT NOT NULL,              -- "completed" | "failed" | "cancelled"
    delivered_at       TEXT,                       -- ISO-8601 UTC; NULL = pencil receipt (mid-flight, retryable on next boot)
    delivery_node      TEXT NOT NULL,              -- ops debug: hostname or pod identity
    UNIQUE (run_id, child_run_id, terminal_kind)
);

CREATE INDEX IF NOT EXISTS idx_sil_scope
    ON subagent_idempotency_ledger (tenant_id, user_id, agent_id, run_id);
```

Insert:

```sql
-- Step 1: pencil-receipt insert (claim ownership; mid-flight marker).
INSERT OR IGNORE INTO subagent_idempotency_ledger
    (tenant_id, user_id, agent_id, run_id, child_run_id, terminal_kind,
     delivery_node)
VALUES (?, ?, ?, ?, ?, ?, ?);
-- Inspect changes() == 0 to detect the "already claimed by another node" case.

-- Step 2: pencil-receipt seal (sets delivered_at) (after successful gate-store write).
UPDATE subagent_idempotency_ledger
   SET delivered_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
 WHERE tenant_id = ? AND run_id = ? AND child_run_id = ? AND terminal_kind = ?
   AND delivered_at IS NULL;
```

### 5.5 Idempotency ledger schema (PostgreSQL)

```sql
-- crates/ironclaw_reborn_event_store/src/postgres/migrations/
-- 0005_subagent_idempotency_ledger.sql

CREATE TABLE IF NOT EXISTS subagent_idempotency_ledger (
    tenant_id          TEXT NOT NULL,
    user_id            TEXT NOT NULL,
    agent_id           TEXT,
    run_id             TEXT NOT NULL,
    child_run_id       TEXT NOT NULL,
    terminal_kind      TEXT NOT NULL,
    delivered_at       TIMESTAMPTZ,                -- NULL = pencil receipt (mid-flight, retryable on next boot)
    delivery_node      TEXT NOT NULL,
    UNIQUE (run_id, child_run_id, terminal_kind)
);

CREATE INDEX IF NOT EXISTS idx_sil_scope
    ON subagent_idempotency_ledger (tenant_id, user_id, agent_id, run_id);
```

Insert:

```sql
-- Step 1: pencil-receipt insert (claim ownership; mid-flight marker).
INSERT INTO subagent_idempotency_ledger
    (tenant_id, user_id, agent_id, run_id, child_run_id, terminal_kind,
     delivery_node)
VALUES ($1, $2, $3, $4, $5, $6, $7)
ON CONFLICT (run_id, child_run_id, terminal_kind) DO NOTHING;
-- Inspect rows_affected() == 0 to detect the "already claimed by another node" case.

-- Step 2: pencil-receipt seal (sets delivered_at) (after successful gate-store write).
UPDATE subagent_idempotency_ledger
   SET delivered_at = NOW()
 WHERE tenant_id = $1 AND run_id = $2 AND child_run_id = $3 AND terminal_kind = $4
   AND delivered_at IS NULL;
```

Both dialects match the in-memory settlement semantics already established in `gate_resolution.rs` where `mark_child_delivered` skips re-recording an already-delivered child (first-writer-wins).

**`delivery_node` contract.** The column records the process / node identity that performed the redelivery — operator debugging only, never load-bearing. Validation MUST happen at the write site (`SubagentRestartReconciler` impl):
- Source: a deployment-supplied configuration value (env var, config file). MUST NOT be sourced from any user-supplied or network-supplied input.
- Max length: 128 characters.
- Allowlist: `[A-Za-z0-9._-]+`. Reject any other character (or replace with `_`) — the column is read by ops dashboards that may interpret control bytes.
- On invalid: substitute the literal string `"unknown"` and log a `warn!` line. Never crash the reconciler over a delivery-node validation failure.

### 5.6 Composition wire-up

Reconciler runs once per process boot — **detached in a background task**, not blocking foreground traffic. Boot sequence in `crates/ironclaw_reborn_composition/src/runtime/local_dev.rs` (local dev) and the production counterpart in `crates/ironclaw_reborn_composition/src/lib.rs`:

1. `build_reborn_event_stores` constructs the durable backends and returns the `SubagentIdempotencyLedger` instance alongside `RebornEventStores`.
2. Composition layer creates a concrete `DurableSubagentRestartReconciler` holding references to: settlement event log (scoped reader), `CapabilityResultStore`, `SubagentResultTombstoneStore`, idempotency ledger, gate store. The reconciler is given its own **dedicated DB connection pool** (`replay_pool`, default 4 connections) — separate from the main runtime pool so replay never starves foreground writes during a recovery storm.
3. **Active-scope enumeration (eager, bounded).** Composition queries the runs table for scopes with non-terminal runs at boot time. This is bounded by active-runs count, not historical user count — typical: low thousands. The enumerated set is `active_scopes: Vec<TurnScope>`. Lazy per-scope replay on first incoming traffic is a future optimization (deferred — would add cold-foreground latency for first-touch tenants after a restart).
4. **Background replay dispatch.** Composition stores an `Arc<ReplayState>` keyed by `TurnScope` (or `(tenant_id, agent_id)`), each tracking `{ in_progress: bool, completed_at: Option<Instant>, last_report: Option<ReplayReport> }`. Composition then `tokio::spawn`s one replay task per active scope (or one task that walks `active_scopes` sequentially with `replay_pool` capping concurrency — implementation choice).

   **Reconciler replay jitter (A.A).** To prevent fleet-wide replay stampedes at rolling-deploy time, each replica waits a uniform-random delay `0..RECONCILER_REPLAY_JITTER_MS` (default 5000 ms; configurable via `RebornEventStoreConfig.reconciler_replay_jitter_ms`) before launching its first replay task. At a 50-replica deploy that completes within a 30-second window, the jitter spreads ~200 concurrent reconciler connections (50 × `replay_pool=4`) over a wider time band, capping instantaneous DB load at ~40 concurrent reconciler conns instead. Foreground latency stays within SLA during the deploy window. Jitter is taken ONCE per process boot — not per-scope — so a single replica's scopes still run sequentially in arrival order under the per-scope admission gate. Jitter does NOT block foreground or blocking-subagent traffic; only the background replay task pays the jitter cost. Set `reconciler_replay_jitter_ms = 0` for single-node deployments where stampede protection is unnecessary.

5. **Admission gate (background mode only).** The `SpawnSubagentPort` for the new background mode (WU-D) consults `ReplayState[scope].completed_at` before admitting a new background spawn:
   - If `completed_at.is_some()` → admit immediately.
   - If `completed_at.is_none()` → reject with a structured `SubagentSpawnError::ReplayInProgress { try_again_after_ms }` so the parent loop's retry logic can re-attempt cleanly.
   - Foreground requests, blocking subagent calls, and all non-background-spawn paths are NEVER gated by replay completion.
6. `RebornLoopComponentGraphReadiness.subagent_restart_reconciler` is set to `RebornComponentReadiness::production_verified(Required)` when durable; `non_durable(required)` for in-memory local-dev. Production fails closed on non-`ProductionVerified`.

**Why background, not sync.** Foreground latency is the primary user-facing SLA. Synchronous boot-block (the previous spec version) made cold start scale with `O(scopes × pending_rows × per-row-cost)` — pathological in multi-tenant deployments. The background model preserves <100 ms foreground cold start regardless of replay backlog. Background-mode admission delay is acceptable because (a) background spawns are by definition not latency-critical, (b) the toggle defaults OFF through WU-G, (c) per-scope gating means tenant A's replay never blocks tenant B's background admissions.

**Why dedicated `replay_pool`.** A 10-second replay over 10k rows would otherwise compete with foreground writes for the main connection pool. The dedicated pool (default 4 connections) caps replay's DB footprint and makes its load operationally observable as a separate metric. Configurable via `RebornEventStoreConfig.replay_pool_size`.

In-memory local-dev stub wires a `NoopSubagentRestartReconciler` returning `ReplayReport::zero()` immediately — no-op, consistent with other non-durable local-dev components (degraded warnings, not hard failures, in `LocalDevTest` mode). `NoopSubagentRestartReconciler` ignores the admission gate (`completed_at` always `Some(Instant::now())`).

**HA replicas — design decision deferred to follow-up.** With multiple replicas serving the same tenant, each replica's reconciler runs `replay` independently. Correctness is preserved (Phase 2b's `INSERT OR IGNORE` arbitrates; the seal UPDATE is single-winner). Cost is N× DB load at boot for N replicas — wasted but bounded. Future HA work may introduce per-tenant leader election (Postgres advisory locks, K8s leases) or work-sharding by `hash(tenant_id) % replica_count`. Track as cross-cutting work; do not block WU-C on this. The current spec is HA-safe but HA-redundant.

### 5.7 Observability contract

Each per-scope replay emits a structured event at completion. Operator dashboards key on `(tenant_id, agent_id)`.

**Per-replay event:**

```
RebornEventKind::SubagentReplayCompleted {
    scope: TurnScope,
    duration_ms: u64,
    pool_size: u32,             // replay_pool capacity at run time
    pending_count: u32,         // rows entering Phase 0
    report: ReplayReport,       // 5-counter struct from §5.2
}
```

**Required metrics** (Prometheus / equivalent):

| Metric | Type | Labels | Meaning |
|---|---|---|---|
| `reborn_subagent_replay_duration_seconds` | Histogram | `tenant_id`, `agent_id` | Wall-clock per-scope replay. P50/P95/P99 buckets. |
| `reborn_subagent_replay_pending_rows`     | Gauge     | `tenant_id`, `agent_id` | Live count of pending rows (Phase 0 output). Sampled per replay. |
| `reborn_subagent_replay_outcomes_total`   | Counter   | `tenant_id`, `agent_id`, `outcome ∈ {redelivered, skipped_idempotent, retryable, skipped_orphan, failed}` | Cumulative per-outcome counter. |
| `reborn_subagent_pencil_age_seconds`      | Gauge     | `tenant_id`, `agent_id` | Max age of any pencil-receipt row in the ledger. Sampled per replay. |
| `reborn_subagent_replay_in_progress`      | Gauge     | `tenant_id`, `agent_id` | 0 or 1. Tracks the background task. |

**Required alerts:**

- **`failed > 0` over any 5-minute window** → page on-call. Real-failure indicator; all phantom failures (orphans, idempotent-skips) are routed to dedicated counters under A+A.
- **`pencil_age_seconds > 60`** → page on-call. A pencil receipt older than 60s indicates either a flaky reconciler impl or a stuck retry loop — neither is normal recovery behavior.
- **`replay_duration_seconds{quantile="0.95"} > 30`** → ops review. Replay should complete in seconds, not tens. P95 above 30s suggests either a connection-pool starvation issue (raise `replay_pool_size`) or a real fan-out scale problem (raise the alarm to engineering).

**Tracing.** Replay opens one span per scope (`reborn.subagent.replay`), with child spans per phase (`phase0.bound`, `phase1.preflight`, `phase2a.cleanup`, `phase2b.claim`, `phase3.load`, `phase4.deliver`). Span attributes include `pending_count`, `outcome counts`, `pool_size`. This is standard OpenTelemetry shape — no custom span format.

**WebUI surfacing (WU-F).** The WebUI's replay-status indicator reads `replay_in_progress` per `(tenant_id, agent_id)` and surfaces "background subagent recovery in progress (N pending)" until the gauge drops to 0. Background-spawn rejection during this window MUST surface to the user as a "starting up, retrying in N seconds" affordance — not as a silent error.

**Why this is required, not optional.** WU-D's background mode produces actions a user can see (subagent spawn + later result delivery). When replay is mid-flight, those actions become latency-uncertain. The observability contract turns that uncertainty into a deterministic operator signal — without it, ops blame the application layer for what is actually durable-state recovery delay. This contract is a prerequisite for the WU-G E2E + parity tests to be authored.

### 5.8 Crate placement

All new types — `SubagentRestartReconciler` trait, `ReplayReport`, `ReconcilerError`, `DurableSubagentRestartReconciler` (libSQL impl), `DurableSubagentRestartReconcilerPostgres` (PostgreSQL impl), `NoopSubagentRestartReconciler`, and the `subagent_idempotency_ledger` migration files — live in `crates/ironclaw_reborn_event_store/`. Canonical owner of Reborn durable backend selection (`events.md` §2). Existing `BoundaryRule` covers it. Already holds both libSQL and filesystem backends. Adding typed repositories for the idempotency ledger here follows the same pattern as the existing libSQL-backed durable event log.

### 5.9 Test plan

Per `.claude/rules/testing.md` "Test Through the Caller" rule — unit tests on reconciler helper functions alone are not sufficient because `replay` gates a gate-store side effect (background child delivery) through multiple intervening components.

**Reconciler replay test** (drives through composition boot path):

```
1. Wire durable backend (libSQL in-process) + gate store + capability result store.
2. Simulate settled background child:
   a. Write settlement log entry for (parent_run_id, child_run_id, terminal_kind=Completed).
   b. Write capability result at corresponding result_ref.
   c. Do NOT write idempotency ledger row (simulates crash before delivery).
3. Drop all in-memory state.
4. Boot new composition with same durable backend.
5. Call reconciler.replay(&scope).await.
6. Assert report.redelivered == 1, skipped == 0, failed == 0.
7. Assert gate store records child as delivered.
```

**Double-replay idempotency test:**

```
1. Run the setup from replay test above.
2. Call reconciler.replay(&scope).await second time.
3. Assert report.redelivered == 0, skipped_idempotent == 1, failed == 0.
4. Assert gate store entry count is still 1.
```

**Tombstoned-result test:**

```
1. Write settlement log entry + capability result.
2. Write tombstone for child_run_id (simulates parent-cancel after settle, before crash).
3. Boot + replay.
4. Assert report.skipped_orphan == 1, redelivered == 0, failed == 0.
   // Tombstone detected in Phase 1 → moved to tombstoned_rows → sealed+skipped.
5. Assert gate store has no entry for child_run_id.
6. Assert the idempotency ledger row exists with delivered_at NOT NULL (sealed).
```

**Missing-capability-result test:**

```
1. Write settlement log entry only; do NOT write capability result.
2. Boot + replay.
3. Assert failed == 1, redelivered == 0.
```

**Crash-between-ledger-insert-and-gate-write test:**

```
1. Write settlement log entry + capability result + capability result store row.
2. Pre-insert an idempotency ledger row for the same (run_id, child_run_id, terminal_kind) — simulates the post-crash state after ledger insert but before gate delivery (delivered_at IS NULL, pencil receipt).
3. Drop all in-memory state. Boot a fresh reconciler against the same durable backend.
4. Call reconciler.replay(&scope).await.
5. Assert report.retryable == 1, report.redelivered == 1, report.failed == 0.
6. Assert gate store has an entry for child_run_id (delivery was completed on retry).
7. Assert idempotency ledger row has delivered_at IS NOT NULL (sealed after successful re-delivery).
```

This test guards the two-phase ledger fix (D1): pencil receipts left by a crash are detected and retried, not permanently skipped.

**Orphan-gate test:**

```
1. Write settlement log entry + capability result for (parent_run_id, child_run_id).
2. Delete the parent gate row (simulates parent-cancel cleanup after settlement was logged).
3. Boot fresh reconciler against the same durable backend.
4. Call reconciler.replay(&scope).await.
5. Assert report.skipped_orphan == 1, redelivered == 0, failed == 0.
6. Assert a tombstone was written for child_run_id with disposition == DiscardedParentGone.
7. Assert the idempotency ledger row exists with delivered_at NOT NULL (sealed).
8. Call reconciler.replay(&scope).await a second time.
9. Assert report.skipped_orphan == 0 (row is sealed, no further work), skipped_idempotent == 1.
```

Guards D9: orphan rows are cleaned up exactly once and never reprocessed.

**Dual-backend parity test** (libSQL vs PostgreSQL, part of WU-G #4431): run all four bodies against both `RebornLibSqlIdempotencyLedger` and `RebornPostgresIdempotencyLedger`, matching the pattern of `assert_settled_action_survives_reopen_and_replays` in `crates/ironclaw_product_workflow_storage/tests/support/mod.rs`.

All tests go in `crates/ironclaw_reborn_event_store/tests/` (contract-test tier, matching `durable_event_store_contract.rs` + `filesystem_event_log_contract.rs` pattern). Run under `cargo test --features integration` for backend-dependent variants.

### 5.10 Risks / open questions

- **Replay throughput.** Crash mid-flight on large fan-out (e.g., 100 children all settled) → reconciler reads 100 log entries, 100 ledger `INSERT OR IGNORE` attempts, 100 gate store writes synchronously with boot. Bounded by `SubagentSpawnLimits.max_depth` = 1 + future `max_concurrent_background_children` cap → per-boot replay time bounded. If concurrent cap is raised significantly, reconciler should be made non-blocking (run in background task, gate new background spawn acceptance until replay completes). Track as follow-up when WU-D sets the concurrent cap.
- **Stale-children GC.** Orphan cleanup (D9) handles the case where the parent run is gone by the time the reconciler runs — those entries are tombstoned and sealed in one pass. Stale tombstones from a deployment where `BoundedSubagentResultTombstoneStore` evicted entries before the durable migration are a separate concern; the durable `FilesystemSubagentTombstoneStore` (§3) eliminates eviction by construction. A time-based TTL GC for long-completed ledger rows is a future optimization, not a correctness requirement under A+A.
- **Capability result tombstoned between settle and replay.** If result at `result_ref` was GC'd between child settled and replay (e.g., result store has TTL), `capability_result_store.load` returns `None` → entry counted `failed`. Ledger row remains, preventing future re-attempt. Correct behavior — a GC'd result cannot be re-delivered — but surfaces as non-zero `failed` in `ReplayReport`. Operators see `warn!`. Documentation for `ReplayReport.failed` must call this out. For WU-C the in-memory `CapabilityResultStore` has no TTL → cannot occur; only materializes if future durable store adds TTL eviction. The reconciler counts this as `failed` (not `skipped`) and the pencil receipt remains in the ledger, so the next boot will retry the capability load. If the result remains missing across N consecutive boots, an operator may manually tombstone the entry; automated stale-pencil GC is a follow-up, not WU-C scope.
- **Feature toggle interaction.** While `subagent.background_enabled` is `false` (default until WU-G), no settlement log entries for background children are written → replay always returns zero `ReplayReport`. When toggle flips back `false` after `true` (rollback), durable rows from ON-period remain; replay on next boot returns `failed` entries for each settled child whose parent loop no longer expects results (gate store entry for blocking-mode parent does not accept background deliveries). Safe — `failed` count increments, ledger row blocks future re-attempt, parent loop unaffected.
- **HA replication makes replay redundant but safe.** Each replica boot runs its own replay. Correctness holds because Phase 2b is row-level idempotent and the seal UPDATE is single-winner. Cost is N× DB load at boot for N replicas. Acceptable for current single-node + warm-standby topologies. If we ever run active-active replicas, introduce per-tenant leader election (advisory lock / K8s lease) — a cross-cutting follow-up, not WU-C scope. The current spec is HA-safe, HA-redundant.
- **Settlement log growth.** Phase 0's LEFT JOIN bounds replay input by outstanding pencil-or-missing rows, so replay's scan size stays proportional to outstanding work — not historical log size. Long-term, sealed rows older than (e.g.) 90 days should be moved to a `subagent_gate_settlement_log_archive` table or summarized via materialized view. Track as ops follow-up; not WU-C scope.
- **Replay pool sizing under load.** Default `replay_pool_size = 4` is fine for typical fan-outs. At sustained 1000+ pending rows per scope, tuning to 8 or 16 may be warranted. Operators surface this via the `replay_duration_seconds` P95 metric. Spec does not mandate auto-tuning; sizing knob is operator-controlled per `RebornEventStoreConfig`.

### 5.11 HA leader election (future, NOT WU-C scope)

**Problem.** In an HA active-active deployment, every replica's reconciler runs `replay` on all scopes the replica owns. Phase 2b's `INSERT OR IGNORE` arbitrates correctness, but every replica performs the same scan + writes — N× redundant DB load. At a 50-replica × 100-scope deployment that's 5,000 reconciler scan+write transactions per boot, all hitting the same row partitions. The A.A jitter mitigates the deploy-time burst but does not reduce total work.

**Long-term answer.** Per-scope leader election via Postgres advisory locks:

```rust
async fn try_become_replay_leader(
    &self,
    scope: &TurnScope,
) -> Result<Option<LeaderHandle>, ReconcilerError> {
    // Compute a stable u64 key from the scope.
    let key = hashtext(format!("reborn.replay:{}:{}:{}",
                                scope.tenant_id,
                                scope.user_id_or_sentinel(),
                                scope.agent_id_or_sentinel()));

    // Transaction-scope lock: auto-releases on transaction end (commit OR crash).
    // No risk of lock leak from a crashed leader.
    let acquired: bool = sqlx::query_scalar!(
        "SELECT pg_try_advisory_xact_lock($1)",
        key as i64,
    ).fetch_one(&mut tx).await?;

    if acquired {
        Ok(Some(LeaderHandle { tx /* held for the duration of replay */ }))
    } else {
        Ok(None)  // Another replica owns replay for this scope; skip.
    }
}
```

Replicas that LOSE the election skip replay for that scope; they still receive settled-child deliveries via the gate-store mailbox as normal. Total fleet-wide reconciler work drops from `O(N × scopes)` to `O(scopes)`.

**Why deferred from WU-C.** (1) Single-node deployments don't need it. (2) The advisory-lock pattern requires PostgreSQL — libSQL has no equivalent. The libSQL deployment shape stays correct without leader election (all replicas replay, INSERT OR IGNORE arbitrates), just redundant. (3) A.A jitter solves 80% of the operational pain at zero architectural cost. (4) The election protocol composes cleanly with D1's two-phase ledger: a leader that crashes mid-replay releases its advisory lock automatically (transaction-scope); the next replica that retries the election picks up where the failed leader left off (D1's pencil receipts are arbitration points; nothing leaks).

**Trigger for promoting from "future" to "ship".** Operational metric: `replay_duration_seconds{P95}` exceeds 30s at fleet-rollout time AND `replay_in_progress` aggregate stays elevated for >60s across replicas. Both metrics are wired in §5.7 — operators can decide based on observed shape.

**libSQL fallback.** When the deployment backend is libSQL, `try_become_replay_leader` returns `Some(LeaderHandle::noop)` unconditionally — every replica is its own leader. libSQL deployments are typically single-node so the redundancy concern does not materialize.

**Spec impact.** None on the current WU-C surface. When this lands as a follow-up, it becomes a new `ReconcilerLeaderElection` trait under `crates/ironclaw_reborn_event_store`, with `PostgresAdvisoryLockLeader` and `LibSqlNoopLeader` implementations. The composition wire-up in §5.6 gains an `Arc<dyn ReconcilerLeaderElection>` injection point; the per-scope `tokio::spawn` body calls `try_become_replay_leader` before Phase 0.

---

## Section 6 — Migration & rollback

### 6.1 In-flight RAM state at deploy

**Decision: accept loss.**

When `subagent.background_enabled` is `false` (default through WU-C/D/E), background subagents cannot be spawned. All four in-memory stores hold state only for blocking subagent runs. Blocking runs are short-lived and complete before any realistic deploy window.

When durable store code (WU-C) lands and toggle is still `false`: behavior unchanged.

When toggle flips `true` for the first time:

- Every subsequent subagent spawn writes its goal, gate registration, tombstone, and capability result to the durable backend.
- Any state that existed only in RAM before the flip — e.g., a blocking run that spawned just before deploy — lives out its natural life and is collected with the process. Not migrated.

**Why safe:** background mode is disabled until WU-G (E2E + parity tests pass), so no background-specific RAM state accumulates before the first durable-toggle flip. `SubagentRestartReconciler`, running at boot against an empty or sparse store, is a no-op.

**Document in WU-C PR description:**

> When `subagent.background_enabled` flips ON, all new subagent spawns durably persist goal, gate, tombstone, and capability result. In-flight RAM-only state from runs that started before the flip is not migrated; it remains RAM-only and is cleaned up with the process at next restart. The reconciler, finding no orphaned durable rows from a previous ON-period, produces no replay events.

### 6.2 Rollback (toggle OFF after ON)

**Recommended behavior: leave durable rows in place; in-memory paths re-activate.**

If `subagent.background_enabled` is set back to `false` after a period it was `true`:

1. `ironclaw_reborn_composition`'s runtime wiring re-selects `InMemoryBoundedSubagentGoalStore` and the in-memory gate/tombstone stores — same path used before WU-C landed.
2. `SubagentRestartReconciler` still runs at boot (required component per `production_readiness.rs`). With toggle off, no new background spawns admitted → no new durable rows written. Reconciler scans durable settlement event log, finds no undelivered rows with living in-memory consumers, exits as no-op.
3. Durable rows written during ON-period remain. Not deleted, cannot be deleted without explicit GC migration. Correct per **LLM data retention rule** in `CLAUDE.md`: "LLM data is never deleted."

**Data-retention policy for rollback rows:**

Rows in durable subagent stores (goal, gate_resolution, tombstone, capability_result, settlement_event_log, idempotency_ledger) written during an ON-period are:

- Read-only artifacts once toggle is OFF.
- Queryable for debugging and audit.
- Never automatically purged; future GC migration may introduce TTL-based cleanup, but ships separately and must be explicitly requested.
- Not replayed into in-memory state after rollback — idempotency ledger entries prevent double-delivery if toggle flips ON again (see §6.3).

**Production-readiness gate:** `production_readiness.rs` checks `SubagentRestartReconciler` with `Required`. After rollback, reconciler must still be wired (even as no-op) or readiness check blocks production startup. In-memory reconciler stub satisfies this in `LocalDevTest` mode; production-safe no-op reconciler must be supplied in `Production` mode.

### 6.3 Re-flip (OFF → ON → OFF → ON)

When `subagent.background_enabled` flips back `true` after a rollback period:

1. `SubagentRestartReconciler` runs at boot, scans `subagent_gate_settlement_log` for rows from previous ON-period whose parent run may still be active.
2. For each row, reconciler runs the §5.3 algorithm:
   - If the gate is gone (`!gate_store.gate_exists`), tombstone the child + seal the ledger row + count as `skipped_orphan`. This is the rollback-period cleanup path.
   - If a sealed ledger row exists (`delivered_at IS NOT NULL`), count as `skipped_idempotent` and skip. Previous ON-period already delivered.
   - If a pencil ledger row exists (`delivered_at IS NULL`), count as `retryable` and re-attempt delivery + seal. Previous ON-period crashed mid-flight.
   - Otherwise insert pencil receipt, deliver, seal. Counts as `redelivered`.
3. Rows that successfully replay become live `SettledChild` notifications in the parent's mailbox.
4. Failures (missing capability result, gate-store error) leave the pencil receipt in place and count as `failed` — the next boot retries.

**Idempotency invariant.** The two-phase ledger (D1) provides the single point of truth for "is this delivery final?":
- `delivered_at IS NOT NULL` → sealed → final → never retry.
- `delivered_at IS NULL` → pencil receipt → mid-flight → retry on every boot until sealed or tombstoned.

Both the seal UPDATE and the gate store's own primary-key idempotency prevent duplicate delivery. The `INSERT OR IGNORE` / `ON CONFLICT DO NOTHING` on the pencil row prevents two nodes from claiming the same delivery simultaneously. Together: at most one delivery per `(run_id, child_run_id, terminal_kind)` tuple regardless of node count, crash count, or rollback count.

---

## Section 7 — Dual-backend parity test (#4431 follow-on)

### 7.1 Test goal

Every persistence behavior introduced by WU-C must be tested against **both** libSQL and PostgreSQL backends, asserting identical observable behavior at the trait boundary. Test does not assert identical SQL plans or storage layouts — it asserts the Rust trait interface produces same results regardless of backend.

Directly addresses `_contract-freeze-index.md` §8: "PostgreSQL/libSQL parity is required for production persistence behavior unless a contract explicitly says a backend is unsupported."

### 7.2 Test placement

Existing parity harness in this repo:

- `crates/ironclaw_hooks_parity/tests/parity_matrix.rs` — hooks-tier behavioral parity matrix.
- `tests/reborn_wrong_scope_access_isolation_parity.rs` — cross-scope isolation parity at integration test tier.
- `tests/support_unit_tests.rs` and `tests/support/reborn/product_workflow.rs` — `RebornProductWorkflowHarness` / `FilesystemIdempotencyLedger` parity helpers with `filesystem_temp` + `filesystem_shared_backend` constructors.

Subagent store parity tests do **not** belong in `ironclaw_hooks_parity` (hooks-specific contract). Correct location:

```
crates/ironclaw_reborn_event_store/tests/parity.rs
```

`crates/ironclaw_reborn_event_store/` is canonical owner of durable backend selection per `events.md` §2 and already contains:

- `tests/durable_event_store_contract.rs`
- `tests/filesystem_event_log_contract.rs`
- `tests/profile_contract.rs`

New `tests/parity.rs` follows this existing contract-test pattern. Already has a `BoundaryRule` entry — no new rule needed for the test file.

### 7.3 Test matrix

The following invariants must be tested against both libSQL and PostgreSQL (under `#[cfg(feature = "integration")]` per `CLAUDE.md`):

**Gate resolution (`SubagentGateResolutionStore` trait)**

- `first_writer_wins_under_concurrent_settle`: two concurrent `mark_child_delivered` calls for same `(gate_ref, child_run_id)` — exactly one returns `true` (gate complete), the other returns `false`; both succeed without error.
- `mark_delivered_is_idempotent`: calling `mark_child_delivered` twice with same args returns `Ok` on both calls; second returns `false`.
- `gate_resolution_scoped_query_excludes_rows_from_other_agents`: insert two `AwaitedChildState` rows under the same `(tenant_id, user_id)` but distinct `agent_id` values (A and B); assert that any query/list operation scoped to `agent_id = A` returns only A-owned rows and never any B-owned row. Guards the §1.7 invariant that every scoped query must include `agent_id` in the WHERE predicate.

**Goal store (`SubagentGoalStore` trait — `FilesystemSubagentGoalStore` backed by libSQL/PostgreSQL `RootFilesystem`)**

- `put_then_get_round_trip`: `put_goal` followed by `get_goal` with same `(TurnScope, TurnRunId)` returns original `SubagentGoal` payload.
- `put_rejects_duplicate_key`: second `put_goal` with same key returns `SubagentGoalStoreError::DuplicateKey` on both backends.
- `delete_goal_is_idempotent`: `delete_goal` called twice on same key returns `Ok` on both calls on both backends.

**Capability result store (`CapabilityResultStore` trait — introduced in WU-C)**

- `write_returns_same_shape`: `write` returns `(LoopResultRef, u64)`; `u64` byte length matches payload byte length; shape identical across backends.
- `read_after_write_returns_identical_bytes`: `read` after `write` returns byte slice byte-for-byte equal to what was written.

**Tombstone store (`SubagentResultTombstoneStore` trait)**

- `write_tombstone_insert_or_ignore`: two `write_tombstone` calls with same `child_run_id` both succeed without error; `read_tombstone` returns first-written value (first-writer-wins).
- `read_miss_returns_none`: `read_tombstone` for unknown `child_run_id` returns `Ok(None)` on both backends.

**Settlement event log (new table, owned by `SubagentRestartReconciler`)**

- `reconciler_replays_undelivered_rows`: write settlement event row, drop in-memory state, construct new reconciler backed by same store, call `replay` — assert parent loop mailbox receives expected `SettledChild` notification.
- `reconciler_is_idempotent_with_ledger`: call `replay` twice — assert mailbox receives exactly one notification.

**Idempotency ledger (new table)**

- `duplicate_insert_returns_skipped_not_error`: two concurrent `begin_or_replay` calls for same `(run_id, child_run_id, terminal_kind)` — one returns `IdempotencyDecision::New`, the other returns `Transient` retry signal (not error), on both backends. Matches existing `filesystem_idempotency_ledger_serializes_concurrent_begin` test pattern in `tests/support_unit_tests.rs`.

### 7.4 Fixture strategy

**libSQL:** in-process libSQL using `libsql::Builder::new_local(":memory:")`. No external service. Pattern used throughout existing `crates/ironclaw_reborn_event_store/tests/` contract tests.

**PostgreSQL:** testcontainers via `testcontainers::runners::AsyncRunner` and `testcontainers-modules::postgres::Postgres` image. Per `CLAUDE.md` current limitation: "Integration tests need testcontainers for PostgreSQL." Tests gated behind `#[cfg(feature = "integration")]`.

Both backends exercised through identical test functions, parameterized by `BackendFixture` enum (or calling same test body twice with different `Arc<dyn RootFilesystem>` / typed store constructors).

### 7.5 CI tier

All parity tests run under:

```bash
cargo test -p ironclaw_reborn_event_store --features integration
```

Broader integration suite:

```bash
cargo test --features integration
```

WU-G ships these tests. Per plan closing criteria, feature toggle `subagent.background_enabled` flips to `true` in production only after WU-G E2E + parity tests pass green.

---

## Section 8 — Scope propagation (`agent_id` columns + indexes)

### 8.1 Requirement

Every durable table introduced by WU-C carries `tenant_id`, `user_id`, and `agent_id` columns per `_contract-freeze-index.md` §2 + §8.

`agent_id` is **nullable** — non-agent runs produce `TurnScope` values where `agent_id` is `None` (`TurnScope` in `crates/ironclaw_turns/src/scope.rs`, field `pub agent_id: Option<AgentId>`).

`user_id` maps to `TurnScope::explicit_owner_user_id()` when present, falls back to `SYSTEM_RESERVED_ID` for ownerless turns (per `TurnScope::to_resource_scope()`).

`tenant_id` is always non-null (`TurnScope.tenant_id: TenantId` is required).

### 8.2 Index policy

Primary lookup index on `(tenant_id, user_id, agent_id, <store-specific discriminant columns>)`.

- **Cross-tenant isolation:** leading `tenant_id` ensures no query can scan rows from other tenants without explicit predicate mismatch.
- **Scope-bounded scans:** `(tenant_id, user_id, agent_id)` prefix matches `TurnScope` → `ResourceScope` projection in `TurnScope::to_resource_scope()` → consistent index semantics across filesystem-backed and typed-repo-backed stores.
- **Uniqueness:** trailing store-specific columns complete the uniqueness guarantee; `(tenant_id, user_id, agent_id)` prefix alone is not unique.

Secondary indexes per store for non-scoped lookup patterns (e.g., lookup by `child_run_id` alone in tombstone store for reconciler's replay scan).

### 8.3 Per-store column + index table

| Store | Table name | Scope cols | Primary lookup index |
|---|---|---|---|
| gate_resolution | `subagent_gate_awaited_children` (+ child_index + deliverable_queue) | `tenant_id TEXT NOT NULL`, `user_id TEXT NOT NULL`, `agent_id TEXT` | `(gate_ref, child_run_id)` UNIQUE; secondary `(tenant_id, user_id, agent_id)` |
| gate_capacity_counter | `subagent_gate_capacity_counter` | `tenant_id TEXT NOT NULL`, `user_id TEXT NOT NULL`, `agent_id TEXT`, `bucket SMALLINT/INTEGER NOT NULL` | `(tenant_id, user_id, agent_id, bucket)` UNIQUE — sharded by bucket = hash(child_run_id) % K (K=`CAPACITY_COUNTER_BUCKETS`, default 16); `undelivered INTEGER CHECK >= 0`; `idx_sgcc_scope` covers SUM cap check |
| goal | filesystem path-based (no SQL table) | path-segment based | `ScopedPath` prefix matches scope |
| capability_result | `capability_results` | `tenant_id TEXT NOT NULL`, `user_id TEXT NOT NULL`, `agent_id TEXT` | `(result_ref)` UNIQUE; secondary `(tenant_id, user_id, run_id, created_at)` |
| tombstone | filesystem path-based (`FilesystemSubagentTombstoneStore`) | path-segment based | `ScopedPath` prefix matches scope |
| settlement_event_log | `subagent_gate_settlement_log` | `tenant_id TEXT NOT NULL`, `user_id TEXT NOT NULL`, `agent_id TEXT` | append-only autoincrement; secondary `(tenant_id, user_id, agent_id)` + `(parent_run_id)` + `(child_run_id)` |
| idempotency_ledger | `subagent_idempotency_ledger` | `tenant_id TEXT NOT NULL`, `user_id TEXT NOT NULL`, `agent_id TEXT` | `(run_id, child_run_id, terminal_kind)` UNIQUE; secondary `(tenant_id, user_id, agent_id, run_id)` |

Notes:

- `gate_resolution` does not have a unique constraint on `(tenant_id, user_id, agent_id, gate_ref)` alone because a gate may have multiple child entries. Flattened into rows keyed on `(gate_ref, child_run_id)`.
- `settlement_event_log` is append-only (no UPDATE or DELETE). Reconciler queries all rows for a scope and checks the idempotency ledger per-row to decide whether to replay (the log itself carries no delivery flag). Marks rows "delivered" by writing the ledger entry, not by updating the log row. Preserves log as immutable audit trail.
- libSQL stores `tenant_id`, `user_id`, `agent_id` as `TEXT`. PostgreSQL stores them as `TEXT` (not `UUID` typed) to match codebase convention in `libsql_migrations.rs`.

### 8.4 `TurnScope` as the scope type threading through trait signatures

The canonical scope type is `TurnScope` from `crates/ironclaw_turns/src/scope.rs`:

```rust
pub struct TurnScope {
    pub tenant_id: TenantId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub thread_id: ThreadId,
    pub thread_owner: TurnThreadOwner,
}
```

- `agent_id: Option<AgentId>` maps directly to nullable `agent_id` column.
- `TurnScope::to_resource_scope()` produces `ironclaw_host_api::ResourceScope` used by `FilesystemSubagentGoalStore` and `FilesystemIdempotencyLedger` for filesystem path dispatch — new typed-repo stores derive column values from same `TurnScope` fields rather than calling `to_resource_scope()`.

`TurnScope` is already the scope parameter in all four existing in-memory store trait signatures (`SubagentGoalStore::put_goal(&self, scope: &TurnScope, ...)`, `SubagentGateResolutionStore`, `SubagentResultTombstoneStore`, `SubagentSpawnGoalStore` alias in `ironclaw_loop_support`). Durable implementations accept the same `&TurnScope` and extract `tenant_id`, `user_id` (from `explicit_owner_user_id()` or sentinel), `agent_id` at write time.

`CapabilityResultStore` trait (introduced in WU-C; does not yet exist) must be defined with `&TurnScope` as scope parameter, consistent with all other store traits in this family.

### 8.5 Migration-script convention

**Where migrations live:**

Legacy v1 database layer uses:
- `src/db/libsql_migrations.rs` — consolidated base schema + `INCREMENTAL_MIGRATIONS` array (versioned `(i64, &str, &str)` tuples).
- `src/db/postgres.rs` — PostgreSQL DDL executed at startup.

Reborn-crate persistence is separate. `ScopedFilesystem`-backed stores (goal store) do not use SQL migrations — filesystem path layout is implied by `TurnScope` → `ResourceScope` → path grammar. Typed-repo stores (gate_resolution, capability_result, settlement_event_log, idempotency_ledger) in `crates/ironclaw_reborn_event_store/` use crate-local migration modules:

```
crates/ironclaw_reborn_event_store/src/libsql/migrations.rs   # libSQL DDL constants
crates/ironclaw_reborn_event_store/src/postgres/migrations.rs # PostgreSQL DDL constants
```

**Naming convention** (matching `libsql_migrations.rs`):

```rust
pub const INCREMENTAL_MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "subagent_gate_resolution",       /* DDL */),
    (2, "subagent_capability_result",     /* DDL */),
    (3, "subagent_settlement_event_log",  /* DDL */),
    (4, "subagent_idempotency_ledger",    /* DDL */),
];
```

Version numbers are **independent** from `src/db/libsql_migrations.rs` — `ironclaw_reborn_event_store` owns its own `_reborn_migrations` tracking table (same `(version, name, applied_at)` schema, different table name to avoid collision). Matches comment in `libsql_migrations.rs`: "libSQL incremental migration version numbers are independent from PostgreSQL migration version numbers."

All DDL uses `CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS` for idempotency.

---

## Closing checklist (before WU-C opens)

- [ ] This spec PR merged.
- [ ] WU-C decides per-store ScopedFilesystem-vs-typed-repo choices match §1 through §5 recommendations (any deviation requires an addendum here).
- [ ] WU-C adds `BoundaryRule` verification step: `cargo test -p ironclaw_architecture` passes with the new types in `ironclaw_reborn_event_store` (existing rule covers; no new entry needed).
- [ ] WU-C adds `SubagentRestartReconciler` impl behind feature-gated build; production-readiness check flips from stub to required.
- [ ] WU-C adds `CapabilityResultStore` trait + impls (in-memory + libSQL + PostgreSQL).
- [ ] WU-C wires `BoundedSubagentResultTombstoneStore` into `SubagentCompletionObserver` (the wiring gap from §3.1).
- [ ] WU-C corrects in-memory tombstone store to first-writer-wins (§3.6).
- [ ] WU-G adds parity test at `crates/ironclaw_reborn_event_store/tests/parity.rs` per §7.
- [ ] WU-C lands the `SubagentResultTombstoneStore::write_tombstone` scope-parameter signature change BEFORE implementing `FilesystemSubagentTombstoneStore` (§3.7).
- [ ] WU-C lands the two-phase idempotency ledger (D1): `delivered_at NULL` column nullable; pencil-insert + pen-seal pattern; matches the existing `IdempotencyLedger::begin_or_replay` precedent in `crates/ironclaw_product_workflow/src/ledger.rs`.
- [ ] WU-C lands orphan-gate handling (D9): reconciler tombstones + seals when `gate_store.gate_exists(gate_ref)` returns false; `gate_exists` becomes a required method on the gate store trait.
- [ ] WU-C extends `ReplayReport` with `retryable: u32` and `skipped_orphan: u32` counters and updates operator dashboards (`warn!` on `failed > 0` only).
- [ ] WU-C implements the §5.3 phase-batched replay algorithm: Phase 0 LEFT JOIN, Phase 1 batched preflight, Phase 2 multi-row ledger writes, Phase 3 `join_all` capability loads, Phase 4 per-row deliver+seal.
- [ ] WU-C lands `replay_pool` config (`RebornEventStoreConfig.replay_pool_size: u32`, default 4). Reconciler MUST use this pool exclusively for replay queries.
- [ ] WU-C dispatches replay via `tokio::spawn` from composition boot, NOT `.await` inline. Foreground traffic accepts immediately on cold start.
- [ ] WU-C wires the per-scope admission gate: `SpawnSubagentPort` for background mode reads `ReplayState[scope].completed_at` before admitting; rejects with `SubagentSpawnError::ReplayInProgress` until complete. Foreground / blocking-subagent paths never consult this gate.
- [ ] WU-C lands eager active-scope enumeration via runs-table query at boot. Lazy per-scope replay is a deferred follow-up.
- [ ] WU-C lands the §5.7 observability contract: emit `RebornEventKind::SubagentReplayCompleted` per-scope; expose the five required metrics; wire the three required alerts.
- [ ] WU-G E2E gates the background-mode toggle (`subagent.background_enabled = true` in production) on the observability dashboard being live AND the three alerts being silent over a 7-day soak.
- [ ] WU-C lands the bucketed capacity counter (D6-A + E.A): `subagent_gate_capacity_counter` table with `(tenant_id, user_id, agent_id, bucket)` PK; `counter_bucket` column on `subagent_gate_awaited_children`; `CAPACITY_COUNTER_BUCKETS = 16` constant in `ironclaw_reborn_event_store` exposed via `RebornEventStoreConfig`; insert / delivery / delete paths use the bucketed transactional protocol per §1.6.
- [ ] WU-C implements `CapabilityResultStore` trait with `Vec<u8>` payload (D8-A). Executor calls `serde_json::to_vec` exactly once; backend INSERTs the bytes directly into BLOB / JSONB without re-serializing. `read()` returns bytes; callers deserialize lazily.
- [ ] WU-C adds `RebornEventStoreConfig.reconciler_replay_jitter_ms: u64` (default 5000) and applies it via `tokio::time::sleep(Duration::from_millis(rand::random::<u64>() % jitter))` immediately before launching the per-process replay task (A.A).
- [ ] §5.10 HA leader election is a tracked follow-up; NOT WU-C scope. WU-C ships the spec-documented invariants without it; promotion gated on the §5.7 metric trigger.

## References

- `docs/plans/2026-06-06-subagent-compaction-impl.md` (parent plan)
- `docs/reborn/2026-06-04-subagent-compaction-design.md` (parent design)
- `docs/reborn/contracts/_contract-freeze-index.md` §1, §2, §8
- `docs/reborn/contracts/events.md` §2
- `docs/reborn/2026-04-25-storage-catalog-and-placement.md` §5.3
- `.claude/rules/database.md`
- `crates/ironclaw_reborn_event_store/src/lib.rs` (canonical durable backend owner)
- `crates/ironclaw_reborn/src/production_readiness.rs` (`RebornLoopProductionComponent`)
- `crates/ironclaw_reborn/src/subagent/gate_resolution.rs` (`BoundedSubagentGateResolutionStore`)
- `crates/ironclaw_reborn/src/subagent/goal_store.rs` (`InMemoryBoundedSubagentGoalStore`, `FilesystemSubagentGoalStore`)
- `crates/ironclaw_reborn/src/subagent/tombstone_store.rs` (`BoundedSubagentResultTombstoneStore`)
- `crates/ironclaw_loop_support/src/capability_port.rs` (`LoopCapabilityResultWriter`)
- `crates/ironclaw_reborn_composition/src/product_live_adapters.rs` (`ProductLiveCapabilityIo`)
- `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs` (boundary rules)
