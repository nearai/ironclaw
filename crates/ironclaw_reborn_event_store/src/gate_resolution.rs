//! Durable gate-resolution store trait + error type.
//!
//! Spec: `docs/reborn/2026-06-08-subagent-durability-spec.md` §1.
//!
//! The trait is implemented by:
//! - [`crate::libsql::LibSqlGateResolutionStore`] (libSQL backend, feature `libsql`)
//! - [`crate::postgres::PostgresGateResolutionStore`] (PostgreSQL backend, feature `postgres`)
//!
//! All scoped queries MUST use the conditional `<agent_predicate>` convention
//! from spec §1.6: `agent_id = ?` when the caller's `TurnScope.agent_id` is
//! `Some`, and `agent_id IS NULL` when `None`. Using
//! `(agent_id = ? OR agent_id IS NULL)` is forbidden — it allows agent-scoped
//! callers to reach system-level (NULL `agent_id`) rows.

use std::collections::HashSet;

use async_trait::async_trait;
use ironclaw_host_api::UserId;
use ironclaw_turns::{GateRef, LoopResultRef, TurnRunId, TurnScope, TurnStatus};
use thiserror::Error;

/// Errors returned by the durable gate-resolution store.
#[derive(Debug, Error)]
pub enum GateResolutionStoreError {
    /// The backend connection pool is unavailable or the database refused the
    /// connection. Callers should treat this as a transient I/O failure and
    /// retry with back-off; it is never caused by the caller's input.
    #[error("gate resolution store backend unavailable: {reason}")]
    Unavailable { reason: String },

    /// A backend I/O error occurred during the named operation (e.g.
    /// `"record_awaited_child"`, `"mark_child_delivered"`). Retrying after a
    /// brief back-off is appropriate; the operation named in `operation` is
    /// useful for structured log correlation.
    #[error("gate resolution store I/O error during {operation}: {reason}")]
    Io {
        operation: &'static str,
        reason: String,
    },

    /// A value could not be serialized to or deserialized from the wire format
    /// required by the backend. This is typically a programming error (e.g. a
    /// non-serializable type in a JSON column) and is not retriable.
    #[error("gate resolution store serialization failed: {reason}")]
    Serialization { reason: String },

    /// The `SUM(undelivered)` capacity counter for the caller's scope has
    /// reached [`MAX_GATE_RECORDS`]. The spawn should be rejected with a
    /// back-pressure signal. The counter decrements when children are delivered
    /// (`mark_child_delivered`) or the gate is deleted (`delete_awaited_child`).
    #[error("gate resolution capacity exceeded for scope")]
    CapacityExceeded,

    /// [`DurableSubagentGateResolutionStore::record_child_terminal`] was called
    /// with a [`TurnStatus`] that is not a terminal state. Only completed,
    /// failed, cancelled, or similarly final statuses are accepted; in-progress
    /// statuses are rejected to prevent premature settlement.
    #[error("gate resolution store: non-terminal status rejected")]
    NonTerminalStatus,
}

impl GateResolutionStoreError {
    pub(crate) fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }
    pub(crate) fn io(operation: &'static str, reason: impl Into<String>) -> Self {
        Self::Io {
            operation,
            reason: reason.into(),
        }
    }
    #[allow(dead_code)]
    pub(crate) fn serialization(reason: impl Into<String>) -> Self {
        Self::Serialization {
            reason: reason.into(),
        }
    }
}

/// Terminal event for a settled awaited child.
///
/// Mirrors `AwaitedChildTerminalEvent` in the in-memory store but is defined
/// here for the durable trait boundary — no dependency on
/// `ironclaw_reborn` private types.
///
/// Passed to [`DurableSubagentGateResolutionStore::record_child_terminal`]; the
/// backing SQL columns are `terminal_status`, `terminal_event_json`, and
/// `settled_at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableTerminalEvent {
    /// The terminal turn status (e.g. completed, failed, cancelled). Must be a
    /// terminal variant; non-terminal values cause
    /// [`GateResolutionStoreError::NonTerminalStatus`].
    pub status: TurnStatus,
    /// Free-form kind tag that further classifies the terminal outcome (e.g.
    /// `"completed"`, `"failed_tool_error"`). Stored as-is in the settlement
    /// log; used by the reconciler for structured reporting.
    pub kind: String,
    /// Event cursor value from the child's event log at the time of settlement.
    /// Stored in the settlement log for replay ordering.
    pub cursor: u64,
    /// Sanitized human-readable reason for non-successful terminal states.
    /// `None` for successful completions. Must not contain raw capability
    /// output, tool results, or other potentially injected content — the
    /// sanitization boundary is the settle-time delivery path.
    pub sanitized_reason: Option<String>,
    /// The `user_id` that owns the child run, if known. Carried through the
    /// settlement record for scope validation; `None` for system-level (NULL
    /// `agent_id`) runs.
    pub owner_user_id: Option<UserId>,
}

/// The number of capacity-counter buckets per scope (K=16, operator-tunable).
///
/// Spec §1 decision 21: spawn writes to `bucket = hash(child_run_id) % K`.
/// Cap check reads `SUM(undelivered) FROM counter WHERE scope`.
pub const CAPACITY_COUNTER_BUCKETS: u32 = 16;

/// Environment variable for overriding the number of capacity-counter buckets.
pub const CAPACITY_COUNTER_BUCKETS_ENV: &str = "CAPACITY_COUNTER_BUCKETS";

/// Read the effective bucket count from the environment.
/// Falls back to [`CAPACITY_COUNTER_BUCKETS`] if the env var is unset or
/// unparseable.
pub fn effective_capacity_counter_buckets() -> u32 {
    std::env::var(CAPACITY_COUNTER_BUCKETS_ENV)
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|&k| k > 0)
        .unwrap_or(CAPACITY_COUNTER_BUCKETS)
}

/// Deterministic bucket index for a given `child_run_id` string and K.
///
/// Uses FNV-1a (a simple, non-cryptographic hash adequate for bucket
/// distribution). Returns `hash(child_run_id_bytes) % k`.
pub fn child_bucket(child_run_id_str: &str, k: u32) -> u32 {
    // FNV-1a 32-bit
    const FNV_OFFSET: u32 = 2_166_136_261;
    const FNV_PRIME: u32 = 16_777_619;
    let hash = child_run_id_str.bytes().fold(FNV_OFFSET, |acc, b| {
        acc.wrapping_mul(FNV_PRIME) ^ (b as u32)
    });
    hash % k
}

/// Maximum number of undelivered awaited-child gate records per `(tenant_id,
/// user_id, agent_id)` scope.
///
/// Enforced via `SUM(undelivered)` across all capacity-counter bucket rows for
/// the scope. [`DurableSubagentGateResolutionStore::record_awaited_child`]
/// returns [`GateResolutionStoreError::CapacityExceeded`] when the sum already
/// equals this value. Mirrors `MAX_GATE_RECORDS` in the in-memory store.
///
/// This cap is **advisory back-pressure, not a hard invariant**: the
/// check-then-increment sequence is not serialized across concurrent spawns
/// (by design — the bucketed counter exists to keep the spawn hot path free
/// of a single contended row, spec decisions 20/21), so a burst of
/// simultaneous spawns may transiently overshoot the cap by up to the number
/// of in-flight spawns. Callers must not rely on the cap as an exact upper
/// bound.
pub const MAX_GATE_RECORDS: u32 = 4096;

/// Durable persistence layer for subagent gate-resolution state (spec §1).
///
/// Backs the three logical maps of `BoundedSubagentGateResolutionStore` with a
/// SQL repository: the primary awaited-child record table, a child-to-gate
/// reverse index, and a deliverable-child queue. A sharded capacity counter
/// (`subagent_gate_capacity_counter`) replaces the in-memory `total_states`
/// field for O(1) cap enforcement on the spawn hot path without a
/// `SELECT COUNT(*)` scan.
///
/// ## First-writer-wins semantics
///
/// All implementations must replicate the in-memory store's first-writer-wins
/// contract:
///
/// - [`record_awaited_child`]: `INSERT OR IGNORE` / `ON CONFLICT DO NOTHING`
///   on `(gate_ref, child_run_id)`. The capacity counter is incremented **only
///   on the first successful insert** — duplicate inserts are silent no-ops and
///   do not double-count capacity.
/// - [`record_child_terminal`]: UPDATE `terminal_status` only when
///   `terminal_status IS NULL`. The settlement log append and the deliverable
///   queue insert also fire only on first settlement.
/// - Delivery claim: `delivered_to_parent = 0` guard. The capacity counter
///   bucket is decremented **only on the claiming call** — the call that flips
///   `delivered_to_parent` from 0 to 1. Subsequent calls with the same key are
///   no-ops (idempotent, no double-decrement).
///
/// ## Scope isolation (spec §1.6)
///
/// Every query MUST use the conditional `<agent_predicate>` convention:
///
/// - `agent_id = ?` when `scope.agent_id` is `Some`.
/// - `agent_id IS NULL` when `scope.agent_id` is `None`.
/// - NEVER `(agent_id = ? OR agent_id IS NULL)` — that pattern lets
///   agent-scoped callers reach system-level (NULL `agent_id`) rows under the
///   same `(tenant_id, user_id)`, breaking cross-agent isolation.
///
/// A caller never observes rows belonging to a different scope; existence
/// queries return `false` (not `Err`) when the named key is absent from the
/// caller's scope.
///
/// ## Capacity cap
///
/// [`MAX_GATE_RECORDS`] (4096) undelivered rows per scope is enforced via
/// `SUM(undelivered)` across [`CAPACITY_COUNTER_BUCKETS`] shard rows.
/// [`record_awaited_child`] returns [`GateResolutionStoreError::CapacityExceeded`]
/// when the sum would exceed the cap. The counter is decremented symmetrically
/// by [`mark_child_delivered`] and [`delete_awaited_child`].
///
/// ## Reconciler-facing methods
///
/// Three methods are consumed by the WU-C3 `SubagentRestartReconciler` (spec
/// §5.2.1): [`gates_exist_batch`], [`redeliver_settled_child`], and
/// [`resolve_undeliverable_batch`]. These must be present before the Phase 3/4
/// replay algorithm can be implemented. See individual method docs for
/// retention and idempotency contracts.
///
/// [`record_awaited_child`]: DurableSubagentGateResolutionStore::record_awaited_child
/// [`record_child_terminal`]: DurableSubagentGateResolutionStore::record_child_terminal
/// [`mark_child_delivered`]: DurableSubagentGateResolutionStore::mark_child_delivered
/// [`delete_awaited_child`]: DurableSubagentGateResolutionStore::delete_awaited_child
/// [`gates_exist_batch`]: DurableSubagentGateResolutionStore::gates_exist_batch
/// [`redeliver_settled_child`]: DurableSubagentGateResolutionStore::redeliver_settled_child
/// [`resolve_undeliverable_batch`]: DurableSubagentGateResolutionStore::resolve_undeliverable_batch
#[async_trait]
pub trait DurableSubagentGateResolutionStore: Send + Sync {
    // ── Core CRUD ──────────────────────────────────────────────────────────

    /// Insert a new awaited-child row and increment the capacity counter.
    ///
    /// Scoped to `(tenant_id, user_id, agent_id)` derived from `scope`. NULL
    /// `agent_id` matches only NULL — never returns or affects rows for a
    /// different agent under the same `(tenant_id, user_id)`.
    ///
    /// Idempotent: a duplicate `(gate_ref, child_run_id)` is silently ignored
    /// via `INSERT OR IGNORE` / `ON CONFLICT DO NOTHING` (first-writer-wins per
    /// spec §1.6, decision 6). The capacity counter bucket is incremented
    /// **only on the first successful insert**; a duplicate call does not alter
    /// the counter.
    ///
    /// # Errors
    ///
    /// - [`GateResolutionStoreError::CapacityExceeded`] — `SUM(undelivered)`
    ///   across all capacity-counter buckets for the scope already equals
    ///   [`MAX_GATE_RECORDS`] (4096).
    /// - [`GateResolutionStoreError::Io`] / [`GateResolutionStoreError::Unavailable`]
    ///   on backend failure.
    async fn record_awaited_child(
        &self,
        scope: &TurnScope,
        record: AwaitedChildRecord,
    ) -> Result<(), GateResolutionStoreError>;

    /// Record the terminal event for a child run (first-writer-wins).
    ///
    /// Scoped to `(tenant_id, user_id, agent_id)` from `scope`. Updates
    /// `terminal_status` / `terminal_event_json` / `settled_at` only when
    /// `terminal_status IS NULL` — subsequent calls for the same
    /// `(gate_ref, child_run_id)` pair are silent no-ops (first writer wins,
    /// spec §1.6 decision 6). On first settlement also appends a row to the
    /// settlement log (`subagent_gate_settlement_log`) and inserts into the
    /// deliverable queue (`subagent_gate_deliverable_queue`). All three writes
    /// land in one transaction. Does NOT touch the capacity counter — the row
    /// remains counted until delivery or deletion.
    ///
    /// # Errors
    ///
    /// - [`GateResolutionStoreError::NonTerminalStatus`] — `event.status` is
    ///   not a terminal turn status.
    /// - [`GateResolutionStoreError::Io`] / [`GateResolutionStoreError::Unavailable`]
    ///   on backend failure.
    async fn record_child_terminal(
        &self,
        scope: &TurnScope,
        gate_ref: GateRef,
        child_run_id: TurnRunId,
        event: DurableTerminalEvent,
    ) -> Result<(), GateResolutionStoreError>;

    /// Flip `terminal_result_written = true` and record `terminal_byte_len`.
    ///
    /// Called by the executor after the capability result store write completes,
    /// as a separate transaction from settlement so the capability write can be
    /// retried without re-running settlement. Scoped to the caller's full
    /// `(tenant_id, user_id, agent_id)` — NULL `agent_id` matches only NULL.
    ///
    /// Idempotent: no-op if the row does not exist or `terminal_result_written`
    /// is already set (guard: `terminal_result_written = 0`). Does not touch
    /// the capacity counter or the deliverable queue.
    ///
    /// # Errors
    ///
    /// - [`GateResolutionStoreError::Io`] / [`GateResolutionStoreError::Unavailable`]
    ///   on backend failure.
    async fn mark_terminal_result_written(
        &self,
        scope: &TurnScope,
        gate_ref: &GateRef,
        child_run_id: TurnRunId,
        byte_len: u64,
    ) -> Result<(), GateResolutionStoreError>;

    /// Claim delivery for a child run in one atomic transaction.
    ///
    /// Flips `delivery_claimed = 1` and `delivered_to_parent = 1`, decrements
    /// the capacity counter for the child's bucket (the bucket recorded at
    /// INSERT time), and deletes the deliverable-queue entry for this specific
    /// `(gate_ref, child_run_id)`. Scoped to the caller's full
    /// `(tenant_id, user_id, agent_id)` — NULL `agent_id` matches only NULL.
    ///
    /// The capacity counter decrement happens **only on this claiming call** —
    /// the call that flips `delivered_to_parent` from 0 to 1. Idempotent: if
    /// the row is already delivered (`delivered_to_parent = 1`), returns
    /// `false` without modifying the counter or queue (no double-decrement).
    ///
    /// Returns `true` when all children under the gate now have
    /// `delivered_to_parent = 1` (the gate is fully resolved), `false`
    /// otherwise or when the row was already delivered.
    ///
    /// # Errors
    ///
    /// - [`GateResolutionStoreError::Io`] / [`GateResolutionStoreError::Unavailable`]
    ///   on backend failure.
    async fn mark_child_delivered(
        &self,
        scope: &TurnScope,
        gate_ref: &GateRef,
        child_run_id: TurnRunId,
    ) -> Result<bool, GateResolutionStoreError>;

    /// Claim the next deliverable terminal state for a child.
    ///
    /// Convenience wrapper over [`claim_all_terminal_states_for_child`] that
    /// returns only the first element of the queue. Scoped to the caller's
    /// full `(tenant_id, user_id, agent_id)` — NULL `agent_id` matches only
    /// NULL; never returns rows belonging to a different agent.
    ///
    /// Returns `Some(record)` if an undelivered-terminal row exists in the
    /// deliverable queue for `child_run_id` in the caller's scope; `None` if
    /// the queue is empty for this child.
    ///
    /// # Errors
    ///
    /// - [`GateResolutionStoreError::Io`] / [`GateResolutionStoreError::Unavailable`]
    ///   on backend failure.
    ///
    /// [`claim_all_terminal_states_for_child`]: DurableSubagentGateResolutionStore::claim_all_terminal_states_for_child
    async fn claim_next_terminal_state_for_child(
        &self,
        scope: &TurnScope,
        child_run_id: TurnRunId,
    ) -> Result<Option<AwaitedChildRow>, GateResolutionStoreError>;

    /// Drain all deliverable terminal states for a child in one call.
    ///
    /// Returns every row in the deliverable queue (`subagent_gate_deliverable_queue`)
    /// for `child_run_id` that is scoped to the caller's
    /// `(tenant_id, user_id, agent_id)` and has `delivered_to_parent = 0`.
    /// An empty `Vec` means the queue is empty for this child in this scope.
    ///
    /// Note: this method reads the queue but does **not** flip
    /// `delivered_to_parent`. Callers must call [`mark_child_delivered`] for
    /// each row to record delivery and decrement the capacity counter.
    ///
    /// Security: query is strictly scoped to the caller's full scope predicate.
    /// NULL `agent_id` matches only NULL — a caller cannot observe rows
    /// belonging to a different `agent_id` under the same `(tenant_id, user_id)`.
    ///
    /// # Errors
    ///
    /// - [`GateResolutionStoreError::Io`] / [`GateResolutionStoreError::Unavailable`]
    ///   on backend failure.
    ///
    /// [`mark_child_delivered`]: DurableSubagentGateResolutionStore::mark_child_delivered
    async fn claim_all_terminal_states_for_child(
        &self,
        scope: &TurnScope,
        child_run_id: TurnRunId,
    ) -> Result<Vec<AwaitedChildRow>, GateResolutionStoreError>;

    /// Delete all rows for a gate across all three tables in one transaction.
    ///
    /// Removes every row for `gate_ref` from `subagent_gate_awaited_children`,
    /// `subagent_gate_child_index`, and `subagent_gate_deliverable_queue`,
    /// decrementing each affected capacity-counter bucket atomically. Scoped to
    /// the caller's `(tenant_id, user_id, agent_id)` — NULL `agent_id` matches
    /// only NULL.
    ///
    /// This is the gate-cleanup path. Primary-table rows deleted here are not
    /// recoverable by the reconciler; the settlement log
    /// (`subagent_gate_settlement_log`) retains the append-only record as the
    /// replay source of truth. The reconciler treats a missing gate row as a
    /// `skipped_orphan` rather than a failure.
    ///
    /// # Errors
    ///
    /// - [`GateResolutionStoreError::Io`] / [`GateResolutionStoreError::Unavailable`]
    ///   on backend failure.
    async fn delete_awaited_child(
        &self,
        scope: &TurnScope,
        gate_ref: &GateRef,
    ) -> Result<(), GateResolutionStoreError>;

    // ── Reconciler-facing methods (spec §5.2.1) ────────────────────────────

    /// Batch existence check for gate refs (reconciler Phase 1, spec §5.2.1).
    ///
    /// Returns the subset of `gate_refs` that have at least one row in
    /// `subagent_gate_awaited_children` for the given scope. Issued as one
    /// batched SELECT; no payload bytes cross this boundary. Scoped to the
    /// caller's `(tenant_id, user_id, agent_id)` — NULL `agent_id` matches
    /// only NULL; never returns refs belonging to a foreign scope.
    ///
    /// Consumed by the WU-C3 `SubagentRestartReconciler` replay Phase 1 to
    /// determine which settlement-log rows still have a live gate row before
    /// attempting redelivery. A gate ref absent from the returned set is treated
    /// as a `skipped_orphan` (gate was cleaned up after settlement).
    ///
    /// # Errors
    ///
    /// - [`GateResolutionStoreError::Io`] / [`GateResolutionStoreError::Unavailable`]
    ///   on backend failure.
    async fn gates_exist_batch(
        &self,
        scope: &TurnScope,
        gate_refs: Vec<GateRef>,
    ) -> Result<HashSet<GateRef>, GateResolutionStoreError>;

    /// Re-drive settlement for a child that was not delivered before a restart
    /// (reconciler Phase 4, spec §5.2.1 decision 29).
    ///
    /// Idempotently ensures the gate row's terminal flags (`terminal_status`,
    /// `terminal_event_json`, `settled_at`) are set from the settlement-log row
    /// and that an entry exists in the deliverable queue — the §1.6 settlement
    /// transaction, re-driven. No payload bytes cross this boundary: the
    /// `result_ref` carried in the settlement log row is stored directly in the
    /// gate row; the parent loop reads payload bytes lazily at drain time
    /// (WU-E), not during reconciler replay.
    ///
    /// The `terminal_status IS NULL` guard on the UPDATE makes this call
    /// idempotent: if settlement already occurred (e.g. a second replica also
    /// ran reconciler) the UPDATE is a no-op and the queue entry is ensured via
    /// `INSERT OR IGNORE`.
    ///
    /// Consumed by the WU-C3 `SubagentRestartReconciler` replay Phase 4.
    ///
    /// Security: the existence check is scoped to the caller's full
    /// `(tenant_id, user_id, agent_id)` — NULL `agent_id` matches only NULL.
    /// A caller naming a foreign `(gate_ref, child_run_id)` receives `false`
    /// rather than accidentally rebinding a foreign row into their queue.
    ///
    /// Returns `true` on successful redelivery (terminal flags set + queue
    /// entry ensured). Returns `false` if the gate row does not exist in the
    /// caller's scope (orphan race — caller counts it as `skipped_orphan`).
    ///
    /// # Errors
    ///
    /// - [`GateResolutionStoreError::Io`] / [`GateResolutionStoreError::Unavailable`]
    ///   on backend failure.
    async fn redeliver_settled_child(
        &self,
        scope: &TurnScope,
        gate_ref: GateRef,
        child_run_id: TurnRunId,
        terminal_status: TurnStatus,
        result_ref: LoopResultRef,
    ) -> Result<bool, GateResolutionStoreError>;

    /// Resolve scope capacity for rows that will never deliver (spec decision 31,
    /// reconciler Phase 2a, spec §5.2.1).
    ///
    /// For each `(gate_ref, child_run_id)` pair: flips `delivered_to_parent = 1`,
    /// decrements the capacity counter for the row's bucket, and deletes the
    /// deliverable-queue entry — the §1.6 delivery-claim transaction, applied
    /// in batch. Scoped to the caller's `(tenant_id, user_id, agent_id)`.
    ///
    /// Used by the WU-C3 reconciler for the `skipped_tombstoned` and
    /// `skipped_orphan` paths (decision 31): children that were tombstoned or
    /// whose gate was deleted never deliver, so their undelivered counter
    /// contribution must be resolved to prevent scope capacity from being
    /// permanently consumed and wedging the scope at the [`MAX_GATE_RECORDS`]
    /// cap.
    ///
    /// **Retention invariant**: this method marks rows undeliverable by flipping
    /// `delivered_to_parent` — it does NOT delete primary-table rows or
    /// settlement-log rows. The settlement log is append-only and must never be
    /// deleted (LLM-data-never-deleted invariant from spec overview + CLAUDE.md).
    ///
    /// Idempotent per row via the `delivered_to_parent = 0` guard — rows
    /// already marked delivered are skipped without error and without a
    /// double-decrement.
    ///
    /// # Errors
    ///
    /// - [`GateResolutionStoreError::Io`] / [`GateResolutionStoreError::Unavailable`]
    ///   on backend failure.
    async fn resolve_undeliverable_batch(
        &self,
        scope: &TurnScope,
        rows: Vec<(GateRef, TurnRunId)>,
    ) -> Result<(), GateResolutionStoreError>;
}

/// A row in `subagent_gate_awaited_children` — returned by claim methods.
///
/// Represents the full persisted lifecycle state of one awaited child under a
/// gate. Returned by [`DurableSubagentGateResolutionStore::claim_next_terminal_state_for_child`]
/// and [`DurableSubagentGateResolutionStore::claim_all_terminal_states_for_child`].
#[derive(Debug, Clone)]
pub struct AwaitedChildRow {
    /// The gate that owns this awaited-child relationship.
    pub gate_ref: GateRef,
    /// The run ID of the awaited child.
    pub child_run_id: TurnRunId,
    /// The run ID of the parent that registered this gate.
    pub parent_run_id: TurnRunId,
    /// The root run ID of the subagent tree — used for fan-out accounting.
    pub tree_root_run_id: TurnRunId,
    /// JSON-encoded `TurnScope` (child scope).
    pub child_scope_json: String,
    /// JSON-encoded `LoopRunContext` (parent run context). Sensitive fields
    /// must be stripped before storage — see the credential audit note on
    /// [`AwaitedChildRecord::parent_run_context_json`].
    pub parent_run_context_json: String,
    /// Binding ref for the source capability that spawned this child.
    pub source_binding_ref: String,
    /// Binding ref for the reply-target slot where the child's result lands.
    pub reply_target_binding_ref: String,
    /// Kind tag for the subagent (e.g. `"loop"`, `"tool_agent"`).
    pub subagent_kind: String,
    /// Capability ID of the spawn capability used to launch this child.
    pub spawn_capability_id: String,
    /// Ref identifying the capability result in the capability result store.
    /// The parent loop reads result bytes lazily from the store using this ref
    /// at drain time (WU-E), not during reconciler replay.
    pub result_ref: LoopResultRef,
    /// Spawn mode string: `"blocking"` or `"background"`.
    pub spawn_mode: String,
    /// Terminal turn status; `None` until the child settles.
    pub terminal_status: Option<TurnStatus>,
    /// JSON-encoded terminal event; `None` until the child settles.
    pub terminal_event_json: Option<String>,
    /// Whether the capability result write has been confirmed after settlement.
    pub terminal_result_written: bool,
    /// Byte length of the written capability result; `0` until confirmed.
    pub terminal_byte_len: u64,
    /// Whether delivery was claimed for this child (set in the same
    /// transaction as `delivered_to_parent`).
    pub delivery_claimed: bool,
    /// Whether this child's terminal state has been delivered to its parent
    /// loop. Once `true`, the capacity counter bucket has been decremented and
    /// the deliverable-queue entry has been removed.
    pub delivered_to_parent: bool,
}

/// The data needed to INSERT a new awaited-child row.
///
/// Passed to [`DurableSubagentGateResolutionStore::record_awaited_child`].
/// The capacity-counter bucket is computed by the implementation from
/// `child_run_id` using [`child_bucket`] and is not a field here.
#[derive(Debug, Clone)]
pub struct AwaitedChildRecord {
    /// The gate that will own this awaited-child relationship.
    pub gate_ref: GateRef,
    /// The run ID of the parent that registered this gate.
    pub parent_run_id: TurnRunId,
    /// The root run ID of the subagent tree.
    pub tree_root_run_id: TurnRunId,
    /// The run ID of the awaited child.
    pub child_run_id: TurnRunId,
    /// The thread ID within the child's scope.
    pub child_thread_id: String,
    /// JSON-encoded `TurnScope` (child scope).
    pub child_scope_json: String,
    /// JSON-encoded `LoopRunContext` (parent run context).
    ///
    /// MUST be stripped of sensitive fields (credentials, secrets) before
    /// constructing this struct — see the credential audit. The raw
    /// `LoopRunContext` is never stored directly.
    pub parent_run_context_json: String,
    /// Binding ref for the source capability that spawned this child.
    pub source_binding_ref: String,
    /// Binding ref for the reply-target slot where the child's result lands.
    pub reply_target_binding_ref: String,
    /// Kind tag for the subagent (e.g. `"loop"`, `"tool_agent"`).
    pub subagent_kind: String,
    /// Capability ID of the spawn capability used to launch this child.
    pub spawn_capability_id: String,
    /// Ref identifying the capability result in the capability result store.
    pub result_ref: LoopResultRef,
    /// Spawn mode string: `"blocking"` or `"background"`.
    pub spawn_mode: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_is_stable_and_bounded() {
        let child_id = "550e8400-e29b-41d4-a716-446655440000";
        let k = CAPACITY_COUNTER_BUCKETS;
        let bucket = child_bucket(child_id, k);
        assert!(bucket < k, "bucket {bucket} must be < {k}");
        // Deterministic: same input, same output.
        assert_eq!(child_bucket(child_id, k), bucket);
    }

    #[test]
    fn buckets_distribute_across_range() {
        let k = CAPACITY_COUNTER_BUCKETS;
        let mut seen = std::collections::HashSet::new();
        for i in 0..200u32 {
            let id = format!("child-run-{i:08x}-beef-dead-cafe-000000000000");
            seen.insert(child_bucket(&id, k));
        }
        // With 200 inputs and K=16, expect reasonable coverage (> 10 distinct buckets).
        assert!(
            seen.len() > 10,
            "expected spread across buckets, got {seen:?}"
        );
    }

    #[test]
    fn effective_capacity_counter_buckets_defaults_to_constant() {
        // Test without the env var set.
        if std::env::var(CAPACITY_COUNTER_BUCKETS_ENV).is_err() {
            assert_eq!(
                effective_capacity_counter_buckets(),
                CAPACITY_COUNTER_BUCKETS
            );
        }
    }
}
