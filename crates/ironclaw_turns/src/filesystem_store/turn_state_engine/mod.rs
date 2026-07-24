//! The turn-state semantics engine (`TurnStateEngine`).
//!
//! This is the single, private execution core of turn-state semantics —
//! admission, lifecycle, spawn-tree, run-transition, checkpoint and event
//! projection logic — materialized transiently inside the
//! [`FilesystemTurnStateRowStore`](super::row_store::FilesystemTurnStateRowStore)
//! `apply` closure from the durable rows. It is NOT a store: it has no
//! durability, no persistence backend, and is not exported from the crate.
//! The one public turn-state store is `FilesystemTurnStateRowStore`.
//!
//! Only [`TurnStateStoreLimits`] (the row store's config) is re-exported
//! publicly from the crate root; the engine type stays `pub(crate)`.
use async_trait::async_trait;
use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    hash::Hash,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{Mutex as AsyncMutex, Notify};

use chrono::Utc;
use ironclaw_host_api::{TenantId, UserId};

use crate::{
    AcceptedMessageRef, AdmissionRejection, AdmissionRejectionReason,
    AllowAllTurnAdmissionLimitProvider, BlockedReason, CancelRunRequest, CancelRunResponse,
    GetLoopCheckpointRequest, GetRunStateRequest, IdempotencyKey, LoopCheckpointRecord,
    LoopCheckpointStore, LoopExitMapping, PutLoopCheckpointRequest, ReplyTargetBindingRef,
    ResumeTurnRequest, ResumeTurnResponse, RetryTurnRequest, RetryTurnResponse,
    RunProfileResolutionError, RunProfileResolutionRequest, RunProfileResolver, SanitizedFailure,
    SourceBindingRef, SpawnTreeReservation, SpawnTreeReservationKey, SubmitChildRunRequest,
    SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnActiveLockKey, TurnActiveLockRecord,
    TurnActor, TurnAdmissionClass, TurnAdmissionLimitProvider, TurnAdmissionPolicy,
    TurnAdmissionReservationRecord, TurnCapacityResource, TurnCheckpointId, TurnCheckpointRecord,
    TurnError, TurnEventKind, TurnIdempotencyErrorReplay, TurnIdempotencyOperationKind,
    TurnIdempotencyOutcomeKind, TurnIdempotencyRecord, TurnIdempotencyReplay, TurnLeaseToken,
    TurnLifecycleEvent, TurnLockVersion, TurnPersistenceSnapshot, TurnRecord, TurnRunId,
    TurnRunProfile, TurnRunRecord, TurnRunState, TurnScope, TurnSpawnTreeStateStore,
    TurnStateStore, TurnStatus,
    admission::{TurnAdmissionBucket, admission_buckets},
    events::{EventCursor, TurnEventPage, TurnEventProjectionSource, project_turn_events},
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimRunsRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest,
        HeartbeatRequest, RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest,
        RecoverExpiredLeasesRequest, RecoverExpiredLeasesResponse, RelinquishRunRequest,
        TurnRunTransitionPort, TurnRunnerOutcome,
    },
};

mod run_status_cell {
    use crate::TurnStatus;

    /// The sole owner of a run's status. The only way to change it is `set`,
    /// which returns a `#[must_use]` receipt — so no status transition can skip
    /// concurrency-slot accounting. A new exit point physically cannot compile
    /// without consuming the receipt.
    #[derive(Debug, Clone)]
    pub(super) struct RunStatusCell(TurnStatus);

    #[must_use = "a status transition may change a concurrency slot; pass it to apply_status_transition"]
    pub(super) enum StatusTransition {
        EnteredRunning,
        LeftRunning,
        Unchanged,
    }

    impl RunStatusCell {
        pub(super) fn new(status: TurnStatus) -> Self {
            Self(status)
        }
        pub(super) fn get(&self) -> TurnStatus {
            self.0
        }
        pub(super) fn set(&mut self, new: TurnStatus) -> StatusTransition {
            let held_slot = super::holds_running_slot(self.0);
            let new_holds_slot = super::holds_running_slot(new);
            self.0 = new;
            match (held_slot, new_holds_slot) {
                (false, true) => StatusTransition::EnteredRunning,
                (true, false) => StatusTransition::LeftRunning,
                _ => StatusTransition::Unchanged,
            }
        }
    }
}
use run_status_cell::{RunStatusCell, StatusTransition};

mod concurrency_limiter;
use concurrency_limiter::{ConcurrencyLimiter, ConcurrencyLimits, OriginClass, RunSlotInfo};

mod admission;
mod idempotency;
mod limits;
mod run_record;
mod snapshot;
mod spawn_tree;
mod transitions;

pub use limits::TurnStateStoreLimits;
use run_record::slot_info_for;

/// Default runner-lease TTL in seconds. A claimed turn's lease expires this
/// many seconds after it is taken or last renewed; an unrenewed lease is
/// reclaimed by `recover_expired_leases`. Primary model-call timeouts must stay
/// below this value (enforced by an invariant test in `run_profile::model`) so
/// a hung provider surfaces as a retryable error before the lease reclaims the
/// runner mid-flight.
pub(crate) const DEFAULT_RUNNER_LEASE_TTL_SECONDS: i64 = 90;

fn holds_running_slot(status: TurnStatus) -> bool {
    matches!(status, TurnStatus::Running | TurnStatus::CancelRequested)
}

pub(crate) struct TurnStateEngine {
    inner: Mutex<Inner>,
    submit_idempotency_ready: Notify,
    admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
    /// Optional durable sink for gate-blocked turns. When set, the store
    /// persists the snapshot on blocked-set changes only (block / resume /
    /// terminate-while-blocked) so a restart can recover parked-on-human turns.
    /// `None` = pure in-memory (the default; used by tests and the stress tool).
    // arch-exempt: optional_arc, genuinely optional — durability is only wired
    // in the single-tenant-volume feature path; no-DB/test/stress builds run
    // without it, plan #5486
    block_persistence: Option<Arc<dyn crate::TurnStateBlockPersistence>>,
    /// Serializes the durable *write* (not the snapshot clone) in
    /// [`Self::persist_blocked_state`] and guards the `last_persisted_seq`
    /// compare-and-store, so concurrent blocked-set changes cannot let an older
    /// snapshot blind-overwrite a newer one. Held only around the sink write,
    /// which fires only on a blocked-set change (off the hot path).
    persist_lock: AsyncMutex<()>,
    /// Monotonic sequence assigned to each block-persistence snapshot at capture
    /// time (under the inner lock, so sequence order matches snapshot order).
    persist_seq: AtomicU64,
    /// Highest sequence already durably written. A persist whose sequence is
    /// below this is stale — a newer (superset) snapshot already landed — so it
    /// skips its write instead of blind-overwriting. This also coalesces a burst
    /// of concurrent blocked-set changes down to the latest snapshot.
    last_persisted_seq: AtomicU64,
    /// Runs that have an outstanding gate-persisted snapshot and therefore still
    /// need a durable *terminal* write. A run enters on block, stays across
    /// resume (its durable state is now live, not terminal), and is cleared when
    /// it reaches a terminal state — at which point we persist once more so the
    /// durable snapshot converges to the terminal state and a restart does not
    /// rehydrate an already-finished run as live.
    gate_persisted_runs: Mutex<HashSet<TurnRunId>>,
}

impl Default for TurnStateEngine {
    fn default() -> Self {
        Self::with_limits(TurnStateStoreLimits::default())
    }
}

/// In-memory value for `Inner::tree_reservations`. `released_children` is
/// the durable dedup record `release_tree_descendants`'s `idempotency_key`
/// checks before decrementing `count` — see `SpawnTreeReservation`'s
/// doc-comment (store.rs) for the full rationale.
#[derive(Debug, Clone, Default)]
struct TreeReservationState {
    count: u64,
    released_children: BTreeSet<TurnRunId>,
}

#[derive(Default)]
struct Inner {
    cursor: u64,
    turns: HashMap<crate::TurnId, TurnRecord>,
    records: HashMap<TurnRunId, RunRecord>,
    queued_runs: VecDeque<TurnRunId>,
    terminal_runs: VecDeque<TurnRunId>,
    active_locks: HashMap<TurnActiveLockKey, TurnActiveLockRecord>,
    checkpoints: Vec<TurnCheckpointRecord>,
    loop_checkpoints: HashMap<TurnCheckpointId, LoopCheckpointRecord>,
    submit_idempotency: HashMap<SubmitIdempotencyKey, Result<SubmitTurnResponse, TurnError>>,
    submit_idempotency_in_flight: HashSet<SubmitIdempotencyKey>,
    resume_idempotency: HashMap<RunIdempotencyKey, Result<ResumeTurnResponse, TurnError>>,
    retry_idempotency: HashMap<RunIdempotencyKey, Result<RetryTurnResponse, TurnError>>,
    cancel_idempotency: HashMap<RunIdempotencyKey, Result<CancelRunResponse, TurnError>>,
    idempotency_records: HashMap<PersistedIdempotencyKey, TurnIdempotencyRecord>,
    submit_idempotency_order: VecDeque<SubmitIdempotencyKey>,
    resume_idempotency_order: VecDeque<RunIdempotencyKey>,
    retry_idempotency_order: VecDeque<RunIdempotencyKey>,
    cancel_idempotency_order: VecDeque<RunIdempotencyKey>,
    idempotency_record_order: VecDeque<PersistedIdempotencyKey>,
    events: Vec<TurnLifecycleEvent>,
    event_retention_floor: EventCursor,
    admission_reservations: HashMap<TurnRunId, TurnAdmissionReservationRecord>,
    tree_reservations: HashMap<SpawnTreeReservationKey, TreeReservationState>,
    limits: TurnStateStoreLimits,
    concurrency: ConcurrencyLimiter,
}

#[derive(Debug, Clone)]
struct RunRecord {
    scope: TurnScope,
    actor: TurnActor,
    turn_id: crate::TurnId,
    run_id: TurnRunId,
    status: RunStatusCell,
    profile: TurnRunProfile,
    resolved_model_route: Option<crate::run_profile::LoopModelRouteSnapshot>,
    model_usage: Option<crate::run_profile::LoopModelUsage>,
    accepted_message_ref: AcceptedMessageRef,
    source_binding_ref: SourceBindingRef,
    reply_target_binding_ref: ReplyTargetBindingRef,
    checkpoint_id: Option<TurnCheckpointId>,
    gate_ref: Option<crate::GateRef>,
    blocked_activity_id: Option<crate::CapabilityActivityId>,
    credential_requirements: Vec<ironclaw_host_api::RuntimeCredentialAuthRequirement>,
    failure: Option<SanitizedFailure>,
    event_cursor: EventCursor,
    runner_id: Option<crate::TurnRunnerId>,
    lease_token: Option<crate::TurnLeaseToken>,
    lease_expires_at: Option<crate::TurnTimestamp>,
    last_heartbeat_at: Option<crate::TurnTimestamp>,
    claim_count: u64,
    received_at: crate::TurnTimestamp,
    parent_run_id: Option<TurnRunId>,
    subagent_depth: u32,
    spawn_tree_root_run_id: Option<TurnRunId>,
    product_context: Option<crate::ProductTurnContext>,
    resume_disposition: Option<crate::GateResumeDisposition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SubmitIdempotencyKey {
    scope: TurnScope,
    key: IdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RunIdempotencyKey {
    scope: TurnScope,
    run_id: TurnRunId,
    key: IdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PersistedIdempotencyKey {
    scope: TurnScope,
    operation: TurnIdempotencyOperationKind,
    run_id: Option<TurnRunId>,
    key: IdempotencyKey,
}

fn profile_resolution_error_to_turn_error(error: RunProfileResolutionError) -> TurnError {
    let reason = match error {
        RunProfileResolutionError::Unauthorized { .. } => AdmissionRejectionReason::Unauthorized,
        RunProfileResolutionError::ProfileUnavailable { .. }
        | RunProfileResolutionError::InvalidRequest { .. } => {
            AdmissionRejectionReason::ProfileRejected
        }
    };
    TurnError::AdmissionRejected(AdmissionRejection::new(reason))
}

fn fresh_turn_run_id() -> TurnRunId {
    TurnRunId::new()
}

fn same_scope_envelope(candidate: &TurnScope, scope: &TurnScope) -> bool {
    candidate.tenant_id == scope.tenant_id
        && candidate.agent_id == scope.agent_id
        && candidate.project_id == scope.project_id
}

fn invalid_lineage(reason: impl Into<String>) -> TurnError {
    TurnError::InvalidRequest {
        reason: reason.into(),
    }
}

struct SubmitInFlightGuard<'a> {
    inner: &'a Mutex<Inner>,
    ready: &'a Notify,
    key: SubmitIdempotencyKey,
}

impl<'a> SubmitInFlightGuard<'a> {
    fn new(inner: &'a Mutex<Inner>, ready: &'a Notify, key: SubmitIdempotencyKey) -> Self {
        Self { inner, ready, key }
    }
}

impl Drop for SubmitInFlightGuard<'_> {
    fn drop(&mut self) {
        let removed = match self.inner.lock() {
            Ok(mut inner) => inner.submit_idempotency_in_flight.remove(&self.key),
            Err(poisoned) => poisoned
                .into_inner()
                .submit_idempotency_in_flight
                .remove(&self.key),
        };
        if removed {
            self.ready.notify_waiters();
        }
    }
}

impl TurnStateEngine {
    pub(crate) fn with_limits(limits: TurnStateStoreLimits) -> Self {
        Self {
            inner: Mutex::new(Inner {
                concurrency: ConcurrencyLimiter::with_limits(ConcurrencyLimits {
                    max_concurrent_runs_per_user: limits.max_concurrent_runs_per_user,
                    max_concurrent_trigger_runs: limits.max_concurrent_trigger_runs,
                    max_concurrent_conversation_runs: limits.max_concurrent_conversation_runs,
                }),
                limits,
                ..Inner::default()
            }),
            submit_idempotency_ready: Notify::new(),
            admission_limit_provider: Arc::new(AllowAllTurnAdmissionLimitProvider),
            block_persistence: None,
            persist_lock: AsyncMutex::new(()),
            persist_seq: AtomicU64::new(0),
            last_persisted_seq: AtomicU64::new(0),
            gate_persisted_runs: Mutex::new(HashSet::new()),
        }
    }

    #[allow(dead_code)] // preserved engine API, in-crate-only post-#6263
    pub(crate) fn with_admission_limit_provider(
        admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
    ) -> Self {
        Self::with_limits_and_admission_limit_provider(
            TurnStateStoreLimits::default(),
            admission_limit_provider,
        )
    }

    pub(crate) fn with_limits_and_admission_limit_provider(
        limits: TurnStateStoreLimits,
        admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
    ) -> Self {
        Self {
            inner: Mutex::new(Inner {
                concurrency: ConcurrencyLimiter::with_limits(ConcurrencyLimits {
                    max_concurrent_runs_per_user: limits.max_concurrent_runs_per_user,
                    max_concurrent_trigger_runs: limits.max_concurrent_trigger_runs,
                    max_concurrent_conversation_runs: limits.max_concurrent_conversation_runs,
                }),
                limits,
                ..Inner::default()
            }),
            submit_idempotency_ready: Notify::new(),
            admission_limit_provider,
            block_persistence: None,
            persist_lock: AsyncMutex::new(()),
            persist_seq: AtomicU64::new(0),
            last_persisted_seq: AtomicU64::new(0),
            gate_persisted_runs: Mutex::new(HashSet::new()),
        }
    }

    pub(crate) fn active_admission_reservations(&self) -> Vec<TurnAdmissionReservationRecord> {
        match self.inner.lock() {
            Ok(inner) => inner.active_admission_reservations(),
            Err(poisoned) => poisoned.into_inner().active_admission_reservations(),
        }
    }

    pub(crate) fn events(&self) -> Vec<TurnLifecycleEvent> {
        match self.inner.lock() {
            Ok(inner) => inner.events.clone(),
            Err(poisoned) => poisoned.into_inner().events.clone(),
        }
    }

    pub(crate) fn events_after(&self, cursor: EventCursor) -> Vec<TurnLifecycleEvent> {
        match self.inner.lock() {
            Ok(inner) => inner
                .events
                .iter()
                .filter(|event| event.cursor > cursor)
                .cloned()
                .collect(),
            Err(poisoned) => poisoned
                .into_inner()
                .events
                .iter()
                .filter(|event| event.cursor > cursor)
                .cloned()
                .collect(),
        }
    }

    pub(crate) fn event_retention_floor(&self) -> EventCursor {
        match self.inner.lock() {
            Ok(inner) => inner.event_retention_floor,
            Err(poisoned) => poisoned.into_inner().event_retention_floor,
        }
    }

    pub(crate) fn turn_record(&self, turn_id: crate::TurnId) -> Option<TurnRecord> {
        match self.inner.lock() {
            Ok(inner) => inner.turns.get(&turn_id).cloned(),
            Err(poisoned) => poisoned.into_inner().turns.get(&turn_id).cloned(),
        }
    }

    pub(crate) fn run_record(&self, run_id: TurnRunId) -> Option<TurnRunRecord> {
        match self.inner.lock() {
            Ok(inner) => inner
                .records
                .get(&run_id)
                .map(RunRecord::persistence_record),
            Err(poisoned) => poisoned
                .into_inner()
                .records
                .get(&run_id)
                .map(RunRecord::persistence_record),
        }
    }

    pub(crate) fn overlay_runner_lease_record(
        &self,
        overlaid: TurnRunRecord,
    ) -> Result<(), TurnError> {
        let mut inner = self.lock_inner()?;
        let Some(record) = inner.records.get_mut(&overlaid.run_id) else {
            return Err(TurnError::ScopeNotFound);
        };
        if !matches!(
            record.status.get(),
            TurnStatus::Running | TurnStatus::CancelRequested
        ) || record.runner_id != overlaid.runner_id
            || record.lease_token != overlaid.lease_token
            || record.runner_id.is_none()
            || record.lease_token.is_none()
        {
            return Ok(());
        }
        if let (Some(current), Some(incoming)) =
            (record.last_heartbeat_at, overlaid.last_heartbeat_at)
            && incoming < current
        {
            return Ok(());
        }
        record.last_heartbeat_at = overlaid.last_heartbeat_at;
        record.lease_expires_at = overlaid.lease_expires_at;
        Ok(())
    }

    pub(crate) fn active_lock_record(&self, scope: &TurnScope) -> Option<TurnActiveLockRecord> {
        let key = TurnActiveLockKey::from(scope);
        match self.inner.lock() {
            Ok(inner) => inner.active_locks.get(&key).cloned(),
            Err(poisoned) => poisoned.into_inner().active_locks.get(&key).cloned(),
        }
    }

    pub(crate) fn admission_reservation(
        &self,
        run_id: TurnRunId,
    ) -> Option<TurnAdmissionReservationRecord> {
        match self.inner.lock() {
            Ok(inner) => inner.admission_reservations.get(&run_id).cloned(),
            Err(poisoned) => poisoned
                .into_inner()
                .admission_reservations
                .get(&run_id)
                .cloned(),
        }
    }

    pub(crate) fn checkpoint_record(
        &self,
        checkpoint_id: TurnCheckpointId,
    ) -> Option<TurnCheckpointRecord> {
        match self.inner.lock() {
            Ok(inner) => inner
                .checkpoints
                .iter()
                .find(|record| record.checkpoint_id == checkpoint_id)
                .cloned(),
            Err(poisoned) => poisoned
                .into_inner()
                .checkpoints
                .iter()
                .find(|record| record.checkpoint_id == checkpoint_id)
                .cloned(),
        }
    }

    pub(crate) fn idempotency_records_for_run_operation(
        &self,
        run_id: TurnRunId,
        operation: TurnIdempotencyOperationKind,
    ) -> Vec<TurnIdempotencyRecord> {
        match self.inner.lock() {
            Ok(inner) => inner
                .idempotency_records
                .values()
                .filter(|record| record.run_id == Some(run_id) && record.operation == operation)
                .cloned()
                .collect(),
            Err(poisoned) => poisoned
                .into_inner()
                .idempotency_records
                .values()
                .filter(|record| record.run_id == Some(run_id) && record.operation == operation)
                .cloned()
                .collect(),
        }
    }

    #[allow(dead_code)] // preserved engine API, in-crate-only post-#6263
    pub(crate) fn from_persistence_snapshot(
        snapshot: TurnPersistenceSnapshot,
        limits: TurnStateStoreLimits,
    ) -> Result<Self, TurnError> {
        Self::from_persistence_snapshot_with_admission_limit_provider(
            snapshot,
            limits,
            Arc::new(AllowAllTurnAdmissionLimitProvider),
        )
    }

    pub(crate) fn from_persistence_snapshot_with_admission_limit_provider(
        snapshot: TurnPersistenceSnapshot,
        limits: TurnStateStoreLimits,
        admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
    ) -> Result<Self, TurnError> {
        Ok(Self {
            inner: Mutex::new(Inner::from_persistence_snapshot(snapshot, limits)?),
            submit_idempotency_ready: Notify::new(),
            admission_limit_provider,
            block_persistence: None,
            persist_lock: AsyncMutex::new(()),
            persist_seq: AtomicU64::new(0),
            last_persisted_seq: AtomicU64::new(0),
            gate_persisted_runs: Mutex::new(HashSet::new()),
        })
    }

    pub(crate) fn persistence_snapshot(&self) -> TurnPersistenceSnapshot {
        match self.inner.lock() {
            Ok(inner) => inner.persistence_snapshot(),
            Err(poisoned) => poisoned.into_inner().persistence_snapshot(),
        }
    }

    /// Whether `run_id` is currently parked on a gate (any `Blocked*` status).
    /// Used to decide whether a terminal transition changed the blocked set and
    /// therefore needs a durable persist.
    fn run_is_blocked(&self, run_id: TurnRunId) -> bool {
        let inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner
            .records
            .get(&run_id)
            .is_some_and(|record| record.status.get().is_blocked())
    }

    /// Mark `run_id` as having an outstanding gate-persisted snapshot that must
    /// be durably cleaned up when the run terminates. No-op (and no lock taken)
    /// when no durable sink is attached, so the default in-memory authority keeps
    /// its exact hot-path cost.
    fn mark_gate_persisted(&self, run_id: TurnRunId) {
        if self.block_persistence.is_none() {
            return;
        }
        let mut set = match self.gate_persisted_runs.lock() {
            Ok(set) => set,
            Err(poisoned) => poisoned.into_inner(),
        };
        set.insert(run_id);
    }

    /// Whether `run_id` has an outstanding gate-persisted snapshot (blocked at
    /// least once and not yet durably cleaned up on termination). Short-circuits
    /// to `false` with no lock when no durable sink is attached.
    fn is_gate_persisted(&self, run_id: TurnRunId) -> bool {
        if self.block_persistence.is_none() {
            return false;
        }
        let set = match self.gate_persisted_runs.lock() {
            Ok(set) => set,
            Err(poisoned) => poisoned.into_inner(),
        };
        set.contains(&run_id)
    }

    /// Drop `run_id` from the gate-persisted set once its terminal state has been
    /// durably written.
    fn clear_gate_persisted(&self, run_id: TurnRunId) {
        let mut set = match self.gate_persisted_runs.lock() {
            Ok(set) => set,
            Err(poisoned) => poisoned.into_inner(),
        };
        set.remove(&run_id);
    }

    /// Capture the persistence snapshot together with a monotonic sequence,
    /// atomically under the inner lock so that sequence order matches snapshot
    /// order (a higher sequence is guaranteed to be a same-or-newer snapshot).
    fn snapshot_with_seq(&self) -> (u64, TurnPersistenceSnapshot) {
        let inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        };
        let seq = self.persist_seq.fetch_add(1, Ordering::SeqCst);
        (seq, inner.persistence_snapshot())
    }

    /// Persist the snapshot through the durable block sink, if one is attached.
    /// Off the hot path — callers invoke this only when the set of gate-persisted
    /// runs changed. Best-effort: the sink logs and swallows its own errors so a
    /// durable-write failure never fails an already-applied transition.
    ///
    /// The snapshot is cloned *without* holding `persist_lock` (so concurrent
    /// captures stay parallel), then only the write is serialized under
    /// `persist_lock` with a stale-skip guard: a snapshot whose sequence is below
    /// the highest already written is a stale superset-loser and is dropped
    /// rather than blind-overwriting the newer durable state (the sink writes with
    /// `CasExpectation::Any`, so ordering is the store's responsibility). This
    /// also coalesces a burst of concurrent blocked-set changes down to the
    /// latest snapshot instead of one durable write per change.
    async fn persist_blocked_state(&self) {
        let Some(sink) = self.block_persistence.clone() else {
            return;
        };
        let (seq, snapshot) = self.snapshot_with_seq();
        let _serialize = self.persist_lock.lock().await;
        if seq < self.last_persisted_seq.load(Ordering::SeqCst) {
            return;
        }
        self.last_persisted_seq.store(seq, Ordering::SeqCst);
        sink.persist(&snapshot).await;
    }

    /// Durably flush the full current turn-state snapshot through the block sink.
    ///
    /// Persist-on-block only writes when the gate-blocked set changes, so between
    /// gate events the durable snapshot lags the live in-memory state. On a
    /// *graceful* shutdown (a planned deploy's SIGTERM) the runtime calls this so
    /// the restart recovers **in-flight** turns too, not just gate-blocked ones —
    /// closing the "deploy silently drops in-progress turns" gap. It writes the
    /// same full snapshot `persist_blocked_state` does (going through the same
    /// sequence/stale-skip guard, so a racing gate persist cannot clobber it), and
    /// is a no-op when no durable sink is attached (the default in-memory store
    /// used by tests, no-DB builds, and the stress tool). A hard crash (SIGKILL /
    /// OOM) still loses in-flight state — bounding that needs a periodic flush,
    /// which is a separate follow-up.
    #[allow(dead_code)] // preserved engine API, in-crate-only post-#6263
    pub(crate) async fn flush(&self) {
        self.persist_blocked_state().await;
    }

    /// After a terminal transition, if `run_id` still had an outstanding
    /// gate-persisted snapshot, write once more so the durable snapshot converges
    /// to the terminal state — otherwise a run that blocked, resumed, and then
    /// completed from `Running` would leave its last durable state as
    /// `Queued`/`Running` and be rehydrated as a live run after restart. Then
    /// stop tracking it. No-op when the transition failed or the run was never
    /// gate-persisted (the plain claim/complete hot path).
    async fn persist_terminal_cleanup(
        &self,
        run_id: TurnRunId,
        gate_persisted: bool,
        result: &Result<TurnRunState, TurnError>,
    ) {
        // Only converge on an actually-terminal outcome: `complete`/`cancel`/`fail`
        // always land terminal here, but `record_runner_failure` may leave the run
        // live (retry), and clearing the tracking there would drop a later real
        // terminal write.
        if gate_persisted
            && result
                .as_ref()
                .is_ok_and(|state| state.status.is_terminal())
        {
            self.persist_blocked_state().await;
            self.clear_gate_persisted(run_id);
        }
    }

    /// Returns the number of runs currently in `TurnStatus::Running` or `TurnStatus::CancelRequested` for the given
    /// (tenant, user) pair. Intended for testing and observability.
    pub(crate) fn running_count_for_user(
        &self,
        tenant: &ironclaw_host_api::TenantId,
        user: &ironclaw_host_api::UserId,
    ) -> u32 {
        match self.inner.lock() {
            Ok(inner) => inner.concurrency.count_for_user(tenant, user),
            Err(poisoned) => poisoned
                .into_inner()
                .concurrency
                .count_for_user(tenant, user),
        }
    }

    /// Returns the number of runs currently in `TurnStatus::Running` or `TurnStatus::CancelRequested` with `ScheduledTrigger` origin
    /// for the given tenant. Intended for testing and observability.
    pub(crate) fn running_trigger_count(&self, tenant: &TenantId) -> u32 {
        match self.inner.lock() {
            Ok(inner) => inner.concurrency.count_for_trigger(tenant),
            Err(poisoned) => poisoned.into_inner().concurrency.count_for_trigger(tenant),
        }
    }

    /// Returns the number of runs currently in `TurnStatus::Running` or `TurnStatus::CancelRequested` with `Inbound` or `WebUi` origin
    /// for the given tenant. Intended for testing and observability.
    pub(crate) fn running_conversation_count(&self, tenant: &TenantId) -> u32 {
        match self.inner.lock() {
            Ok(inner) => inner.concurrency.count_for_conversation(tenant),
            Err(poisoned) => poisoned
                .into_inner()
                .concurrency
                .count_for_conversation(tenant),
        }
    }

    fn lock_inner(&self) -> Result<MutexGuard<'_, Inner>, TurnError> {
        self.inner.lock().map_err(|_| TurnError::Unavailable {
            reason: "turn state store mutex poisoned".to_string(),
        })
    }

    async fn wait_for_or_claim_submit_idempotency(
        &self,
        idempotency_key: &SubmitIdempotencyKey,
    ) -> Result<Option<Result<SubmitTurnResponse, TurnError>>, TurnError> {
        loop {
            // Subscribe to the Notify BEFORE locking so we can't miss a
            // notification that fires between our "in-flight" check and when we
            // would otherwise start waiting.
            let notified = self.submit_idempotency_ready.notified();
            {
                let mut inner = self.lock_inner()?;
                if let Some(result) = inner.submit_idempotency.get(idempotency_key) {
                    return Ok(Some(result.clone()));
                }
                if inner
                    .submit_idempotency_in_flight
                    .insert(idempotency_key.clone())
                {
                    return Ok(None);
                }
                // `inner` (the MutexGuard) is dropped here at the end of this
                // inner block, releasing the Mutex before we await below.
            }
            // Await cooperatively — no Mutex held, no OS thread blocked.
            notified.await;
        }
    }
}

#[async_trait]
impl TurnEventProjectionSource for TurnStateEngine {
    async fn read_turn_events_after(
        &self,
        scope: &TurnScope,
        owner_user_id: Option<&UserId>,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        let inner = self.lock_inner()?;
        Ok(project_turn_events(
            &inner.events,
            scope,
            owner_user_id,
            after,
            limit,
            inner.event_retention_floor,
        ))
    }

    async fn read_turn_event_log_after(
        &self,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        let inner = self.lock_inner()?;
        let after = after.unwrap_or_default();
        if inner.event_retention_floor > EventCursor::default()
            && after < inner.event_retention_floor
        {
            return Ok(TurnEventPage {
                entries: Vec::new(),
                next_cursor: inner.event_retention_floor,
                truncated: false,
                rebase_required: Some(inner.event_retention_floor),
            });
        }
        let mut entries = inner
            .events
            .iter()
            .filter(|event| event.cursor > after)
            .cloned()
            .collect::<Vec<_>>();
        entries.sort_by_key(|event| event.cursor);
        let truncated = entries.len() > limit;
        if truncated {
            entries.truncate(limit);
        }
        let next_cursor = entries.last().map_or(after, |event| event.cursor);
        Ok(TurnEventPage {
            entries,
            next_cursor,
            truncated,
            rebase_required: None,
        })
    }
}

#[async_trait]
impl LoopCheckpointStore for TurnStateEngine {
    async fn put_loop_checkpoint(
        &self,
        request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        let checkpoint_id = TurnCheckpointId::new();
        let record = LoopCheckpointRecord {
            checkpoint_id,
            scope: request.scope,
            turn_id: request.turn_id,
            run_id: request.run_id,
            state_ref: request.state_ref,
            schema_id: request.schema_id,
            schema_version: request.schema_version,
            kind: request.kind,
            gate_ref: request.gate_ref,
            created_at: Utc::now(),
        };
        let mut inner = self.lock_inner()?;
        inner.loop_checkpoints.insert(checkpoint_id, record.clone());
        Ok(record)
    }

    async fn get_loop_checkpoint(
        &self,
        request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        let inner = self.lock_inner()?;
        let Some(record) = inner.loop_checkpoints.get(&request.checkpoint_id) else {
            return Ok(None);
        };
        if record.scope == request.scope
            && record.turn_id == request.turn_id
            && record.run_id == request.run_id
            && record.checkpoint_id == request.checkpoint_id
        {
            Ok(Some(record.clone()))
        } else {
            Ok(None)
        }
    }
}

#[async_trait]
impl TurnStateStore for TurnStateEngine {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let idempotency_key = SubmitIdempotencyKey {
            scope: request.scope.clone(),
            key: request.idempotency_key.clone(),
        };
        if let Some(result) = self
            .wait_for_or_claim_submit_idempotency(&idempotency_key)
            .await?
        {
            return result;
        }
        let _in_flight_guard = SubmitInFlightGuard::new(
            &self.inner,
            &self.submit_idempotency_ready,
            idempotency_key.clone(),
        );

        if request.parent_run_id.is_some()
            || request.subagent_depth != 0
            || request.spawn_tree_root_run_id.is_some()
        {
            let mut inner = self.lock_inner()?;
            if let Some(result) = inner.submit_idempotency.get(&idempotency_key).cloned() {
                return result;
            }
            let response = Err(TurnError::InvalidRequest {
                reason: "child runs must be submitted through submit_child_turn".to_string(),
            });
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            return response;
        }

        let admission_result = admission_policy.check_submit(&request);

        {
            let mut inner = self.lock_inner()?;
            if let Some(result) = inner.submit_idempotency.get(&idempotency_key).cloned() {
                return result;
            }

            if let Err(rejection) = admission_result {
                let response = Err(TurnError::AdmissionRejected(rejection));
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                return response;
            }
        }

        let profile_resolution = run_profile_resolver
            .resolve_run_profile(RunProfileResolutionRequest {
                requested_run_profile: request.requested_run_profile.clone(),
                ..RunProfileResolutionRequest::interactive_default()
            })
            .await;

        let mut inner = self.lock_inner()?;
        if let Some(result) = inner.submit_idempotency.get(&idempotency_key).cloned() {
            return result;
        }
        let profile = match profile_resolution {
            Ok(resolved) => TurnRunProfile::from_resolved(resolved),
            Err(error) => {
                let response = Err(profile_resolution_error_to_turn_error(error));
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                return response;
            }
        };

        let lock_key = TurnActiveLockKey::from(&request.scope);
        if let Some(response) = inner.thread_busy(&lock_key) {
            return Err(TurnError::ThreadBusy(response));
        }

        let turn_id = crate::TurnId::new();
        let run_id = request.requested_run_id.unwrap_or_else(fresh_turn_run_id);
        if inner.records.contains_key(&run_id) {
            let response = Err(TurnError::Conflict {
                reason: "requested_run_id already bound".to_string(),
            });
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            return response;
        }
        let admission_class = profile.admission_class.clone();
        if let Err(rejection) = inner.reserve_admission(
            run_id,
            admission_class.clone(),
            &request.scope,
            &request.actor,
            self.admission_limit_provider.as_ref(),
        ) {
            let response = Err(TurnError::AdmissionRejected(rejection));
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            return response;
        }
        let cursor = inner.next_cursor();
        let turn_record = TurnRecord {
            turn_id,
            scope: request.scope.clone(),
            actor: request.actor.clone(),
            accepted_message_ref: request.accepted_message_ref.clone(),
            source_binding_ref: request.source_binding_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            created_at: request.received_at,
        };
        let mut record = RunRecord::queued(QueuedRunFields {
            scope: request.scope.clone(),
            actor: request.actor,
            turn_id,
            run_id,
            profile: profile.clone(),
            accepted_message_ref: request.accepted_message_ref.clone(),
            source_binding_ref: request.source_binding_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            event_cursor: cursor,
            received_at: request.received_at,
        });
        record.resolved_model_route = request
            .requested_model
            .as_deref()
            .and_then(crate::run_profile::LoopModelRouteSnapshot::advisory);
        record.product_context = request.product_context;
        inner.turns.insert(turn_id, turn_record);
        inner.active_locks.insert(
            lock_key.clone(),
            TurnActiveLockRecord {
                key: lock_key,
                run_id,
                status: TurnStatus::Queued,
                lock_version: TurnLockVersion::new(1),
                acquired_at: request.received_at,
                updated_at: request.received_at,
            },
        );
        inner.queued_runs.push_back(run_id);
        inner.records.insert(run_id, record.clone());
        inner.push_event(&record, TurnEventKind::Submitted, None, None);

        let response = Ok(SubmitTurnResponse::Accepted {
            turn_id,
            run_id,
            status: TurnStatus::Queued,
            resolved_run_profile_id: profile.id,
            resolved_run_profile_version: profile.version,
            event_cursor: cursor,
            accepted_message_ref: request.accepted_message_ref,
            reply_target_binding_ref: request.reply_target_binding_ref,
        });
        inner.remember_submit_idempotency(
            idempotency_key.clone(),
            response.clone(),
            record.received_at,
        );
        response
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let mut did_resume = false;
        let result = {
            let mut inner = self.lock_inner()?;
            let idempotency_key = RunIdempotencyKey {
                scope: request.scope.clone(),
                run_id: request.run_id,
                key: request.idempotency_key.clone(),
            };
            if let Some(replayed) = inner.resume_idempotency.get(&idempotency_key) {
                // Idempotent replay — no state change, so no durable write.
                replayed.clone()
            } else {
                let result = inner.resume_turn_once(&request);
                inner.remember_resume_idempotency(idempotency_key, result.clone(), Utc::now());
                did_resume = result.is_ok();
                result
            }
        };
        // A gate-blocked run just left the blocked set — persist so the durable
        // snapshot no longer replays it as blocked on restart.
        if did_resume {
            self.persist_blocked_state().await;
        }
        result
    }

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        let mut inner = self.lock_inner()?;
        let idempotency_key = RunIdempotencyKey {
            scope: request.scope.clone(),
            run_id: request.run_id,
            key: request.idempotency_key.clone(),
        };
        if let Some(result) = inner.retry_idempotency.get(&idempotency_key) {
            return result.clone();
        }
        let result = inner.retry_turn_once(&request, self.admission_limit_provider.as_ref());
        inner.remember_retry_idempotency(idempotency_key, result.clone(), Utc::now());
        result
    }

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        let mut inner = self.lock_inner()?;
        let idempotency_key = RunIdempotencyKey {
            scope: request.scope.clone(),
            run_id: request.run_id,
            key: request.idempotency_key.clone(),
        };
        if let Some(result) = inner.cancel_idempotency.get(&idempotency_key) {
            return result.clone();
        }
        let result = inner.request_cancel_once(&request);
        inner.remember_cancel_idempotency(idempotency_key, result.clone(), Utc::now());
        result
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        let inner = self.lock_inner()?;
        inner
            .records
            .get(&request.run_id)
            .filter(|record| record.scope == request.scope)
            .map(RunRecord::state)
            .ok_or(TurnError::ScopeNotFound)
    }
}

impl Inner {
    fn next_cursor(&mut self) -> EventCursor {
        self.cursor = self.cursor.saturating_add(1);
        EventCursor(self.cursor)
    }

    fn next_lease_expiry(&self, now: crate::TurnTimestamp) -> crate::TurnTimestamp {
        now.checked_add_signed(self.limits.runner_lease_ttl)
            .unwrap_or(now)
    }

    fn push_event(
        &mut self,
        record: &RunRecord,
        kind: TurnEventKind,
        sanitized_reason: Option<String>,
        detail: Option<String>,
    ) {
        let blocked_gate = if kind == TurnEventKind::Blocked {
            record.gate_ref.clone().and_then(|gate_ref| {
                crate::events::TurnBlockedGateKind::from_status(record.status.get()).map(
                    |gate_kind| crate::events::TurnBlockedGateMetadata {
                        gate_ref,
                        gate_kind,
                        activity_id: record.blocked_activity_id,
                        credential_requirements: record.credential_requirements.clone(),
                    },
                )
            })
        } else {
            None
        };
        let retryable = (kind == TurnEventKind::Failed).then(|| self.failed_run_retryable(record));
        self.events.push(TurnLifecycleEvent {
            cursor: record.event_cursor,
            scope: record.scope.clone(),
            occurred_at: Some(Utc::now()),
            owner_user_id: crate::events::lifecycle_owner_user_id(
                &record.scope,
                Some(&record.actor.user_id),
            ),
            run_id: record.run_id,
            status: record.status.get(),
            kind,
            blocked_gate,
            sanitized_reason,
            retryable,
            detail,
        });
        if self.events.len() > self.limits.max_events {
            let excess = self.events.len() - self.limits.max_events;
            if let Some(last_pruned) = self.events.get(excess.saturating_sub(1)) {
                self.event_retention_floor = self.event_retention_floor.max(last_pruned.cursor);
            }
            self.events.drain(0..excess);
        }
    }

    fn take_record(&mut self, run_id: TurnRunId) -> Result<RunRecord, TurnError> {
        self.records.remove(&run_id).ok_or(TurnError::ScopeNotFound)
    }

    fn apply_status_transition(&mut self, transition: StatusTransition, record: &RunRecord) {
        match transition {
            StatusTransition::EnteredRunning => {
                self.concurrency.on_enter_running(slot_info_for(record));
            }
            StatusTransition::LeftRunning => {
                self.concurrency.on_leave_running(slot_info_for(record));
            }
            StatusTransition::Unchanged => {}
        }
    }
}

/// The per-call fields of a freshly-submitted [`RunRecord`]. Everything else
/// on a new run has a fixed initial value ([`RunRecord::queued`] fills it in),
/// so a new lease/gate/checkpoint field added to `RunRecord` gets its
/// fresh-run default in exactly one place instead of at every submit site.
struct QueuedRunFields {
    scope: TurnScope,
    actor: TurnActor,
    turn_id: crate::TurnId,
    run_id: TurnRunId,
    profile: TurnRunProfile,
    accepted_message_ref: AcceptedMessageRef,
    source_binding_ref: SourceBindingRef,
    reply_target_binding_ref: ReplyTargetBindingRef,
    event_cursor: EventCursor,
    received_at: crate::TurnTimestamp,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AllowAllTurnAdmissionPolicy, ResolvedRunProfile, RunProfileId, RunProfileVersion,
        TurnLeaseToken, TurnRunnerId,
    };
    use async_trait::async_trait;
    use chrono::Duration as ChronoDuration;
    use ironclaw_host_api::{AgentId, ProjectId, ThreadId};

    struct TestRunProfileResolver;

    #[async_trait]
    impl RunProfileResolver for TestRunProfileResolver {
        async fn resolve_run_profile(
            &self,
            _request: RunProfileResolutionRequest,
        ) -> Result<ResolvedRunProfile, RunProfileResolutionError> {
            Ok(ResolvedRunProfile::legacy_compatibility(
                RunProfileId::default_profile(),
                RunProfileVersion::new(1),
                false,
            ))
        }
    }

    #[tokio::test]
    async fn terminal_pruning_removes_orphaned_turn_records() {
        let limits = TurnStateStoreLimits::default().set_max_terminal_records(1);
        let store = TurnStateEngine::with_limits(limits);
        let policy = AllowAllTurnAdmissionPolicy;
        let resolver = TestRunProfileResolver;
        let scope = TurnScope::new(
            TenantId::new("tenant-turn-prune").unwrap(),
            Some(AgentId::new("agent-turn-prune").unwrap()),
            Some(ProjectId::new("project-turn-prune").unwrap()),
            ThreadId::new("thread-turn-prune").unwrap(),
        );

        for index in 0..2 {
            let response = store
                .submit_turn(
                    SubmitTurnRequest {
                        requested_model: None,
                        scope: scope.clone(),
                        actor: TurnActor::new(UserId::new(format!("user-{index}")).unwrap()),
                        accepted_message_ref: AcceptedMessageRef::new(format!("accepted-{index}"))
                            .unwrap(),
                        source_binding_ref: SourceBindingRef::new(format!("source-{index}"))
                            .unwrap(),
                        reply_target_binding_ref: ReplyTargetBindingRef::new(format!(
                            "reply-{index}"
                        ))
                        .unwrap(),
                        idempotency_key: IdempotencyKey::new(format!("submit-{index}")).unwrap(),
                        requested_run_profile: None,
                        requested_run_id: None,
                        received_at: Utc::now(),
                        parent_run_id: None,
                        subagent_depth: 0,
                        spawn_tree_root_run_id: None,
                        product_context: None,
                    },
                    &policy,
                    &resolver,
                )
                .await
                .unwrap();
            let SubmitTurnResponse::Accepted { run_id, .. } = response;
            let runner_id = TurnRunnerId::new();
            let lease_token = TurnLeaseToken::new();
            let claimed = store
                .claim_next_run(ClaimRunRequest {
                    runner_id,
                    lease_token,
                    scope_filter: Some(scope.clone()),
                })
                .await
                .unwrap()
                .expect("submitted run should be claimable");
            assert_eq!(claimed.state.run_id, run_id);
            store
                .complete_run(CompleteRunRequest {
                    run_id,
                    runner_id,
                    lease_token,
                })
                .await
                .unwrap();
        }

        let snapshot = store.persistence_snapshot();
        assert_eq!(
            snapshot.runs.len(),
            1,
            "terminal run retention cap should prune old terminal runs"
        );
        assert_eq!(
            snapshot.turns.len(),
            1,
            "pruning a terminal run must also prune its orphaned turn record"
        );
        assert_eq!(
            snapshot.turns[0].turn_id, snapshot.runs[0].turn_id,
            "remaining turn record should belong to the retained run"
        );
    }

    #[tokio::test]
    async fn overlay_runner_lease_record_ignores_stale_heartbeat() {
        let store = TurnStateEngine::default();
        let policy = AllowAllTurnAdmissionPolicy;
        let resolver = TestRunProfileResolver;
        let scope = TurnScope::new(
            TenantId::new("tenant-lease-overlay").unwrap(),
            Some(AgentId::new("agent-lease-overlay").unwrap()),
            Some(ProjectId::new("project-lease-overlay").unwrap()),
            ThreadId::new("thread-lease-overlay").unwrap(),
        );
        let response = store
            .submit_turn(
                SubmitTurnRequest {
                    requested_model: None,
                    scope: scope.clone(),
                    actor: TurnActor::new(UserId::new("user-lease-overlay").unwrap()),
                    accepted_message_ref: AcceptedMessageRef::new("accepted-lease-overlay")
                        .unwrap(),
                    source_binding_ref: SourceBindingRef::new("source-lease-overlay").unwrap(),
                    reply_target_binding_ref: ReplyTargetBindingRef::new("reply-lease-overlay")
                        .unwrap(),
                    idempotency_key: IdempotencyKey::new("submit-lease-overlay").unwrap(),
                    requested_run_profile: None,
                    requested_run_id: None,
                    received_at: Utc::now(),
                    parent_run_id: None,
                    subagent_depth: 0,
                    spawn_tree_root_run_id: None,
                    product_context: None,
                },
                &policy,
                &resolver,
            )
            .await
            .unwrap();
        let SubmitTurnResponse::Accepted { run_id, .. } = response;
        store
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: Some(scope),
            })
            .await
            .unwrap()
            .expect("submitted run should be claimable");
        let original = store.run_record(run_id).expect("claimed run record");
        let original_heartbeat = original
            .last_heartbeat_at
            .expect("claimed run has heartbeat timestamp");
        let original_expiry = original
            .lease_expires_at
            .expect("claimed run has lease expiry");

        let mut stale = original.clone();
        stale.last_heartbeat_at = Some(original_heartbeat - ChronoDuration::seconds(1));
        stale.lease_expires_at = Some(original_expiry - ChronoDuration::seconds(1));
        store.overlay_runner_lease_record(stale).unwrap();
        let after_stale = store.run_record(run_id).expect("claimed run record");
        assert_eq!(after_stale.last_heartbeat_at, Some(original_heartbeat));
        assert_eq!(after_stale.lease_expires_at, Some(original_expiry));

        let mut newer = original.clone();
        let newer_heartbeat = original_heartbeat + ChronoDuration::seconds(1);
        let newer_expiry = original_expiry + ChronoDuration::seconds(1);
        newer.last_heartbeat_at = Some(newer_heartbeat);
        newer.lease_expires_at = Some(newer_expiry);
        store.overlay_runner_lease_record(newer).unwrap();
        let after_newer = store.run_record(run_id).expect("claimed run record");
        assert_eq!(after_newer.last_heartbeat_at, Some(newer_heartbeat));
        assert_eq!(after_newer.lease_expires_at, Some(newer_expiry));
    }
}
