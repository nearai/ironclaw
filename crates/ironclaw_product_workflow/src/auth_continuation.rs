//! Product-auth continuation handling.
//!
//! This module consumes the `ironclaw_auth` continuation vocabulary and routes
//! turn-gate resume continuations through the same trusted `TurnCoordinator`
//! boundary as the WebUI gate-resolution path. It intentionally does not define
//! another auth-flow model or handle non-turn continuation variants.

use std::sync::Arc;

use ironclaw_auth::{AuthContinuationEvent, AuthContinuationRef, AuthProductError};
use ironclaw_turns::{
    GateRef, IdempotencyKey, ResumeTurnPrecondition, ResumeTurnRequest, TurnActor, TurnCoordinator,
    TurnError, TurnErrorCategory, TurnRunId, TurnScope,
};
use uuid::Uuid;

use crate::binding_ref::{
    AUTH_CONTINUATION_BINDING_REF_RAW_MAX_BYTES, binding_ref_segment, bounded_idempotency_key,
    bounded_reply_target_binding_ref, bounded_source_binding_ref,
};
use crate::{AuthContinuationRejectionKind, ProductWorkflowError};

#[derive(Clone)]
pub struct ProductAuthTurnGateResumeDispatcher {
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

impl ProductAuthTurnGateResumeDispatcher {
    pub fn new(turn_coordinator: Arc<dyn TurnCoordinator>) -> Self {
        Self { turn_coordinator }
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
                        workflow_error_kind = workflow_error_kind(&error),
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

    pub async fn dispatch_turn_gate_resume(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<TurnRunId, ProductWorkflowError> {
        let AuthContinuationRef::TurnGateResume {
            turn_run_ref,
            gate_ref,
        } = &event.continuation
        else {
            return Err(ProductWorkflowError::AuthContinuationRejected {
                kind: AuthContinuationRejectionKind::NotTurnGateResume,
            });
        };

        let scope = turn_scope_from_auth_event(&event)?;
        let actor = TurnActor::new(event.scope.resource.user_id.clone());
        let run_id = parse_turn_run_id(turn_run_ref.as_str())?;
        let gate_resolution_ref = parse_gate_ref(gate_ref.as_str())?;
        let binding_id = auth_continuation_binding_id(event.flow_id, &run_id, gate_ref.as_str());
        let idempotency_key = idempotency_key_for_binding(&binding_id)?;

        self.turn_coordinator
            .resume_turn(ResumeTurnRequest {
                scope,
                actor,
                run_id,
                gate_resolution_ref,
                source_binding_ref: bounded_source_binding_ref(
                    "auth-continuation-src",
                    &binding_id,
                    AUTH_CONTINUATION_BINDING_REF_RAW_MAX_BYTES,
                )
                .map_err(binding_ref_error)?,
                reply_target_binding_ref: bounded_reply_target_binding_ref(
                    "auth-continuation-reply",
                    &binding_id,
                    AUTH_CONTINUATION_BINDING_REF_RAW_MAX_BYTES,
                )
                .map_err(binding_ref_error)?,
                idempotency_key,
                precondition: ResumeTurnPrecondition::BlockedAuthGate,
            })
            .await
            .map_err(map_auth_resume_error)?;

        Ok(run_id)
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

fn auth_error_for_continuation_dispatch(error: &ProductWorkflowError) -> AuthProductError {
    match error {
        ProductWorkflowError::TurnSubmissionFailed { error }
        | ProductWorkflowError::TurnResumeDenied { error }
            if error.category() == TurnErrorCategory::Unavailable =>
        {
            AuthProductError::BackendUnavailable
        }
        ProductWorkflowError::TurnResumeDenied { error }
            if error.category() == TurnErrorCategory::Conflict =>
        {
            AuthProductError::BackendUnavailable
        }
        ProductWorkflowError::TurnSubmissionFailed { error }
        | ProductWorkflowError::TurnResumeDenied { error }
            if error.category() == TurnErrorCategory::Unauthorized =>
        {
            AuthProductError::CrossScopeDenied
        }
        ProductWorkflowError::TurnSubmissionFailed { error }
        | ProductWorkflowError::TurnResumeDenied { error }
            if error.category() == TurnErrorCategory::ScopeNotFound =>
        {
            AuthProductError::UnknownOrExpiredFlow
        }
        ProductWorkflowError::TurnSubmissionFailed { .. } => AuthProductError::InvalidRequest {
            reason: "auth continuation turn resume failed".to_string(),
        },
        ProductWorkflowError::Transient { .. } => AuthProductError::BackendUnavailable,
        ProductWorkflowError::TurnResumeDenied { .. } => AuthProductError::InvalidRequest {
            reason: "auth continuation turn resume denied".to_string(),
        },
        ProductWorkflowError::AuthContinuationRejected { kind } => {
            AuthProductError::InvalidRequest {
                reason: kind.sanitized_reason().to_string(),
            }
        }
        ProductWorkflowError::TurnResumeRejected { .. }
        | ProductWorkflowError::TurnSubmissionRejected { .. } => AuthProductError::InvalidRequest {
            reason: "auth continuation rejected".to_string(),
        },
        _ => AuthProductError::InvalidRequest {
            reason: "auth continuation dispatch failed".to_string(),
        },
    }
}

fn workflow_error_kind(error: &ProductWorkflowError) -> &'static str {
    match error {
        ProductWorkflowError::TurnSubmissionRejected { .. } => "turn_submission_rejected",
        ProductWorkflowError::TurnSubmissionFailed { error } => match error.category() {
            TurnErrorCategory::ThreadBusy => "turn_thread_busy",
            TurnErrorCategory::AdmissionRejected => "turn_admission_rejected",
            TurnErrorCategory::CapacityExceeded => "turn_capacity_exceeded",
            TurnErrorCategory::ScopeNotFound => "turn_scope_not_found",
            TurnErrorCategory::Unauthorized => "turn_unauthorized",
            TurnErrorCategory::InvalidRequest => "turn_invalid_request",
            TurnErrorCategory::Unavailable => "turn_unavailable",
            TurnErrorCategory::Conflict => "turn_conflict",
        },
        ProductWorkflowError::TurnResumeRejected { .. } => "turn_resume_rejected",
        ProductWorkflowError::AuthContinuationRejected { kind } => match kind {
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
        ProductWorkflowError::TurnResumeDenied { error } => match error.category() {
            TurnErrorCategory::ThreadBusy => "turn_resume_thread_busy",
            TurnErrorCategory::AdmissionRejected => "turn_resume_admission_rejected",
            TurnErrorCategory::CapacityExceeded => "turn_resume_capacity_exceeded",
            TurnErrorCategory::ScopeNotFound => "turn_resume_scope_not_found",
            TurnErrorCategory::Unauthorized => "turn_resume_unauthorized",
            TurnErrorCategory::InvalidRequest => "turn_resume_invalid_request",
            TurnErrorCategory::Unavailable => "turn_resume_unavailable",
            TurnErrorCategory::Conflict => "turn_resume_conflict",
        },
        ProductWorkflowError::Transient { .. } => "transient",
        _ => "workflow_error",
    }
}

fn map_auth_resume_error(error: TurnError) -> ProductWorkflowError {
    match error {
        TurnError::InvalidTransition { .. } | TurnError::InvalidRequest { .. } => {
            ProductWorkflowError::AuthContinuationRejected {
                kind: AuthContinuationRejectionKind::UnauthorizedBlockedGate,
            }
        }
        TurnError::Unauthorized | TurnError::ScopeNotFound | TurnError::LeaseMismatch => {
            ProductWorkflowError::TurnResumeDenied { error }
        }
        error => ProductWorkflowError::TurnSubmissionFailed { error },
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
) -> Result<TurnScope, ProductWorkflowError> {
    let Some(thread_id) = event.scope.resource.thread_id.clone() else {
        return Err(ProductWorkflowError::AuthContinuationRejected {
            kind: AuthContinuationRejectionKind::MissingThreadScope,
        });
    };
    Ok(TurnScope::new(
        event.scope.resource.tenant_id.clone(),
        event.scope.resource.agent_id.clone(),
        event.scope.resource.project_id.clone(),
        thread_id,
    ))
}

fn parse_turn_run_id(value: &str) -> Result<TurnRunId, ProductWorkflowError> {
    Uuid::parse_str(value)
        .map(TurnRunId::from_uuid)
        .map_err(|_| ProductWorkflowError::AuthContinuationRejected {
            kind: AuthContinuationRejectionKind::InvalidTurnRunRef,
        })
}

fn parse_gate_ref(value: &str) -> Result<GateRef, ProductWorkflowError> {
    GateRef::new(value.to_string()).map_err(|_| ProductWorkflowError::AuthContinuationRejected {
        kind: AuthContinuationRejectionKind::InvalidGateRef,
    })
}

fn idempotency_key_for_binding(binding_id: &str) -> Result<IdempotencyKey, ProductWorkflowError> {
    bounded_idempotency_key(
        "auth-continuation",
        binding_id,
        AUTH_CONTINUATION_BINDING_REF_RAW_MAX_BYTES,
    )
    .map_err(|_| ProductWorkflowError::AuthContinuationRejected {
        kind: AuthContinuationRejectionKind::InvalidIdempotencyKey,
    })
}

fn binding_ref_error(_reason: String) -> ProductWorkflowError {
    ProductWorkflowError::AuthContinuationRejected {
        kind: AuthContinuationRejectionKind::InvalidBindingRef,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_auth::{
        AuthContinuationEvent, AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthGateRef,
        AuthProductError, AuthProductScope, AuthSessionId, AuthSurface, LifecyclePackageRef,
        TurnRunRef,
    };
    use ironclaw_host_api::{
        AgentId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
    };
    use ironclaw_turns::{
        AcceptedMessageRef, BlockedReason, CancelRunRequest, CancelRunResponse,
        DefaultTurnCoordinator, EventCursor, GetRunStateRequest, IdempotencyKey,
        InMemoryTurnStateStore, LoopCheckpointStateRef, ReplyTargetBindingRef, ResumeTurnRequest,
        ResumeTurnResponse, RunProfileId, RunProfileRequest, RunProfileVersion, SourceBindingRef,
        SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCheckpointId, TurnCoordinator,
        TurnError, TurnId, TurnLeaseToken, TurnRunId, TurnRunState, TurnRunnerId, TurnScope,
        TurnStatus,
        runner::{BlockRunRequest, ClaimRunRequest, TurnRunTransitionPort},
    };

    use super::*;

    struct RecordingTurnCoordinator {
        resumes: Mutex<Vec<ResumeTurnRequest>>,
        state: Mutex<Option<TurnRunState>>,
        resume_error: Mutex<Option<TurnError>>,
    }

    impl Default for RecordingTurnCoordinator {
        fn default() -> Self {
            Self {
                resumes: Mutex::new(Vec::new()),
                state: Mutex::new(None),
                resume_error: Mutex::new(None),
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
            let state = self
                .state
                .lock()
                .expect("state lock")
                .clone()
                .ok_or(TurnError::ScopeNotFound)?;
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
            self.resumes.lock().expect("resume lock").push(request);
            Ok(ResumeTurnResponse {
                run_id,
                status: TurnStatus::Running,
                event_cursor: EventCursor::default(),
            })
        }

        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            panic!("cancel_run is not used by auth continuation tests");
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            self.state
                .lock()
                .expect("state lock")
                .clone()
                .ok_or(TurnError::ScopeNotFound)
        }
    }

    fn scoped_event(continuation: AuthContinuationRef) -> AuthContinuationEvent {
        let thread_id = ThreadId::new("thread-auth").unwrap();
        let resource = ResourceScope {
            tenant_id: TenantId::new("tenant-auth").unwrap(),
            user_id: UserId::new("alice").unwrap(),
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
            credential_account_id: None,
            emitted_at: Utc::now(),
        }
    }

    fn run_state(run_id: TurnRunId, status: TurnStatus, gate_ref: Option<&str>) -> TurnRunState {
        TurnRunState {
            scope: TurnScope::new(
                TenantId::new("tenant-auth").unwrap(),
                Some(AgentId::new("agent-auth").unwrap()),
                Some(ProjectId::new("project-auth").unwrap()),
                ThreadId::new("thread-auth").unwrap(),
            ),
            actor: Some(TurnActor::new(UserId::new("alice").unwrap())),
            turn_id: TurnId::new(),
            run_id,
            status,
            accepted_message_ref: AcceptedMessageRef::new("message-auth").unwrap(),
            source_binding_ref: SourceBindingRef::new("source-auth").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-auth").unwrap(),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref: gate_ref.map(|value| GateRef::new(value).unwrap()),
            failure: None,
            event_cursor: EventCursor::default(),
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

        let err = dispatcher
            .dispatch_turn_gate_resume(event)
            .await
            .expect_err("non-auth gates must not resume through auth continuation");

        assert!(matches!(
            err,
            ProductWorkflowError::AuthContinuationRejected {
                kind: AuthContinuationRejectionKind::UnauthorizedBlockedGate
            }
        ));
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

        let err = dispatcher
            .dispatch_turn_gate_resume(event)
            .await
            .expect_err("stale auth gate callbacks must not resume a different gate");

        assert!(matches!(
            err,
            ProductWorkflowError::AuthContinuationRejected {
                kind: AuthContinuationRejectionKind::UnauthorizedBlockedGate
            }
        ));
        assert!(coordinator.resumes().is_empty());
    }

    #[tokio::test]
    async fn turn_gate_continuation_rejects_cross_scope_resume_through_real_coordinator() {
        let store = Arc::new(InMemoryTurnStateStore::default());
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
                scope: scope.clone(),
                actor: actor.clone(),
                accepted_message_ref: AcceptedMessageRef::new("message-auth-real").unwrap(),
                source_binding_ref: SourceBindingRef::new("source-auth-real").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply-auth-real").unwrap(),
                requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
                idempotency_key: IdempotencyKey::new("idem-auth-real-submit").unwrap(),
                received_at: Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
            })
            .await
            .expect("submit turn");
        let SubmitTurnResponse::Accepted { run_id, .. } = submit;
        let runner_id = TurnRunnerId::new();
        let lease_token = TurnLeaseToken::new();
        store
            .claim_next_run(ClaimRunRequest {
                runner_id,
                lease_token,
                scope_filter: Some(scope),
            })
            .await
            .expect("claim run")
            .expect("queued run exists");
        store
            .block_run(BlockRunRequest {
                run_id,
                runner_id,
                lease_token,
                checkpoint_id: TurnCheckpointId::new(),
                state_ref: LoopCheckpointStateRef::new("checkpoint:auth-real").unwrap(),
                reason: BlockedReason::Auth {
                    gate_ref: GateRef::new("gate:auth-real").unwrap(),
                },
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

        assert!(matches!(err, ProductWorkflowError::TurnResumeDenied { .. }));
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
            ProductWorkflowError::TurnSubmissionFailed { .. }
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
                auth_error_for_continuation_dispatch(&ProductWorkflowError::TurnResumeDenied {
                    error,
                });

            assert_eq!(auth_error.code(), AuthErrorCode::BackendUnavailable);
        }
    }

    #[test]
    fn auth_error_for_continuation_dispatch_maps_transient_and_catch_all() {
        let transient = auth_error_for_continuation_dispatch(&ProductWorkflowError::Transient {
            reason: "store timeout".to_string(),
        });
        assert_eq!(transient.code(), AuthErrorCode::BackendUnavailable);

        let catch_all =
            auth_error_for_continuation_dispatch(&ProductWorkflowError::UnknownInstallation);
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
                &ProductWorkflowError::AuthContinuationRejected { kind },
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
            ProductWorkflowError::AuthContinuationRejected {
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
            ProductWorkflowError::AuthContinuationRejected {
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
            ProductWorkflowError::AuthContinuationRejected {
                kind: AuthContinuationRejectionKind::MissingThreadScope
            }
        ));
    }

    #[test]
    fn workflow_error_kind_capacity_exceeded_returns_expected_strings() {
        let submission = ProductWorkflowError::TurnSubmissionFailed {
            error: TurnError::capacity_exceeded(
                ironclaw_turns::TurnCapacityResource::SpawnTreeDescendants,
                4,
            ),
        };
        assert_eq!(workflow_error_kind(&submission), "turn_capacity_exceeded");

        let resume = ProductWorkflowError::TurnResumeDenied {
            error: TurnError::capacity_exceeded(
                ironclaw_turns::TurnCapacityResource::SubmitTurn,
                7,
            ),
        };
        assert_eq!(
            workflow_error_kind(&resume),
            "turn_resume_capacity_exceeded"
        );
    }
}
