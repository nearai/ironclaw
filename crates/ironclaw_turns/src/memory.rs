use async_trait::async_trait;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::Hash,
    sync::{Condvar, Mutex, MutexGuard},
};

use chrono::Utc;

use crate::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, GetRunStateRequest, IdempotencyKey,
    ReplyTargetBindingRef, ResumeTurnRequest, ResumeTurnResponse, SanitizedFailure,
    SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnActiveLockKey,
    TurnActiveLockRecord, TurnActor, TurnAdmissionPolicy, TurnCheckpointId, TurnCheckpointRecord,
    TurnError, TurnEventKind, TurnIdempotencyErrorReplay, TurnIdempotencyOperationKind,
    TurnIdempotencyOutcomeKind, TurnIdempotencyRecord, TurnIdempotencyReplay, TurnLifecycleEvent,
    TurnLockVersion, TurnPersistenceSnapshot, TurnRecord, TurnRunId, TurnRunProfile, TurnRunRecord,
    TurnRunState, TurnScope, TurnStateStore, TurnStatus,
    events::EventCursor,
    runner::{
        BlockRunRequest, CancelRunCompletionRequest, ClaimRunRequest, ClaimedTurnRun,
        CompleteRunRequest, FailRunRequest, HeartbeatRequest, TurnRunTransitionPort,
    },
};

const MAX_EVENTS: usize = 10_000;
const MAX_TERMINAL_RECORDS: usize = 10_000;
const MAX_IDEMPOTENCY_RECORDS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InMemoryTurnStateStoreLimits {
    pub max_events: usize,
    pub max_terminal_records: usize,
    pub max_idempotency_records: usize,
}

impl Default for InMemoryTurnStateStoreLimits {
    fn default() -> Self {
        Self {
            max_events: MAX_EVENTS,
            max_terminal_records: MAX_TERMINAL_RECORDS,
            max_idempotency_records: MAX_IDEMPOTENCY_RECORDS,
        }
    }
}

pub struct InMemoryTurnStateStore {
    inner: Mutex<Inner>,
    submit_idempotency_ready: Condvar,
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
    limits: InMemoryTurnStateStoreLimits,
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

impl InMemoryTurnStateStore {
    pub fn with_limits(limits: InMemoryTurnStateStoreLimits) -> Self {
        Self {
            inner: Mutex::new(Inner {
                limits,
                ..Inner::default()
            }),
            submit_idempotency_ready: Condvar::new(),
        }
    }

    pub fn events(&self) -> Vec<TurnLifecycleEvent> {
        match self.inner.lock() {
            Ok(inner) => inner.events.clone(),
            Err(poisoned) => poisoned.into_inner().events.clone(),
        }
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
impl TurnStateStore for InMemoryTurnStateStore {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let idempotency_key = SubmitIdempotencyKey {
            scope: request.scope.clone(),
            key: request.idempotency_key.clone(),
        };
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
        drop(inner);

        let admission_result = admission_policy.check_submit(&request);

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

        let lock_key = TurnActiveLockKey::from(&request.scope);
        if let Some(active_lock) = inner.active_locks.get(&lock_key)
            && let Some(record) = inner.records.get(&active_lock.run_id)
            && record.status.keeps_active_lock()
        {
            let response = Err(TurnError::ThreadBusy(ThreadBusy {
                active_run_id: active_lock.run_id,
                status: record.status,
                event_cursor: record.event_cursor,
            }));
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            inner.submit_idempotency_in_flight.remove(&idempotency_key);
            self.submit_idempotency_ready.notify_all();
            return response;
        }

        let turn_id = crate::TurnId::new();
        let run_id = TurnRunId::new();
        let cursor = inner.next_cursor();
        let profile = TurnRunProfile::resolve(request.requested_run_profile.as_ref());
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
        record.last_heartbeat_at = Some(now);
        record.claim_count = record.claim_count.saturating_add(1);
        record.event_cursor = inner.next_cursor();
        inner.update_active_lock(&record, now);
        let claimed = ClaimedTurnRun {
            state: record.state(),
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
            ensure_lease(&record, request.runner_id, request.lease_token)?;
            let now = Utc::now();
            record.last_heartbeat_at = Some(now);
            record.event_cursor = inner.next_cursor();
            inner.touch_active_lock(&record, now);
            inner.push_event(&record, TurnEventKind::RunnerHeartbeat, None);
            Ok(record.event_cursor)
        })();
        inner.records.insert(record.run_id, record);
        result
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        let mut inner = self.lock_inner()?;
        let mut record = inner.take_record(request.run_id)?;
        let result = (|| {
            ensure_lease(&record, request.runner_id, request.lease_token)?;
            if !matches!(record.status, TurnStatus::Running) {
                return Err(TurnError::InvalidTransition {
                    from: record.status,
                    to: request.reason.status(),
                });
            }
            let now = Utc::now();
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
}

impl Inner {
    fn next_cursor(&mut self) -> EventCursor {
        self.cursor = self.cursor.saturating_add(1);
        EventCursor(self.cursor)
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
        let mut idempotency_records = self
            .idempotency_records
            .values()
            .cloned()
            .collect::<Vec<_>>();
        idempotency_records.sort_by_key(|record| record.created_at);
        TurnPersistenceSnapshot {
            turns,
            runs,
            active_locks,
            checkpoints,
            idempotency_records,
        }
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
                | TurnStatus::BlockedResource => (TurnStatus::Cancelled, TurnEventKind::Cancelled),
                TurnStatus::Running
                | TurnStatus::CancelRequested
                | TurnStatus::RecoveryRequired => {
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
            ensure_lease(&record, runner_id, lease_token)?;
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
            ensure_lease(&record, runner_id, lease_token)?;
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

fn ensure_lease(
    record: &RunRecord,
    runner_id: crate::TurnRunnerId,
    lease_token: crate::TurnLeaseToken,
) -> Result<(), TurnError> {
    if record.runner_id != Some(runner_id) || record.lease_token != Some(lease_token) {
        return Err(TurnError::LeaseMismatch);
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
