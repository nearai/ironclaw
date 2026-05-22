//! Product-auth continuation handling.
//!
//! This module consumes the `ironclaw_auth` continuation vocabulary and routes
//! turn-gate resume continuations through the same trusted `TurnCoordinator`
//! boundary as the WebUI gate-resolution path. It intentionally does not define
//! another auth-flow model or handle non-turn continuation variants.

use std::sync::Arc;

use ironclaw_auth::{AuthContinuationEvent, AuthContinuationRef};
use ironclaw_turns::{
    GateRef, IdempotencyKey, ResumeTurnPrecondition, ResumeTurnRequest, TurnActor, TurnCoordinator,
    TurnError, TurnRunId, TurnScope,
};
use uuid::Uuid;

use crate::ProductWorkflowError;
use crate::binding_ref::{bounded_reply_target_binding_ref, bounded_source_binding_ref};

#[derive(Clone)]
pub struct ProductAuthTurnGateResumeDispatcher {
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

impl ProductAuthTurnGateResumeDispatcher {
    pub fn new(turn_coordinator: Arc<dyn TurnCoordinator>) -> Self {
        Self { turn_coordinator }
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
            return Err(ProductWorkflowError::TurnSubmissionRejected {
                reason: "auth continuation is not a turn-gate resume".to_string(),
            });
        };

        let scope = turn_scope_from_auth_event(&event)?;
        let actor = TurnActor::new(event.scope.resource.user_id.clone());
        let run_id = parse_turn_run_id(turn_run_ref.as_str())?;
        let gate_resolution_ref = parse_gate_ref(gate_ref.as_str())?;
        let binding_id = format!("{}|{}|{}", event.flow_id, run_id, gate_ref.as_str());

        self.turn_coordinator
            .resume_turn(ResumeTurnRequest {
                scope,
                actor,
                run_id,
                gate_resolution_ref,
                precondition: ResumeTurnPrecondition::BlockedAuthGate,
                source_binding_ref: bounded_source_binding_ref(
                    "auth-continuation-src",
                    &binding_id,
                    220,
                )
                .map_err(binding_ref_error)?,
                reply_target_binding_ref: bounded_reply_target_binding_ref(
                    "auth-continuation-reply",
                    &binding_id,
                    220,
                )
                .map_err(binding_ref_error)?,
                idempotency_key: idempotency_key_for_flow(event.flow_id.to_string())?,
            })
            .await
            .map_err(map_auth_resume_error)?;

        Ok(run_id)
    }
}

fn map_auth_resume_error(error: TurnError) -> ProductWorkflowError {
    match error {
        TurnError::InvalidTransition { .. } | TurnError::InvalidRequest { .. } => {
            ProductWorkflowError::TurnResumeRejected {
                reason: "auth continuation does not match a blocked auth gate".to_string(),
            }
        }
        error => ProductWorkflowError::TurnSubmissionFailed { error },
    }
}

fn turn_scope_from_auth_event(
    event: &AuthContinuationEvent,
) -> Result<TurnScope, ProductWorkflowError> {
    let Some(thread_id) = event.scope.resource.thread_id.clone() else {
        return Err(ProductWorkflowError::TurnSubmissionRejected {
            reason: "auth turn-gate continuation is missing thread scope".to_string(),
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
        .map_err(|_| ProductWorkflowError::TurnSubmissionRejected {
            reason: "invalid auth continuation turn_run_ref".to_string(),
        })
}

fn parse_gate_ref(value: &str) -> Result<GateRef, ProductWorkflowError> {
    GateRef::new(value.to_string()).map_err(|reason| ProductWorkflowError::TurnSubmissionRejected {
        reason: format!("invalid auth continuation gate_ref: {reason}"),
    })
}

fn idempotency_key_for_flow(flow_id: String) -> Result<IdempotencyKey, ProductWorkflowError> {
    IdempotencyKey::new(format!("auth-continuation:{flow_id}")).map_err(|reason| {
        ProductWorkflowError::TurnSubmissionRejected {
            reason: format!("invalid auth continuation idempotency key: {reason}"),
        }
    })
}

fn binding_ref_error(reason: String) -> ProductWorkflowError {
    ProductWorkflowError::TurnSubmissionRejected {
        reason: format!("invalid auth continuation binding ref: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_auth::{
        AuthContinuationEvent, AuthContinuationRef, AuthFlowId, AuthGateRef, AuthProductScope,
        AuthSessionId, AuthSurface, LifecyclePackageRef, TurnRunRef,
    };
    use ironclaw_host_api::{
        AgentId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
    };
    use ironclaw_turns::{
        AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GetRunStateRequest,
        ReplyTargetBindingRef, ResumeTurnRequest, ResumeTurnResponse, RunProfileId,
        RunProfileVersion, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
        TurnCoordinator, TurnError, TurnId, TurnRunId, TurnRunState, TurnStatus,
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
            ProductWorkflowError::TurnResumeRejected { .. }
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
            ProductWorkflowError::TurnResumeRejected { .. }
        ));
        assert!(coordinator.resumes().is_empty());
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
            ProductWorkflowError::TurnSubmissionRejected { .. }
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
            ProductWorkflowError::TurnSubmissionRejected { .. }
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
            ProductWorkflowError::TurnSubmissionRejected { .. }
        ));
    }
}
