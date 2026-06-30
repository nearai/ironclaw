use async_trait::async_trait;
use std::{
    collections::HashMap,
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
};
use tracing::debug;

const MAX_PREPARED_RUN_IDS: usize = 4096;

use crate::{
    AdmissionRejection, CancelRunRequest, CancelRunResponse, GetRunStateRequest,
    InMemoryRunProfileResolver, ResumeTurnRequest, ResumeTurnResponse, RunProfileResolver,
    SubmitChildRunRequest, SubmitTurnRequest, SubmitTurnResponse, TurnCapacityResource, TurnError,
    TurnRunId, TurnRunState, TurnScope, TurnSpawnTreeStateStore, TurnStateStore, TurnStatus,
    events::EventCursor,
    lifecycle::{LifecyclePublicationErrorPort, NoopLifecyclePublicationErrorPort},
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
    /// Mint a run id without side effects. This intentionally has no default
    /// implementation so every coordinator opts into prepared-run semantics.
    async fn prepare_turn(&self, scope: TurnScope) -> Result<TurnRunId, TurnError>;

    /// Release a run id minted by `prepare_turn` when the caller fails before
    /// submitting it. Coordinators without a prepared-id cache can treat this
    /// as a no-op.
    async fn abort_prepared_turn(&self, _run_id: TurnRunId) -> Result<(), TurnError> {
        Ok(())
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

#[async_trait]
pub trait TurnSpawnTreePort: Send + Sync {
    /// Submit a child run by deriving lineage from the persisted parent and
    /// holding the spawn-tree reservation around the underlying turn submit.
    async fn submit_child_run(
        &self,
        request: SubmitChildRunRequest,
    ) -> Result<SubmitTurnResponse, TurnError>;
}

pub struct DefaultTurnCoordinator<S: ?Sized> {
    store: Arc<S>,
    admission_policy: Arc<dyn TurnAdmissionPolicy>,
    run_profile_resolver: Arc<dyn RunProfileResolver>,
    wake_notifier: Arc<dyn TurnRunWakeNotifier>,
    publication_error_port: Arc<dyn LifecyclePublicationErrorPort>,
    // Per-coordinator binding of run ids handed out by `prepare_turn` to the
    // scope they were prepared under. `submit_turn` consumes the reservation
    // when `requested_run_id` is set and rejects cross-scope submission so a
    // prepared id cannot be used to inject lineage into a different scope.
    prepared_run_id_scopes: Mutex<HashMap<TurnRunId, TurnScope>>,
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
            publication_error_port: Arc::new(NoopLifecyclePublicationErrorPort),
            prepared_run_id_scopes: Mutex::new(HashMap::new()),
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

    pub fn with_lifecycle_publication_error_port(
        mut self,
        port: Arc<dyn LifecyclePublicationErrorPort>,
    ) -> Self {
        self.publication_error_port = port;
        self
    }

    fn record_prepared_run_id(&self, run_id: TurnRunId, scope: TurnScope) -> Result<(), TurnError> {
        let mut prepared = match self.prepared_run_id_scopes.lock() {
            Ok(prepared) => prepared,
            Err(poisoned) => poisoned.into_inner(),
        };
        if prepared.len() >= MAX_PREPARED_RUN_IDS {
            return Err(TurnError::CapacityExceeded {
                resource: TurnCapacityResource::SubmitTurn,
                cap: MAX_PREPARED_RUN_IDS as u64,
            });
        }
        prepared.insert(run_id, scope);
        Ok(())
    }

    fn consume_prepared_run_id(&self, run_id: TurnRunId) -> Option<TurnScope> {
        let mut prepared = match self.prepared_run_id_scopes.lock() {
            Ok(prepared) => prepared,
            Err(poisoned) => poisoned.into_inner(),
        };
        prepared.remove(&run_id)
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

fn submit_event_cursor(response: &SubmitTurnResponse) -> EventCursor {
    let SubmitTurnResponse::Accepted { event_cursor, .. } = response;
    *event_cursor
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

fn deferred_publication_error(
    port: &dyn LifecyclePublicationErrorPort,
    cursor: EventCursor,
) -> Result<(), TurnError> {
    match port.take_lifecycle_publication_error(cursor) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[async_trait]
impl<S> TurnCoordinator for DefaultTurnCoordinator<S>
where
    S: TurnStateStore + ?Sized + 'static,
{
    async fn prepare_turn(&self, scope: TurnScope) -> Result<TurnRunId, TurnError> {
        let run_id = TurnRunId::new();
        self.record_prepared_run_id(run_id, scope)?;
        Ok(run_id)
    }

    async fn abort_prepared_turn(&self, run_id: TurnRunId) -> Result<(), TurnError> {
        self.consume_prepared_run_id(run_id);
        Ok(())
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        // If the caller passed a run id that came out of prepare_turn, verify
        // it is being submitted under the same scope it was prepared under,
        // unless this is a child run (parent_run_id set). Subagent spawn
        // legitimately prepares a run id in the parent scope and submits it
        // under a different child scope. Reservations are consumed on the
        // first submit attempt; a second attempt with the same id falls back
        // to the store's duplicate-bound check.
        if let Some(requested) = request.requested_run_id
            && let Some(prepared_scope) = self.consume_prepared_run_id(requested)
            && request.parent_run_id.is_none()
            && prepared_scope != request.scope
        {
            return Err(TurnError::Unauthorized);
        }
        let scope = request.scope.clone();
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
        deferred_publication_error(
            self.publication_error_port.as_ref(),
            submit_event_cursor(&response),
        )?;
        Ok(response)
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let scope = request.scope.clone();
        let response = self.store.resume_turn(request).await?;
        notify_queued_run_best_effort(
            self.wake_notifier.as_ref(),
            resume_wake(scope.clone(), &response),
        );
        deferred_publication_error(self.publication_error_port.as_ref(), response.event_cursor)?;
        Ok(response)
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        let scope = request.scope.clone();
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
            deferred_publication_error(
                self.publication_error_port.as_ref(),
                response.event_cursor,
            )?;
        }
        Ok(response)
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.store.get_run_state(request).await
    }
}

#[async_trait]
impl<S> TurnSpawnTreePort for DefaultTurnCoordinator<S>
where
    S: TurnSpawnTreeStateStore + ?Sized + 'static,
{
    async fn submit_child_run(
        &self,
        request: SubmitChildRunRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let child_scope = request.child_scope.clone();
        let response = self
            .store
            .submit_child_turn(
                request,
                self.admission_policy.as_ref(),
                self.run_profile_resolver.as_ref(),
            )
            .await?;
        notify_queued_run_best_effort(
            self.wake_notifier.as_ref(),
            submit_wake(child_scope, &response),
        );
        deferred_publication_error(
            self.publication_error_port.as_ref(),
            submit_event_cursor(&response),
        )?;
        Ok(response)
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

    async fn abort_prepared_turn(&self, run_id: TurnRunId) -> Result<(), TurnError> {
        self.as_ref().abort_prepared_turn(run_id).await
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

#[async_trait]
impl<C> TurnSpawnTreePort for Arc<C>
where
    C: TurnSpawnTreePort + ?Sized,
{
    async fn submit_child_run(
        &self,
        request: SubmitChildRunRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.as_ref().submit_child_run(request).await
    }
}
