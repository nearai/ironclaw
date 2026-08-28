//! Product-auth continuation handling.
//!
//! This module consumes the `ironclaw_auth` continuation vocabulary and routes
//! turn-gate resume continuations through the same trusted `TurnCoordinator`
//! boundary as the WebUI gate-resolution path. It intentionally does not define
//! another auth-flow model or handle non-turn continuation variants.

use ironclaw_product_contracts::lifecycle_service::{
    LifecycleProductContext, LifecycleProductService, LifecycleProductSurfaceContext,
};

use std::sync::Arc;

use async_trait::async_trait;
pub use ironclaw_auth::RebornAuthContinuationDispatcher as ProductAuthContinuationDispatcher;
use ironclaw_auth::{AuthContinuationEvent, AuthContinuationRef, AuthProductError};
use ironclaw_host_api::turn::{IdempotencyKey, TurnGateRef, TurnRunId, TurnScope, TurnStatus};
use ironclaw_notifications::{
    NotificationInboxError, NotificationInboxStorePort, NotificationKind,
    NotificationMutationRequest, NotificationRecipient,
};
use ironclaw_turns::{
    GateResumeDisposition, GetRunStateRequest, ResumeTurnPrecondition, ResumeTurnRequest,
    TurnCoordinator, TurnError, TurnErrorCategory,
};
use uuid::Uuid;

use crate::binding_ref::{
    AUTH_CONTINUATION_BINDING_REF_RAW_MAX_BYTES, binding_ref_segment, bounded_idempotency_key,
};
use crate::{
    AuthContinuationRejectionKind, LifecyclePackageKind, LifecyclePackageRef,
    LifecycleProductAction, ProductSurfaceFailure,
};

struct LifecycleAuthContinuationDispatcher {
    lifecycle: Arc<dyn LifecycleProductService>,
    inner: Arc<dyn ProductAuthContinuationDispatcher>,
}

pub fn lifecycle_auth_continuation_dispatcher(
    lifecycle: Arc<dyn LifecycleProductService>,
    inner: Arc<dyn ProductAuthContinuationDispatcher>,
) -> Arc<dyn ProductAuthContinuationDispatcher> {
    Arc::new(LifecycleAuthContinuationDispatcher { lifecycle, inner })
}

#[async_trait]
impl ProductAuthContinuationDispatcher for LifecycleAuthContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        if let AuthContinuationRef::LifecycleActivation { package_ref } = &event.continuation {
            let package_ref =
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_ref.as_str())
                    .map_err(|_| AuthProductError::LifecycleActivationFailed)?;
            let context = LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
                tenant_id: event.scope.resource.tenant_id.clone(),
                user_id: event.scope.resource.user_id.clone(),
                agent_id: event.scope.resource.agent_id.clone(),
                project_id: event.scope.resource.project_id.clone(),
            });
            self.lifecycle
                .execute(
                    context,
                    LifecycleProductAction::ExtensionInstall { package_ref },
                )
                .await
                .map_err(|error| {
                    tracing::debug!(
                        %error,
                        "product auth lifecycle activation continuation failed"
                    );
                    AuthProductError::LifecycleActivationFailed
                })?;
        }
        self.inner.dispatch_auth_continuation(event).await
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.inner.dispatch_canceled_auth_continuation(event).await
    }
}

#[derive(Clone)]
pub struct ProductAuthTurnGateResumeDispatcher {
    turn_coordinator: Arc<dyn TurnCoordinator>,
    notification_inbox: Option<Arc<dyn NotificationInboxStorePort>>,
}

impl ProductAuthTurnGateResumeDispatcher {
    pub fn new(turn_coordinator: Arc<dyn TurnCoordinator>) -> Self {
        Self {
            turn_coordinator,
            notification_inbox: None,
        }
    }

    pub fn with_notification_inbox(
        mut self,
        notification_inbox: Arc<dyn NotificationInboxStorePort>,
    ) -> Self {
        self.notification_inbox = Some(notification_inbox);
        self
    }

    pub async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        if matches!(
            &event.continuation,
            AuthContinuationRef::TurnGateResume { .. }
        ) {
            let flow_id = event.flow_id;
            self.dispatch_turn_gate_resume(event)
                .await
                .map(|_| ())
                .map_err(|error| {
                    let auth_error = auth_error_for_continuation_dispatch(&error);
                    tracing::debug!(
                        %flow_id,
                        auth_error_code = ?auth_error.code(),
                        surface_error_kind = surface_error_kind(&error),
                        "product auth turn-gate continuation dispatch failed"
                    );
                    auth_error
                })
        } else {
            tracing::debug!(
                flow_id = %event.flow_id,
                continuation_kind = continuation_kind(&event.continuation),
                "non-turn auth continuation deferred to follow-up handler"
            );
            Ok(())
        }
    }

    pub async fn dispatch_canceled_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        if matches!(
            &event.continuation,
            AuthContinuationRef::TurnGateResume { .. }
        ) {
            let flow_id = event.flow_id;
            self.dispatch_turn_gate(event, Some(GateResumeDisposition::Denied), true)
                .await
                .map(|_| ())
                .map_err(|error| {
                    let auth_error = auth_error_for_continuation_dispatch(&error);
                    tracing::debug!(
                        %flow_id,
                        auth_error_code = ?auth_error.code(),
                        surface_error_kind = surface_error_kind(&error),
                        "canceled product-auth turn-gate denial failed"
                    );
                    auth_error
                })
        } else {
            Ok(())
        }
    }

    pub async fn dispatch_turn_gate_resume(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<TurnRunId, ProductSurfaceFailure> {
        // Tolerate an already-settled gate exactly like the deny path does:
        // a completed continuation is re-dispatched whenever the durable
        // `continuation_emitted_at` fence was not stamped (e.g. the fan-out
        // sweep was incomplete and the whole dispatch stays retryable). On
        // replay the primary run has typically already resumed — its gate is
        // no longer the blocked gate — and that is the settled outcome this
        // continuation wanted, not an error to retry forever.
        self.dispatch_turn_gate(event, None, true).await
    }

    async fn dispatch_turn_gate(
        &self,
        event: AuthContinuationEvent,
        resume_disposition: Option<GateResumeDisposition>,
        ignore_stale_gate: bool,
    ) -> Result<TurnRunId, ProductSurfaceFailure> {
        let AuthContinuationRef::TurnGateResume {
            turn_run_ref,
            gate_ref,
        } = &event.continuation
        else {
            return Err(ProductSurfaceFailure::AuthContinuationRejected {
                kind: AuthContinuationRejectionKind::NotTurnGateResume,
            });
        };

        let run_id = parse_turn_run_id(turn_run_ref.as_str())?;
        let scope = turn_scope_from_auth_event(&event)?;
        let state = self
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
            .map_err(map_auth_resume_error)?;
        let gate_resolution_ref = parse_gate_ref(gate_ref.as_str())?;
        let denied = resume_disposition == Some(GateResumeDisposition::Denied);
        let gate_is_current = state.status == TurnStatus::BlockedAuth
            && state.gate_ref.as_ref() == Some(&gate_resolution_ref);
        if ignore_stale_gate && !gate_is_current {
            if !denied {
                self.resolve_auth_notification(
                    &state.scope,
                    &event.scope.resource.user_id,
                    run_id,
                    &gate_resolution_ref,
                )
                .await?;
            }
            return Ok(run_id);
        }
        let actor = state
            .actor
            .ok_or(ProductSurfaceFailure::AuthContinuationRejected {
                kind: AuthContinuationRejectionKind::UnauthorizedBlockedGate,
            })?;
        let mut binding_id =
            auth_continuation_binding_id(event.flow_id, &run_id, gate_ref.as_str());
        if let Some(disposition) = &resume_disposition {
            binding_id.push_str(&binding_ref_segment("disposition", disposition.as_str()));
        }
        let idempotency_key = idempotency_key_for_binding(&binding_id)?;
        self.turn_coordinator
            .resume_turn(ResumeTurnRequest {
                scope,
                actor,
                run_id,
                gate_resolution_ref: gate_resolution_ref.clone(),
                idempotency_key,
                precondition: ResumeTurnPrecondition::BlockedAuthGate,
                resume_disposition,
            })
            .await
            .map_err(map_auth_resume_error)?;

        if !denied && self.notification_inbox.is_some() {
            // `resume_turn` may return a cached success before checking the
            // current process cursor. Read the committed state back so a
            // replay cannot settle a newly-current instance of the same gate.
            let committed_state = self
                .turn_coordinator
                .get_run_state(GetRunStateRequest {
                    scope: state.scope.clone(),
                    run_id,
                })
                .await
                .map_err(map_auth_resume_error)?;
            let gate_is_still_current = committed_state.status == TurnStatus::BlockedAuth
                && committed_state.gate_ref.as_ref() == Some(&gate_resolution_ref);
            if !gate_is_still_current {
                self.resolve_auth_notification(
                    &committed_state.scope,
                    &event.scope.resource.user_id,
                    run_id,
                    &gate_resolution_ref,
                )
                .await?;
            }
        }

        Ok(run_id)
    }

    async fn resolve_auth_notification(
        &self,
        scope: &TurnScope,
        fallback_user_id: &ironclaw_host_api::ids::UserId,
        run_id: TurnRunId,
        gate_ref: &TurnGateRef,
    ) -> Result<(), ProductSurfaceFailure> {
        let Some(inbox) = self.notification_inbox.as_ref() else {
            return Ok(());
        };
        resolve_auth_notification(inbox.as_ref(), scope, fallback_user_id, run_id, gate_ref).await
    }
}

pub(crate) async fn resolve_auth_notification(
    inbox: &dyn NotificationInboxStorePort,
    scope: &TurnScope,
    fallback_user_id: &ironclaw_host_api::ids::UserId,
    run_id: TurnRunId,
    gate_ref: &TurnGateRef,
) -> Result<(), ProductSurfaceFailure> {
    let notification_id = crate::run_delivery::run_notification_inbox_id(
        run_id,
        NotificationKind::AuthenticationRequired,
        Some(gate_ref.as_str()),
    )
    .map_err(|error| ProductSurfaceFailure::Transient {
        reason: format!("build auth Inbox notification id failed: {error}"),
    })?;
    let owner_user_id = scope
        .explicit_owner_user_id()
        .cloned()
        .unwrap_or_else(|| fallback_user_id.clone());
    match inbox
        .resolve(NotificationMutationRequest {
            recipient: NotificationRecipient {
                tenant_id: scope.tenant_id.clone(),
                user_id: owner_user_id,
            },
            notification_id,
            occurred_at: chrono::Utc::now(),
        })
        .await
    {
        Ok(_) | Err(NotificationInboxError::NotificationNotFound) => Ok(()),
        Err(error) => Err(ProductSurfaceFailure::Transient {
            reason: format!("resolve auth Inbox notification failed: {error}"),
        }),
    }
}

#[async_trait]
impl ProductAuthContinuationDispatcher for ProductAuthTurnGateResumeDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        ProductAuthTurnGateResumeDispatcher::dispatch_auth_continuation(self, event).await
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        ProductAuthTurnGateResumeDispatcher::dispatch_canceled_auth_continuation(self, event).await
    }
}

fn continuation_kind(continuation: &AuthContinuationRef) -> &'static str {
    match continuation {
        AuthContinuationRef::SetupOnly => "setup_only",
        AuthContinuationRef::LifecycleActivation { .. } => "lifecycle_activation",
        AuthContinuationRef::ProductActionResume { .. } => "product_action_resume",
        AuthContinuationRef::TurnGateResume { .. } => "turn_gate_resume",
    }
}

fn auth_error_for_continuation_dispatch(error: &ProductSurfaceFailure) -> AuthProductError {
    match error {
        ProductSurfaceFailure::TurnSubmissionFailed { error }
        | ProductSurfaceFailure::TurnResumeDenied { error }
            if error.category() == TurnErrorCategory::Unavailable =>
        {
            AuthProductError::BackendUnavailable
        }
        ProductSurfaceFailure::TurnResumeDenied { error }
            if error.category() == TurnErrorCategory::Conflict =>
        {
            AuthProductError::BackendUnavailable
        }
        ProductSurfaceFailure::TurnSubmissionFailed { error }
        | ProductSurfaceFailure::TurnResumeDenied { error }
            if error.category() == TurnErrorCategory::Unauthorized =>
        {
            AuthProductError::CrossScopeDenied
        }
        ProductSurfaceFailure::TurnSubmissionFailed { error }
        | ProductSurfaceFailure::TurnResumeDenied { error }
            if error.category() == TurnErrorCategory::ScopeNotFound =>
        {
            AuthProductError::UnknownOrExpiredFlow
        }
        ProductSurfaceFailure::TurnSubmissionFailed { .. } => AuthProductError::InvalidRequest {
            reason: "auth continuation turn resume failed".to_string(),
        },
        ProductSurfaceFailure::Transient { .. } => AuthProductError::BackendUnavailable,
        ProductSurfaceFailure::TurnResumeDenied { .. } => AuthProductError::InvalidRequest {
            reason: "auth continuation turn resume denied".to_string(),
        },
        ProductSurfaceFailure::AuthContinuationRejected { kind } => {
            AuthProductError::InvalidRequest {
                reason: kind.sanitized_reason().to_string(),
            }
        }
        ProductSurfaceFailure::TurnResumeRejected { .. }
        | ProductSurfaceFailure::TurnSubmissionRejected { .. } => {
            AuthProductError::InvalidRequest {
                reason: "auth continuation rejected".to_string(),
            }
        }
        _ => AuthProductError::InvalidRequest {
            reason: "auth continuation dispatch failed".to_string(),
        },
    }
}

fn surface_error_kind(error: &ProductSurfaceFailure) -> &'static str {
    match error {
        ProductSurfaceFailure::TurnSubmissionRejected { .. } => "turn_submission_rejected",
        ProductSurfaceFailure::TurnSubmissionFailed { error } => match error.category() {
            TurnErrorCategory::ThreadBusy => "turn_thread_busy",
            TurnErrorCategory::AdmissionRejected => "turn_admission_rejected",
            TurnErrorCategory::CapacityExceeded => "turn_capacity_exceeded",
            TurnErrorCategory::ScopeNotFound => "turn_scope_not_found",
            TurnErrorCategory::Unauthorized => "turn_unauthorized",
            TurnErrorCategory::InvalidRequest => "turn_invalid_request",
            TurnErrorCategory::Unavailable => "turn_unavailable",
            TurnErrorCategory::Conflict => "turn_conflict",
        },
        ProductSurfaceFailure::TurnResumeRejected { .. } => "turn_resume_rejected",
        ProductSurfaceFailure::AuthContinuationRejected { kind } => match kind {
            AuthContinuationRejectionKind::NotTurnGateResume => {
                "auth_continuation_not_turn_gate_resume"
            }
            AuthContinuationRejectionKind::MissingThreadScope => {
                "auth_continuation_missing_thread_scope"
            }
            AuthContinuationRejectionKind::InvalidTurnRunRef => {
                "auth_continuation_invalid_turn_run_ref"
            }
            AuthContinuationRejectionKind::InvalidGateRef => "auth_continuation_invalid_gate_ref",
            AuthContinuationRejectionKind::InvalidIdempotencyKey => {
                "auth_continuation_invalid_idempotency_key"
            }
            AuthContinuationRejectionKind::InvalidBindingRef => {
                "auth_continuation_invalid_binding_ref"
            }
            AuthContinuationRejectionKind::UnauthorizedBlockedGate => {
                "auth_continuation_unauthorized_blocked_gate"
            }
        },
        ProductSurfaceFailure::TurnResumeDenied { error } => match error.category() {
            TurnErrorCategory::ThreadBusy => "turn_resume_thread_busy",
            TurnErrorCategory::AdmissionRejected => "turn_resume_admission_rejected",
            TurnErrorCategory::CapacityExceeded => "turn_resume_capacity_exceeded",
            TurnErrorCategory::ScopeNotFound => "turn_resume_scope_not_found",
            TurnErrorCategory::Unauthorized => "turn_resume_unauthorized",
            TurnErrorCategory::InvalidRequest => "turn_resume_invalid_request",
            TurnErrorCategory::Unavailable => "turn_resume_unavailable",
            TurnErrorCategory::Conflict => "turn_resume_conflict",
        },
        ProductSurfaceFailure::Transient { .. } => "transient",
        _ => "surface_error",
    }
}

fn map_auth_resume_error(error: TurnError) -> ProductSurfaceFailure {
    match error {
        TurnError::InvalidTransition { .. } | TurnError::InvalidRequest { .. } => {
            ProductSurfaceFailure::AuthContinuationRejected {
                kind: AuthContinuationRejectionKind::UnauthorizedBlockedGate,
            }
        }
        TurnError::Unauthorized | TurnError::ScopeNotFound | TurnError::LeaseMismatch => {
            ProductSurfaceFailure::TurnResumeDenied { error }
        }
        error => ProductSurfaceFailure::TurnSubmissionFailed { error },
    }
}

fn auth_continuation_binding_id(
    flow_id: ironclaw_auth::AuthFlowId,
    run_id: &TurnRunId,
    gate_ref: &str,
) -> String {
    format!(
        "{}{}{}{}",
        binding_ref_segment("surface", "auth-continuation"),
        binding_ref_segment("flow", &flow_id.to_string()),
        binding_ref_segment("run", &run_id.to_string()),
        binding_ref_segment("gate", gate_ref)
    )
}

fn turn_scope_from_auth_event(
    event: &AuthContinuationEvent,
) -> Result<TurnScope, ProductSurfaceFailure> {
    let Some(thread_id) = event.scope.resource.thread_id.clone() else {
        return Err(ProductSurfaceFailure::AuthContinuationRejected {
            kind: AuthContinuationRejectionKind::MissingThreadScope,
        });
    };
    Ok(TurnScope::new_with_owner(
        event.scope.resource.tenant_id.clone(),
        event.scope.resource.agent_id.clone(),
        event.scope.resource.project_id.clone(),
        thread_id,
        Some(event.scope.resource.user_id.clone()),
    ))
}

fn parse_turn_run_id(value: &str) -> Result<TurnRunId, ProductSurfaceFailure> {
    Uuid::parse_str(value)
        .map(TurnRunId::from_uuid)
        .map_err(|_| ProductSurfaceFailure::AuthContinuationRejected {
            kind: AuthContinuationRejectionKind::InvalidTurnRunRef,
        })
}

fn parse_gate_ref(value: &str) -> Result<TurnGateRef, ProductSurfaceFailure> {
    TurnGateRef::new(value.to_string()).map_err(|_| {
        ProductSurfaceFailure::AuthContinuationRejected {
            kind: AuthContinuationRejectionKind::InvalidGateRef,
        }
    })
}

fn idempotency_key_for_binding(binding_id: &str) -> Result<IdempotencyKey, ProductSurfaceFailure> {
    bounded_idempotency_key(
        "auth-continuation",
        binding_id,
        AUTH_CONTINUATION_BINDING_REF_RAW_MAX_BYTES,
    )
    .map_err(|_| ProductSurfaceFailure::AuthContinuationRejected {
        kind: AuthContinuationRejectionKind::InvalidIdempotencyKey,
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_auth::{
        AuthContinuationEvent, AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthGateRef,
        AuthProductError, AuthProductScope, AuthProviderId, AuthSessionId, AuthSurface,
        LifecyclePackageRef, TurnRunRef,
    };
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        ids::{AgentId, InvocationId, ProcessId, ProjectId, TenantId, ThreadId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
        resource::ResourceScope,
        turn::TurnGateRef,
    };
    use ironclaw_notifications::{
        LifecycleRef, ListNotificationsRequest, NotificationAction, NotificationInboxStore,
        NotificationInboxStorePort, NotificationInitialState, NotificationKind,
        NotificationMutationRequest, NotificationRecipient, NotificationSeverity,
        NotificationSource, PublishNotificationRequest,
    };
    use ironclaw_processes::{
        ClaimProcessesRequest, ProcessCheckpointRef, ProcessKind, ProcessSuspension,
        ProcessSuspensionKind, ProcessWorkerId, SuspendProcessRequest,
    };
    use ironclaw_turns::test_support::in_memory_agent_turn_process_system;
    use ironclaw_turns::{
        AcceptedMessageRef, CancelRunRequest, CancelRunResponse, DefaultTurnCoordinator,
        EventCursor, GetRunStateRequest, IdempotencyKey, ResumeTurnRequest, ResumeTurnResponse,
        RunProfileId, RunProfileRequest, RunProfileVersion, SubmitTurnRequest, SubmitTurnResponse,
        TurnActor, TurnCoordinator, TurnError, TurnId, TurnRunId, TurnRunState, TurnScope,
        TurnStatus,
    };

    use super::*;

    mod notification_lifecycle;

    struct RecordingTurnCoordinator {
        resumes: Mutex<Vec<ResumeTurnRequest>>,
        state: Mutex<Option<TurnRunState>>,
        resume_error: Mutex<Option<TurnError>>,
        resume_cache: Mutex<HashMap<String, ResumeTurnResponse>>,
    }

    impl Default for RecordingTurnCoordinator {
        fn default() -> Self {
            Self {
                resumes: Mutex::new(Vec::new()),
                state: Mutex::new(None),
                resume_error: Mutex::new(None),
                resume_cache: Mutex::new(HashMap::new()),
            }
        }
    }

    impl RecordingTurnCoordinator {
        fn resumes(&self) -> Vec<ResumeTurnRequest> {
            self.resumes.lock().expect("resume lock").clone()
        }

        fn set_state(&self, state: TurnRunState) {
            *self.state.lock().expect("state lock") = Some(state);
        }

        fn fail_resume_with(&self, error: TurnError) {
            *self.resume_error.lock().expect("resume error lock") = Some(error);
        }
    }

    fn notification_inbox() -> Arc<NotificationInboxStore<InMemoryBackend>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/notifications").expect("notification mount alias"),
            VirtualPath::new("/engine/test/auth-continuation-notifications")
                .expect("notification mount target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("notification mount view");
        Arc::new(NotificationInboxStore::new(
            Arc::new(ScopedFilesystem::with_fixed_view(
                Arc::new(InMemoryBackend::new()),
                mounts,
            )),
            ironclaw_notifications::NOTIFICATION_INBOX_MAX_RECORDS,
        ))
    }

    #[async_trait]
    impl TurnCoordinator for RecordingTurnCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            Ok(TurnRunId::new())
        }

        async fn submit_turn(
            &self,
            _request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            panic!("submit_turn is not used by auth continuation tests");
        }

        async fn resume_turn(
            &self,
            request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            if let Some(cached) = self
                .resume_cache
                .lock()
                .expect("resume cache lock")
                .get(request.idempotency_key.as_str())
                .cloned()
            {
                self.resumes.lock().expect("resume lock").push(request);
                return Ok(cached);
            }
            let state = self
                .state
                .lock()
                .expect("state lock")
                .clone()
                .ok_or(TurnError::ScopeNotFound)?;
            if state.scope != request.scope {
                return Err(TurnError::ScopeNotFound);
            }
            if state.actor.as_ref() != Some(&request.actor) {
                return Err(TurnError::Unauthorized);
            }
            if let Some(required) = request.precondition.required_status()
                && state.status != required
            {
                return Err(TurnError::InvalidTransition {
                    from: state.status,
                    to: TurnStatus::Queued,
                });
            }
            if !matches!(
                state.status,
                TurnStatus::BlockedApproval | TurnStatus::BlockedAuth | TurnStatus::BlockedResource
            ) {
                return Err(TurnError::InvalidTransition {
                    from: state.status,
                    to: TurnStatus::Queued,
                });
            }
            if state.gate_ref.as_ref() != Some(&request.gate_resolution_ref) {
                return Err(TurnError::InvalidRequest {
                    reason: "gate resolution reference mismatch".to_string(),
                });
            }
            if let Some(error) = self.resume_error.lock().expect("resume error lock").take() {
                return Err(error);
            }
            let run_id = request.run_id;
            let cache_key = request.idempotency_key.as_str().to_string();
            self.resumes.lock().expect("resume lock").push(request);
            let response = ResumeTurnResponse {
                run_id,
                status: TurnStatus::Running,
                event_cursor: EventCursor::default(),
            };
            self.resume_cache
                .lock()
                .expect("resume cache lock")
                .insert(cache_key, response.clone());
            let mut state = self.state.lock().expect("state lock");
            let state = state.as_mut().ok_or(TurnError::ScopeNotFound)?;
            state.status = TurnStatus::Running;
            state.gate_ref = None;
            Ok(response)
        }

        async fn retry_turn(
            &self,
            _request: ironclaw_turns::RetryTurnRequest,
        ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
            panic!("retry_turn is not used by auth continuation tests");
        }

        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            panic!("cancel_run is not used by auth continuation tests");
        }

        async fn get_run_state(
            &self,
            request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            let state = self
                .state
                .lock()
                .expect("state lock")
                .clone()
                .ok_or(TurnError::ScopeNotFound)?;
            if state.scope != request.scope || state.run_id != request.run_id {
                return Err(TurnError::ScopeNotFound);
            }
            Ok(state)
        }
    }

    fn scoped_event(continuation: AuthContinuationRef) -> AuthContinuationEvent {
        scoped_event_for_owner("alice", continuation)
    }

    fn scoped_event_for_owner(
        owner_user_id: &str,
        continuation: AuthContinuationRef,
    ) -> AuthContinuationEvent {
        let thread_id = ThreadId::new("thread-auth").unwrap();
        let resource = ResourceScope {
            tenant_id: TenantId::new("tenant-auth").unwrap(),
            user_id: UserId::new(owner_user_id).unwrap(),
            agent_id: Some(AgentId::new("agent-auth").unwrap()),
            project_id: Some(ProjectId::new("project-auth").unwrap()),
            mission_id: None,
            thread_id: Some(thread_id),
            invocation_id: InvocationId::new(),
        };
        AuthContinuationEvent {
            flow_id: AuthFlowId::new(),
            scope: AuthProductScope::new(resource, AuthSurface::Callback)
                .with_session_id(AuthSessionId::new("session-auth").unwrap()),
            continuation,
            provider: AuthProviderId::new("google").unwrap(),
            credential_account_id: None,
            emitted_at: Utc::now(),
        }
    }

    fn run_state(run_id: TurnRunId, status: TurnStatus, gate_ref: Option<&str>) -> TurnRunState {
        run_state_for_actor_owner(run_id, status, gate_ref, "alice", "alice")
    }

    fn run_state_for_actor_owner(
        run_id: TurnRunId,
        status: TurnStatus,
        gate_ref: Option<&str>,
        actor_user_id: &str,
        owner_user_id: &str,
    ) -> TurnRunState {
        TurnRunState {
            scope: TurnScope::new_with_owner(
                TenantId::new("tenant-auth").unwrap(),
                Some(AgentId::new("agent-auth").unwrap()),
                Some(ProjectId::new("project-auth").unwrap()),
                ThreadId::new("thread-auth").unwrap(),
                Some(UserId::new(owner_user_id).unwrap()),
            ),
            actor: Some(TurnActor::new(UserId::new(actor_user_id).unwrap())),
            turn_id: TurnId::new(),
            run_id,
            status,
            accepted_message_ref: AcceptedMessageRef::new("message-auth").unwrap(),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            output_contract: Default::default(),
            allow_steering: true,
            resolved_model_route: None,
            model_usage: None,
            execution_outcome: None,
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref: gate_ref.map(|value| TurnGateRef::new(value).unwrap()),
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor::default(),
            product_context: None,
            resume_disposition: None,
        }
    }

    #[tokio::test]
    async fn turn_gate_continuation_resumes_through_turn_coordinator() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone());
        let run_id = TurnRunId::new();
        coordinator.set_state(run_state(
            run_id,
            TurnStatus::BlockedAuth,
            Some("gate:auth"),
        ));
        let event = scoped_event(AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new("gate:auth").unwrap(),
        });

        let resumed_run_id = dispatcher
            .dispatch_turn_gate_resume(event)
            .await
            .expect("dispatch");

        assert_eq!(resumed_run_id, run_id);
        let resumes = coordinator.resumes();
        assert_eq!(resumes.len(), 1);
        assert_eq!(resumes[0].run_id, run_id);
        assert_eq!(resumes[0].gate_resolution_ref.as_str(), "gate:auth");
        assert_eq!(
            resumes[0].precondition,
            ResumeTurnPrecondition::BlockedAuthGate
        );
        assert_eq!(resumes[0].actor.user_id.as_str(), "alice");
        assert_eq!(resumes[0].scope.thread_id.as_str(), "thread-auth");
        assert_eq!(
            resumes[0]
                .scope
                .explicit_owner_user_id()
                .map(UserId::as_str),
            Some("alice")
        );
        assert!(
            resumes[0]
                .idempotency_key
                .as_str()
                .starts_with("auth-continuation:")
        );
        assert!(resumes[0].idempotency_key.as_str().contains("surface:"));
        assert!(resumes[0].idempotency_key.as_str().contains("flow:"));
        assert!(resumes[0].idempotency_key.as_str().contains("run:"));
        assert!(resumes[0].idempotency_key.as_str().contains("gate:"));
    }

    #[tokio::test]
    async fn canceled_turn_gate_continuation_denies_exact_blocked_auth_gate() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbox = notification_inbox();
        let run_id = TurnRunId::new();
        let gate_ref = TurnGateRef::new("gate:auth").expect("gate ref");
        coordinator.set_state(run_state_for_actor_owner(
            run_id,
            TurnStatus::BlockedAuth,
            Some(gate_ref.as_str()),
            "authenticated-actor",
            "alice",
        ));
        let notification_id = crate::run_delivery::run_notification_inbox_id(
            run_id,
            NotificationKind::AuthenticationRequired,
            Some(gate_ref.as_str()),
        )
        .expect("notification id");
        inbox
            .publish(PublishNotificationRequest {
                id: notification_id,
                recipient: NotificationRecipient {
                    tenant_id: TenantId::new("tenant-auth").expect("tenant"),
                    user_id: UserId::new("alice").expect("user"),
                },
                kind: NotificationKind::AuthenticationRequired,
                severity: NotificationSeverity::Warning,
                source: NotificationSource {
                    thread_id: ThreadId::new("thread-auth").expect("thread"),
                    turn_run_id: Some(run_id),
                    lifecycle_ref: Some(
                        LifecycleRef::new(gate_ref.as_str()).expect("lifecycle ref"),
                    ),
                    credential_providers: Vec::new(),
                },
                action: NotificationAction::OpenThread {
                    thread_id: ThreadId::new("thread-auth").expect("thread"),
                },
                initial_state: NotificationInitialState::Open,
                occurred_at: Utc::now(),
            })
            .await
            .expect("seed auth notification");
        let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone())
            .with_notification_inbox(Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>);
        let event = scoped_event(AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new(gate_ref.as_str()).unwrap(),
        });

        dispatcher
            .dispatch_canceled_auth_continuation(event)
            .await
            .expect("canceled auth gate denial");

        let resumes = coordinator.resumes();
        assert_eq!(resumes.len(), 1);
        assert_eq!(resumes[0].actor.user_id.as_str(), "authenticated-actor");
        assert_eq!(
            resumes[0].precondition,
            ResumeTurnPrecondition::BlockedAuthGate
        );
        assert_eq!(
            resumes[0].resume_disposition,
            Some(GateResumeDisposition::Denied)
        );
        assert!(
            resumes[0]
                .idempotency_key
                .as_str()
                .contains(&binding_ref_segment(
                    "disposition",
                    GateResumeDisposition::Denied.as_str()
                ))
        );
        let page = inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: TenantId::new("tenant-auth").expect("tenant"),
                    user_id: UserId::new("alice").expect("user"),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list denied auth notification");
        assert!(
            page.notifications[0].resolved_at.is_none(),
            "a canceled auth continuation is a denial, not verified recovery"
        );
    }

    #[tokio::test]
    async fn canceled_continuation_leaves_stale_or_non_turn_gate_untouched() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let inbox = notification_inbox();
        let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone())
            .with_notification_inbox(Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>);
        let run_id = TurnRunId::new();
        let stale_gate_ref = TurnGateRef::new("gate:stale-auth").expect("stale gate ref");
        coordinator.set_state(run_state(
            run_id,
            TurnStatus::BlockedAuth,
            Some("gate:new-auth"),
        ));
        inbox
            .publish(PublishNotificationRequest {
                id: crate::run_delivery::run_notification_inbox_id(
                    run_id,
                    NotificationKind::AuthenticationRequired,
                    Some(stale_gate_ref.as_str()),
                )
                .expect("notification id"),
                recipient: NotificationRecipient {
                    tenant_id: TenantId::new("tenant-auth").expect("tenant"),
                    user_id: UserId::new("alice").expect("user"),
                },
                kind: NotificationKind::AuthenticationRequired,
                severity: NotificationSeverity::Warning,
                source: NotificationSource {
                    thread_id: ThreadId::new("thread-auth").expect("thread"),
                    turn_run_id: Some(run_id),
                    lifecycle_ref: Some(
                        LifecycleRef::new(stale_gate_ref.as_str()).expect("lifecycle ref"),
                    ),
                    credential_providers: Vec::new(),
                },
                action: NotificationAction::OpenThread {
                    thread_id: ThreadId::new("thread-auth").expect("thread"),
                },
                initial_state: NotificationInitialState::Open,
                occurred_at: Utc::now(),
            })
            .await
            .expect("seed stale auth notification");
        let stale = scoped_event(AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new(stale_gate_ref.as_str()).unwrap(),
        });

        dispatcher
            .dispatch_canceled_auth_continuation(stale)
            .await
            .expect("a stale cancellation is already converged");
        dispatcher
            .dispatch_canceled_auth_continuation(scoped_event(AuthContinuationRef::SetupOnly))
            .await
            .expect("setup-only cancellation has no turn side effect");

        assert!(coordinator.resumes().is_empty());
        let page = inbox
            .list(ListNotificationsRequest {
                recipient: NotificationRecipient {
                    tenant_id: TenantId::new("tenant-auth").expect("tenant"),
                    user_id: UserId::new("alice").expect("user"),
                },
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list stale denied notification");
        assert!(
            page.notifications[0].resolved_at.is_none(),
            "a stale denial replay cannot prove credential recovery"
        );
    }

    /// A replayed RESUME continuation whose gate already settled converges as
    /// a no-op instead of erroring forever. Completed continuations replay
    /// whenever the durable `continuation_emitted_at` fence was not stamped —
    /// e.g. the blocked-run fan-out sweep was incomplete and the whole
    /// dispatch stayed retryable; by then the primary run has typically
    /// resumed (or re-blocked on a NEW gate), which is the settled outcome the
    /// continuation wanted.
    #[tokio::test]
    async fn resume_continuation_leaves_settled_gate_untouched() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone());
        let run_id = TurnRunId::new();
        // Re-blocked on a NEW gate: the replayed resume for the old gate must
        // not touch it.
        coordinator.set_state(run_state(
            run_id,
            TurnStatus::BlockedAuth,
            Some("gate:new-auth"),
        ));
        dispatcher
            .dispatch_auth_continuation(scoped_event(AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
                gate_ref: AuthGateRef::new("gate:stale-auth").unwrap(),
            }))
            .await
            .expect("a replayed resume for a superseded gate converges");

        // Already resumed (no longer blocked at all): same convergence.
        coordinator.set_state(run_state(run_id, TurnStatus::Queued, None));
        dispatcher
            .dispatch_auth_continuation(scoped_event(AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
                gate_ref: AuthGateRef::new("gate:stale-auth").unwrap(),
            }))
            .await
            .expect("a replayed resume for an already-resumed run converges");

        assert!(coordinator.resumes().is_empty());
    }

    #[tokio::test]
    async fn turn_gate_continuation_uses_subject_scope_and_original_actor() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone());
        let run_id = TurnRunId::new();
        coordinator.set_state(run_state_for_actor_owner(
            run_id,
            TurnStatus::BlockedAuth,
            Some("gate:auth"),
            "alice",
            "team-agent",
        ));
        let event = scoped_event_for_owner(
            "team-agent",
            AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
                gate_ref: AuthGateRef::new("gate:auth").unwrap(),
            },
        );

        let resumed_run_id = dispatcher
            .dispatch_turn_gate_resume(event)
            .await
            .expect("dispatch");

        assert_eq!(resumed_run_id, run_id);
        let resumes = coordinator.resumes();
        assert_eq!(resumes.len(), 1);
        assert_eq!(resumes[0].actor.user_id.as_str(), "alice");
        assert_eq!(
            resumes[0]
                .scope
                .explicit_owner_user_id()
                .map(UserId::as_str),
            Some("team-agent")
        );
    }

    #[tokio::test]
    async fn turn_gate_continuation_rejects_non_auth_gate() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone());
        let run_id = TurnRunId::new();
        coordinator.set_state(run_state(
            run_id,
            TurnStatus::BlockedApproval,
            Some("gate:auth"),
        ));
        let event = scoped_event(AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new("gate:auth").unwrap(),
        });

        // The safety property is side-effect freedom: an auth continuation
        // must never resolve a non-auth gate. It converges as a settled no-op
        // (replay-tolerant) instead of erroring forever — the run left
        // BlockedAuth, so this continuation's business is done.
        dispatcher
            .dispatch_turn_gate_resume(event)
            .await
            .expect("a non-auth-blocked run converges without a resume");

        assert!(coordinator.resumes().is_empty());
    }

    #[tokio::test]
    async fn turn_gate_continuation_rejects_mismatched_auth_gate_ref() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone());
        let run_id = TurnRunId::new();
        coordinator.set_state(run_state(
            run_id,
            TurnStatus::BlockedAuth,
            Some("gate:other-auth"),
        ));
        let event = scoped_event(AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new("gate:auth").unwrap(),
        });

        // The safety property is side-effect freedom: a stale continuation
        // must never resolve a DIFFERENT gate. It converges as a settled
        // no-op (replay-tolerant) instead of erroring forever — the gate it
        // was minted for is gone.
        dispatcher
            .dispatch_turn_gate_resume(event)
            .await
            .expect("a superseded gate converges without a resume");

        assert!(coordinator.resumes().is_empty());
    }

    #[tokio::test]
    async fn turn_gate_continuation_rejects_cross_scope_resume_through_real_coordinator() {
        let process_system = in_memory_agent_turn_process_system();
        let store = Arc::new(process_system.runtime());
        let transitions = process_system.transitions();
        let coordinator = Arc::new(DefaultTurnCoordinator::new(store.clone()));
        let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone());
        let scope = TurnScope::new(
            TenantId::new("tenant-auth").unwrap(),
            Some(AgentId::new("agent-auth").unwrap()),
            Some(ProjectId::new("project-auth").unwrap()),
            ThreadId::new("thread-auth").unwrap(),
        );
        let actor = TurnActor::new(UserId::new("alice").unwrap());
        let submit = coordinator
            .submit_turn(SubmitTurnRequest {
                subagent_activation_provenance: None,
                scope: scope.clone(),
                requested_model: None,
                actor: actor.clone(),
                accepted_message_ref: AcceptedMessageRef::new("message-auth-real").unwrap(),
                requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
                output_contract: None,
                idempotency_key: IdempotencyKey::new("idem-auth-real-submit").unwrap(),
                received_at: Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: None,
            })
            .await
            .expect("submit turn");
        let SubmitTurnResponse::Accepted { run_id, .. } = submit;
        let worker_id = ProcessWorkerId::from_trusted(
            ironclaw_turns::TurnRunnerId::new().as_uuid().to_string(),
        );
        let claimed = transitions
            .claim_next_processes(ClaimProcessesRequest {
                worker_id,
                scope_filter: None,
                process_id_filter: None,
                process_kind_filter: Some(ProcessKind::AgentTurn),
                max_processes: 1,
            })
            .await
            .expect("claim run")
            .into_iter()
            .next()
            .expect("queued run exists");
        assert_eq!(
            claimed.state.process_id,
            ProcessId::from_uuid(run_id.as_uuid())
        );
        transitions
            .suspend_process(SuspendProcessRequest {
                process_id: claimed.state.process_id,
                worker_id: claimed.worker_id,
                lease_token: claimed.lease_token,
                checkpoint_ref: ProcessCheckpointRef::new("checkpoint:auth-real").unwrap(),
                suspension: ProcessSuspension {
                    kind: ProcessSuspensionKind::Authorization,
                    gate_ref: Some(TurnGateRef::new("gate:auth-real").unwrap()),
                    activity_id: None,
                    credential_requirements: Vec::new(),
                    detail: None,
                },
                metadata: None,
            })
            .await
            .expect("block auth gate");
        let mut event = scoped_event(AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new("gate:auth-real").unwrap(),
        });
        event.scope.resource.tenant_id = TenantId::new("tenant-other").unwrap();

        let err = dispatcher
            .dispatch_turn_gate_resume(event)
            .await
            .expect_err("cross-scope continuation must not resume");

        assert!(matches!(
            err,
            ProductSurfaceFailure::TurnResumeDenied { .. }
        ));
    }

    #[tokio::test]
    async fn turn_gate_continuation_maps_resume_failure_to_turn_submission_failed() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone());
        let run_id = TurnRunId::new();
        coordinator.set_state(run_state(
            run_id,
            TurnStatus::BlockedAuth,
            Some("gate:auth"),
        ));
        coordinator.fail_resume_with(TurnError::Unavailable {
            reason: "coordinator offline".to_string(),
        });
        let event = scoped_event(AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new("gate:auth").unwrap(),
        });

        let err = dispatcher
            .dispatch_turn_gate_resume(event)
            .await
            .expect_err("resume failure should be preserved");

        assert!(matches!(
            err,
            ProductSurfaceFailure::TurnSubmissionFailed { .. }
        ));
    }

    #[test]
    fn auth_error_for_continuation_dispatch_preserves_retryable_resume_denials() {
        for error in [
            TurnError::Unavailable {
                reason: "turn coordinator offline".to_string(),
            },
            TurnError::LeaseMismatch,
        ] {
            let auth_error =
                auth_error_for_continuation_dispatch(&ProductSurfaceFailure::TurnResumeDenied {
                    error,
                });

            assert_eq!(auth_error.code(), AuthErrorCode::BackendUnavailable);
        }
    }

    #[test]
    fn auth_error_for_continuation_dispatch_maps_transient_and_catch_all() {
        let transient = auth_error_for_continuation_dispatch(&ProductSurfaceFailure::Transient {
            reason: "store timeout".to_string(),
        });
        assert_eq!(transient.code(), AuthErrorCode::BackendUnavailable);

        let catch_all =
            auth_error_for_continuation_dispatch(&ProductSurfaceFailure::UnknownInstallation);
        assert_eq!(catch_all.code(), AuthErrorCode::InvalidRequest);
        assert!(matches!(
            catch_all,
            AuthProductError::InvalidRequest { reason }
                if reason == "auth continuation dispatch failed"
        ));
    }

    #[test]
    fn auth_continuation_rejection_kind_returns_stable_static_strings() {
        for (kind, expected) in [
            (
                AuthContinuationRejectionKind::NotTurnGateResume,
                "auth continuation is not a turn-gate resume",
            ),
            (
                AuthContinuationRejectionKind::MissingThreadScope,
                "invalid auth continuation scope",
            ),
            (
                AuthContinuationRejectionKind::InvalidTurnRunRef,
                "invalid auth continuation run reference",
            ),
            (
                AuthContinuationRejectionKind::InvalidGateRef,
                "invalid auth continuation gate reference",
            ),
            (
                AuthContinuationRejectionKind::InvalidIdempotencyKey,
                "invalid auth continuation idempotency key",
            ),
            (
                AuthContinuationRejectionKind::InvalidBindingRef,
                "invalid auth continuation binding ref",
            ),
            (
                AuthContinuationRejectionKind::UnauthorizedBlockedGate,
                "auth continuation does not match an authorized blocked auth gate",
            ),
        ] {
            let auth_error = auth_error_for_continuation_dispatch(
                &ProductSurfaceFailure::AuthContinuationRejected { kind },
            );

            assert!(matches!(
                auth_error,
                AuthProductError::InvalidRequest { reason } if reason == expected
            ));
        }
    }

    #[tokio::test]
    async fn turn_gate_continuation_rejects_invalid_turn_run_ref() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone());
        let event = scoped_event(AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new("not-a-uuid").unwrap(),
            gate_ref: AuthGateRef::new("gate:auth").unwrap(),
        });

        let err = dispatcher
            .dispatch_turn_gate_resume(event)
            .await
            .expect_err("invalid run ref should reject before resume");

        assert!(matches!(
            err,
            ProductSurfaceFailure::AuthContinuationRejected {
                kind: AuthContinuationRejectionKind::InvalidTurnRunRef
            }
        ));
        assert!(coordinator.resumes().is_empty());
    }

    #[tokio::test]
    async fn turn_gate_dispatcher_rejects_non_turn_continuations() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone());
        let event = scoped_event(AuthContinuationRef::LifecycleActivation {
            package_ref: LifecyclePackageRef::new("github").unwrap(),
        });

        let err = dispatcher
            .dispatch_turn_gate_resume(event)
            .await
            .expect_err("non-turn continuations are owned by the caller");

        assert!(matches!(
            err,
            ProductSurfaceFailure::AuthContinuationRejected {
                kind: AuthContinuationRejectionKind::NotTurnGateResume
            }
        ));
        assert!(coordinator.resumes().is_empty());
    }

    #[tokio::test]
    async fn turn_gate_continuation_requires_thread_scope() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator);
        let run_id = TurnRunId::new();
        let mut event = scoped_event(AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new("gate:auth").unwrap(),
        });
        event.scope.resource.thread_id = None;

        let err = dispatcher
            .dispatch_turn_gate_resume(event)
            .await
            .expect_err("thread scope is required");

        assert!(matches!(
            err,
            ProductSurfaceFailure::AuthContinuationRejected {
                kind: AuthContinuationRejectionKind::MissingThreadScope
            }
        ));
    }

    #[tokio::test]
    async fn dispatch_auth_continuation_skips_coordinator_for_non_turn_continuations() {
        use ironclaw_auth::{LifecyclePackageRef, ProductActionRef};

        let non_turn_continuations = [
            AuthContinuationRef::SetupOnly,
            AuthContinuationRef::LifecycleActivation {
                package_ref: LifecyclePackageRef::new("github").unwrap(),
            },
            AuthContinuationRef::ProductActionResume {
                action_ref: ProductActionRef::new("action:install").unwrap(),
            },
        ];

        for continuation in non_turn_continuations {
            let coordinator = Arc::new(RecordingTurnCoordinator::default());
            let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone());
            // No set_state — any get_run_state call would return ScopeNotFound,
            // causing dispatch_auth_continuation to return Err rather than Ok(()).
            let event = scoped_event(continuation);

            let result = dispatcher.dispatch_auth_continuation(event).await;

            assert!(
                result.is_ok(),
                "non-turn continuation should return Ok(()), got: {result:?}"
            );
            assert!(
                coordinator.resumes().is_empty(),
                "non-turn continuation must not call resume_turn on the coordinator"
            );
        }
    }

    #[test]
    fn surface_error_kind_capacity_exceeded_returns_expected_strings() {
        let submission = ProductSurfaceFailure::TurnSubmissionFailed {
            error: TurnError::capacity_exceeded(
                ironclaw_turns::TurnCapacityResource::SpawnTreeDescendants,
                4,
            ),
        };
        assert_eq!(surface_error_kind(&submission), "turn_capacity_exceeded");

        let resume = ProductSurfaceFailure::TurnResumeDenied {
            error: TurnError::capacity_exceeded(
                ironclaw_turns::TurnCapacityResource::SubmitTurn,
                7,
            ),
        };
        assert_eq!(surface_error_kind(&resume), "turn_resume_capacity_exceeded");
    }
}
