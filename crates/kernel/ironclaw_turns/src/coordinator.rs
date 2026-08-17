use async_trait::async_trait;
use ironclaw_observability::live_latency_started_at;
use std::{
    collections::HashMap,
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
    time::Instant,
};
use tracing::debug;

const MAX_PREPARED_RUN_IDS: usize = 4096;

fn trace_coordinator_latency_ok(
    operation: &'static str,
    scope: &TurnScope,
    run_id: Option<TurnRunId>,
    started_at: Option<Instant>,
) {
    let run_id = run_id.map(|id| id.to_string()).unwrap_or_default();
    ironclaw_observability::live_latency_trace_ok!(
        "turn_coordinator",
        operation,
        started_at,
        tenant_id = %scope.tenant_id,
        agent_id = scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        project_id = scope.project_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        thread_id = %scope.thread_id,
        owner_user_id = scope.explicit_owner_user_id().map(|id| id.as_str()).unwrap_or(""),
        run_id = run_id.as_str(),
        "turn coordinator operation completed",
    );
}

fn trace_coordinator_latency_error<E: ?Sized>(
    operation: &'static str,
    scope: &TurnScope,
    run_id: Option<TurnRunId>,
    started_at: Option<Instant>,
    _error: &E,
) {
    let run_id = run_id.map(|id| id.to_string()).unwrap_or_default();
    ironclaw_observability::live_latency_trace_error!(
        "turn_coordinator",
        operation,
        started_at,
        "turn_error",
        tenant_id = %scope.tenant_id,
        agent_id = scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        project_id = scope.project_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        thread_id = %scope.thread_id,
        owner_user_id = scope.explicit_owner_user_id().map(|id| id.as_str()).unwrap_or(""),
        run_id = run_id.as_str(),
        "turn coordinator operation failed",
    );
}

use crate::{
    AdmissionRejection, AdmissionRejectionReason, AgentTurnRuntimePort,
    AgentTurnSpawnTreeRuntimePort, CancelRunRequest, CancelRunResponse, EventCursor,
    GetRunStateRequest, ResumeTurnRequest, ResumeTurnResponse, RetryTurnRequest, RetryTurnResponse,
    RunProfileId, RunProfileRequest, SubmitChildRunRequest, SubmitTurnRequest, SubmitTurnResponse,
    TurnCapacityResource, TurnError, TurnRunId, TurnRunState, TurnScope, TurnStatus,
    process_projection::AgentTurnProcessRuntime,
};
use ironclaw_host_api::prepared_context::{
    PreparedContextSource, PreparedTurnDeclarations, TurnLimits,
};
use ironclaw_loop_contracts::{
    InMemoryRunProfileResolver, ResolvedRunProfile, RunProfileResolutionError,
    RunProfileResolutionRequest, RunProfileResolver,
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

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError>;

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
    process_runtime: Option<AgentTurnProcessRuntime>,
    // Admission probe for prepared contexts. A submission carrying
    // no binding refs must point at a prepared context (the probe answers
    // with its journaled declarations) or it is rejected fail-closed — the
    // guard against a conversation workflow silently losing its reply route.
    prepared_context_source: Option<Arc<dyn PreparedContextSource>>,
    // Per-coordinator binding of run ids handed out by `prepare_turn` to the
    // scope they were prepared under. `submit_turn` consumes the reservation
    // when `requested_run_id` is set and rejects cross-scope submission so a
    // prepared id cannot be used to inject lineage into a different scope.
    prepared_run_id_scopes: Mutex<HashMap<TurnRunId, TurnScope>>,
}

impl<S> DefaultTurnCoordinator<S>
where
    S: AgentTurnRuntimePort + ?Sized,
{
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            admission_policy: Arc::new(AllowAllTurnAdmissionPolicy),
            run_profile_resolver: Arc::new(InMemoryRunProfileResolver::default()),
            wake_notifier: Arc::new(NoopTurnRunWakeNotifier),
            process_runtime: None,
            prepared_context_source: None,
            prepared_run_id_scopes: Mutex::new(HashMap::new()),
        }
    }

    /// Wire the prepared context probe. Without it, every
    /// submission with `None` binding refs is rejected at admission.
    pub fn with_prepared_context_source(mut self, source: Arc<dyn PreparedContextSource>) -> Self {
        self.prepared_context_source = Some(source);
        self
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

    pub fn with_process_runtime(mut self, runtime: AgentTurnProcessRuntime) -> Self {
        self.process_runtime = Some(runtime);
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

    /// Unbound-profile derivation (unbound-turn design §4.2): admission
    /// derives the run profile from what the submitted thread holds. When the
    /// caller passes no profile hint, the coordinator probes the thread's
    /// prepared-context record — prepared threads resolve the unbound
    /// profiles (`unbound_structured` when the journaled output contract is a
    /// JSON schema, `unbound_default` otherwise) and their declared limits
    /// narrow the resolved budget; ordinary conversation threads resolve
    /// exactly as before. An explicit `unbound_*` hint must match a prepared
    /// thread's derived profile (fail-closed); any other explicit hint is a
    /// trusted product path (triggers, subagents) and skips the probe.
    async fn derive_unbound_submission(
        &self,
        mut request: SubmitTurnRequest,
    ) -> Result<(SubmitTurnRequest, Option<PreparedTurnDeclarations>), TurnError> {
        if request.parent_run_id.is_some() {
            return Ok((request, None));
        }
        let hint_is_unbound = request
            .requested_run_profile
            .as_ref()
            .is_some_and(|hint| RunProfileId::from_request(hint).is_unbound());
        if request.requested_run_profile.is_some() && !hint_is_unbound {
            return Ok((request, None));
        }
        let Some(source) = &self.prepared_context_source else {
            if hint_is_unbound {
                // An explicit unbound hint without a wired probe cannot be
                // validated against a prepared context: fail closed.
                return Err(profile_rejected());
            }
            return Ok((request, None));
        };
        let declarations = match source
            .read_declarations(
                &request.scope,
                &request.actor,
                &request.accepted_message_ref,
            )
            .await
        {
            Ok(Some(declarations)) => declarations,
            Ok(None) => {
                if hint_is_unbound {
                    // Unbound profiles exist only for prepared threads.
                    return Err(profile_rejected());
                }
                return Ok((request, None));
            }
            Err(error) => {
                // debug!, not warn!: background diagnostics stay off the REPL.
                debug!(%error, "prepared-context read failed during unbound admission");
                return Err(TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::Unavailable,
                )));
            }
        };
        let derived = if declarations.output.is_json_schema() {
            RunProfileId::unbound_structured()
        } else {
            RunProfileId::unbound_default()
        };
        match &request.requested_run_profile {
            None => {
                request.requested_run_profile = Some(
                    RunProfileRequest::new(derived.as_str())
                        .map_err(|reason| TurnError::InvalidRequest { reason })?,
                );
            }
            Some(hint) if hint.as_str() == derived.as_str() => {}
            Some(_) => return Err(profile_rejected()),
        }
        Ok((request, Some(declarations)))
    }

    fn consume_prepared_run_id(&self, run_id: TurnRunId) -> Option<TurnScope> {
        let mut prepared = match self.prepared_run_id_scopes.lock() {
            Ok(prepared) => prepared,
            Err(poisoned) => poisoned.into_inner(),
        };
        prepared.remove(&run_id)
    }
}

/// Per-submit resolver decorator that clamps the resolved profile's budget
/// ceilings by the declared narrowing-only limits (unbound-turn design
/// §4.2: limits narrow profile ceilings, never widen them).
struct DeclaredLimitsNarrowingResolver<'a> {
    inner: &'a dyn RunProfileResolver,
    limits: TurnLimits,
}

#[async_trait]
impl RunProfileResolver for DeclaredLimitsNarrowingResolver<'_> {
    async fn resolve_run_profile(
        &self,
        request: RunProfileResolutionRequest,
    ) -> Result<ResolvedRunProfile, RunProfileResolutionError> {
        let mut resolved = self.inner.resolve_run_profile(request).await?;
        if let Some(cap) = self.limits.max_model_calls {
            resolved.resource_budget_policy.max_model_calls =
                resolved.resource_budget_policy.max_model_calls.min(cap);
        }
        if let Some(cap) = self.limits.max_capability_invocations {
            resolved.resource_budget_policy.max_capability_invocations = resolved
                .resource_budget_policy
                .max_capability_invocations
                .min(cap);
        }
        if let Some(cap) = self.limits.max_wall_clock_seconds {
            resolved.resource_budget_policy.max_wall_clock_seconds = Some(
                resolved
                    .resource_budget_policy
                    .max_wall_clock_seconds
                    .map_or(cap, |ceiling| ceiling.min(cap)),
            );
        }
        Ok(resolved)
    }
}

fn profile_rejected() -> TurnError {
    TurnError::AdmissionRejected(AdmissionRejection::new(
        AdmissionRejectionReason::ProfileRejected,
    ))
}

fn wake_from(
    scope: TurnScope,
    run_id: TurnRunId,
    status: TurnStatus,
    event_cursor: EventCursor,
) -> TurnRunWake {
    TurnRunWake {
        scope,
        run_id,
        status,
        event_cursor,
    }
}

fn submit_wake(scope: TurnScope, response: &SubmitTurnResponse) -> TurnRunWake {
    let SubmitTurnResponse::Accepted {
        run_id,
        status,
        event_cursor,
        ..
    } = response;
    wake_from(scope, *run_id, *status, *event_cursor)
}

fn resume_wake(scope: TurnScope, response: &ResumeTurnResponse) -> TurnRunWake {
    wake_from(
        scope,
        response.run_id,
        response.status,
        response.event_cursor,
    )
}

fn retry_wake(scope: TurnScope, response: &RetryTurnResponse) -> TurnRunWake {
    wake_from(
        scope,
        response.run_id,
        response.status,
        response.event_cursor,
    )
}

fn cancel_wake(scope: TurnScope, response: &CancelRunResponse) -> TurnRunWake {
    wake_from(
        scope,
        response.run_id,
        response.status,
        response.event_cursor,
    )
}

fn notify_queued_run_best_effort(notifier: &dyn TurnRunWakeNotifier, wake: TurnRunWake) {
    match catch_unwind(AssertUnwindSafe(|| notifier.notify_queued_run(wake))) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => debug!(error = %error, "turn run wake notification failed"),
        Err(_) => debug!("turn run wake notifier panicked"),
    }
}

#[async_trait]
impl<S> TurnCoordinator for DefaultTurnCoordinator<S>
where
    S: AgentTurnRuntimePort + ?Sized + 'static,
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
        let started_at = live_latency_started_at();
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
        let (request, declared_limits) = self.derive_unbound_submission(request).await?;
        let narrowing_resolver = declared_limits
            .filter(|declarations| !declarations.limits.is_unlimited())
            .map(|declarations| DeclaredLimitsNarrowingResolver {
                inner: self.run_profile_resolver.as_ref(),
                limits: declarations.limits,
            });
        let resolver: &dyn RunProfileResolver = match &narrowing_resolver {
            Some(narrowing) => narrowing,
            None => self.run_profile_resolver.as_ref(),
        };
        let scope = request.scope.clone();
        let response = match &self.process_runtime {
            Some(runtime) => {
                runtime
                    .submit_turn(request, self.admission_policy.as_ref(), resolver)
                    .await
            }
            None => {
                self.store
                    .submit_turn(request, self.admission_policy.as_ref(), resolver)
                    .await
            }
        };
        let response = match response {
            Ok(response) => {
                let SubmitTurnResponse::Accepted { run_id, .. } = &response;
                trace_coordinator_latency_ok(
                    "store_submit_turn",
                    &scope,
                    Some(*run_id),
                    started_at,
                );
                response
            }
            Err(error) => {
                trace_coordinator_latency_error(
                    "store_submit_turn",
                    &scope,
                    None,
                    started_at,
                    &error,
                );
                return Err(error);
            }
        };
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
        let wake_started_at = live_latency_started_at();
        notify_queued_run_best_effort(
            self.wake_notifier.as_ref(),
            submit_wake(scope.clone(), &response),
        );
        trace_coordinator_latency_ok("notify_queued_run", &scope, Some(*run_id), wake_started_at);
        Ok(response)
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let started_at = live_latency_started_at();
        let scope = request.scope.clone();
        let response = match &self.process_runtime {
            Some(runtime) => runtime.resume_turn(request).await,
            None => self.store.resume_turn(request).await,
        };
        let response = match response {
            Ok(response) => {
                trace_coordinator_latency_ok(
                    "store_resume_turn",
                    &scope,
                    Some(response.run_id),
                    started_at,
                );
                response
            }
            Err(error) => {
                trace_coordinator_latency_error(
                    "store_resume_turn",
                    &scope,
                    None,
                    started_at,
                    &error,
                );
                return Err(error);
            }
        };
        let wake_started_at = live_latency_started_at();
        notify_queued_run_best_effort(
            self.wake_notifier.as_ref(),
            resume_wake(scope.clone(), &response),
        );
        trace_coordinator_latency_ok(
            "notify_resumed_run",
            &scope,
            Some(response.run_id),
            wake_started_at,
        );
        Ok(response)
    }

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        let scope = request.scope.clone();
        let response = match &self.process_runtime {
            Some(runtime) => runtime.retry_turn(request).await?,
            None => self.store.retry_turn(request).await?,
        };
        notify_queued_run_best_effort(
            self.wake_notifier.as_ref(),
            retry_wake(scope.clone(), &response),
        );
        Ok(response)
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        let started_at = live_latency_started_at();
        let scope = request.scope.clone();
        let response = match &self.process_runtime {
            Some(runtime) => runtime.cancel_run(request).await,
            None => self.store.request_cancel(request).await,
        };
        let response = match response {
            Ok(response) => {
                trace_coordinator_latency_ok(
                    "store_cancel_run",
                    &scope,
                    Some(response.run_id),
                    started_at,
                );
                response
            }
            Err(error) => {
                trace_coordinator_latency_error(
                    "store_cancel_run",
                    &scope,
                    None,
                    started_at,
                    &error,
                );
                return Err(error);
            }
        };
        // Wake on `CancelRequested` (the cooperative case) AND on any terminal
        // transition. Registered handles otherwise rely solely on the polling
        // fallback to discover a direct-to-terminal cancellation, which would
        // leave them in the requester map until the polling task next ticks.
        if response.status == TurnStatus::CancelRequested || response.status.is_terminal() {
            let wake_started_at = live_latency_started_at();
            notify_queued_run_best_effort(
                self.wake_notifier.as_ref(),
                cancel_wake(scope.clone(), &response),
            );
            trace_coordinator_latency_ok(
                "notify_cancelled_run",
                &scope,
                Some(response.run_id),
                wake_started_at,
            );
        }
        Ok(response)
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        match &self.process_runtime {
            Some(runtime) => runtime.get_run_state(&request.scope, request.run_id).await,
            None => self.store.get_run_state(request).await,
        }
    }
}

#[async_trait]
impl<S> TurnSpawnTreePort for DefaultTurnCoordinator<S>
where
    S: AgentTurnSpawnTreeRuntimePort + ?Sized + 'static,
{
    async fn submit_child_run(
        &self,
        request: SubmitChildRunRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let started_at = live_latency_started_at();
        let child_scope = request.child_scope.clone();
        let response = match &self.process_runtime {
            Some(runtime) => {
                runtime
                    .submit_child_turn(
                        request,
                        self.admission_policy.as_ref(),
                        self.run_profile_resolver.as_ref(),
                    )
                    .await
            }
            None => {
                self.store
                    .submit_child_turn(
                        request,
                        self.admission_policy.as_ref(),
                        self.run_profile_resolver.as_ref(),
                    )
                    .await
            }
        };
        let response = match response {
            Ok(response) => {
                let SubmitTurnResponse::Accepted { run_id, .. } = &response;
                trace_coordinator_latency_ok(
                    "store_submit_child_turn",
                    &child_scope,
                    Some(*run_id),
                    started_at,
                );
                response
            }
            Err(error) => {
                trace_coordinator_latency_error(
                    "store_submit_child_turn",
                    &child_scope,
                    None,
                    started_at,
                    &error,
                );
                return Err(error);
            }
        };
        let SubmitTurnResponse::Accepted { run_id, .. } = &response;
        let wake_started_at = live_latency_started_at();
        notify_queued_run_best_effort(
            self.wake_notifier.as_ref(),
            submit_wake(child_scope.clone(), &response),
        );
        trace_coordinator_latency_ok(
            "notify_child_run",
            &child_scope,
            Some(*run_id),
            wake_started_at,
        );
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

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        self.as_ref().retry_turn(request).await
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

#[cfg(test)]
mod declared_limits_tests {
    use super::*;
    use ironclaw_loop_contracts::{InMemoryRunProfileResolver, RunProfileResolutionRequest};

    #[tokio::test]
    async fn declared_limits_narrow_profile_ceilings_and_never_widen_them() {
        let inner = InMemoryRunProfileResolver::default();
        let baseline = inner
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("baseline profile");
        let ceiling_calls = baseline.resource_budget_policy.max_model_calls;
        let ceiling_invocations = baseline.resource_budget_policy.max_capability_invocations;
        assert_eq!(
            baseline.resource_budget_policy.max_wall_clock_seconds, None,
            "the interactive profile declares no wall-clock ceiling"
        );

        // Narrowing: every declared limit below the ceiling binds; a
        // wall-clock declaration binds even with no profile ceiling.
        let resolver = DeclaredLimitsNarrowingResolver {
            inner: &inner,
            limits: TurnLimits {
                max_model_calls: Some(ceiling_calls - 1),
                max_capability_invocations: Some(ceiling_invocations + 1_000),
                max_wall_clock_seconds: Some(60),
            },
        };
        let narrowed = resolver
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("narrowed profile");
        assert_eq!(
            narrowed.resource_budget_policy.max_model_calls,
            ceiling_calls - 1
        );
        assert_eq!(
            narrowed.resource_budget_policy.max_capability_invocations, ceiling_invocations,
            "a declared limit above the profile ceiling must not widen it"
        );
        assert_eq!(
            narrowed.resource_budget_policy.max_wall_clock_seconds,
            Some(60)
        );

        // A profile wall-clock ceiling is only ever shortened.
        let resolver = DeclaredLimitsNarrowingResolver {
            inner: &resolver,
            limits: TurnLimits {
                max_model_calls: None,
                max_capability_invocations: None,
                max_wall_clock_seconds: Some(3_600),
            },
        };
        let twice_narrowed = resolver
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("twice-narrowed profile");
        assert_eq!(
            twice_narrowed.resource_budget_policy.max_wall_clock_seconds,
            Some(60),
            "a larger declared wall clock must not extend the narrowed ceiling"
        );
    }
}
