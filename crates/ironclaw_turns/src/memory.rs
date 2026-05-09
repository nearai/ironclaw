use async_trait::async_trait;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::Hash,
    sync::{Arc, Condvar, Mutex, MutexGuard},
};

use chrono::{Duration as ChronoDuration, Utc};

use crate::{
    AcceptedMessageRef, AdmissionRejection, AdmissionRejectionReason,
    AllowAllTurnAdmissionLimitProvider, BlockedReason, CancelRunRequest, CancelRunResponse,
    GetLoopCheckpointRequest, GetRunStateRequest, IdempotencyKey, LoopCheckpointRecord,
    LoopCheckpointStore, LoopExitMapping, PutLoopCheckpointRequest, ReplyTargetBindingRef,
    ResumeTurnRequest, ResumeTurnResponse, RunProfileResolutionError, RunProfileResolutionRequest,
    RunProfileResolver, SanitizedFailure, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
    ThreadBusy, TurnActiveLockKey, TurnActiveLockRecord, TurnActor, TurnAdmissionClass,
    TurnAdmissionLimitProvider, TurnAdmissionPolicy, TurnAdmissionReservationRecord,
    TurnCheckpointId, TurnCheckpointRecord, TurnError, TurnEventKind, TurnIdempotencyErrorReplay,
    TurnIdempotencyOperationKind, TurnIdempotencyOutcomeKind, TurnIdempotencyRecord,
    TurnIdempotencyReplay, TurnLifecycleEvent, TurnLockVersion, TurnPersistenceSnapshot,
    TurnRecord, TurnRunId, TurnRunProfile, TurnRunRecord, TurnRunState, TurnScope, TurnStateStore,
    TurnStatus,
    admission::{TurnAdmissionBucket, admission_buckets},
    events::{EventCursor, TurnEventPage, TurnEventProjectionSource, project_turn_events},
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordRecoveryRequiredRequest, RecoverExpiredLeasesRequest, RecoverExpiredLeasesResponse,
        TurnRunTransitionPort, TurnRunnerOutcome,
    },
};

const MAX_EVENTS: usize = 10_000;
const MAX_TERMINAL_RECORDS: usize = 10_000;
const MAX_IDEMPOTENCY_RECORDS: usize = 10_000;
const DEFAULT_RUNNER_LEASE_TTL_SECONDS: i64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InMemoryTurnStateStoreLimits {
    pub max_events: usize,
    pub max_terminal_records: usize,
    pub max_idempotency_records: usize,
    pub runner_lease_ttl: ChronoDuration,
}

impl Default for InMemoryTurnStateStoreLimits {
    fn default() -> Self {
        Self {
            max_events: MAX_EVENTS,
            max_terminal_records: MAX_TERMINAL_RECORDS,
            max_idempotency_records: MAX_IDEMPOTENCY_RECORDS,
            runner_lease_ttl: ChronoDuration::seconds(DEFAULT_RUNNER_LEASE_TTL_SECONDS),
        }
    }
}

pub struct InMemoryTurnStateStore {
    inner: Mutex<Inner>,
    submit_idempotency_ready: Condvar,
    admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
}

impl Default for InMemoryTurnStateStore {
    fn default() -> Self {
        Self::with_limits(InMemoryTurnStateStoreLimits::default())
    }
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
    cancel_idempotency: HashMap<RunIdempotencyKey, Result<CancelRunResponse, TurnError>>,
    idempotency_records: HashMap<PersistedIdempotencyKey, TurnIdempotencyRecord>,
    submit_idempotency_order: VecDeque<SubmitIdempotencyKey>,
    resume_idempotency_order: VecDeque<RunIdempotencyKey>,
    cancel_idempotency_order: VecDeque<RunIdempotencyKey>,
    idempotency_record_order: VecDeque<PersistedIdempotencyKey>,
    events: Vec<TurnLifecycleEvent>,
    event_retention_floor: EventCursor,
    admission_reservations: HashMap<TurnRunId, TurnAdmissionReservationRecord>,
    limits: InMemoryTurnStateStoreLimits,
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
    status: TurnStatus,
    profile: TurnRunProfile,
    accepted_message_ref: AcceptedMessageRef,
    source_binding_ref: SourceBindingRef,
    reply_target_binding_ref: ReplyTargetBindingRef,
    checkpoint_id: Option<TurnCheckpointId>,
    gate_ref: Option<crate::GateRef>,
    failure: Option<SanitizedFailure>,
    event_cursor: EventCursor,
    runner_id: Option<crate::TurnRunnerId>,
    lease_token: Option<crate::TurnLeaseToken>,
    lease_expires_at: Option<crate::TurnTimestamp>,
    last_heartbeat_at: Option<crate::TurnTimestamp>,
    claim_count: u64,
    received_at: crate::TurnTimestamp,
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

struct SubmitInFlightGuard<'a> {
    inner: &'a Mutex<Inner>,
    ready: &'a Condvar,
    key: SubmitIdempotencyKey,
}

impl<'a> SubmitInFlightGuard<'a> {
    fn new(inner: &'a Mutex<Inner>, ready: &'a Condvar, key: SubmitIdempotencyKey) -> Self {
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
            self.ready.notify_all();
        }
    }
}

impl InMemoryTurnStateStore {
    pub fn with_limits(limits: InMemoryTurnStateStoreLimits) -> Self {
        Self {
            inner: Mutex::new(Inner {
                limits,
                ..Inner::default()
            }),
            submit_idempotency_ready: Condvar::new(),
            admission_limit_provider: Arc::new(AllowAllTurnAdmissionLimitProvider),
        }
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
                limits,
                ..Inner::default()
            }),
            submit_idempotency_ready: Condvar::new(),
            admission_limit_provider,
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
            submit_idempotency_ready: Condvar::new(),
            admission_limit_provider,
        })
    }

    pub fn persistence_snapshot(&self) -> TurnPersistenceSnapshot {
        match self.inner.lock() {
            Ok(inner) => inner.persistence_snapshot(),
            Err(poisoned) => poisoned.into_inner().persistence_snapshot(),
        }
    }

    fn lock_inner(&self) -> Result<MutexGuard<'_, Inner>, TurnError> {
        self.inner.lock().map_err(|_| TurnError::Unavailable {
            reason: "turn state store mutex poisoned".to_string(),
        })
    }

    fn wait_for_submit_idempotency<'a>(
        &self,
        inner: MutexGuard<'a, Inner>,
    ) -> Result<MutexGuard<'a, Inner>, TurnError> {
        self.submit_idempotency_ready
            .wait(inner)
            .map_err(|_| TurnError::Unavailable {
                reason: "turn state store mutex poisoned".to_string(),
            })
    }
}

#[async_trait]
impl TurnEventProjectionSource for InMemoryTurnStateStore {
    async fn read_turn_events_after(
        &self,
        scope: &TurnScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        let inner = self.lock_inner()?;
        Ok(project_turn_events(
            &inner.events,
            scope,
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
        {
            let mut inner = self.lock_inner()?;
            loop {
                if let Some(result) = inner.submit_idempotency.get(&idempotency_key) {
                    return result.clone();
                }
                if inner
                    .submit_idempotency_in_flight
                    .insert(idempotency_key.clone())
                {
                    break;
                }
                inner = self.wait_for_submit_idempotency(inner)?;
            }
        }
        let _in_flight_guard = SubmitInFlightGuard::new(
            &self.inner,
            &self.submit_idempotency_ready,
            idempotency_key.clone(),
        );

        let admission_result = admission_policy.check_submit(&request);

        {
            let mut inner = self.lock_inner()?;
            if let Some(result) = inner.submit_idempotency.get(&idempotency_key).cloned() {
                inner.submit_idempotency_in_flight.remove(&idempotency_key);
                self.submit_idempotency_ready.notify_all();
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
                self.submit_idempotency_ready.notify_all();
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
            self.submit_idempotency_ready.notify_all();
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
                self.submit_idempotency_ready.notify_all();
                return response;
            }
        };

        let lock_key = TurnActiveLockKey::from(&request.scope);
        if let Some(response) = inner.thread_busy(&lock_key) {
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_all();
            return Err(TurnError::ThreadBusy(response));
        }

        let turn_id = crate::TurnId::new();
        let run_id = TurnRunId::new();
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
            self.submit_idempotency_ready.notify_all();
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
            status: TurnStatus::Queued,
            profile: profile.clone(),
            accepted_message_ref: request.accepted_message_ref.clone(),
            source_binding_ref: request.source_binding_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            checkpoint_id: None,
            gate_ref: None,
            failure: None,
            event_cursor: cursor,
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: request.received_at,
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
        inner.push_event(&record, TurnEventKind::Submitted, None);

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
        self.submit_idempotency_ready.notify_all();
        response
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let mut inner = self.lock_inner()?;
        let idempotency_key = RunIdempotencyKey {
            scope: request.scope.clone(),
            run_id: request.run_id,
            key: request.idempotency_key.clone(),
        };
        if let Some(result) = inner.resume_idempotency.get(&idempotency_key) {
            return result.clone();
        }
        let result = inner.resume_turn_once(&request);
        inner.remember_resume_idempotency(idempotency_key, result.clone(), Utc::now());
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
impl TurnRunTransitionPort for InMemoryTurnStateStore {
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        let mut inner = self.lock_inner()?;
        let Some(run_id) = inner.pop_matching_queued_run(request.scope_filter.as_ref()) else {
            return Ok(None);
        };
        let mut record = inner.take_record(run_id)?;
        let now = Utc::now();
        record.status = TurnStatus::Running;
        record.runner_id = Some(request.runner_id);
        record.lease_token = Some(request.lease_token);
        record.lease_expires_at = Some(inner.next_lease_expiry(now));
        record.last_heartbeat_at = Some(now);
        record.claim_count = record.claim_count.saturating_add(1);
        record.event_cursor = inner.next_cursor();
        inner.update_active_lock(&record, now);
        let claimed = ClaimedTurnRun {
            state: record.state(),
            resolved_run_profile: record.profile.resolved.clone(),
            runner_id: request.runner_id,
            lease_token: request.lease_token,
        };
        inner.push_event(&record, TurnEventKind::RunnerClaimed, None);
        inner.records.insert(run_id, record);
        Ok(Some(claimed))
    }

    async fn heartbeat(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        let mut inner = self.lock_inner()?;
        let mut record = inner.take_record(request.run_id)?;
        let result = (|| {
            let now = Utc::now();
            ensure_active_lease(&record, request.runner_id, request.lease_token, now)?;
            record.last_heartbeat_at = Some(now);
            record.lease_expires_at = Some(inner.next_lease_expiry(now));
            record.event_cursor = inner.next_cursor();
            inner.touch_active_lock(&record, now);
            inner.push_event(&record, TurnEventKind::RunnerHeartbeat, None);
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

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        let mut inner = self.lock_inner()?;
        let mut record = inner.take_record(request.run_id)?;
        let result = (|| {
            let now = Utc::now();
            ensure_active_lease(&record, request.runner_id, request.lease_token, now)?;
            if !matches!(record.status, TurnStatus::Running) {
                return Err(TurnError::InvalidTransition {
                    from: record.status,
                    to: request.reason.status(),
                });
            }
            record.status = request.reason.status();
            record.checkpoint_id = Some(request.checkpoint_id);
            record.gate_ref = Some(request.reason.gate_ref().clone());
            record.runner_id = None;
            record.lease_token = None;
            record.lease_expires_at = None;
            record.event_cursor = inner.next_cursor();
            inner.record_checkpoint(
                &record,
                request.checkpoint_id,
                request.reason.gate_ref().clone(),
                now,
            );
            inner.update_active_lock(&record, now);
            let state = record.state();
            inner.push_event(&record, TurnEventKind::Blocked, None);
            Ok(state)
        })();
        inner.records.insert(record.run_id, record);
        result
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        let mut inner = self.lock_inner()?;
        inner.terminal_transition(
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Completed,
            None,
            TurnEventKind::Completed,
        )
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        let mut inner = self.lock_inner()?;
        inner.cancel_completion_transition(request.run_id, request.runner_id, request.lease_token)
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        let mut inner = self.lock_inner()?;
        inner.terminal_transition(
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Failed,
            Some(request.failure),
            TurnEventKind::Failed,
        )
    }

    async fn record_recovery_required(
        &self,
        request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        let mut inner = self.lock_inner()?;
        inner.recovery_required_transition(
            request.run_id,
            request.runner_id,
            request.lease_token,
            request.failure,
        )
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        let mut inner = self.lock_inner()?;
        inner.apply_validated_loop_exit_transition(
            request.run_id,
            request.runner_id,
            request.lease_token,
            request.mapping,
        )
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
        for run in snapshot.runs {
            cursor = cursor.max(run.event_cursor.0);
            let actor = turns
                .get(&run.turn_id)
                .map(|turn| turn.actor.clone())
                .ok_or_else(|| TurnError::Unavailable {
                    reason: "turn run references missing turn record".to_string(),
                })?;
            if run.status == TurnStatus::Queued {
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
                    status: run.status,
                    profile: run.profile,
                    accepted_message_ref: run.accepted_message_ref,
                    source_binding_ref: run.source_binding_ref,
                    reply_target_binding_ref: run.reply_target_binding_ref,
                    checkpoint_id: run.checkpoint_id,
                    gate_ref: run.gate_ref,
                    failure: run.failure,
                    event_cursor: run.event_cursor,
                    runner_id: run.runner_id,
                    lease_token: run.lease_token,
                    lease_expires_at: run.lease_expires_at,
                    last_heartbeat_at: run.last_heartbeat_at,
                    claim_count: run.claim_count,
                    received_at: run.received_at,
                },
            );
        }

        let mut active_locks = HashMap::new();
        for lock in snapshot.active_locks {
            active_locks.insert(lock.key.clone(), lock);
        }

        let mut submit_idempotency = HashMap::new();
        let mut resume_idempotency = HashMap::new();
        let mut cancel_idempotency = HashMap::new();
        let mut idempotency_records = HashMap::new();
        let mut submit_idempotency_order = VecDeque::new();
        let mut resume_idempotency_order = VecDeque::new();
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
            if record.status.is_terminal() {
                reservation.released = true;
            }
            admission_reservations.insert(reservation.run_id, reservation);
        }
        for record in records.values() {
            if record.status.keeps_active_lock() {
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
            cancel_idempotency,
            idempotency_records,
            submit_idempotency_order,
            resume_idempotency_order,
            cancel_idempotency_order,
            idempotency_record_order,
            events,
            event_retention_floor: snapshot.event_retention_floor,
            admission_reservations,
            limits,
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
    ) {
        self.events.push(TurnLifecycleEvent {
            cursor: record.event_cursor,
            scope: record.scope.clone(),
            run_id: record.run_id,
            status: record.status,
            kind,
            sanitized_reason,
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
                    record.status,
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
            record.status = TurnStatus::RecoveryRequired;
            record.runner_id = None;
            record.lease_token = None;
            record.lease_expires_at = None;
            record.event_cursor = self.next_cursor();
            self.update_active_lock(&record, request.now);
            let state = record.state();
            self.push_event(
                &record,
                TurnEventKind::RecoveryRequired,
                Some("lease_expired".to_string()),
            );
            recovered.push(state);
            self.records.insert(run_id, record);
        }
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
            self.limits.max_idempotency_records.saturating_mul(3),
        );
    }

    fn take_record(&mut self, run_id: TurnRunId) -> Result<RunRecord, TurnError> {
        self.records.remove(&run_id).ok_or(TurnError::ScopeNotFound)
    }

    fn pop_matching_queued_run(&mut self, scope_filter: Option<&TurnScope>) -> Option<TurnRunId> {
        let queued_count = self.queued_runs.len();
        for _ in 0..queued_count {
            let run_id = self.queued_runs.pop_front()?;
            let Some(record) = self.records.get(&run_id) else {
                continue;
            };
            if record.status != TurnStatus::Queued {
                continue;
            }
            if scope_filter.is_none_or(|scope| scope == &record.scope) {
                return Some(run_id);
            }
            self.queued_runs.push_back(run_id);
        }
        None
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
            if !matches!(
                record.status,
                TurnStatus::BlockedApproval | TurnStatus::BlockedAuth | TurnStatus::BlockedResource
            ) {
                return Err(TurnError::InvalidTransition {
                    from: record.status,
                    to: TurnStatus::Queued,
                });
            }
            if record.gate_ref.as_ref() != Some(&request.gate_resolution_ref) {
                return Err(TurnError::InvalidRequest {
                    reason: "gate resolution reference mismatch".to_string(),
                });
            }
            let now = Utc::now();
            record.status = TurnStatus::Queued;
            record.gate_ref = None;
            record.source_binding_ref = request.source_binding_ref.clone();
            record.reply_target_binding_ref = request.reply_target_binding_ref.clone();
            record.event_cursor = self.next_cursor();
            self.update_active_lock(&record, now);
            self.queued_runs.push_back(record.run_id);
            let response = ResumeTurnResponse {
                run_id: record.run_id,
                status: record.status,
                event_cursor: record.event_cursor,
            };
            self.push_event(&record, TurnEventKind::Resumed, None);
            Ok(response)
        })();
        self.records.insert(record.run_id, record);
        result
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
            if record.status.is_terminal() {
                return Ok(CancelRunResponse {
                    run_id: record.run_id,
                    status: record.status,
                    event_cursor: record.event_cursor,
                    already_terminal: true,
                });
            }
            let (next_status, event_kind) = match record.status {
                TurnStatus::Queued
                | TurnStatus::BlockedApproval
                | TurnStatus::BlockedAuth
                | TurnStatus::BlockedResource
                | TurnStatus::RecoveryRequired => (TurnStatus::Cancelled, TurnEventKind::Cancelled),
                TurnStatus::Running | TurnStatus::CancelRequested => {
                    (TurnStatus::CancelRequested, TurnEventKind::CancelRequested)
                }
                status => {
                    return Ok(CancelRunResponse {
                        run_id: record.run_id,
                        status,
                        event_cursor: record.event_cursor,
                        already_terminal: true,
                    });
                }
            };
            let now = Utc::now();
            record.status = next_status;
            if record.status.is_terminal() {
                record.failure = None;
                self.release_active_lock(&record);
                self.remove_queued_run(record.run_id);
            } else {
                self.update_active_lock(&record, now);
            }
            record.event_cursor = self.next_cursor();
            let response = CancelRunResponse {
                run_id: record.run_id,
                status: record.status,
                event_cursor: record.event_cursor,
                already_terminal: false,
            };
            self.push_event(
                &record,
                event_kind,
                Some(request.reason.category().to_string()),
            );
            if record.status.is_terminal() {
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
            if record.status != TurnStatus::CancelRequested {
                return Err(TurnError::InvalidTransition {
                    from: record.status,
                    to: TurnStatus::Cancelled,
                });
            }
            record.status = TurnStatus::Cancelled;
            record.failure = None;
            record.runner_id = None;
            record.lease_token = None;
            record.lease_expires_at = None;
            record.event_cursor = self.next_cursor();
            self.release_active_lock(&record);
            self.remove_queued_run(record.run_id);
            let state = record.state();
            self.push_event(&record, TurnEventKind::Cancelled, None);
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
            if record.status == TurnStatus::CancelRequested || record.status.is_terminal() {
                return Err(TurnError::InvalidTransition {
                    from: record.status,
                    to: status,
                });
            }
            record.status = status;
            record.failure = failure.clone();
            record.runner_id = None;
            record.lease_token = None;
            record.lease_expires_at = None;
            record.event_cursor = self.next_cursor();
            self.release_active_lock(&record);
            self.remove_queued_run(record.run_id);
            let state = record.state();
            self.push_event(&record, kind, failure.map(SanitizedFailure::into_category));
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
                LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Cancelled) => {
                    if record.status == TurnStatus::CancelRequested {
                        self.cancel_claimed_record(record)
                    } else {
                        self.recover_claimed_record(
                            record,
                            SanitizedFailure::from_trusted_static("interrupted_unexpectedly"),
                        )
                    }
                }
                LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Blocked {
                    checkpoint_id,
                    reason,
                }) => self.block_claimed_record(record, checkpoint_id, reason),
                LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Failed { failure }) => {
                    self.fail_claimed_record(record, failure)
                }
                LoopExitMapping::RecoveryRequired { failure } => {
                    self.recover_claimed_record(record, failure)
                }
            }
        })();
        match result {
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
        if record.status != TurnStatus::Running {
            let from = record.status;
            return AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: TurnStatus::Completed,
                },
            };
        }
        record.status = TurnStatus::Completed;
        record.failure = None;
        record.runner_id = None;
        record.lease_token = None;
        record.lease_expires_at = None;
        record.event_cursor = self.next_cursor();
        self.release_active_lock(&record);
        self.remove_queued_run(record.run_id);
        let state = record.state();
        self.push_event(&record, TurnEventKind::Completed, None);
        self.mark_terminal(record.run_id);
        AppliedLoopTransition::Applied {
            record: Box::new(record),
            state: Box::new(state),
            prune_terminal: true,
        }
    }

    fn cancel_claimed_record(&mut self, mut record: RunRecord) -> AppliedLoopTransition {
        if record.status != TurnStatus::CancelRequested {
            let from = record.status;
            return AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: TurnStatus::Cancelled,
                },
            };
        }
        record.status = TurnStatus::Cancelled;
        record.failure = None;
        record.runner_id = None;
        record.lease_token = None;
        record.lease_expires_at = None;
        record.event_cursor = self.next_cursor();
        self.release_active_lock(&record);
        self.remove_queued_run(record.run_id);
        let state = record.state();
        self.push_event(&record, TurnEventKind::Cancelled, None);
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
        reason: BlockedReason,
    ) -> AppliedLoopTransition {
        if record.status != TurnStatus::Running {
            let from = record.status;
            return AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: reason.status(),
                },
            };
        }
        let now = Utc::now();
        record.status = reason.status();
        record.checkpoint_id = Some(checkpoint_id);
        record.gate_ref = Some(reason.gate_ref().clone());
        record.runner_id = None;
        record.lease_token = None;
        record.lease_expires_at = None;
        record.event_cursor = self.next_cursor();
        self.record_checkpoint(&record, checkpoint_id, reason.gate_ref().clone(), now);
        self.update_active_lock(&record, now);
        let state = record.state();
        self.push_event(&record, TurnEventKind::Blocked, None);
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
        if record.status != TurnStatus::Running {
            let from = record.status;
            return AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: TurnStatus::Failed,
                },
            };
        }
        record.status = TurnStatus::Failed;
        record.failure = Some(failure.clone());
        record.runner_id = None;
        record.lease_token = None;
        record.lease_expires_at = None;
        record.event_cursor = self.next_cursor();
        self.release_active_lock(&record);
        self.remove_queued_run(record.run_id);
        let state = record.state();
        self.push_event(
            &record,
            TurnEventKind::Failed,
            Some(failure.into_category()),
        );
        self.mark_terminal(record.run_id);
        AppliedLoopTransition::Applied {
            record: Box::new(record),
            state: Box::new(state),
            prune_terminal: true,
        }
    }

    fn recover_claimed_record(
        &mut self,
        mut record: RunRecord,
        failure: SanitizedFailure,
    ) -> AppliedLoopTransition {
        if !matches!(
            record.status,
            TurnStatus::Running | TurnStatus::CancelRequested
        ) {
            let from = record.status;
            return AppliedLoopTransition::Rejected {
                record: Box::new(record),
                error: TurnError::InvalidTransition {
                    from,
                    to: TurnStatus::RecoveryRequired,
                },
            };
        }
        record.status = TurnStatus::RecoveryRequired;
        record.failure = Some(failure.clone());
        record.runner_id = None;
        record.lease_token = None;
        record.lease_expires_at = None;
        record.event_cursor = self.next_cursor();
        self.update_active_lock(&record, Utc::now());
        let state = record.state();
        self.push_event(
            &record,
            TurnEventKind::RecoveryRequired,
            Some(failure.into_category()),
        );
        AppliedLoopTransition::Applied {
            record: Box::new(record),
            state: Box::new(state),
            prune_terminal: false,
        }
    }

    fn recovery_required_transition(
        &mut self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        failure: SanitizedFailure,
    ) -> Result<TurnRunState, TurnError> {
        let mut record = self.take_record(run_id)?;
        let result = (|| {
            ensure_active_lease(&record, runner_id, lease_token, Utc::now())?;
            if !matches!(
                record.status,
                TurnStatus::Running | TurnStatus::CancelRequested
            ) {
                return Err(TurnError::InvalidTransition {
                    from: record.status,
                    to: TurnStatus::RecoveryRequired,
                });
            }
            record.status = TurnStatus::RecoveryRequired;
            record.failure = Some(failure.clone());
            record.runner_id = None;
            record.lease_token = None;
            record.lease_expires_at = None;
            record.event_cursor = self.next_cursor();
            self.update_active_lock(&record, Utc::now());
            let state = record.state();
            self.push_event(
                &record,
                TurnEventKind::RecoveryRequired,
                Some(failure.into_category()),
            );
            Ok(state)
        })();
        self.records.insert(record.run_id, record);
        result
    }

    fn update_active_lock(&mut self, record: &RunRecord, updated_at: crate::TurnTimestamp) {
        let lock_key = TurnActiveLockKey::from(&record.scope);
        if let Some(lock) = self.active_locks.get_mut(&lock_key)
            && lock.run_id == record.run_id
        {
            lock.status = record.status;
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
        record.status.keeps_active_lock().then_some(ThreadBusy {
            active_run_id: active_lock.run_id,
            status: record.status,
            event_cursor: record.event_cursor,
        })
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
            sequence,
            status: record.status,
            gate_ref,
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
                .is_some_and(|record| record.status.is_terminal())
            {
                self.records.remove(&run_id);
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
            status: self.status,
            profile: self.profile.clone(),
            checkpoint_id: self.checkpoint_id,
            gate_ref: self.gate_ref.clone(),
            failure: self.failure.clone(),
            event_cursor: self.event_cursor,
            runner_id: self.runner_id,
            lease_token: self.lease_token,
            lease_expires_at: self.lease_expires_at,
            last_heartbeat_at: self.last_heartbeat_at,
            claim_count: self.claim_count,
            received_at: self.received_at,
        }
    }

    fn state(&self) -> TurnRunState {
        let _ = &self.actor;
        TurnRunState {
            scope: self.scope.clone(),
            turn_id: self.turn_id,
            run_id: self.run_id,
            status: self.status,
            accepted_message_ref: self.accepted_message_ref.clone(),
            source_binding_ref: self.source_binding_ref.clone(),
            reply_target_binding_ref: self.reply_target_binding_ref.clone(),
            resolved_run_profile_id: self.profile.id.clone(),
            resolved_run_profile_version: self.profile.version,
            received_at: self.received_at,
            checkpoint_id: self.checkpoint_id,
            gate_ref: self.gate_ref.clone(),
            failure: self.failure.clone(),
            event_cursor: self.event_cursor,
        }
    }
}

fn persisted_key_for_record(record: &TurnIdempotencyRecord) -> PersistedIdempotencyKey {
    PersistedIdempotencyKey {
        scope: record.scope.clone(),
        operation: record.operation,
        run_id: match record.operation {
            TurnIdempotencyOperationKind::Submit => None,
            TurnIdempotencyOperationKind::Resume | TurnIdempotencyOperationKind::Cancel => {
                record.run_id
            }
        },
        key: record.key.clone(),
    }
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
