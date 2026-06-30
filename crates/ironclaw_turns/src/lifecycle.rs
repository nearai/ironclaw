use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::Utc;
use tracing::debug;

use crate::{
    CancelRunRequest, CancelRunResponse, GetRunStateRequest, ResumeTurnRequest, ResumeTurnResponse,
    RunProfileResolver, SubmitChildRunRequest, SubmitTurnRequest, SubmitTurnResponse,
    TurnAdmissionPolicy, TurnCommittedEventObserver, TurnError, TurnEventKind, TurnEventSink,
    TurnLifecycleEvent, TurnRunId, TurnRunRecord, TurnRunState, TurnSpawnTreeStateStore,
    TurnStateStore, TurnStatus,
    events::{EventCursor, lifecycle_owner_user_id},
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest, RecoverExpiredLeasesRequest,
        RecoverExpiredLeasesResponse, RelinquishRunRequest, TurnRunTransitionPort,
    },
    store::SpawnTreeReservation,
};

const MAX_DELIVERED_EVENT_CURSORS: usize = 4096;

/// Lifecycle fanout abstraction used by the publishing store decorator.
///
/// Required observers model in-process consistency side effects. Best-effort
/// sinks model projections/notifications that must not fail the committed
/// mutation.
///
/// Two publication paths preserve the `TurnCommittedEventObserver` split:
/// coordinator-origin operations (submit, resume, request_cancel, submit_child)
/// emit only a lifecycle event and call `observe_committed_event`, while
/// runner-origin transitions (claim, block, complete, fail, cancel, recovery,
/// validated-loop-exit) carry the committed `TurnRunState` and call
/// `observe_committed_state`. Best-effort sinks always see the derived
/// `TurnLifecycleEvent` regardless of origin.
#[async_trait]
pub trait TurnLifecycleEventBus: Send + Sync {
    fn subscribe_required(
        &self,
        observer: Arc<dyn TurnCommittedEventObserver>,
    ) -> Result<(), TurnError>;

    fn subscribe_best_effort(&self, sink: Arc<dyn TurnEventSink>) -> Result<(), TurnError>;

    async fn publish_event(&self, event: TurnLifecycleEvent) -> Result<(), TurnError>;

    async fn publish_state(
        &self,
        state: TurnRunState,
        event: TurnLifecycleEvent,
    ) -> Result<(), TurnError>;
}

/// Side channel used by the publishing decorator to surface required-observer
/// errors from coordinator-origin transitions that have already committed.
///
/// The base `TurnStateStore` trait stays free of lifecycle concerns; the
/// decorator implements this trait, and the coordinator queries it after each
/// store call. Plain stores have nothing to surface and use
/// [`NoopLifecyclePublicationErrorPort`].
pub trait LifecyclePublicationErrorPort: Send + Sync {
    fn take_lifecycle_publication_error(&self, cursor: EventCursor) -> Option<TurnError>;
}

#[derive(Debug, Default)]
pub struct NoopLifecyclePublicationErrorPort;

impl LifecyclePublicationErrorPort for NoopLifecyclePublicationErrorPort {
    fn take_lifecycle_publication_error(&self, _cursor: EventCursor) -> Option<TurnError> {
        None
    }
}

/// Default in-process lifecycle fanout bus.
///
/// Required subscribers are invoked first and their errors are returned to the
/// publisher. Best-effort sinks run afterward and are logged on failure.
#[derive(Default)]
pub struct DefaultTurnLifecycleEventBus {
    subscribers: Mutex<TurnLifecycleEventSubscribers>,
}

#[derive(Default)]
struct TurnLifecycleEventSubscribers {
    required: Vec<Arc<dyn TurnCommittedEventObserver>>,
    best_effort: Vec<Arc<dyn TurnEventSink>>,
}

struct TurnLifecycleEventSubscriberSnapshot {
    required: Vec<Arc<dyn TurnCommittedEventObserver>>,
    best_effort: Vec<Arc<dyn TurnEventSink>>,
}

impl DefaultTurnLifecycleEventBus {
    pub fn new() -> Self {
        Self::default()
    }

    fn subscriber_snapshot(&self) -> Result<TurnLifecycleEventSubscriberSnapshot, TurnError> {
        let subscribers = self
            .subscribers
            .lock()
            .map_err(|_| TurnError::Unavailable {
                reason: "turn lifecycle event bus subscriber mutex poisoned".to_string(),
            })?;
        Ok(TurnLifecycleEventSubscriberSnapshot {
            required: subscribers.required.clone(),
            best_effort: subscribers.best_effort.clone(),
        })
    }

    async fn fanout_best_effort(
        &self,
        sinks: &[Arc<dyn TurnEventSink>],
        event: TurnLifecycleEvent,
    ) {
        for sink in sinks {
            if let Err(error) = sink.publish(event.clone()).await {
                debug!(error = %error, "turn lifecycle event sink publish failed");
            }
        }
    }
}

#[async_trait]
impl TurnLifecycleEventBus for DefaultTurnLifecycleEventBus {
    fn subscribe_required(
        &self,
        observer: Arc<dyn TurnCommittedEventObserver>,
    ) -> Result<(), TurnError> {
        let mut subscribers = self
            .subscribers
            .lock()
            .map_err(|_| TurnError::Unavailable {
                reason: "turn lifecycle event bus subscriber mutex poisoned".to_string(),
            })?;
        subscribers.required.push(observer);
        Ok(())
    }

    fn subscribe_best_effort(&self, sink: Arc<dyn TurnEventSink>) -> Result<(), TurnError> {
        let mut subscribers = self
            .subscribers
            .lock()
            .map_err(|_| TurnError::Unavailable {
                reason: "turn lifecycle event bus subscriber mutex poisoned".to_string(),
            })?;
        subscribers.best_effort.push(sink);
        Ok(())
    }

    async fn publish_event(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        let snapshot = self.subscriber_snapshot()?;
        // Run every required observer, capturing the first error, so a single
        // observer failure does not skip remaining observers or best-effort
        // sinks. Sinks must always see committed events for projection
        // durability; the first observer error is returned to the caller after
        // fanout completes.
        let mut first_observer_error: Option<TurnError> = None;
        for observer in &snapshot.required {
            if !observer.observes_event(&event) {
                continue;
            }
            if let Err(error) = observer.observe_committed_event(event.clone()).await
                && first_observer_error.is_none()
            {
                first_observer_error = Some(error);
            }
        }
        self.fanout_best_effort(&snapshot.best_effort, event).await;
        match first_observer_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn publish_state(
        &self,
        state: TurnRunState,
        event: TurnLifecycleEvent,
    ) -> Result<(), TurnError> {
        let snapshot = self.subscriber_snapshot()?;
        let mut first_observer_error: Option<TurnError> = None;
        for observer in &snapshot.required {
            if !observer.observes_state(&state) {
                continue;
            }
            if let Err(error) = observer.observe_committed_state(state.clone()).await
                && first_observer_error.is_none()
            {
                first_observer_error = Some(error);
            }
        }
        self.fanout_best_effort(&snapshot.best_effort, event).await;
        match first_observer_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

/// Decorates a turn-state store with post-commit lifecycle publication.
///
/// The wrapper implements `TurnStateStore`, `TurnSpawnTreeStateStore`,
/// `TurnRunTransitionPort`, and [`LifecyclePublicationErrorPort`] so coordinator
/// and runner mutations share one event construction and fanout path.
/// Publication runs after the inner store call returns, which avoids
/// re-entering backend mutation locks from observers.
pub struct LifecyclePublishingTurnStateStore<S: ?Sized> {
    inner: Arc<S>,
    bus: Arc<dyn TurnLifecycleEventBus>,
    delivered_event_cursors: Mutex<HashSet<EventCursor>>,
    deferred_publication_errors: Mutex<HashMap<EventCursor, TurnError>>,
}

impl<S: ?Sized> LifecyclePublishingTurnStateStore<S> {
    pub fn new(inner: Arc<S>, bus: Arc<dyn TurnLifecycleEventBus>) -> Self {
        Self {
            inner,
            bus,
            delivered_event_cursors: Mutex::new(HashSet::new()),
            deferred_publication_errors: Mutex::new(HashMap::new()),
        }
    }

    /// Returns true when this cursor has not yet been published successfully
    /// and the caller should attempt publication. The cursor is NOT marked
    /// delivered until [`Self::mark_event_cursor_delivered`] runs after a
    /// successful publish, so a transient observer error allows idempotent
    /// retries to re-attempt publication instead of permanently dropping
    /// required side effects.
    fn try_begin_publish(&self, cursor: EventCursor) -> Result<bool, TurnError> {
        let delivered =
            self.delivered_event_cursors
                .lock()
                .map_err(|_| TurnError::Unavailable {
                    reason: "turn lifecycle event cursor mutex poisoned".to_string(),
                })?;
        Ok(!delivered.contains(&cursor))
    }

    fn mark_event_cursor_delivered(&self, cursor: EventCursor) -> Result<(), TurnError> {
        let mut delivered =
            self.delivered_event_cursors
                .lock()
                .map_err(|_| TurnError::Unavailable {
                    reason: "turn lifecycle event cursor mutex poisoned".to_string(),
                })?;
        if delivered.len() >= MAX_DELIVERED_EVENT_CURSORS {
            delivered.clear();
        }
        delivered.insert(cursor);
        Ok(())
    }

    async fn publish_event_once_deferred(
        &self,
        event: TurnLifecycleEvent,
    ) -> Result<(), TurnError> {
        let cursor = event.cursor;
        if !self.try_begin_publish(cursor)? {
            return Ok(());
        }
        match self.bus.publish_event(event).await {
            Ok(()) => {
                self.mark_event_cursor_delivered(cursor)?;
            }
            Err(error) => {
                // Do NOT mark delivered on failure: idempotent retries of the
                // same submit/resume/cancel cursor must be allowed to
                // re-attempt publication so a transient observer error cannot
                // permanently drop required side effects.
                let mut errors = self.deferred_publication_errors.lock().map_err(|_| {
                    TurnError::Unavailable {
                        reason: "turn lifecycle publication error mutex poisoned".to_string(),
                    }
                })?;
                errors.insert(cursor, error);
            }
        }
        Ok(())
    }

    async fn publish_state_once(
        &self,
        state: TurnRunState,
        event: TurnLifecycleEvent,
    ) -> Result<(), TurnError> {
        let cursor = event.cursor;
        if !self.try_begin_publish(cursor)? {
            return Ok(());
        }
        match self.bus.publish_state(state, event).await {
            Ok(()) => {
                self.mark_event_cursor_delivered(cursor)?;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    async fn publish_state_once_best_effort(
        &self,
        state: TurnRunState,
        event: TurnLifecycleEvent,
        context: &'static str,
    ) {
        let cursor = event.cursor;
        let should_publish = match self.try_begin_publish(cursor) {
            Ok(should_publish) => should_publish,
            Err(error) => {
                debug!(error = %error, "turn lifecycle cursor check failed after {context}");
                return;
            }
        };
        if !should_publish {
            return;
        }
        match self.bus.publish_state(state, event).await {
            Ok(()) => {
                if let Err(error) = self.mark_event_cursor_delivered(cursor) {
                    debug!(error = %error, "turn lifecycle delivered mark failed after {context}");
                }
            }
            Err(error) => {
                debug!(error = %error, "turn lifecycle publication failed after {context}");
            }
        }
    }
}

impl<S: ?Sized> LifecyclePublicationErrorPort for LifecyclePublishingTurnStateStore<S>
where
    S: Send + Sync,
{
    fn take_lifecycle_publication_error(&self, cursor: EventCursor) -> Option<TurnError> {
        let mut errors = match self.deferred_publication_errors.lock() {
            Ok(errors) => errors,
            Err(poisoned) => poisoned.into_inner(),
        };
        errors.remove(&cursor)
    }
}

fn submit_event(request: &SubmitTurnRequest, response: &SubmitTurnResponse) -> TurnLifecycleEvent {
    let SubmitTurnResponse::Accepted {
        run_id,
        status,
        event_cursor,
        ..
    } = response;
    TurnLifecycleEvent {
        cursor: *event_cursor,
        scope: request.scope.clone(),
        occurred_at: Some(request.received_at),
        owner_user_id: lifecycle_owner_user_id(&request.scope, Some(&request.actor.user_id)),
        run_id: *run_id,
        status: *status,
        kind: TurnEventKind::Submitted,
        blocked_gate: None,
        sanitized_reason: None,
    }
}

fn child_submit_event(
    request: &SubmitChildRunRequest,
    response: &SubmitTurnResponse,
) -> TurnLifecycleEvent {
    let SubmitTurnResponse::Accepted {
        run_id,
        status,
        event_cursor,
        ..
    } = response;
    TurnLifecycleEvent {
        cursor: *event_cursor,
        scope: request.child_scope.clone(),
        occurred_at: Some(request.received_at),
        owner_user_id: lifecycle_owner_user_id(&request.child_scope, Some(&request.actor.user_id)),
        run_id: *run_id,
        status: *status,
        kind: TurnEventKind::Submitted,
        blocked_gate: None,
        sanitized_reason: None,
    }
}

fn resume_event(request: &ResumeTurnRequest, response: &ResumeTurnResponse) -> TurnLifecycleEvent {
    TurnLifecycleEvent {
        cursor: response.event_cursor,
        scope: request.scope.clone(),
        occurred_at: Some(Utc::now()),
        owner_user_id: lifecycle_owner_user_id(&request.scope, Some(&request.actor.user_id)),
        run_id: response.run_id,
        status: response.status,
        kind: TurnEventKind::Resumed,
        blocked_gate: None,
        sanitized_reason: None,
    }
}

fn cancel_event(
    request: &CancelRunRequest,
    response: &CancelRunResponse,
) -> Option<TurnLifecycleEvent> {
    if response.already_terminal {
        return None;
    }
    let kind = if response.status == TurnStatus::CancelRequested {
        TurnEventKind::CancelRequested
    } else {
        TurnEventKind::Cancelled
    };
    Some(TurnLifecycleEvent {
        cursor: response.event_cursor,
        scope: request.scope.clone(),
        occurred_at: Some(Utc::now()),
        owner_user_id: lifecycle_owner_user_id(
            &request.scope,
            response.actor.as_ref().map(|actor| &actor.user_id),
        ),
        run_id: response.run_id,
        status: response.status,
        kind,
        blocked_gate: None,
        sanitized_reason: Some(request.reason.category().to_string()),
    })
}

fn event_kind_for_state(state: &TurnRunState) -> TurnEventKind {
    match state.status {
        TurnStatus::Running => TurnEventKind::RunnerClaimed,
        TurnStatus::BlockedApproval
        | TurnStatus::BlockedAuth
        | TurnStatus::BlockedResource
        | TurnStatus::BlockedDependentRun
        | TurnStatus::BlockedExternalTool => TurnEventKind::Blocked,
        TurnStatus::Completed => TurnEventKind::Completed,
        TurnStatus::Cancelled => TurnEventKind::Cancelled,
        TurnStatus::Failed => TurnEventKind::Failed,
        TurnStatus::RecoveryRequired => TurnEventKind::RecoveryRequired,
        TurnStatus::Queued | TurnStatus::CancelRequested => TurnEventKind::RunnerHeartbeat,
    }
}

fn sanitized_reason_for_state(state: &TurnRunState) -> Option<String> {
    match state.status {
        TurnStatus::Failed | TurnStatus::RecoveryRequired => state
            .failure
            .as_ref()
            .map(|failure| failure.category().to_string()),
        _ => None,
    }
}

#[async_trait]
impl<S> TurnStateStore for LifecyclePublishingTurnStateStore<S>
where
    S: TurnStateStore + ?Sized,
{
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let event_request = request.clone();
        let response = self
            .inner
            .submit_turn(request, admission_policy, run_profile_resolver)
            .await?;
        self.publish_event_once_deferred(submit_event(&event_request, &response))
            .await?;
        Ok(response)
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let event_request = request.clone();
        let response = self.inner.resume_turn(request).await?;
        self.publish_event_once_deferred(resume_event(&event_request, &response))
            .await?;
        Ok(response)
    }

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        let event_request = request.clone();
        let response = self.inner.request_cancel(request).await?;
        if let Some(event) = cancel_event(&event_request, &response) {
            self.publish_event_once_deferred(event).await?;
        }
        Ok(response)
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.inner.get_run_state(request).await
    }
}

#[async_trait]
impl<S> TurnSpawnTreeStateStore for LifecyclePublishingTurnStateStore<S>
where
    S: TurnSpawnTreeStateStore + ?Sized,
{
    async fn submit_child_turn(
        &self,
        request: SubmitChildRunRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let event_request = request.clone();
        let response = self
            .inner
            .submit_child_turn(request, admission_policy, run_profile_resolver)
            .await?;
        self.publish_event_once_deferred(child_submit_event(&event_request, &response))
            .await?;
        Ok(response)
    }

    async fn children_of(
        &self,
        scope: &crate::TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        self.inner.children_of(scope, run_id).await
    }

    async fn get_run_record(
        &self,
        scope: &crate::TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        self.inner.get_run_record(scope, run_id).await
    }

    async fn reserve_tree_descendants(
        &self,
        scope: &crate::TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError> {
        self.inner
            .reserve_tree_descendants(scope, root_run_id, delta, cap)
            .await
    }

    async fn release_tree_descendants(
        &self,
        scope: &crate::TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
    ) -> Result<(), TurnError> {
        self.inner
            .release_tree_descendants(scope, root_run_id, delta)
            .await
    }
}

#[async_trait]
impl<S> TurnRunTransitionPort for LifecyclePublishingTurnStateStore<S>
where
    S: TurnRunTransitionPort + ?Sized,
{
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        let claimed = self.inner.claim_next_run(request).await?;
        if let Some(claimed) = &claimed {
            let event = TurnLifecycleEvent::from_run_state(
                &claimed.state,
                TurnEventKind::RunnerClaimed,
                None,
            );
            self.publish_state_once_best_effort(claimed.state.clone(), event, "committed claim")
                .await;
        }
        Ok(claimed)
    }

    async fn heartbeat(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        self.inner.heartbeat(request).await
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        let response = self.inner.recover_expired_leases(request).await?;
        for state in &response.recovered {
            let event = TurnLifecycleEvent::from_run_state(
                state,
                event_kind_for_state(state),
                sanitized_reason_for_state(state),
            );
            self.publish_state_once_best_effort(state.clone(), event, "committed lease recovery")
                .await;
        }
        Ok(response)
    }

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.inner.record_model_route_snapshot(request).await
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        let state = self.inner.block_run(request).await?;
        let event = TurnLifecycleEvent::from_run_state(&state, TurnEventKind::Blocked, None);
        self.publish_state_once(state.clone(), event).await?;
        Ok(state)
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        let state = self.inner.complete_run(request).await?;
        let event = TurnLifecycleEvent::from_run_state(&state, TurnEventKind::Completed, None);
        self.publish_state_once(state.clone(), event).await?;
        Ok(state)
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.cancel_run(request).await?;
        let event = TurnLifecycleEvent::from_run_state(&state, TurnEventKind::Cancelled, None);
        self.publish_state_once(state.clone(), event).await?;
        Ok(state)
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        let state = self.inner.fail_run(request).await?;
        let event = TurnLifecycleEvent::from_run_state(
            &state,
            TurnEventKind::Failed,
            sanitized_reason_for_state(&state),
        );
        self.publish_state_once(state.clone(), event).await?;
        Ok(state)
    }

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.record_runner_failure(request).await?;
        let event = TurnLifecycleEvent::from_run_state(
            &state,
            event_kind_for_state(&state),
            sanitized_reason_for_state(&state),
        );
        self.publish_state_once(state.clone(), event).await?;
        Ok(state)
    }

    async fn relinquish_run(
        &self,
        request: RelinquishRunRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.relinquish_run(request).await?;
        let event = TurnLifecycleEvent::from_run_state(
            &state,
            event_kind_for_state(&state),
            sanitized_reason_for_state(&state),
        );
        self.publish_state_once(state.clone(), event).await?;
        Ok(state)
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.apply_validated_loop_exit(request).await?;
        let event = TurnLifecycleEvent::from_run_state(
            &state,
            event_kind_for_state(&state),
            sanitized_reason_for_state(&state),
        );
        self.publish_state_once(state.clone(), event).await?;
        Ok(state)
    }
}
