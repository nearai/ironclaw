use async_trait::async_trait;
use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
    sync::{Mutex, MutexGuard},
};

use crate::{
    CancelRunRequest, CancelRunResponse, GetRunStateRequest, IdempotencyKey, ReplyTargetBindingRef,
    ResumeTurnRequest, ResumeTurnResponse, SanitizedFailure, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, ThreadBusy, TurnActor, TurnCheckpointId, TurnError, TurnEventKind,
    TurnLifecycleEvent, TurnRunId, TurnRunProfile, TurnRunState, TurnScope, TurnStateStore,
    TurnStatus,
    events::EventCursor,
    runner::{
        BlockRunRequest, ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest,
        HeartbeatRequest, TurnRunTransitionPort,
    },
};

const MAX_EVENTS: usize = 10_000;
const MAX_TERMINAL_RECORDS: usize = 10_000;
const MAX_IDEMPOTENCY_RECORDS: usize = 10_000;

#[derive(Default)]
pub struct InMemoryTurnStateStore {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    cursor: u64,
    records: HashMap<TurnRunId, RunRecord>,
    queued_runs: VecDeque<TurnRunId>,
    terminal_runs: VecDeque<TurnRunId>,
    active_locks: HashMap<TurnLockKey, TurnRunId>,
    submit_idempotency: HashMap<SubmitIdempotencyKey, Result<SubmitTurnResponse, TurnError>>,
    resume_idempotency: HashMap<RunIdempotencyKey, Result<ResumeTurnResponse, TurnError>>,
    cancel_idempotency: HashMap<RunIdempotencyKey, Result<CancelRunResponse, TurnError>>,
    events: Vec<TurnLifecycleEvent>,
}

#[derive(Debug, Clone)]
struct RunRecord {
    scope: TurnScope,
    actor: TurnActor,
    turn_id: crate::TurnId,
    run_id: TurnRunId,
    status: TurnStatus,
    profile: TurnRunProfile,
    source_binding_ref: SourceBindingRef,
    reply_target_binding_ref: ReplyTargetBindingRef,
    checkpoint_id: Option<TurnCheckpointId>,
    gate_ref: Option<crate::GateRef>,
    failure: Option<SanitizedFailure>,
    event_cursor: EventCursor,
    runner_id: Option<crate::TurnRunnerId>,
    lease_token: Option<crate::TurnLeaseToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TurnLockKey {
    scope: TurnScope,
}

impl From<&TurnScope> for TurnLockKey {
    fn from(scope: &TurnScope) -> Self {
        Self {
            scope: scope.clone(),
        }
    }
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

impl InMemoryTurnStateStore {
    pub fn events(&self) -> Vec<TurnLifecycleEvent> {
        match self.inner.lock() {
            Ok(inner) => inner.events.clone(),
            Err(poisoned) => poisoned.into_inner().events.clone(),
        }
    }

    fn lock_inner(&self) -> Result<MutexGuard<'_, Inner>, TurnError> {
        self.inner.lock().map_err(|_| TurnError::Backend {
            reason: "turn state store mutex poisoned".to_string(),
        })
    }
}

#[async_trait]
impl TurnStateStore for InMemoryTurnStateStore {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let mut inner = self.lock_inner()?;
        let idempotency_key = SubmitIdempotencyKey {
            scope: request.scope.clone(),
            key: request.idempotency_key.clone(),
        };
        if let Some(result) = inner.submit_idempotency.get(&idempotency_key) {
            return result.clone();
        }

        let lock_key = TurnLockKey::from(&request.scope);
        if let Some(active_run_id) = inner.active_locks.get(&lock_key)
            && let Some(record) = inner.records.get(active_run_id)
            && record.status.keeps_active_lock()
        {
            return Err(TurnError::ThreadBusy(ThreadBusy {
                active_run_id: *active_run_id,
                status: record.status,
                event_cursor: record.event_cursor,
            }));
        }

        let turn_id = crate::TurnId::new();
        let run_id = TurnRunId::new();
        let cursor = inner.next_cursor();
        let record = RunRecord {
            scope: request.scope.clone(),
            actor: request.actor,
            turn_id,
            run_id,
            status: TurnStatus::Queued,
            profile: request.profile.clone(),
            source_binding_ref: request.source_binding_ref,
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            checkpoint_id: None,
            gate_ref: None,
            failure: None,
            event_cursor: cursor,
            runner_id: None,
            lease_token: None,
        };
        inner.active_locks.insert(lock_key, run_id);
        inner.queued_runs.push_back(run_id);
        inner.records.insert(run_id, record.clone());
        inner.push_event(&record, TurnEventKind::Submitted, None);

        let response = Ok(SubmitTurnResponse::Accepted {
            turn_id,
            run_id,
            status: TurnStatus::Queued,
            profile: request.profile,
            event_cursor: cursor,
            reply_target_binding_ref: request.reply_target_binding_ref,
        });
        inner
            .submit_idempotency
            .insert(idempotency_key, response.clone());
        inner.prune_idempotency_records();
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
        inner
            .resume_idempotency
            .insert(idempotency_key, result.clone());
        inner.prune_idempotency_records();
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
        inner
            .cancel_idempotency
            .insert(idempotency_key, result.clone());
        inner.prune_idempotency_records();
        result
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        let inner = self.lock_inner()?;
        inner
            .records
            .get(&request.run_id)
            .filter(|record| record.scope == request.scope)
            .map(RunRecord::state)
            .ok_or(TurnError::NotFound)
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
        record.status = TurnStatus::Running;
        record.runner_id = Some(request.runner_id);
        record.lease_token = Some(request.lease_token);
        record.event_cursor = inner.next_cursor();
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
            record.event_cursor = inner.next_cursor();
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
            record.status = request.reason.status();
            record.checkpoint_id = Some(request.checkpoint_id);
            record.gate_ref = Some(request.reason.gate_ref().clone());
            record.runner_id = None;
            record.lease_token = None;
            record.event_cursor = inner.next_cursor();
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
        if self.events.len() > MAX_EVENTS {
            let excess = self.events.len() - MAX_EVENTS;
            self.events.drain(0..excess);
        }
    }

    fn take_record(&mut self, run_id: TurnRunId) -> Result<RunRecord, TurnError> {
        self.records.remove(&run_id).ok_or(TurnError::NotFound)
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
                return Err(TurnError::NotFound);
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
                return Err(TurnError::NotFound);
            }
            record.status = TurnStatus::Queued;
            record.gate_ref = None;
            record.event_cursor = self.next_cursor();
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
                return Err(TurnError::NotFound);
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
            record.status = next_status;
            if record.status.is_terminal() {
                self.release_active_lock(&record);
                self.remove_queued_run(record.run_id);
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
            if record.status.is_terminal() {
                return Err(TurnError::InvalidTransition {
                    from: record.status,
                    to: status,
                });
            }
            record.status = status;
            record.failure = failure.clone();
            record.runner_id = None;
            record.lease_token = None;
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

    fn release_active_lock(&mut self, record: &RunRecord) {
        let lock_key = TurnLockKey::from(&record.scope);
        if self.active_locks.get(&lock_key) == Some(&record.run_id) {
            self.active_locks.remove(&lock_key);
        }
    }

    fn mark_terminal(&mut self, run_id: TurnRunId) {
        self.terminal_runs.push_back(run_id);
    }

    fn prune_terminal_records(&mut self) {
        while self.terminal_runs.len() > MAX_TERMINAL_RECORDS {
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

    fn prune_idempotency_records(&mut self) {
        prune_map(&mut self.submit_idempotency, MAX_IDEMPOTENCY_RECORDS);
        prune_map(&mut self.resume_idempotency, MAX_IDEMPOTENCY_RECORDS);
        prune_map(&mut self.cancel_idempotency, MAX_IDEMPOTENCY_RECORDS);
    }
}

impl RunRecord {
    fn state(&self) -> TurnRunState {
        let _ = &self.actor;
        TurnRunState {
            scope: self.scope.clone(),
            turn_id: self.turn_id,
            run_id: self.run_id,
            status: self.status,
            profile: self.profile.clone(),
            source_binding_ref: self.source_binding_ref.clone(),
            reply_target_binding_ref: self.reply_target_binding_ref.clone(),
            checkpoint_id: self.checkpoint_id,
            gate_ref: self.gate_ref.clone(),
            failure: self.failure.clone(),
            event_cursor: self.event_cursor,
        }
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

fn prune_map<K, V>(map: &mut HashMap<K, V>, max_len: usize)
where
    K: Clone + Eq + Hash,
{
    while map.len() > max_len {
        let Some(key) = map.keys().next().cloned() else {
            break;
        };
        map.remove(&key);
    }
}
