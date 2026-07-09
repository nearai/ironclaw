// arch-exempt: large_file, in-memory turn state decomposition, plan #5662
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

use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_host_api::{TenantId, UserId};

use crate::{
    AcceptedMessageRef, AdmissionRejection, AdmissionRejectionReason,
    AllowAllTurnAdmissionLimitProvider, BlockedReason, CancelRunRequest, CancelRunResponse,
    GateRef, GetLoopCheckpointRequest, GetRunStateRequest, IdempotencyKey, LoopCheckpointRecord,
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

/// Resolve the secret-scrubbed, model-visible detail to attach to a lifecycle
/// event. Only `Failed` events carry the failure record's detail; every other
/// event kind has no failure cause and yields `None`.
fn failure_detail_for_event(
    kind: &TurnEventKind,
    failure: Option<&SanitizedFailure>,
) -> Option<String> {
    (*kind == TurnEventKind::Failed)
        .then(|| failure.and_then(|failure| failure.detail().map(str::to_string)))
        .flatten()
}

fn holds_running_slot(status: TurnStatus) -> bool {
    matches!(status, TurnStatus::Running | TurnStatus::CancelRequested)
}

const MAX_EVENTS: usize = 10_000;
const MAX_TERMINAL_RECORDS: usize = 10_000;
const MAX_IDEMPOTENCY_RECORDS: usize = 10_000;
/// Default runner-lease TTL in seconds. A claimed turn's lease expires this
/// many seconds after it is taken or last renewed; an unrenewed lease is
/// reclaimed by `recover_expired_leases`. Primary model-call timeouts must stay
/// below this value (enforced by an invariant test in `run_profile::model`) so
/// a hung provider surfaces as a retryable error before the lease reclaims the
/// runner mid-flight.
pub(crate) const DEFAULT_RUNNER_LEASE_TTL_SECONDS: i64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InMemoryTurnStateStoreLimits {
    pub max_events: usize,
    pub max_terminal_records: usize,
    pub max_idempotency_records: usize,
    pub runner_lease_ttl: ChronoDuration,
    /// Max runs in `TurnStatus::Running` per (tenant_id, owner user_id).
    /// `None` = unlimited (current behavior).
    ///
    /// Cap attribution uses the explicit thread owner when present
    /// (`TurnThreadOwner::ExplicitUser`). For `ActorFallback` owners the
    /// submitting actor's `user_id` is used instead, so those runs are still
    /// capped. Only genuinely `Ownerless` runs (no owner at all) are uncapped
    /// and never contribute to this counter.
    pub max_concurrent_runs_per_user: Option<std::num::NonZeroU32>,
    /// Max runs in `TurnStatus::Running` for `ScheduledTrigger` origin.
    /// `None` = unlimited. Runs without `product_context` are never counted.
    pub max_concurrent_trigger_runs: Option<std::num::NonZeroU32>,
    /// Max runs in `TurnStatus::Running` for `Inbound` or `WebUi` origin.
    /// `None` = unlimited. Runs without `product_context` are never counted.
    pub max_concurrent_conversation_runs: Option<std::num::NonZeroU32>,
}

impl Default for InMemoryTurnStateStoreLimits {
    fn default() -> Self {
        Self {
            max_events: MAX_EVENTS,
            max_terminal_records: MAX_TERMINAL_RECORDS,
            max_idempotency_records: MAX_IDEMPOTENCY_RECORDS,
            runner_lease_ttl: ChronoDuration::seconds(DEFAULT_RUNNER_LEASE_TTL_SECONDS),
            max_concurrent_runs_per_user: None,
            max_concurrent_trigger_runs: None,
            max_concurrent_conversation_runs: None,
        }
    }
}

pub struct InMemoryTurnStateStore {
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

impl Default for InMemoryTurnStateStore {
    fn default() -> Self {
        Self::with_limits(InMemoryTurnStateStoreLimits::default())
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
    limits: InMemoryTurnStateStoreLimits,
    concurrency: ConcurrencyLimiter,
}

enum AppliedLoopTransition {
    Applied {
        record: Box<RunRecord>,
        state: Box<TurnRunState>,
        prune_terminal: bool,
    },
    Rejected {
        record: Box<RunRecord>,
        error: TurnError,
    },
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

impl InMemoryTurnStateStore {
    pub fn with_limits(limits: InMemoryTurnStateStoreLimits) -> Self {
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

    /// Attach a durable sink for gate-blocked turns (persist-on-block). Off the
    /// hot path — the sink is called only when the blocked-run set changes.
    pub fn with_block_persistence(
        mut self,
        block_persistence: Arc<dyn crate::TurnStateBlockPersistence>,
    ) -> Self {
        self.block_persistence = Some(block_persistence);
        // Seed the gate-persisted set from any *gate-touched, not-yet-terminal*
        // run rehydrated from a recovery snapshot via `from_persistence_snapshot`.
        // `mark_gate_persisted` only fires for runs that block *live* after the
        // sink is attached, so the restore path must be seeded here or a recovered
        // run would reach a terminal state with `is_gate_persisted == false`, skip
        // its durable terminal-convergence write, and be resurrected as live on
        // the next restart.
        //
        // A currently-blocked run is the obvious case, but a run that blocked then
        // resumed is persisted as `Queued`/`Running`, so status alone misses it.
        // `checkpoint_id` is set only on the gate-block paths (`block_run` and the
        // loop-exit block) and is not cleared on resume, so it is the durable
        // marker that a not-yet-terminal run was gate-touched — seed from that.
        let gate_touched: Vec<TurnRunId> = {
            let inner = match self.inner.lock() {
                Ok(inner) => inner,
                Err(poisoned) => poisoned.into_inner(),
            };
            inner
                .records
                .iter()
                .filter(|(_, record)| {
                    !record.status.get().is_terminal() && record.checkpoint_id.is_some()
                })
                .map(|(run_id, _)| *run_id)
                .collect()
        };
        if !gate_touched.is_empty() {
            let mut set = match self.gate_persisted_runs.lock() {
                Ok(set) => set,
                Err(poisoned) => poisoned.into_inner(),
            };
            set.extend(gate_touched);
        }
        self
    }

    pub fn with_admission_limit_provider(
        admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
    ) -> Self {
        Self::with_limits_and_admission_limit_provider(
            InMemoryTurnStateStoreLimits::default(),
            admission_limit_provider,
        )
    }

    pub fn with_limits_and_admission_limit_provider(
        limits: InMemoryTurnStateStoreLimits,
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

    pub fn active_admission_reservations(&self) -> Vec<TurnAdmissionReservationRecord> {
        match self.inner.lock() {
            Ok(inner) => inner.active_admission_reservations(),
            Err(poisoned) => poisoned.into_inner().active_admission_reservations(),
        }
    }

    pub fn events(&self) -> Vec<TurnLifecycleEvent> {
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

    pub fn from_persistence_snapshot(
        snapshot: TurnPersistenceSnapshot,
        limits: InMemoryTurnStateStoreLimits,
    ) -> Result<Self, TurnError> {
        Self::from_persistence_snapshot_with_admission_limit_provider(
            snapshot,
            limits,
            Arc::new(AllowAllTurnAdmissionLimitProvider),
        )
    }

    pub fn from_persistence_snapshot_with_admission_limit_provider(
        snapshot: TurnPersistenceSnapshot,
        limits: InMemoryTurnStateStoreLimits,
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

    pub fn persistence_snapshot(&self) -> TurnPersistenceSnapshot {
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
    pub async fn flush(&self) {
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
    pub fn running_count_for_user(
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
    pub fn running_trigger_count(&self, tenant: &TenantId) -> u32 {
        match self.inner.lock() {
            Ok(inner) => inner.concurrency.count_for_trigger(tenant),
            Err(poisoned) => poisoned.into_inner().concurrency.count_for_trigger(tenant),
        }
    }

    /// Returns the number of runs currently in `TurnStatus::Running` or `TurnStatus::CancelRequested` with `Inbound` or `WebUi` origin
    /// for the given tenant. Intended for testing and observability.
    pub fn running_conversation_count(&self, tenant: &TenantId) -> u32 {
        match self.inner.lock() {
            Ok(inner) => inner.concurrency.count_for_conversation(tenant),
            Err(poisoned) => poisoned
                .into_inner()
                .concurrency
                .count_for_conversation(tenant),
        }
    }

    pub fn blocked_approval_runs_for_actor(
        &self,
        scope: &TurnScope,
        actor: &TurnActor,
    ) -> Result<Vec<TurnRunState>, TurnError> {
        let inner = self.lock_inner()?;
        let mut runs = inner
            .records
            .values()
            .filter(|record| {
                record.scope.same_thread(scope)
                    && record.actor == *actor
                    && record.status.get() == TurnStatus::BlockedApproval
                    && record.gate_ref.is_some()
            })
            .map(RunRecord::state)
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| run.run_id.as_uuid());
        Ok(runs)
    }

    pub fn approval_run_for_actor_and_gate(
        &self,
        scope: &TurnScope,
        actor: &TurnActor,
        gate_ref: &GateRef,
    ) -> Result<Option<TurnRunId>, TurnError> {
        let inner = self.lock_inner()?;
        let active = inner
            .records
            .values()
            .find(|record| {
                record.scope.same_thread(scope)
                    && record.actor == *actor
                    && record.status.get() == TurnStatus::BlockedApproval
                    && record.gate_ref.as_ref() == Some(gate_ref)
            })
            .map(|record| record.run_id);
        if active.is_some() {
            return Ok(active);
        }

        let mut historical = inner
            .checkpoints
            .iter()
            .filter(|checkpoint| {
                checkpoint.status == TurnStatus::BlockedApproval
                    && &checkpoint.gate_ref == gate_ref
                    && checkpoint
                        .scope
                        .as_ref()
                        .is_none_or(|stored| stored.same_thread(scope))
            })
            .filter_map(|checkpoint| {
                inner
                    .records
                    .get(&checkpoint.run_id)
                    .filter(|record| record.scope.same_thread(scope) && record.actor == *actor)
                    .map(|record| record.run_id)
            })
            .collect::<Vec<_>>();
        historical.sort_by_key(|run_id| run_id.as_uuid());
        historical.dedup();
        Ok(historical.into_iter().next())
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
impl TurnEventProjectionSource for InMemoryTurnStateStore {
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
}

#[async_trait]
impl LoopCheckpointStore for InMemoryTurnStateStore {
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
impl TurnStateStore for InMemoryTurnStateStore {
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
                inner.submit_idempotency_in_flight.remove(&idempotency_key);
                self.submit_idempotency_ready.notify_waiters();
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
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
            return response;
        }

        let admission_result = admission_policy.check_submit(&request);

        {
            let mut inner = self.lock_inner()?;
            if let Some(result) = inner.submit_idempotency.get(&idempotency_key).cloned() {
                inner.submit_idempotency_in_flight.remove(&idempotency_key);
                self.submit_idempotency_ready.notify_waiters();
                return result;
            }

            if let Err(rejection) = admission_result {
                let response = Err(TurnError::AdmissionRejected(rejection));
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                inner.submit_idempotency_in_flight.remove(&idempotency_key);
                self.submit_idempotency_ready.notify_waiters();
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
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
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
                inner.submit_idempotency_in_flight.remove(&idempotency_key);
                self.submit_idempotency_ready.notify_waiters();
                return response;
            }
        };

        let lock_key = TurnActiveLockKey::from(&request.scope);
        if let Some(response) = inner.thread_busy(&lock_key) {
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
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
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
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
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
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
        let record = RunRecord {
            scope: request.scope.clone(),
            actor: request.actor,
            turn_id,
            run_id,
            status: RunStatusCell::new(TurnStatus::Queued),
            profile: profile.clone(),
            resolved_model_route: None,
            accepted_message_ref: request.accepted_message_ref.clone(),
            source_binding_ref: request.source_binding_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: cursor,
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: request.received_at,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: request.product_context,
            resume_disposition: None,
        };
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
        inner.submit_idempotency_in_flight.remove(&idempotency_key);
        self.submit_idempotency_ready.notify_waiters();
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

#[async_trait]
impl TurnSpawnTreeStateStore for InMemoryTurnStateStore {
    async fn submit_child_turn(
        &self,
        request: SubmitChildRunRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let idempotency_key = SubmitIdempotencyKey {
            scope: request.child_scope.clone(),
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

        let submit_template = {
            let mut inner = self.lock_inner()?;
            if let Some(result) = inner.submit_idempotency.get(&idempotency_key).cloned() {
                inner.submit_idempotency_in_flight.remove(&idempotency_key);
                self.submit_idempotency_ready.notify_waiters();
                return result;
            }
            let Some(parent) = inner
                .records
                .get(&request.parent_run_id)
                .filter(|record| record.scope == request.parent_scope)
            else {
                let response = Err(TurnError::ScopeNotFound);
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                inner.submit_idempotency_in_flight.remove(&idempotency_key);
                self.submit_idempotency_ready.notify_waiters();
                return response;
            };
            if !same_scope_envelope(&parent.scope, &request.child_scope) {
                let response = Err(TurnError::Unauthorized);
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                inner.submit_idempotency_in_flight.remove(&idempotency_key);
                self.submit_idempotency_ready.notify_waiters();
                return response;
            }
            if parent.subagent_depth == u32::MAX {
                let response = Err(invalid_lineage("subagent depth would overflow"));
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                inner.submit_idempotency_in_flight.remove(&idempotency_key);
                self.submit_idempotency_ready.notify_waiters();
                return response;
            }
            SubmitTurnRequest {
                scope: request.child_scope.clone(),
                actor: request.actor.clone(),
                accepted_message_ref: request.accepted_message_ref.clone(),
                source_binding_ref: request.source_binding_ref.clone(),
                reply_target_binding_ref: request.reply_target_binding_ref.clone(),
                requested_run_profile: request.requested_run_profile.clone(),
                idempotency_key: request.idempotency_key.clone(),
                received_at: request.received_at,
                requested_run_id: request.requested_run_id,
                parent_run_id: Some(parent.run_id),
                subagent_depth: parent.subagent_depth + 1,
                spawn_tree_root_run_id: Some(
                    parent.spawn_tree_root_run_id.unwrap_or(parent.run_id),
                ),
                product_context: parent.product_context.clone(),
            }
        };

        let admission_result = admission_policy.check_submit(&submit_template);
        {
            let mut inner = self.lock_inner()?;
            if let Some(result) = inner.submit_idempotency.get(&idempotency_key).cloned() {
                inner.submit_idempotency_in_flight.remove(&idempotency_key);
                self.submit_idempotency_ready.notify_waiters();
                return result;
            }
            if let Err(rejection) = admission_result {
                let response = Err(TurnError::AdmissionRejected(rejection));
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                inner.submit_idempotency_in_flight.remove(&idempotency_key);
                self.submit_idempotency_ready.notify_waiters();
                return response;
            }
        }

        let profile_resolution = run_profile_resolver
            .resolve_run_profile(RunProfileResolutionRequest {
                requested_run_profile: submit_template.requested_run_profile.clone(),
                ..RunProfileResolutionRequest::interactive_default()
            })
            .await;

        let mut inner = self.lock_inner()?;
        if let Some(result) = inner.submit_idempotency.get(&idempotency_key).cloned() {
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
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
                inner.submit_idempotency_in_flight.remove(&idempotency_key);
                self.submit_idempotency_ready.notify_waiters();
                return response;
            }
        };

        let Some(parent) = inner
            .records
            .get(&request.parent_run_id)
            .filter(|record| record.scope == request.parent_scope)
        else {
            let response = Err(TurnError::ScopeNotFound);
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
            return response;
        };
        if !same_scope_envelope(&parent.scope, &request.child_scope) {
            let response = Err(TurnError::Unauthorized);
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
            return response;
        }
        if parent.subagent_depth == u32::MAX {
            let response = Err(invalid_lineage("subagent depth would overflow"));
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
            return response;
        }
        let parent_run_id = parent.run_id;
        let subagent_depth = parent.subagent_depth + 1;
        let root_run_id = parent.spawn_tree_root_run_id.unwrap_or(parent.run_id);
        let parent_product_context = parent.product_context.clone();
        let Some(root) = inner.records.get(&root_run_id) else {
            let response = Err(TurnError::ScopeNotFound);
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
            return response;
        };
        if !same_scope_envelope(&root.scope, &request.child_scope) {
            let response = Err(TurnError::Unauthorized);
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
            return response;
        }
        if root.spawn_tree_root_run_id.unwrap_or(root.run_id) != root.run_id {
            let response = Err(invalid_lineage(
                "root_run_id must identify the spawn tree root",
            ));
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
            return response;
        }

        let lock_key = TurnActiveLockKey::from(&request.child_scope);
        if let Some(response) = inner.thread_busy(&lock_key) {
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
            return Err(TurnError::ThreadBusy(response));
        }
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
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
            return response;
        }

        let reservation_key = SpawnTreeReservationKey::new(&request.child_scope, root_run_id);
        let previous_state = inner
            .tree_reservations
            .get(&reservation_key)
            .cloned()
            .unwrap_or_default();
        let previous_tree_count = previous_state.count;
        let next_tree_count = previous_tree_count.checked_add(1).ok_or_else(|| {
            TurnError::capacity_exceeded(
                TurnCapacityResource::SpawnTreeDescendants,
                u64::from(request.spawn_tree_descendant_cap),
            )
        });
        let next_tree_count = match next_tree_count {
            Ok(next) if next <= u64::from(request.spawn_tree_descendant_cap) => next,
            _ => {
                let response = Err(TurnError::capacity_exceeded(
                    TurnCapacityResource::SpawnTreeDescendants,
                    u64::from(request.spawn_tree_descendant_cap),
                ));
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                inner.submit_idempotency_in_flight.remove(&idempotency_key);
                self.submit_idempotency_ready.notify_waiters();
                return response;
            }
        };
        inner.tree_reservations.insert(
            reservation_key.clone(),
            TreeReservationState {
                count: next_tree_count,
                released_children: previous_state.released_children.clone(),
            },
        );

        let admission_class = profile.admission_class.clone();
        if let Err(rejection) = inner.reserve_admission(
            run_id,
            admission_class.clone(),
            &request.child_scope,
            &request.actor,
            self.admission_limit_provider.as_ref(),
        ) {
            if previous_tree_count == 0 {
                inner.tree_reservations.remove(&reservation_key);
            } else {
                inner
                    .tree_reservations
                    .insert(reservation_key, previous_state);
            }
            let response = Err(TurnError::AdmissionRejected(rejection));
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_waiters();
            return response;
        }

        let turn_id = crate::TurnId::new();
        let cursor = inner.next_cursor();
        let turn_record = TurnRecord {
            turn_id,
            scope: request.child_scope.clone(),
            actor: request.actor.clone(),
            accepted_message_ref: request.accepted_message_ref.clone(),
            source_binding_ref: request.source_binding_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            created_at: request.received_at,
        };
        let record = RunRecord {
            scope: request.child_scope.clone(),
            actor: request.actor,
            turn_id,
            run_id,
            status: RunStatusCell::new(TurnStatus::Queued),
            profile: profile.clone(),
            resolved_model_route: None,
            accepted_message_ref: request.accepted_message_ref.clone(),
            source_binding_ref: request.source_binding_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: cursor,
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: request.received_at,
            parent_run_id: Some(parent_run_id),
            subagent_depth,
            spawn_tree_root_run_id: Some(root_run_id),
            product_context: parent_product_context,
            resume_disposition: None,
        };
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
        inner.submit_idempotency_in_flight.remove(&idempotency_key);
        self.submit_idempotency_ready.notify_waiters();
        response
    }

    async fn children_of(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        let inner = self.lock_inner()?;
        let Some(parent) = inner.records.get(&run_id) else {
            return Ok(Vec::new());
        };
        if parent.scope != *scope {
            return Ok(Vec::new());
        }
        let mut children = inner
            .records
            .values()
            .filter(|record| {
                same_scope_envelope(&record.scope, scope) && record.parent_run_id == Some(run_id)
            })
            .map(RunRecord::persistence_record)
            .collect::<Vec<_>>();
        children.sort_by_key(|record| record.received_at);
        Ok(children)
    }

    async fn get_run_record(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        let inner = self.lock_inner()?;
        Ok(inner
            .records
            .get(&run_id)
            .filter(|record| record.scope == *scope)
            .map(RunRecord::persistence_record))
    }

    async fn reserve_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError> {
        if delta == 0 {
            return Err(TurnError::InvalidRequest {
                reason: "reservation delta must be greater than zero".to_string(),
            });
        }
        let mut inner = self.lock_inner()?;
        let Some(root) = inner.records.get(&root_run_id) else {
            return Err(TurnError::ScopeNotFound);
        };
        if !same_scope_envelope(&root.scope, scope) {
            return Err(TurnError::Unauthorized);
        }
        let canonical_root_run_id = root.spawn_tree_root_run_id.unwrap_or(root.run_id);
        if canonical_root_run_id != root.run_id {
            return Err(TurnError::InvalidRequest {
                reason: "root_run_id must identify the spawn tree root".to_string(),
            });
        }
        let key = SpawnTreeReservationKey::new(scope, canonical_root_run_id);
        let current = inner
            .tree_reservations
            .get(&key)
            .map(|state| state.count)
            .unwrap_or(0);
        let next = current.checked_add(u64::from(delta)).ok_or_else(|| {
            TurnError::capacity_exceeded(TurnCapacityResource::SpawnTreeDescendants, u64::from(cap))
        })?;
        if next > u64::from(cap) {
            return Err(TurnError::capacity_exceeded(
                TurnCapacityResource::SpawnTreeDescendants,
                u64::from(cap),
            ));
        }
        let released_children = inner
            .tree_reservations
            .get(&key)
            .map(|state| state.released_children.clone())
            .unwrap_or_default();
        inner.tree_reservations.insert(
            key,
            TreeReservationState {
                count: next,
                released_children: released_children.clone(),
            },
        );
        Ok(SpawnTreeReservation {
            scope: scope.clone(),
            root_run_id: canonical_root_run_id,
            descendant_count: next,
            released_children,
        })
    }

    async fn release_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        idempotency_key: TurnRunId,
    ) -> Result<(), TurnError> {
        let mut inner = self.lock_inner()?;
        let Some(root) = inner.records.get(&root_run_id) else {
            return Err(TurnError::ScopeNotFound);
        };
        if !same_scope_envelope(&root.scope, scope) {
            return Err(TurnError::Unauthorized);
        }
        let canonical_root_run_id = root.spawn_tree_root_run_id.unwrap_or(root.run_id);
        if canonical_root_run_id != root.run_id {
            return Err(TurnError::InvalidRequest {
                reason: "root_run_id must identify the spawn tree root".to_string(),
            });
        }
        let key = SpawnTreeReservationKey::new(scope, canonical_root_run_id);
        let mut released_reservation = false;
        if let Some(state) = inner.tree_reservations.get_mut(&key) {
            // §5.5 round-5/6 idempotency: a release already recorded for
            // this child is a no-op, not a second decrement — this is what
            // makes a retried release call (recovery re-driving an edge
            // stuck at `Claimed`) safe to repeat unconditionally.
            if !state.released_children.insert(idempotency_key) {
                return Ok(());
            }
            let previous = state.count;
            if previous < u64::from(delta) {
                // Reject over-release loudly so callers can diagnose
                // double-release bugs instead of silently zeroing the
                // reservation and uncapping the spawn tree. Undo the just-
                // inserted dedup marker so a legitimate retry after fixing
                // the caller bug isn't permanently swallowed as a no-op.
                state.released_children.remove(&idempotency_key);
                return Err(TurnError::InvalidRequest {
                    reason: "release delta exceeds current reservation count".to_string(),
                });
            }
            state.count = previous - u64::from(delta);
            if state.count == 0 {
                inner.tree_reservations.remove(&key);
                released_reservation = true;
            }
        }
        if released_reservation
            && inner
                .records
                .get(&canonical_root_run_id)
                .is_some_and(|record| record.status.get().is_terminal())
            && !inner.terminal_runs.contains(&canonical_root_run_id)
        {
            if inner.terminal_runs.len() >= inner.limits.max_terminal_records {
                inner.records.remove(&canonical_root_run_id);
                inner.admission_reservations.remove(&canonical_root_run_id);
                return Ok(());
            }
            inner.terminal_runs.push_back(canonical_root_run_id);
            inner.prune_terminal_records();
        }
        Ok(())
    }

    async fn prune_released_child(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        let mut inner = self.lock_inner()?;
        let Some(root) = inner.records.get(&root_run_id) else {
            // The reservation (and its tree) may already be fully released
            // and gone — benign, matches `release_tree_descendants`'s own
            // missing-root handling being the only hard error case there.
            return Ok(());
        };
        if !same_scope_envelope(&root.scope, scope) {
            return Err(TurnError::Unauthorized);
        }
        let canonical_root_run_id = root.spawn_tree_root_run_id.unwrap_or(root.run_id);
        let key = SpawnTreeReservationKey::new(scope, canonical_root_run_id);
        if let Some(state) = inner.tree_reservations.get_mut(&key) {
            state.released_children.remove(&child_run_id);
        }
        Ok(())
    }
}

#[async_trait]
impl TurnRunTransitionPort for InMemoryTurnStateStore {
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        let mut inner = self.lock_inner()?;
        inner.claim_matching_queued_run(
            request.runner_id,
            request.lease_token,
            request.scope_filter.as_ref(),
        )
    }

    async fn claim_next_runs(
        &self,
        request: ClaimRunsRequest,
    ) -> Result<Vec<ClaimedTurnRun>, TurnError> {
        let mut inner = self.lock_inner()?;
        let mut claimed_runs = Vec::new();
        for _ in 0..request.max_runs {
            let Some(claimed) = inner.claim_matching_queued_run(
                request.runner_id,
                TurnLeaseToken::new(),
                request.scope_filter.as_ref(),
            )?
            else {
                break;
            };
            claimed_runs.push(claimed);
        }
        Ok(claimed_runs)
    }

    async fn heartbeat(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        let mut inner = self.lock_inner()?;
        let mut record = inner.take_record(request.run_id)?;
        let result = (|| {
            let now = Utc::now();
            ensure_active_lease(&record, request.runner_id, request.lease_token, now)?;
            if record.status.get() != TurnStatus::Running {
                return Err(TurnError::InvalidTransition {
                    from: record.status.get(),
                    to: TurnStatus::Running,
                });
            }
            record.last_heartbeat_at = Some(now);
            record.lease_expires_at = Some(inner.next_lease_expiry(now));
            record.event_cursor = inner.next_cursor();
            inner.touch_active_lock(&record, now);
            inner.push_event(&record, TurnEventKind::RunnerHeartbeat, None, None);
            Ok(record.event_cursor)
        })();
        inner.records.insert(record.run_id, record);
        result
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        let mut inner = self.lock_inner()?;
        Ok(inner.recover_expired_leases(request))
    }

    async fn latest_resumable_checkpoint(
        &self,
        scope: &TurnScope,
        turn_id: crate::TurnId,
        run_id: TurnRunId,
    ) -> Result<Option<TurnCheckpointId>, TurnError> {
        let inner = self.lock_inner()?;
        Ok(inner.latest_resumable_loop_checkpoint(scope, turn_id, run_id))
    }

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        let mut inner = self.lock_inner()?;
        let mut record = inner.take_record(request.run_id)?;
        let result = (|| {
            let now = Utc::now();
            ensure_active_lease(&record, request.runner_id, request.lease_token, now)?;
            if record.status.get() != TurnStatus::Running {
                return Err(TurnError::InvalidTransition {
                    from: record.status.get(),
                    to: TurnStatus::Running,
                });
            }
            request
                .snapshot
                .validate()
                .map_err(|reason| TurnError::InvalidRequest { reason })?;
            if let Some(existing) = &record.resolved_model_route {
                if existing != &request.snapshot {
                    return Err(TurnError::Conflict {
                        reason: "run already has a different resolved model route".to_string(),
                    });
                }
                return Ok(record.state());
            }
            record.resolved_model_route = Some(request.snapshot);
            inner.touch_active_lock(&record, now);
            Ok(record.state())
        })();
        inner.records.insert(record.run_id, record);
        result
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        let result = {
            let mut inner = self.lock_inner()?;
            let mut record = inner.take_record(request.run_id)?;
            let inner_result = (|| {
                let now = Utc::now();
                ensure_active_lease(&record, request.runner_id, request.lease_token, now)?;
                if !matches!(record.status.get(), TurnStatus::Running) {
                    return Err(TurnError::InvalidTransition {
                        from: record.status.get(),
                        to: request.reason.status(),
                    });
                }
                // Running → Blocked: decrement per-user running counter.
                let transition = record.status.set(request.reason.status());
                inner.apply_status_transition(transition, &record);
                record.checkpoint_id = Some(request.checkpoint_id);
                record.gate_ref = Some(request.reason.gate_ref().clone());
                record.blocked_activity_id = None;
                record.credential_requirements = request.reason.credential_requirements().to_vec();
                record.runner_id = None;
                record.lease_token = None;
                record.lease_expires_at = None;
                record.event_cursor = inner.next_cursor();
                inner.record_checkpoint(
                    &record,
                    request.checkpoint_id,
                    request.state_ref,
                    request.reason.gate_ref().clone(),
                    now,
                );
                inner.update_active_lock(&record, now);
                let state = record.state();
                inner.push_event(&record, TurnEventKind::Blocked, None, None);
                Ok(state)
            })();
            inner.records.insert(record.run_id, record);
            inner_result
        };
        // A run just parked on a gate — track it for durable terminal cleanup
        // and persist so the blocked turn survives a process restart (off the hot
        // path; only fires on a block).
        if result.is_ok() {
            self.mark_gate_persisted(request.run_id);
            self.persist_blocked_state().await;
        }
        result
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        let gate_persisted = self.is_gate_persisted(request.run_id);
        let result = {
            let mut inner = self.lock_inner()?;
            inner.terminal_transition(
                request.run_id,
                request.runner_id,
                request.lease_token,
                TurnStatus::Completed,
                None,
                TurnEventKind::Completed,
            )
        };
        self.persist_terminal_cleanup(request.run_id, gate_persisted, &result)
            .await;
        result
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        let gate_persisted = self.is_gate_persisted(request.run_id);
        let result = {
            let mut inner = self.lock_inner()?;
            inner.cancel_completion_transition(
                request.run_id,
                request.runner_id,
                request.lease_token,
            )
        };
        self.persist_terminal_cleanup(request.run_id, gate_persisted, &result)
            .await;
        result
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        let gate_persisted = self.is_gate_persisted(request.run_id);
        let result = {
            let mut inner = self.lock_inner()?;
            inner.terminal_transition(
                request.run_id,
                request.runner_id,
                request.lease_token,
                TurnStatus::Failed,
                Some(request.failure),
                TurnEventKind::Failed,
            )
        };
        self.persist_terminal_cleanup(request.run_id, gate_persisted, &result)
            .await;
        result
    }

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        // A runner failure can also terminate a run (Running → Failed,
        // CancelRequested → Cancelled), so it must converge the durable snapshot
        // for a gate-persisted run exactly like the other terminal paths —
        // otherwise a restart could resurrect the finished run as live.
        let gate_persisted = self.is_gate_persisted(request.run_id);
        let result = {
            let mut inner = self.lock_inner()?;
            inner.runner_failure_transition(
                request.run_id,
                request.runner_id,
                request.lease_token,
                request.failure,
            )
        };
        self.persist_terminal_cleanup(request.run_id, gate_persisted, &result)
            .await;
        result
    }

    async fn relinquish_run(
        &self,
        request: RelinquishRunRequest,
    ) -> Result<TurnRunState, TurnError> {
        let mut inner = self.lock_inner()?;
        inner.relinquish_transition(request.run_id, request.runner_id, request.lease_token)
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        let tracked_before = self.is_gate_persisted(request.run_id);
        let result = {
            let mut inner = self.lock_inner()?;
            inner.apply_validated_loop_exit_transition(
                request.run_id,
                request.runner_id,
                request.lease_token,
                request.mapping,
            )
        };
        // A validated loop exit can either park a run on a gate or terminate one
        // (possibly a previously-blocked one). Persist so the durable snapshot
        // stays accurate across restart, and converge the terminal case so a
        // finished run is not rehydrated as live.
        if result.is_ok() && self.block_persistence.is_some() {
            if self.run_is_blocked(request.run_id) {
                self.mark_gate_persisted(request.run_id);
                self.persist_blocked_state().await;
            } else if tracked_before {
                self.persist_blocked_state().await;
                self.clear_gate_persisted(request.run_id);
            }
        }
        result
    }
}

impl Inner {
    fn from_persistence_snapshot(
        snapshot: TurnPersistenceSnapshot,
        limits: InMemoryTurnStateStoreLimits,
    ) -> Result<Self, TurnError> {
        let mut cursor = 0;
        let turns = snapshot
            .turns
            .into_iter()
            .map(|record| (record.turn_id, record))
            .collect::<HashMap<_, _>>();
        let mut records = HashMap::new();
        let mut queued_runs = VecDeque::new();
        let mut terminal_runs = VecDeque::new();
        let mut active_locks = HashMap::new();
        for lock in snapshot.active_locks {
            active_locks.insert(lock.key.clone(), lock);
        }
        for run in snapshot.runs {
            cursor = cursor.max(run.event_cursor.0);
            let actor = turns
                .get(&run.turn_id)
                .map(|turn| turn.actor.clone())
                .ok_or_else(|| TurnError::Unavailable {
                    reason: "turn run references missing turn record".to_string(),
                })?;
            let has_non_queued_active_lock = active_locks
                .values()
                .any(|lock| lock.run_id == run.run_id && lock.status != TurnStatus::Queued);
            if run.status == TurnStatus::Queued && !has_non_queued_active_lock {
                queued_runs.push_back(run.run_id);
            }
            if run.status.is_terminal() {
                terminal_runs.push_back(run.run_id);
            }
            records.insert(
                run.run_id,
                RunRecord {
                    scope: run.scope,
                    actor,
                    turn_id: run.turn_id,
                    run_id: run.run_id,
                    status: RunStatusCell::new(run.status),
                    profile: run.profile,
                    resolved_model_route: run.resolved_model_route,
                    accepted_message_ref: run.accepted_message_ref,
                    source_binding_ref: run.source_binding_ref,
                    reply_target_binding_ref: run.reply_target_binding_ref,
                    checkpoint_id: run.checkpoint_id,
                    gate_ref: run.gate_ref,
                    blocked_activity_id: run.blocked_activity_id,
                    credential_requirements: run.credential_requirements,
                    failure: run.failure,
                    event_cursor: run.event_cursor,
                    runner_id: run.runner_id,
                    lease_token: run.lease_token,
                    lease_expires_at: run.lease_expires_at,
                    last_heartbeat_at: run.last_heartbeat_at,
                    claim_count: run.claim_count,
                    received_at: run.received_at,
                    parent_run_id: run.parent_run_id,
                    subagent_depth: run.subagent_depth,
                    spawn_tree_root_run_id: run.spawn_tree_root_run_id,
                    product_context: run.product_context,
                    resume_disposition: run.resume_disposition,
                },
            );
        }

        let mut submit_idempotency = HashMap::new();
        let mut resume_idempotency = HashMap::new();
        let mut retry_idempotency = HashMap::new();
        let mut cancel_idempotency = HashMap::new();
        let mut idempotency_records = HashMap::new();
        let mut submit_idempotency_order = VecDeque::new();
        let mut resume_idempotency_order = VecDeque::new();
        let mut retry_idempotency_order = VecDeque::new();
        let mut cancel_idempotency_order = VecDeque::new();
        let mut idempotency_record_order = VecDeque::new();
        let mut ordered_idempotency_records = snapshot.idempotency_records;
        ordered_idempotency_records.sort_by_key(|record| record.created_at);
        for record in ordered_idempotency_records {
            let persisted_key = persisted_key_for_record(&record);
            idempotency_record_order.push_back(persisted_key.clone());
            idempotency_records.insert(persisted_key, record.clone());
            match record.operation {
                TurnIdempotencyOperationKind::Submit => {
                    if let Some(replay) = record.replay_submit() {
                        let key = SubmitIdempotencyKey {
                            scope: record.scope.clone(),
                            key: record.key.clone(),
                        };
                        submit_idempotency_order.push_back(key.clone());
                        submit_idempotency.insert(key, replay);
                    }
                }
                TurnIdempotencyOperationKind::Resume => {
                    if let (Some(run_id), Some(replay)) = (record.run_id, record.replay_resume()) {
                        let key = RunIdempotencyKey {
                            scope: record.scope.clone(),
                            run_id,
                            key: record.key.clone(),
                        };
                        resume_idempotency_order.push_back(key.clone());
                        resume_idempotency.insert(key, replay);
                    } else {
                        debug_malformed_idempotency_record(&record);
                    }
                }
                TurnIdempotencyOperationKind::Retry => {
                    if let (Some(run_id), Some(replay)) = (record.run_id, record.replay_retry()) {
                        let key = RunIdempotencyKey {
                            scope: record.scope.clone(),
                            run_id,
                            key: record.key.clone(),
                        };
                        retry_idempotency_order.push_back(key.clone());
                        retry_idempotency.insert(key, replay);
                    } else if record.run_id.is_some()
                        && matches!(record.replay, TurnIdempotencyReplay::RetryThreadBusy(_))
                    {
                        // Retry ThreadBusy records are retained in the durable snapshot for
                        // auditability, but are intentionally not replayable.
                    } else {
                        debug_malformed_idempotency_record(&record);
                    }
                }
                TurnIdempotencyOperationKind::Cancel => {
                    if let (Some(run_id), Some(replay)) = (record.run_id, record.replay_cancel()) {
                        let key = RunIdempotencyKey {
                            scope: record.scope.clone(),
                            run_id,
                            key: record.key.clone(),
                        };
                        cancel_idempotency_order.push_back(key.clone());
                        cancel_idempotency.insert(key, replay);
                    } else {
                        debug_malformed_idempotency_record(&record);
                    }
                }
            }
        }

        let loop_checkpoints = snapshot
            .loop_checkpoints
            .into_iter()
            .map(|record| (record.checkpoint_id, record))
            .collect::<HashMap<_, _>>();

        let events = snapshot.events;
        cursor = cursor.max(events.iter().map(|event| event.cursor.0).max().unwrap_or(0));
        cursor = cursor.max(snapshot.event_retention_floor.0);
        let mut admission_reservations = HashMap::new();
        for mut reservation in snapshot.admission_reservations {
            let Some(record) = records.get(&reservation.run_id) else {
                continue;
            };
            if record.status.get().is_terminal() {
                reservation.released = true;
            }
            admission_reservations.insert(reservation.run_id, reservation);
        }
        for record in records.values() {
            if record.status.get().keeps_active_lock() {
                let admission_class = record.profile.admission_class.clone();
                let buckets = admission_buckets(&record.scope, &record.actor, &admission_class);
                let needs_canonical_reservation = admission_reservations
                    .get(&record.run_id)
                    .is_none_or(|reservation| {
                        reservation.released
                            || reservation.admission_class != admission_class
                            || reservation.buckets != buckets
                    });
                if needs_canonical_reservation {
                    admission_reservations.insert(
                        record.run_id,
                        TurnAdmissionReservationRecord {
                            run_id: record.run_id,
                            admission_class,
                            buckets,
                            released: false,
                        },
                    );
                }
            }
        }

        let mut tree_reservations = HashMap::new();
        for reservation in snapshot.spawn_tree_reservations {
            tree_reservations.insert(
                SpawnTreeReservationKey::new(&reservation.scope, reservation.root_run_id),
                TreeReservationState {
                    count: reservation.descendant_count,
                    released_children: reservation.released_children,
                },
            );
        }

        let concurrency_limits = ConcurrencyLimits {
            max_concurrent_runs_per_user: limits.max_concurrent_runs_per_user,
            max_concurrent_trigger_runs: limits.max_concurrent_trigger_runs,
            max_concurrent_conversation_runs: limits.max_concurrent_conversation_runs,
        };
        let concurrency = ConcurrencyLimiter::rebuild_from(
            concurrency_limits,
            records
                .values()
                .filter(|record| holds_running_slot(record.status.get()))
                .map(|record| slot_info_for(record)),
        );

        Ok(Self {
            cursor,
            turns,
            records,
            queued_runs,
            terminal_runs,
            active_locks,
            checkpoints: snapshot.checkpoints,
            loop_checkpoints,
            submit_idempotency,
            submit_idempotency_in_flight: HashSet::new(),
            resume_idempotency,
            retry_idempotency,
            cancel_idempotency,
            idempotency_records,
            submit_idempotency_order,
            resume_idempotency_order,
            retry_idempotency_order,
            cancel_idempotency_order,
            idempotency_record_order,
            events,
            event_retention_floor: snapshot.event_retention_floor,
            admission_reservations,
            tree_reservations,
            limits,
            concurrency,
        })
    }

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
        let retryable = (kind == TurnEventKind::Failed).then(|| record.checkpoint_id.is_some());
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

    fn persistence_snapshot(&self) -> TurnPersistenceSnapshot {
        let mut turns = self.turns.values().cloned().collect::<Vec<_>>();
        turns.sort_by_key(|record| record.created_at);
        let mut runs = self
            .records
            .values()
            .map(RunRecord::persistence_record)
            .collect::<Vec<_>>();
        runs.sort_by_key(|record| record.event_cursor);
        let mut active_locks = self.active_locks.values().cloned().collect::<Vec<_>>();
        active_locks.sort_by_key(|record| record.acquired_at);
        let mut checkpoints = self.checkpoints.clone();
        checkpoints.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.sequence.cmp(&b.sequence))
        });
        let mut loop_checkpoints = self.loop_checkpoints.values().cloned().collect::<Vec<_>>();
        loop_checkpoints.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.checkpoint_id.as_uuid().cmp(&b.checkpoint_id.as_uuid()))
        });
        let mut idempotency_records = self
            .idempotency_records
            .values()
            .cloned()
            .collect::<Vec<_>>();
        idempotency_records.sort_by_key(|record| record.created_at);
        let mut admission_reservations = self
            .admission_reservations
            .values()
            .cloned()
            .collect::<Vec<_>>();
        admission_reservations.sort_by_key(|reservation| reservation.run_id.to_string());
        let mut spawn_tree_reservations = self
            .tree_reservations
            .iter()
            .filter_map(|(key, state)| {
                let root = self.records.get(&key.root_run_id)?;
                Some(SpawnTreeReservation {
                    scope: root.scope.clone(),
                    root_run_id: key.root_run_id,
                    descendant_count: state.count,
                    released_children: state.released_children.clone(),
                })
            })
            .collect::<Vec<_>>();
        spawn_tree_reservations.sort_by_key(|reservation| reservation.root_run_id.to_string());
        TurnPersistenceSnapshot {
            turns,
            runs,
            active_locks,
            checkpoints,
            loop_checkpoints,
            idempotency_records,
            events: self.events.clone(),
            event_retention_floor: self.event_retention_floor,
            admission_reservations,
            spawn_tree_reservations,
        }
    }

    fn recover_expired_leases(
        &mut self,
        request: RecoverExpiredLeasesRequest,
    ) -> RecoverExpiredLeasesResponse {
        let expired_run_ids = self
            .records
            .iter()
            .filter_map(|(run_id, record)| {
                if !matches!(
                    record.status.get(),
                    TurnStatus::Running | TurnStatus::CancelRequested
                ) {
                    return None;
                }
                if request
                    .scope_filter
                    .as_ref()
                    .is_some_and(|scope| scope != &record.scope)
                {
                    return None;
                }
                if record
                    .lease_expires_at
                    .is_some_and(|expires_at| expires_at <= request.now)
                {
                    Some(*run_id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let mut recovered = Vec::with_capacity(expired_run_ids.len());
        for run_id in expired_run_ids {
            let Some(mut record) = self.records.remove(&run_id) else {
                continue;
            };
            // Running and CancelRequested both hold a runner-claimed lease (incremented at
            // claim). Decrement for both since the lease expiry terminates the run.
            let outcome = expired_lease_terminal_outcome(record.status.get());
            let transition = record.status.set(outcome.status);
            self.apply_status_transition(transition, &record);
            if record.status.get() == TurnStatus::Failed {
                record.checkpoint_id =
                    self.latest_resumable_loop_checkpoint(&record.scope, record.turn_id, run_id);
            }
            record.failure = outcome.failure;
            record.runner_id = None;
            record.lease_token = None;
            record.lease_expires_at = None;
            record.event_cursor = self.next_cursor();
            self.release_active_lock(&record);
            self.remove_queued_run(record.run_id);
            let state = record.state();
            let event_detail =
                failure_detail_for_event(&outcome.event_kind, record.failure.as_ref());
            self.push_event(
                &record,
                outcome.event_kind,
                outcome.event_detail,
                event_detail,
            );
            self.mark_terminal(record.run_id);
            recovered.push(state);
            self.records.insert(run_id, record);
        }
        self.prune_terminal_records();
        RecoverExpiredLeasesResponse { recovered }
    }

    fn remember_submit_idempotency(
        &mut self,
        key: SubmitIdempotencyKey,
        result: Result<SubmitTurnResponse, TurnError>,
        created_at: crate::TurnTimestamp,
    ) {
        if !self.submit_idempotency.contains_key(&key) {
            self.submit_idempotency_order.push_back(key.clone());
        }
        let record = submit_idempotency_record(&key, &result, created_at);
        self.remember_persisted_idempotency(record);
        self.submit_idempotency.insert(key, result);
        let removed = prune_ordered_map(
            &mut self.submit_idempotency,
            &mut self.submit_idempotency_order,
            self.limits.max_idempotency_records,
        );
        for key in removed {
            self.remove_persisted_submit_idempotency(&key);
        }
        self.prune_idempotency_records();
    }

    fn remember_resume_idempotency(
        &mut self,
        key: RunIdempotencyKey,
        result: Result<ResumeTurnResponse, TurnError>,
        created_at: crate::TurnTimestamp,
    ) {
        if !self.resume_idempotency.contains_key(&key) {
            self.resume_idempotency_order.push_back(key.clone());
        }
        let record = resume_idempotency_record(&key, &result, created_at);
        self.remember_persisted_idempotency(record);
        self.resume_idempotency.insert(key, result);
        let removed = prune_ordered_map(
            &mut self.resume_idempotency,
            &mut self.resume_idempotency_order,
            self.limits.max_idempotency_records,
        );
        for key in removed {
            self.remove_persisted_run_idempotency(TurnIdempotencyOperationKind::Resume, &key);
        }
        self.prune_idempotency_records();
    }

    fn remember_retry_idempotency(
        &mut self,
        key: RunIdempotencyKey,
        result: Result<RetryTurnResponse, TurnError>,
        created_at: crate::TurnTimestamp,
    ) {
        let replayable = !matches!(
            result,
            Err(TurnError::ThreadBusy(_) | TurnError::AdmissionRejected(_))
        );
        if !matches!(result, Err(TurnError::AdmissionRejected(_))) {
            let record = retry_idempotency_record(&key, &result, created_at);
            self.remember_persisted_idempotency(record);
        }
        if replayable {
            if !self.retry_idempotency.contains_key(&key) {
                self.retry_idempotency_order.push_back(key.clone());
            }
            self.retry_idempotency.insert(key, result);
            let removed = prune_ordered_map(
                &mut self.retry_idempotency,
                &mut self.retry_idempotency_order,
                self.limits.max_idempotency_records,
            );
            for key in removed {
                self.remove_persisted_run_idempotency(TurnIdempotencyOperationKind::Retry, &key);
            }
        }
        self.prune_idempotency_records();
    }

    fn remember_cancel_idempotency(
        &mut self,
        key: RunIdempotencyKey,
        result: Result<CancelRunResponse, TurnError>,
        created_at: crate::TurnTimestamp,
    ) {
        if !self.cancel_idempotency.contains_key(&key) {
            self.cancel_idempotency_order.push_back(key.clone());
        }
        let record = cancel_idempotency_record(&key, &result, created_at);
        self.remember_persisted_idempotency(record);
        self.cancel_idempotency.insert(key, result);
        let removed = prune_ordered_map(
            &mut self.cancel_idempotency,
            &mut self.cancel_idempotency_order,
            self.limits.max_idempotency_records,
        );
        for key in removed {
            self.remove_persisted_run_idempotency(TurnIdempotencyOperationKind::Cancel, &key);
        }
        self.prune_idempotency_records();
    }

    fn remember_persisted_idempotency(&mut self, record: TurnIdempotencyRecord) {
        let key = persisted_key_for_record(&record);
        if !self.idempotency_records.contains_key(&key) {
            self.idempotency_record_order.push_back(key.clone());
        }
        self.idempotency_records.insert(key, record);
    }

    fn remove_persisted_submit_idempotency(&mut self, key: &SubmitIdempotencyKey) {
        self.idempotency_records.remove(&persisted_submit_key(key));
    }

    fn remove_persisted_run_idempotency(
        &mut self,
        operation: TurnIdempotencyOperationKind,
        key: &RunIdempotencyKey,
    ) {
        self.idempotency_records
            .remove(&persisted_run_key(operation, key));
    }

    fn prune_idempotency_records(&mut self) {
        let _removed = prune_ordered_map(
            &mut self.idempotency_records,
            &mut self.idempotency_record_order,
            self.limits.max_idempotency_records.saturating_mul(4),
        );
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

    fn pop_matching_queued_run(&mut self, scope_filter: Option<&TurnScope>) -> Option<TurnRunId> {
        let queued_count = self.queued_runs.len();
        for _ in 0..queued_count {
            let run_id = self.queued_runs.pop_front()?;
            let Some(record) = self.records.get(&run_id) else {
                continue;
            };
            if record.status.get() != TurnStatus::Queued {
                continue;
            }
            let scope_ok = scope_filter.is_none_or(|scope| scope == &record.scope);
            let cap_ok = self.concurrency.can_claim(&slot_info_for(record));
            if scope_ok && cap_ok {
                return Some(run_id);
            }
            self.queued_runs.push_back(run_id);
        }
        None
    }

    fn claim_matching_queued_run(
        &mut self,
        runner_id: crate::TurnRunnerId,
        lease_token: TurnLeaseToken,
        scope_filter: Option<&TurnScope>,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        let Some(run_id) = self.pop_matching_queued_run(scope_filter) else {
            return Ok(None);
        };
        let mut record = self.take_record(run_id)?;
        let now = Utc::now();
        let transition = record.status.set(TurnStatus::Running);
        record.runner_id = Some(runner_id);
        record.lease_token = Some(lease_token);
        record.lease_expires_at = Some(self.next_lease_expiry(now));
        record.last_heartbeat_at = Some(now);
        record.claim_count = record.claim_count.saturating_add(1);
        record.event_cursor = self.next_cursor();
        self.update_active_lock(&record, now);
        self.apply_status_transition(transition, &record);
        let claimed = ClaimedTurnRun {
            state: record.state(),
            resolved_run_profile: record.profile.resolved.clone(),
            runner_id,
            lease_token,
        };
        self.push_event(&record, TurnEventKind::RunnerClaimed, None, None);
        self.records.insert(run_id, record);
        Ok(Some(claimed))
    }

    fn remove_queued_run(&mut self, run_id: TurnRunId) {
        self.queued_runs
            .retain(|queued_run_id| *queued_run_id != run_id);
    }

    fn resume_turn_once(
        &mut self,
        request: &ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let mut record = self.take_record(request.run_id)?;
        let result = (|| {
            if record.scope != request.scope {
                return Err(TurnError::ScopeNotFound);
            }
            let resumable_status = match request.precondition.required_status() {
                Some(required) => record.status.get() == required,
                None => matches!(
                    record.status.get(),
                    TurnStatus::BlockedApproval
                        | TurnStatus::BlockedAuth
                        | TurnStatus::BlockedResource
                ),
            };
            if !resumable_status {
                return Err(TurnError::InvalidTransition {
                    from: record.status.get(),
                    to: TurnStatus::Queued,
                });
            }
            if record.actor != request.actor {
                return Err(TurnError::Unauthorized);
            }
            if record.gate_ref.as_ref() != Some(&request.gate_resolution_ref) {
                return Err(TurnError::InvalidRequest {
                    reason: "gate resolution reference mismatch".to_string(),
                });
            }
            let now = Utc::now();
            let transition = record.status.set(TurnStatus::Queued);
            self.apply_status_transition(transition, &record);
            record.resume_disposition = request.resume_disposition.clone();
            record.gate_ref = None;
            record.credential_requirements = Vec::new();
            record.source_binding_ref = request.source_binding_ref.clone();
            record.reply_target_binding_ref = request.reply_target_binding_ref.clone();
            record.event_cursor = self.next_cursor();
            self.update_active_lock(&record, now);
            self.queued_runs.push_back(record.run_id);
            let response = ResumeTurnResponse {
                run_id: record.run_id,
                status: record.status.get(),
                event_cursor: record.event_cursor,
            };
            self.push_event(&record, TurnEventKind::Resumed, None, None);
            Ok(response)
        })();
        self.records.insert(record.run_id, record);
        result
    }

    fn retry_turn_once(
        &mut self,
        request: &RetryTurnRequest,
        admission_limit_provider: &dyn TurnAdmissionLimitProvider,
    ) -> Result<RetryTurnResponse, TurnError> {
        let (
            lock_key,
            source_checkpoint,
            scope,
            actor,
            turn_id,
            profile,
            accepted_message_ref,
            parent_run_id,
            subagent_depth,
            spawn_tree_root_run_id,
            product_context,
        ) = {
            let Some(failed) = self.records.get(&request.run_id) else {
                return Err(TurnError::ScopeNotFound);
            };
            if failed.scope != request.scope {
                return Err(TurnError::ScopeNotFound);
            }
            if failed.actor != request.actor {
                return Err(TurnError::Unauthorized);
            }
            if failed.status.get() != TurnStatus::Failed || !self.is_latest_run_for_turn(failed) {
                return Err(TurnError::RunNotRetryable {
                    run_id: request.run_id,
                });
            }
            let Some(source_checkpoint_id) = failed.checkpoint_id else {
                return Err(TurnError::RunNotRetryable {
                    run_id: request.run_id,
                });
            };
            let Some(source_checkpoint) =
                self.retryable_loop_checkpoint(failed, source_checkpoint_id)
            else {
                return Err(TurnError::RunNotRetryable {
                    run_id: request.run_id,
                });
            };
            (
                TurnActiveLockKey::from(&failed.scope),
                source_checkpoint,
                failed.scope.clone(),
                failed.actor.clone(),
                failed.turn_id,
                failed.profile.clone(),
                failed.accepted_message_ref.clone(),
                failed.parent_run_id,
                failed.subagent_depth,
                failed.spawn_tree_root_run_id,
                failed.product_context.clone(),
            )
        };
        if let Some(response) = self.thread_busy(&lock_key) {
            return Err(TurnError::ThreadBusy(response));
        }

        let now = Utc::now();
        let mut new_run_id = fresh_turn_run_id();
        while self.records.contains_key(&new_run_id) {
            new_run_id = fresh_turn_run_id();
        }
        let admission_class = profile.admission_class.clone();
        if let Err(rejection) = self.reserve_admission(
            new_run_id,
            admission_class,
            &scope,
            &actor,
            admission_limit_provider,
        ) {
            return Err(TurnError::AdmissionRejected(rejection));
        }
        let retry_checkpoint_id =
            self.link_loop_checkpoint_for_retry(&source_checkpoint, new_run_id, now);
        let event_cursor = self.next_cursor();
        let record = RunRecord {
            scope,
            actor,
            turn_id,
            run_id: new_run_id,
            status: RunStatusCell::new(TurnStatus::Queued),
            profile,
            resolved_model_route: None,
            accepted_message_ref,
            source_binding_ref: request.source_binding_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            checkpoint_id: Some(retry_checkpoint_id),
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor,
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: now,
            parent_run_id,
            subagent_depth,
            spawn_tree_root_run_id,
            product_context,
            resume_disposition: None,
        };
        self.active_locks.insert(
            lock_key.clone(),
            TurnActiveLockRecord {
                key: lock_key,
                run_id: new_run_id,
                status: TurnStatus::Queued,
                lock_version: TurnLockVersion::new(1),
                acquired_at: now,
                updated_at: now,
            },
        );
        self.queued_runs.push_back(new_run_id);
        self.records.insert(new_run_id, record.clone());
        self.push_event(&record, TurnEventKind::Resumed, None, None);
        Ok(RetryTurnResponse {
            run_id: new_run_id,
            status: TurnStatus::Queued,
            event_cursor,
        })
    }

    fn request_cancel_once(
        &mut self,
        request: &CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        let mut record = self.take_record(request.run_id)?;
        let result = (|| {
            if record.scope != request.scope {
                return Err(TurnError::ScopeNotFound);
            }
            if record.actor != request.actor {
                return Err(TurnError::Unauthorized);
            }
            if record.status.get().is_terminal() {
                return Ok(CancelRunResponse {
                    run_id: record.run_id,
                    status: record.status.get(),
                    event_cursor: record.event_cursor,
                    already_terminal: true,
                    actor: Some(record.actor.clone()),
                });
            }
            let (next_status, event_kind) = match record.status.get() {
                TurnStatus::Queued
                | TurnStatus::BlockedApproval
                | TurnStatus::BlockedAuth
                | TurnStatus::BlockedResource
                | TurnStatus::BlockedDependentRun => {
                    (TurnStatus::Cancelled, TurnEventKind::Cancelled)
                }
                TurnStatus::Running | TurnStatus::CancelRequested => {
                    (TurnStatus::CancelRequested, TurnEventKind::CancelRequested)
                }
                status => {
                    return Ok(CancelRunResponse {
                        run_id: record.run_id,
                        status,
                        event_cursor: record.event_cursor,
                        already_terminal: true,
                        actor: Some(record.actor.clone()),
                    });
                }
            };
            let now = Utc::now();
            let transition = record.status.set(next_status);
            self.apply_status_transition(transition, &record);
            if record.status.get().is_terminal() {
                record.failure = None;
                self.release_active_lock(&record);
                self.remove_queued_run(record.run_id);
            } else {
                self.update_active_lock(&record, now);
            }
            record.event_cursor = self.next_cursor();
            let response = CancelRunResponse {
                run_id: record.run_id,
                status: record.status.get(),
                event_cursor: record.event_cursor,
                already_terminal: false,
                actor: Some(record.actor.clone()),
            };
            self.push_event(
                &record,
                event_kind,
                Some(request.reason.category().to_string()),
                None,
            );
            if record.status.get().is_terminal() {
                self.mark_terminal(record.run_id);
            }
            Ok(response)
        })();
        self.records.insert(record.run_id, record);
        self.prune_terminal_records();
        result
    }

    fn cancel_completion_transition(
        &mut self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
    ) -> Result<TurnRunState, TurnError> {
        let mut record = self.take_record(run_id)?;
        let result = (|| {
            ensure_active_lease(&record, runner_id, lease_token, Utc::now())?;
            if record.status.get() != TurnStatus::CancelRequested {
                return Err(TurnError::InvalidTransition {
                    from: record.status.get(),
                    to: TurnStatus::Cancelled,
                });
            }
            // CancelRequested → Cancelled: the run was Running when it was incremented at
            // claim; decrement now that the runner is fully releasing it.
            let transition = record.status.set(TurnStatus::Cancelled);
            self.apply_status_transition(transition, &record);
            record.failure = None;
            record.runner_id = None;
            record.lease_token = None;
            record.lease_expires_at = None;
            record.event_cursor = self.next_cursor();
            self.release_active_lock(&record);
            self.remove_queued_run(record.run_id);
            let state = record.state();
            self.push_event(&record, TurnEventKind::Cancelled, None, None);
            self.mark_terminal(record.run_id);
            Ok(state)
        })();
        self.records.insert(record.run_id, record);
        self.prune_terminal_records();
        result
    }

    fn terminal_transition(
        &mut self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        status: TurnStatus,
        failure: Option<SanitizedFailure>,
        kind: TurnEventKind,
    ) -> Result<TurnRunState, TurnError> {
        let mut record = self.take_record(run_id)?;
        let result = (|| {
            ensure_active_lease(&record, runner_id, lease_token, Utc::now())?;
            if record.status.get() == TurnStatus::CancelRequested
                || record.status.get().is_terminal()
            {
                return Err(TurnError::InvalidTransition {
                    from: record.status.get(),
                    to: status,
                });
            }
            // Old status must be Running (only non-CancelRequested, non-terminal status with a
            // lease); decrement the per-user running counter.
            let transition = record.status.set(status);
            self.apply_status_transition(transition, &record);
            if status == TurnStatus::Failed {
                // Preserve retryability: resolve to the latest resumable
                // checkpoint (BeforeModel/BeforeBlock) so lease-expired and
                // externally-failed runs can be retried, matching their
                // user-facing "Retry the run." summary. Resolves to None when
                // no resumable checkpoint exists, keeping the projected
                // `retryable` flag consistent with `retry_turn` validation
                // (both gate on a resumable-kind checkpoint).
                record.checkpoint_id = self.latest_resumable_loop_checkpoint(
                    &record.scope,
                    record.turn_id,
                    record.run_id,
                );
            }
            record.failure = failure.clone();
            record.runner_id = None;
            record.lease_token = None;
            record.lease_expires_at = None;
            record.event_cursor = self.next_cursor();
            self.release_active_lock(&record);
            self.remove_queued_run(record.run_id);
            let state = record.state();
            let event_detail = failure_detail_for_event(&kind, failure.as_ref());
            self.push_event(
                &record,
                kind,
                failure.map(SanitizedFailure::into_category),
                event_detail,
            );
            self.mark_terminal(record.run_id);
            Ok(state)
        })();
        self.records.insert(record.run_id, record);
        self.prune_terminal_records();
        result
    }

    fn apply_validated_loop_exit_transition(
        &mut self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        mapping: LoopExitMapping,
    ) -> Result<TurnRunState, TurnError> {
        let record = self.take_record(run_id)?;
        let result = (|| {
            if let Err(error) = ensure_active_lease(&record, runner_id, lease_token, Utc::now()) {
                return AppliedLoopTransition::Rejected {
                    record: Box::new(record),
                    error,
                };
            }
            match mapping {
                LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed) => {
                    self.complete_claimed_record(record)
                }
                LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Cancelled) => self
                    .cancel_or_fail_claimed_record(
                        record,
                        SanitizedFailure::from_trusted_static("interrupted_unexpectedly"),
                    ),
                LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Blocked {
                    checkpoint_id,
                    state_ref,
                    reason,
                    blocked_activity_id,
                }) => self.block_claimed_record(
                    record,
                    checkpoint_id,
                    state_ref,
                    reason,
                    blocked_activity_id,
                ),
                LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Failed { failure }) => {
                    self.fail_claimed_record(record, failure)
                }
                LoopExitMapping::RecoveryRequired { failure } => {
                    self.cancel_or_fail_claimed_record(record, failure)
                }
            }
        })();
        self.commit_transition(result)
    }

    fn commit_transition(
        &mut self,
        transition: AppliedLoopTransition,
    ) -> Result<TurnRunState, TurnError> {
        match transition {
            AppliedLoopTransition::Applied {
                record,
                state,
                prune_terminal,
            } => {
                self.records.insert(record.run_id, *record);
                if prune_terminal {
                    self.prune_terminal_records();
                }
                Ok(*state)
            }
            AppliedLoopTransition::Rejected { record, error } => {
                self.records.insert(record.run_id, *record);
                Err(error)
            }
        }
    }

    fn complete_claimed_record(&mut self, mut record: RunRecord) -> AppliedLoopTransition {
        if record.status.get() != TurnStatus::Running {
            let from = record.status.get();
            return AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: TurnStatus::Completed,
                },
            };
        }
        // Running → Completed: decrement per-user running counter.
        let transition = record.status.set(TurnStatus::Completed);
        self.apply_status_transition(transition, &record);
        record.failure = None;
        record.runner_id = None;
        record.lease_token = None;
        record.lease_expires_at = None;
        record.event_cursor = self.next_cursor();
        self.release_active_lock(&record);
        self.remove_queued_run(record.run_id);
        let state = record.state();
        self.push_event(&record, TurnEventKind::Completed, None, None);
        self.mark_terminal(record.run_id);
        AppliedLoopTransition::Applied {
            record: Box::new(record),
            state: Box::new(state),
            prune_terminal: true,
        }
    }

    fn cancel_claimed_record(&mut self, mut record: RunRecord) -> AppliedLoopTransition {
        if record.status.get() != TurnStatus::CancelRequested {
            let from = record.status.get();
            return AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: TurnStatus::Cancelled,
                },
            };
        }
        // CancelRequested → Cancelled via loop exit: the run was Running when incremented at
        // claim; decrement now that the runner is fully releasing it.
        let transition = record.status.set(TurnStatus::Cancelled);
        self.apply_status_transition(transition, &record);
        record.failure = None;
        record.runner_id = None;
        record.lease_token = None;
        record.lease_expires_at = None;
        record.event_cursor = self.next_cursor();
        self.release_active_lock(&record);
        self.remove_queued_run(record.run_id);
        let state = record.state();
        self.push_event(&record, TurnEventKind::Cancelled, None, None);
        self.mark_terminal(record.run_id);
        AppliedLoopTransition::Applied {
            record: Box::new(record),
            state: Box::new(state),
            prune_terminal: true,
        }
    }

    fn block_claimed_record(
        &mut self,
        mut record: RunRecord,
        checkpoint_id: TurnCheckpointId,
        state_ref: crate::run_profile::LoopCheckpointStateRef,
        reason: BlockedReason,
        blocked_activity_id: Option<crate::CapabilityActivityId>,
    ) -> AppliedLoopTransition {
        if record.status.get() != TurnStatus::Running {
            let from = record.status.get();
            return AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: reason.status(),
                },
            };
        }
        // Running → Blocked: decrement per-user running counter.
        let now = Utc::now();
        let transition = record.status.set(reason.status());
        self.apply_status_transition(transition, &record);
        record.checkpoint_id = Some(checkpoint_id);
        record.gate_ref = Some(reason.gate_ref().clone());
        record.blocked_activity_id = blocked_activity_id;
        record.credential_requirements = reason.credential_requirements().to_vec();
        record.runner_id = None;
        record.lease_token = None;
        record.lease_expires_at = None;
        record.event_cursor = self.next_cursor();
        self.record_checkpoint(
            &record,
            checkpoint_id,
            state_ref,
            reason.gate_ref().clone(),
            now,
        );
        self.update_active_lock(&record, now);
        let state = record.state();
        self.push_event(&record, TurnEventKind::Blocked, None, None);
        AppliedLoopTransition::Applied {
            record: Box::new(record),
            state: Box::new(state),
            prune_terminal: false,
        }
    }

    fn fail_claimed_record(
        &mut self,
        mut record: RunRecord,
        failure: SanitizedFailure,
    ) -> AppliedLoopTransition {
        if record.status.get() != TurnStatus::Running {
            let from = record.status.get();
            return AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: TurnStatus::Failed,
                },
            };
        }
        let retry_checkpoint_id =
            self.latest_resumable_loop_checkpoint(&record.scope, record.turn_id, record.run_id);
        // Running → Failed: decrement per-user running counter.
        let transition = record.status.set(TurnStatus::Failed);
        self.apply_status_transition(transition, &record);
        record.checkpoint_id = retry_checkpoint_id;
        record.failure = Some(failure.clone());
        record.runner_id = None;
        record.lease_token = None;
        record.lease_expires_at = None;
        record.event_cursor = self.next_cursor();
        self.release_active_lock(&record);
        self.remove_queued_run(record.run_id);
        let state = record.state();
        let event_detail = failure.detail().map(str::to_string);
        self.push_event(
            &record,
            TurnEventKind::Failed,
            Some(failure.into_category()),
            event_detail,
        );
        self.mark_terminal(record.run_id);
        AppliedLoopTransition::Applied {
            record: Box::new(record),
            state: Box::new(state),
            prune_terminal: true,
        }
    }

    fn cancel_or_fail_claimed_record(
        &mut self,
        record: RunRecord,
        failure: SanitizedFailure,
    ) -> AppliedLoopTransition {
        let from = record.status.get();
        match from {
            TurnStatus::Running => {
                // Mirror terminal_transition: a runner-reported failure keeps the
                // run retryable from its latest resumable checkpoint rather than
                // discarding it.
                self.fail_claimed_record(record, failure)
            }
            TurnStatus::CancelRequested => self.cancel_claimed_record(record),
            _ => AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: TurnStatus::Failed,
                },
            },
        }
    }

    fn runner_failure_transition(
        &mut self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        failure: SanitizedFailure,
    ) -> Result<TurnRunState, TurnError> {
        let record = self.take_record(run_id)?;
        let transition = (|| {
            if let Err(error) = ensure_active_lease(&record, runner_id, lease_token, Utc::now()) {
                return AppliedLoopTransition::Rejected {
                    record: Box::new(record),
                    error,
                };
            }
            self.cancel_or_fail_claimed_record(record, failure)
        })();
        self.commit_transition(transition)
    }

    fn relinquish_transition(
        &mut self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
    ) -> Result<TurnRunState, TurnError> {
        let now = Utc::now();
        let mut record = self.take_record(run_id)?;
        let mut requeue = false;
        let result = (|| {
            ensure_active_lease(&record, runner_id, lease_token, now)?;
            if !matches!(
                record.status.get(),
                TurnStatus::Running | TurnStatus::CancelRequested
            ) {
                return Err(TurnError::InvalidTransition {
                    from: record.status.get(),
                    to: TurnStatus::Queued,
                });
            }
            let (new_status, failure, event_kind) = match record.status.get() {
                TurnStatus::Running => {
                    // Running → Queued (relinquish): decrement per-user running counter.
                    requeue = true;
                    (TurnStatus::Queued, None, TurnEventKind::RunnerHeartbeat)
                }
                TurnStatus::CancelRequested => {
                    // CancelRequested → Cancelled (relinquish): the run was Running when
                    // incremented at claim; decrement now.
                    (TurnStatus::Cancelled, None, TurnEventKind::Cancelled)
                }
                _ => unreachable!("status checked above"),
            };
            let transition = record.status.set(new_status);
            self.apply_status_transition(transition, &record);
            record.failure = failure;
            record.runner_id = None;
            record.lease_token = None;
            record.lease_expires_at = None;
            record.event_cursor = self.next_cursor();
            if requeue {
                self.update_active_lock(&record, now);
            } else {
                self.release_active_lock(&record);
                self.remove_queued_run(record.run_id);
            }
            let state = record.state();
            self.push_event(&record, event_kind, None, None);
            if !requeue {
                self.mark_terminal(record.run_id);
            }
            Ok(state)
        })();
        self.records.insert(record.run_id, record);
        if requeue && result.is_ok() {
            self.queued_runs.push_back(run_id);
        }
        if result
            .as_ref()
            .is_ok_and(|state| state.status.is_terminal())
        {
            self.prune_terminal_records();
        }
        result
    }

    fn update_active_lock(&mut self, record: &RunRecord, updated_at: crate::TurnTimestamp) {
        let lock_key = TurnActiveLockKey::from(&record.scope);
        if let Some(lock) = self.active_locks.get_mut(&lock_key)
            && lock.run_id == record.run_id
        {
            lock.status = record.status.get();
            lock.lock_version = lock.lock_version.incremented();
            lock.updated_at = updated_at;
        }
    }

    fn touch_active_lock(&mut self, record: &RunRecord, updated_at: crate::TurnTimestamp) {
        let lock_key = TurnActiveLockKey::from(&record.scope);
        if let Some(lock) = self.active_locks.get_mut(&lock_key)
            && lock.run_id == record.run_id
        {
            lock.updated_at = updated_at;
        }
    }

    fn thread_busy(&self, lock_key: &TurnActiveLockKey) -> Option<ThreadBusy> {
        let active_lock = self.active_locks.get(lock_key)?;
        let record = self.records.get(&active_lock.run_id)?;
        record
            .status
            .get()
            .keeps_active_lock()
            .then_some(ThreadBusy {
                active_run_id: active_lock.run_id,
                status: record.status.get(),
                event_cursor: record.event_cursor,
            })
    }

    fn is_latest_run_for_turn(&self, record: &RunRecord) -> bool {
        !self.records.values().any(|candidate| {
            candidate.turn_id == record.turn_id && candidate.event_cursor > record.event_cursor
        })
    }

    fn retryable_loop_checkpoint(
        &self,
        record: &RunRecord,
        checkpoint_id: TurnCheckpointId,
    ) -> Option<LoopCheckpointRecord> {
        self.loop_checkpoints
            .get(&checkpoint_id)
            .filter(|checkpoint| {
                checkpoint.scope == record.scope
                    && checkpoint.turn_id == record.turn_id
                    && checkpoint.run_id == record.run_id
                    && matches!(
                        checkpoint.kind,
                        crate::run_profile::LoopCheckpointKind::BeforeModel
                            | crate::run_profile::LoopCheckpointKind::BeforeBlock
                    )
            })
            .cloned()
    }

    fn link_loop_checkpoint_for_retry(
        &mut self,
        source: &LoopCheckpointRecord,
        retry_run_id: TurnRunId,
        created_at: crate::TurnTimestamp,
    ) -> TurnCheckpointId {
        let checkpoint_id = TurnCheckpointId::new();
        self.loop_checkpoints.insert(
            checkpoint_id,
            LoopCheckpointRecord {
                checkpoint_id,
                scope: source.scope.clone(),
                turn_id: source.turn_id,
                run_id: retry_run_id,
                state_ref: source.state_ref.clone(),
                schema_id: source.schema_id.clone(),
                schema_version: source.schema_version,
                kind: source.kind,
                gate_ref: source.gate_ref.clone(),
                created_at,
            },
        );
        checkpoint_id
    }

    fn latest_resumable_loop_checkpoint(
        &self,
        scope: &TurnScope,
        turn_id: crate::TurnId,
        run_id: TurnRunId,
    ) -> Option<TurnCheckpointId> {
        self.loop_checkpoints
            .values()
            .filter(|checkpoint| {
                checkpoint.scope == *scope
                    && checkpoint.turn_id == turn_id
                    && checkpoint.run_id == run_id
                    && matches!(
                        checkpoint.kind,
                        crate::run_profile::LoopCheckpointKind::BeforeModel
                            | crate::run_profile::LoopCheckpointKind::BeforeBlock
                    )
            })
            .max_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then_with(|| a.checkpoint_id.as_uuid().cmp(&b.checkpoint_id.as_uuid()))
            })
            .map(|checkpoint| checkpoint.checkpoint_id)
    }

    fn reserve_admission(
        &mut self,
        run_id: TurnRunId,
        admission_class: TurnAdmissionClass,
        scope: &TurnScope,
        actor: &TurnActor,
        limit_provider: &dyn TurnAdmissionLimitProvider,
    ) -> Result<(), AdmissionRejection> {
        let buckets = admission_buckets(scope, actor, &admission_class);
        for bucket in &buckets {
            let limit = limit_provider
                .limit_for(bucket)
                .map_err(|_| AdmissionRejection::new(AdmissionRejectionReason::Unavailable))?;
            if let Some(max_active) = limit.max_active {
                let active_count = self.active_admission_count(bucket);
                if active_count >= max_active {
                    return Err(
                        AdmissionRejection::new(AdmissionRejectionReason::TenantLimit)
                            .with_capacity_denial(crate::TurnAdmissionCapacityDenial {
                                axis_kind: bucket.axis_kind,
                                bucket_kind: bucket.bucket_kind,
                                admission_class: bucket.admission_class.clone(),
                                limit: max_active,
                                active_count,
                                retry_after_ms: limit.retry_after_ms,
                            }),
                    );
                }
            }
        }
        self.admission_reservations.insert(
            run_id,
            TurnAdmissionReservationRecord {
                run_id,
                admission_class,
                buckets,
                released: false,
            },
        );
        Ok(())
    }

    fn active_admission_count(&self, bucket: &TurnAdmissionBucket) -> u64 {
        self.admission_reservations
            .values()
            .filter(|reservation| {
                !reservation.released
                    && reservation
                        .buckets
                        .iter()
                        .any(|reserved| reserved == bucket)
            })
            .count() as u64
    }

    fn active_admission_reservations(&self) -> Vec<TurnAdmissionReservationRecord> {
        self.admission_reservations
            .values()
            .filter(|reservation| !reservation.released)
            .cloned()
            .collect()
    }

    fn release_admission(&mut self, run_id: TurnRunId) {
        if let Some(reservation) = self.admission_reservations.get_mut(&run_id) {
            reservation.released = true;
        }
    }

    fn record_checkpoint(
        &mut self,
        record: &RunRecord,
        checkpoint_id: TurnCheckpointId,
        state_ref: crate::run_profile::LoopCheckpointStateRef,
        gate_ref: crate::GateRef,
        created_at: crate::TurnTimestamp,
    ) {
        let sequence = self
            .checkpoints
            .iter()
            .filter(|checkpoint| checkpoint.run_id == record.run_id)
            .count()
            .saturating_add(1) as u64;
        self.checkpoints.push(TurnCheckpointRecord {
            checkpoint_id,
            run_id: record.run_id,
            scope: Some(record.scope.clone()),
            sequence,
            status: record.status.get(),
            gate_ref,
            kind: crate::run_profile::LoopCheckpointKind::BeforeBlock,
            state_ref,
            created_at,
        });
    }

    fn release_active_lock(&mut self, record: &RunRecord) {
        let lock_key = TurnActiveLockKey::from(&record.scope);
        if self
            .active_locks
            .get(&lock_key)
            .is_some_and(|lock| lock.run_id == record.run_id)
        {
            self.active_locks.remove(&lock_key);
        }
    }

    fn mark_terminal(&mut self, run_id: TurnRunId) {
        self.release_admission(run_id);
        self.terminal_runs.push_back(run_id);
    }

    fn prune_terminal_records(&mut self) {
        while self.terminal_runs.len() > self.limits.max_terminal_records {
            let Some(run_id) = self.terminal_runs.pop_front() else {
                break;
            };
            if self
                .records
                .get(&run_id)
                .is_some_and(|record| record.status.get().is_terminal())
                && !self
                    .tree_reservations
                    .keys()
                    .any(|reservation| reservation.root_run_id == run_id)
            {
                if let Some(record) = self.records.remove(&run_id) {
                    let turn_id = record.turn_id;
                    if !self
                        .records
                        .values()
                        .any(|record| record.turn_id == turn_id)
                    {
                        self.turns.remove(&turn_id);
                    }
                }
                self.admission_reservations.remove(&run_id);
            }
        }
    }
}

impl RunRecord {
    fn persistence_record(&self) -> TurnRunRecord {
        TurnRunRecord {
            run_id: self.run_id,
            turn_id: self.turn_id,
            scope: self.scope.clone(),
            accepted_message_ref: self.accepted_message_ref.clone(),
            source_binding_ref: self.source_binding_ref.clone(),
            reply_target_binding_ref: self.reply_target_binding_ref.clone(),
            status: self.status.get(),
            profile: self.profile.clone(),
            resolved_model_route: self.resolved_model_route.clone(),
            checkpoint_id: self.checkpoint_id,
            gate_ref: self.gate_ref.clone(),
            blocked_activity_id: self.blocked_activity_id,
            credential_requirements: self.credential_requirements.clone(),
            failure: self.failure.clone(),
            event_cursor: self.event_cursor,
            runner_id: self.runner_id,
            lease_token: self.lease_token,
            lease_expires_at: self.lease_expires_at,
            last_heartbeat_at: self.last_heartbeat_at,
            claim_count: self.claim_count,
            received_at: self.received_at,
            parent_run_id: self.parent_run_id,
            subagent_depth: self.subagent_depth,
            spawn_tree_root_run_id: self.spawn_tree_root_run_id,
            product_context: self.product_context.clone(),
            resume_disposition: self.resume_disposition.clone(),
        }
    }

    fn state(&self) -> TurnRunState {
        TurnRunState {
            scope: self.scope.clone(),
            actor: Some(self.actor.clone()),
            turn_id: self.turn_id,
            run_id: self.run_id,
            status: self.status.get(),
            accepted_message_ref: self.accepted_message_ref.clone(),
            source_binding_ref: self.source_binding_ref.clone(),
            reply_target_binding_ref: self.reply_target_binding_ref.clone(),
            resolved_run_profile_id: self.profile.id.clone(),
            resolved_run_profile_version: self.profile.version,
            resolved_model_route: self.resolved_model_route.clone(),
            received_at: self.received_at,
            checkpoint_id: self.checkpoint_id,
            gate_ref: self.gate_ref.clone(),
            blocked_activity_id: self.blocked_activity_id,
            credential_requirements: self.credential_requirements.clone(),
            failure: self.failure.clone(),
            event_cursor: self.event_cursor,
            product_context: self.product_context.clone(),
            resume_disposition: self.resume_disposition.clone(),
        }
    }
}

fn slot_info_for(record: &RunRecord) -> RunSlotInfo<'_> {
    RunSlotInfo {
        tenant_id: &record.scope.tenant_id,
        thread_owner: &record.scope.thread_owner,
        actor_user_id: &record.actor.user_id,
        product_context: match record.product_context.as_ref().map(|pc| pc.origin) {
            Some(crate::TurnOriginKind::ScheduledTrigger) => Some(OriginClass::Trigger),
            Some(crate::TurnOriginKind::Inbound) | Some(crate::TurnOriginKind::WebUi) => {
                Some(OriginClass::Conversation)
            }
            None => None,
        },
    }
}

fn persisted_key_for_record(record: &TurnIdempotencyRecord) -> PersistedIdempotencyKey {
    PersistedIdempotencyKey {
        scope: record.scope.clone(),
        operation: record.operation,
        run_id: match record.operation {
            TurnIdempotencyOperationKind::Submit => None,
            TurnIdempotencyOperationKind::Resume
            | TurnIdempotencyOperationKind::Retry
            | TurnIdempotencyOperationKind::Cancel => record.run_id,
        },
        key: record.key.clone(),
    }
}

/// A persisted record whose replay payload no longer matches its operation
/// kind (or lacks a run id) cannot be rehydrated; the duplicate-request guard
/// for that key is lost until the operation is re-recorded. Surface it instead
/// of dropping silently. Logs metadata only — never the replay payload.
fn debug_malformed_idempotency_record(record: &TurnIdempotencyRecord) {
    tracing::debug!(
        operation = ?record.operation,
        run_id = ?record.run_id,
        "skipping malformed idempotency record during snapshot load; replay guard lost for this key"
    );
}

fn persisted_submit_key(key: &SubmitIdempotencyKey) -> PersistedIdempotencyKey {
    PersistedIdempotencyKey {
        scope: key.scope.clone(),
        operation: TurnIdempotencyOperationKind::Submit,
        run_id: None,
        key: key.key.clone(),
    }
}

fn persisted_run_key(
    operation: TurnIdempotencyOperationKind,
    key: &RunIdempotencyKey,
) -> PersistedIdempotencyKey {
    PersistedIdempotencyKey {
        scope: key.scope.clone(),
        operation,
        run_id: Some(key.run_id),
        key: key.key.clone(),
    }
}

fn submit_idempotency_record(
    key: &SubmitIdempotencyKey,
    result: &Result<SubmitTurnResponse, TurnError>,
    created_at: crate::TurnTimestamp,
) -> TurnIdempotencyRecord {
    let (turn_id, run_id, outcome, replay) = match result {
        Ok(
            response @ SubmitTurnResponse::Accepted {
                turn_id, run_id, ..
            },
        ) => (
            Some(*turn_id),
            Some(*run_id),
            TurnIdempotencyOutcomeKind::Accepted,
            TurnIdempotencyReplay::SubmitAccepted(response.clone()),
        ),
        Err(TurnError::ThreadBusy(busy)) => (
            None,
            Some(busy.active_run_id),
            TurnIdempotencyOutcomeKind::ThreadBusy,
            TurnIdempotencyReplay::SubmitThreadBusy(busy.clone()),
        ),
        Err(TurnError::AdmissionRejected(rejection)) => (
            None,
            None,
            TurnIdempotencyOutcomeKind::AdmissionRejected,
            TurnIdempotencyReplay::SubmitAdmissionRejected(rejection.clone()),
        ),
        Err(error) => (
            None,
            None,
            TurnIdempotencyOutcomeKind::from_error(error),
            TurnIdempotencyReplay::Error(TurnIdempotencyErrorReplay::from_error(error)),
        ),
    };
    TurnIdempotencyRecord {
        scope: key.scope.clone(),
        operation: TurnIdempotencyOperationKind::Submit,
        key: key.key.clone(),
        turn_id,
        run_id,
        outcome,
        replay,
        created_at,
        expires_at: None,
    }
}

fn resume_idempotency_record(
    key: &RunIdempotencyKey,
    result: &Result<ResumeTurnResponse, TurnError>,
    created_at: crate::TurnTimestamp,
) -> TurnIdempotencyRecord {
    let (outcome, replay) = match result {
        Ok(response) => (
            TurnIdempotencyOutcomeKind::Resumed,
            TurnIdempotencyReplay::ResumeSucceeded(response.clone()),
        ),
        Err(error) => (
            TurnIdempotencyOutcomeKind::from_error(error),
            TurnIdempotencyReplay::Error(TurnIdempotencyErrorReplay::from_error(error)),
        ),
    };
    TurnIdempotencyRecord {
        scope: key.scope.clone(),
        operation: TurnIdempotencyOperationKind::Resume,
        key: key.key.clone(),
        turn_id: None,
        run_id: Some(key.run_id),
        outcome,
        replay,
        created_at,
        expires_at: None,
    }
}

fn retry_idempotency_record(
    key: &RunIdempotencyKey,
    result: &Result<RetryTurnResponse, TurnError>,
    created_at: crate::TurnTimestamp,
) -> TurnIdempotencyRecord {
    let (outcome, replay) = match result {
        Ok(response) => (
            TurnIdempotencyOutcomeKind::Retried,
            TurnIdempotencyReplay::RetrySucceeded(response.clone()),
        ),
        Err(TurnError::ThreadBusy(busy)) => (
            TurnIdempotencyOutcomeKind::ThreadBusy,
            TurnIdempotencyReplay::RetryThreadBusy(busy.clone()),
        ),
        Err(error) => (
            TurnIdempotencyOutcomeKind::from_error(error),
            TurnIdempotencyReplay::Error(TurnIdempotencyErrorReplay::from_error(error)),
        ),
    };
    TurnIdempotencyRecord {
        scope: key.scope.clone(),
        operation: TurnIdempotencyOperationKind::Retry,
        key: key.key.clone(),
        turn_id: None,
        run_id: Some(key.run_id),
        outcome,
        replay,
        created_at,
        expires_at: None,
    }
}

fn cancel_idempotency_record(
    key: &RunIdempotencyKey,
    result: &Result<CancelRunResponse, TurnError>,
    created_at: crate::TurnTimestamp,
) -> TurnIdempotencyRecord {
    let (outcome, replay) = match result {
        Ok(response) => (
            TurnIdempotencyOutcomeKind::CancelRecorded,
            TurnIdempotencyReplay::CancelRecorded(response.clone()),
        ),
        Err(error) => (
            TurnIdempotencyOutcomeKind::from_error(error),
            TurnIdempotencyReplay::Error(TurnIdempotencyErrorReplay::from_error(error)),
        ),
    };
    TurnIdempotencyRecord {
        scope: key.scope.clone(),
        operation: TurnIdempotencyOperationKind::Cancel,
        key: key.key.clone(),
        turn_id: None,
        run_id: Some(key.run_id),
        outcome,
        replay,
        created_at,
        expires_at: None,
    }
}

fn ensure_active_lease(
    record: &RunRecord,
    runner_id: crate::TurnRunnerId,
    lease_token: crate::TurnLeaseToken,
    now: crate::TurnTimestamp,
) -> Result<(), TurnError> {
    if record.runner_id != Some(runner_id) || record.lease_token != Some(lease_token) {
        return Err(TurnError::LeaseMismatch);
    }
    if record
        .lease_expires_at
        .is_some_and(|expires_at| expires_at <= now)
    {
        return Err(TurnError::Conflict {
            reason: "turn run lease expired".to_string(),
        });
    }
    Ok(())
}

struct LeaseExpiredOutcome {
    status: TurnStatus,
    failure: Option<SanitizedFailure>,
    event_kind: TurnEventKind,
    event_detail: Option<String>,
}

fn expired_lease_terminal_outcome(status: TurnStatus) -> LeaseExpiredOutcome {
    match status {
        TurnStatus::CancelRequested => LeaseExpiredOutcome {
            status: TurnStatus::Cancelled,
            failure: None,
            event_kind: TurnEventKind::Cancelled,
            event_detail: None,
        },
        _ => {
            let failure = SanitizedFailure::from_trusted_static("lease_expired");
            LeaseExpiredOutcome {
                status: TurnStatus::Failed,
                failure: Some(failure.clone()),
                event_kind: TurnEventKind::Failed,
                event_detail: Some(failure.into_category()),
            }
        }
    }
}

fn prune_ordered_map<K, V>(
    map: &mut HashMap<K, V>,
    order: &mut VecDeque<K>,
    max_len: usize,
) -> Vec<K>
where
    K: Eq + Hash,
{
    let mut removed = Vec::new();
    while map.len() > max_len {
        let Some(key) = order.pop_front() else {
            break;
        };
        if map.remove(&key).is_some() {
            removed.push(key);
        }
    }

    while order.front().is_some_and(|key| !map.contains_key(key)) {
        order.pop_front();
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AllowAllTurnAdmissionPolicy, ResolvedRunProfile, RunProfileId, RunProfileVersion,
        TurnLeaseToken, TurnRunnerId,
    };
    use async_trait::async_trait;
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
        let limits = InMemoryTurnStateStoreLimits {
            max_terminal_records: 1,
            ..InMemoryTurnStateStoreLimits::default()
        };
        let store = InMemoryTurnStateStore::with_limits(limits);
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
        let store = InMemoryTurnStateStore::default();
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
