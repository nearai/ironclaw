use async_trait::async_trait;
use chrono::Utc;
use std::{
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};
use tracing::debug;

use crate::{
    AdmissionRejection, CancelRunRequest, CancelRunResponse, GetRunStateRequest,
    InMemoryRunProfileResolver, ResumeTurnRequest, ResumeTurnResponse, RunProfileResolver,
    SubmitTurnRequest, SubmitTurnResponse, TurnError, TurnEventKind, TurnEventSink,
    TurnLifecycleEvent, TurnRunId, TurnRunState, TurnScope, TurnStateStore, TurnStatus,
    events::EventCursor,
};

pub trait TurnAdmissionPolicy: Send + Sync {
    fn check_submit(&self, request: &SubmitTurnRequest) -> Result<(), AdmissionRejection>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRunWake {
    pub scope: TurnScope,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub event_cursor: EventCursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TurnRunWakeNotifyError {
    DeliveryUnavailable,
}

impl fmt::Display for TurnRunWakeNotifyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeliveryUnavailable => formatter.write_str("delivery_unavailable"),
        }
    }
}

impl std::error::Error for TurnRunWakeNotifyError {}

pub trait TurnRunWakeNotifier: Send + Sync {
    fn notify_queued_run(&self, wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError>;
}

#[derive(Debug, Default)]
pub struct NoopTurnRunWakeNotifier;

impl TurnRunWakeNotifier for NoopTurnRunWakeNotifier {
    fn notify_queued_run(&self, _wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct AllowAllTurnAdmissionPolicy;

impl TurnAdmissionPolicy for AllowAllTurnAdmissionPolicy {
    fn check_submit(&self, _request: &SubmitTurnRequest) -> Result<(), AdmissionRejection> {
        Ok(())
    }
}

#[async_trait]
pub trait TurnCoordinator: Send + Sync {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError>;

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError>;

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError>;

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError>;
}

pub struct DefaultTurnCoordinator<S: ?Sized> {
    store: Arc<S>,
    admission_policy: Arc<dyn TurnAdmissionPolicy>,
    run_profile_resolver: Arc<dyn RunProfileResolver>,
    wake_notifier: Arc<dyn TurnRunWakeNotifier>,
    event_sink: Option<Arc<dyn TurnEventSink>>,
}

impl<S> DefaultTurnCoordinator<S>
where
    S: TurnStateStore + ?Sized,
{
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            admission_policy: Arc::new(AllowAllTurnAdmissionPolicy),
            run_profile_resolver: Arc::new(InMemoryRunProfileResolver::default()),
            wake_notifier: Arc::new(NoopTurnRunWakeNotifier),
            event_sink: None,
        }
    }

    pub fn with_admission_policy(mut self, policy: Arc<dyn TurnAdmissionPolicy>) -> Self {
        self.admission_policy = policy;
        self
    }

    pub fn with_run_profile_resolver(mut self, resolver: Arc<dyn RunProfileResolver>) -> Self {
        self.run_profile_resolver = resolver;
        self
    }

    pub fn with_wake_notifier(mut self, notifier: Arc<dyn TurnRunWakeNotifier>) -> Self {
        self.wake_notifier = notifier;
        self
    }

    pub fn with_event_sink(mut self, sink: Arc<dyn TurnEventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }
}

fn submit_wake(scope: TurnScope, response: &SubmitTurnResponse) -> TurnRunWake {
    let SubmitTurnResponse::Accepted {
        run_id,
        status,
        event_cursor,
        ..
    } = response;
    TurnRunWake {
        scope,
        run_id: *run_id,
        status: *status,
        event_cursor: *event_cursor,
    }
}

fn resume_wake(scope: TurnScope, response: &ResumeTurnResponse) -> TurnRunWake {
    TurnRunWake {
        scope,
        run_id: response.run_id,
        status: response.status,
        event_cursor: response.event_cursor,
    }
}

fn cancel_wake(scope: TurnScope, response: &CancelRunResponse) -> TurnRunWake {
    TurnRunWake {
        scope,
        run_id: response.run_id,
        status: response.status,
        event_cursor: response.event_cursor,
    }
}

fn notify_queued_run_best_effort(notifier: &dyn TurnRunWakeNotifier, wake: TurnRunWake) {
    match catch_unwind(AssertUnwindSafe(|| notifier.notify_queued_run(wake))) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => debug!(error = %error, "turn run wake notification failed"),
        Err(_) => debug!("turn run wake notifier panicked"),
    }
}

async fn publish_turn_event_best_effort(
    sink: Option<&Arc<dyn TurnEventSink>>,
    event: TurnLifecycleEvent,
) {
    let Some(sink) = sink else {
        return;
    };
    if let Err(error) = sink.publish(event).await {
        debug!(error = %error, "turn lifecycle event sink publish failed");
    }
}

#[async_trait]
impl<S> TurnCoordinator for DefaultTurnCoordinator<S>
where
    S: TurnStateStore + ?Sized + 'static,
{
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let scope = request.scope.clone();
        let event_scope = request.scope.clone();
        let actor = request.actor.clone();
        let occurred_at = request.received_at;
        let response = self
            .store
            .submit_turn(
                request,
                self.admission_policy.as_ref(),
                self.run_profile_resolver.as_ref(),
            )
            .await?;
        let SubmitTurnResponse::Accepted {
            run_id,
            status,
            resolved_run_profile_id,
            resolved_run_profile_version,
            ..
        } = &response;
        debug!(
            run_id = %run_id,
            status = ?status,
            resolved_run_profile_id = resolved_run_profile_id.as_str(),
            resolved_run_profile_version = resolved_run_profile_version.as_u64(),
            "turn coordinator accepted turn with resolved run profile"
        );
        notify_queued_run_best_effort(self.wake_notifier.as_ref(), submit_wake(scope, &response));
        let SubmitTurnResponse::Accepted {
            run_id,
            status,
            event_cursor,
            ..
        } = &response;
        publish_turn_event_best_effort(
            self.event_sink.as_ref(),
            TurnLifecycleEvent {
                cursor: *event_cursor,
                scope: event_scope,
                occurred_at: Some(occurred_at),
                owner_user_id: Some(actor.user_id),
                run_id: *run_id,
                status: *status,
                kind: TurnEventKind::Submitted,
                blocked_gate: None,
                sanitized_reason: None,
            },
        )
        .await;
        Ok(response)
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let scope = request.scope.clone();
        let actor = request.actor.clone();
        let response = self.store.resume_turn(request).await?;
        notify_queued_run_best_effort(
            self.wake_notifier.as_ref(),
            resume_wake(scope.clone(), &response),
        );
        publish_turn_event_best_effort(
            self.event_sink.as_ref(),
            TurnLifecycleEvent {
                cursor: response.event_cursor,
                scope,
                occurred_at: Some(Utc::now()),
                owner_user_id: Some(actor.user_id),
                run_id: response.run_id,
                status: response.status,
                kind: TurnEventKind::Resumed,
                blocked_gate: None,
                sanitized_reason: None,
            },
        )
        .await;
        Ok(response)
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        let scope = request.scope.clone();
        let actor = request.actor.clone();
        let reason = request.reason;
        let response = self.store.request_cancel(request).await?;
        // Wake on `CancelRequested` (the cooperative case) AND on any terminal
        // transition. Registered handles otherwise rely solely on the polling
        // fallback to discover a direct-to-terminal cancellation, which would
        // leave them in the requester map until the polling task next ticks.
        if response.status == TurnStatus::CancelRequested || response.status.is_terminal() {
            notify_queued_run_best_effort(
                self.wake_notifier.as_ref(),
                cancel_wake(scope.clone(), &response),
            );
        }
        if !response.already_terminal {
            let kind = if response.status == TurnStatus::CancelRequested {
                TurnEventKind::CancelRequested
            } else {
                TurnEventKind::Cancelled
            };
            publish_turn_event_best_effort(
                self.event_sink.as_ref(),
                TurnLifecycleEvent {
                    cursor: response.event_cursor,
                    scope,
                    occurred_at: Some(Utc::now()),
                    owner_user_id: Some(actor.user_id),
                    run_id: response.run_id,
                    status: response.status,
                    kind,
                    blocked_gate: None,
                    sanitized_reason: Some(reason.category().to_string()),
                },
            )
            .await;
        }
        Ok(response)
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.store.get_run_state(request).await
    }
}

#[async_trait]
impl<C> TurnCoordinator for Arc<C>
where
    C: TurnCoordinator + ?Sized,
{
    async fn prepare_turn(&self, scope: TurnScope) -> Result<TurnRunId, TurnError> {
        self.as_ref().prepare_turn(scope).await
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.as_ref().submit_turn(request).await
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        self.as_ref().resume_turn(request).await
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        self.as_ref().cancel_run(request).await
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.as_ref().get_run_state(request).await
    }
}
