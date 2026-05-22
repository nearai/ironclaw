//! Product-auth continuation handling.
//!
//! This module consumes the `ironclaw_auth` continuation vocabulary and routes
//! turn-gate resume continuations through the same trusted `TurnCoordinator`
//! boundary as the WebUI gate-resolution path. It intentionally does not define
//! another auth-flow model.

use std::sync::Arc;

use ironclaw_auth::{AuthContinuationEvent, AuthContinuationRef};
use ironclaw_turns::{
    GateRef, IdempotencyKey, ReplyTargetBindingRef, ResumeTurnRequest, SourceBindingRef, TurnActor,
    TurnCoordinator, TurnRunId, TurnScope,
};
use uuid::Uuid;

use crate::ProductWorkflowError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthContinuationDispatchOutcome {
    SetupOnly,
    LifecycleActivation,
    ProductActionResume,
    TurnGateResumed { run_id: TurnRunId },
}

#[derive(Clone)]
pub struct ProductAuthContinuationDispatcher {
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

impl ProductAuthContinuationDispatcher {
    pub fn new(turn_coordinator: Arc<dyn TurnCoordinator>) -> Self {
        Self { turn_coordinator }
    }

    pub async fn dispatch(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<AuthContinuationDispatchOutcome, ProductWorkflowError> {
        let AuthContinuationRef::TurnGateResume {
            turn_run_ref,
            gate_ref,
        } = &event.continuation
        else {
            return Ok(outcome_for_non_turn_continuation(&event.continuation));
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
                source_binding_ref: bounded_ref("auth-continuation-src", &binding_id)?,
                reply_target_binding_ref: bounded_ref("auth-continuation-reply", &binding_id)?,
                idempotency_key: idempotency_key_for_flow(event.flow_id.to_string())?,
            })
            .await
            .map_err(|error| ProductWorkflowError::TurnSubmissionFailed { error })?;

        Ok(AuthContinuationDispatchOutcome::TurnGateResumed { run_id })
    }
}

fn outcome_for_non_turn_continuation(
    continuation: &AuthContinuationRef,
) -> AuthContinuationDispatchOutcome {
    match continuation {
        AuthContinuationRef::SetupOnly => AuthContinuationDispatchOutcome::SetupOnly,
        AuthContinuationRef::LifecycleActivation { .. } => {
            AuthContinuationDispatchOutcome::LifecycleActivation
        }
        AuthContinuationRef::ProductActionResume { .. } => {
            AuthContinuationDispatchOutcome::ProductActionResume
        }
        AuthContinuationRef::TurnGateResume { .. } => unreachable!("handled by caller"),
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

trait RefFactory: Sized {
    fn build(value: String) -> Result<Self, String>;
}

impl RefFactory for SourceBindingRef {
    fn build(value: String) -> Result<Self, String> {
        Self::new(value)
    }
}

impl RefFactory for ReplyTargetBindingRef {
    fn build(value: String) -> Result<Self, String> {
        Self::new(value)
    }
}

fn bounded_ref<T: RefFactory>(prefix: &str, raw: &str) -> Result<T, ProductWorkflowError> {
    let value = if raw.len() <= 220 && !raw.chars().any(|c| c == '\0' || c.is_control()) {
        format!("{prefix}:{raw}")
    } else {
        let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, raw.as_bytes());
        format!("{prefix}:{id}")
    };
    T::build(value).map_err(|reason| ProductWorkflowError::TurnSubmissionRejected {
        reason: format!("invalid auth continuation binding ref: {reason}"),
    })
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
        CancelRunRequest, CancelRunResponse, EventCursor, GetRunStateRequest, ResumeTurnRequest,
        ResumeTurnResponse, SubmitTurnRequest, SubmitTurnResponse, TurnCoordinator, TurnError,
        TurnRunId, TurnRunState, TurnStatus,
    };

    use super::*;

    #[derive(Default)]
    struct RecordingTurnCoordinator {
        resumes: Mutex<Vec<ResumeTurnRequest>>,
    }

    impl RecordingTurnCoordinator {
        fn resumes(&self) -> Vec<ResumeTurnRequest> {
            self.resumes.lock().expect("resume lock").clone()
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
            panic!("get_run_state is not used by auth continuation tests");
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

    #[tokio::test]
    async fn turn_gate_continuation_resumes_through_turn_coordinator() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let dispatcher = ProductAuthContinuationDispatcher::new(coordinator.clone());
        let run_id = TurnRunId::new();
        let event = scoped_event(AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new("gate:auth").unwrap(),
        });

        let outcome = dispatcher.dispatch(event).await.expect("dispatch");

        assert_eq!(
            outcome,
            AuthContinuationDispatchOutcome::TurnGateResumed { run_id }
        );
        let resumes = coordinator.resumes();
        assert_eq!(resumes.len(), 1);
        assert_eq!(resumes[0].run_id, run_id);
        assert_eq!(resumes[0].gate_resolution_ref.as_str(), "gate:auth");
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
    async fn non_turn_continuations_do_not_resume_turns() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let dispatcher = ProductAuthContinuationDispatcher::new(coordinator.clone());
        let event = scoped_event(AuthContinuationRef::LifecycleActivation {
            package_ref: LifecyclePackageRef::new("github").unwrap(),
        });

        let outcome = dispatcher.dispatch(event).await.expect("dispatch");

        assert_eq!(
            outcome,
            AuthContinuationDispatchOutcome::LifecycleActivation
        );
        assert!(coordinator.resumes().is_empty());
    }

    #[tokio::test]
    async fn turn_gate_continuation_requires_thread_scope() {
        let coordinator = Arc::new(RecordingTurnCoordinator::default());
        let dispatcher = ProductAuthContinuationDispatcher::new(coordinator);
        let run_id = TurnRunId::new();
        let mut event = scoped_event(AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new(run_id.to_string()).unwrap(),
            gate_ref: AuthGateRef::new("gate:auth").unwrap(),
        });
        event.scope.resource.thread_id = None;

        let err = dispatcher
            .dispatch(event)
            .await
            .expect_err("thread scope is required");

        assert!(matches!(
            err,
            ProductWorkflowError::TurnSubmissionRejected { .. }
        ));
    }
}
