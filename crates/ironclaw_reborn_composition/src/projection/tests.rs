use super::turn_events::{
    FailureExplanationInput, FailureExplanationProvider, ModelFailureExplanationProvider,
    WEBUI_TURN_EVENT_PAGE_LIMIT, bounded_failure_explanation,
};
use super::*;

use async_trait::async_trait;
use ironclaw_auth::{AuthProviderId, OAuthAuthorizationUrl};
use ironclaw_event_projections::{
    CapabilityActivityProjection, ProjectionSnapshot, ThreadTimeline,
};
use ironclaw_events::{InMemoryDurableEventLog, RuntimeEvent};
use ironclaw_host_api::{
    Action, AgentId, ApprovalRequest, ApprovalRequestId, CapabilityId, CorrelationId, ExtensionId,
    InvocationId, NetworkMethod, NetworkScheme, NetworkTarget, Principal, ResourceEstimate,
    ResourceScope, RuntimeCredentialAccountProviderId, RuntimeCredentialAuthRequirement,
    RuntimeHttpEgress, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeKind, TenantId,
    ThreadId, UserId,
};
use ironclaw_product_adapters::{
    AuthPromptChallengeKind, CapabilityActivityStatusView, ProductOutboundEnvelope,
    ProductOutboundPayload, ProductProjectionItem,
};
use ironclaw_run_state::{
    ApprovalRecord, ApprovalRequestStore, InMemoryApprovalRequestStore, RunStateError,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor as TurnEventCursor,
    GateRef, GetRunStateRequest, ResumeTurnRequest, ResumeTurnResponse, RunProfileId,
    RunProfileVersion, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
    TurnBlockedGateKind, TurnBlockedGateMetadata, TurnError, TurnEventKind, TurnEventPage,
    TurnLifecycleEvent, TurnRunId, TurnRunState, TurnStatus,
    run_profile::{
        LoopSafeSummary, SystemInferenceError, SystemInferencePort, SystemInferenceRequest,
        SystemInferenceResponse, SystemInferenceTaskId, SystemTaskKind,
    },
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;

use crate::AuthChallengeView;

mod cursor_validation;
mod display_preview;
mod display_preview_runtime;
mod failure_explanation;
mod live_progress_stream;
mod runtime_stream;
mod turn_stream;
mod turn_stream_auth;

fn long_test_id(prefix: &str, character: char) -> String {
    format!("{prefix}-{}", character.to_string().repeat(96))
}

fn resource_scope(
    tenant_id: &TenantId,
    user_id: &UserId,
    agent_id: &AgentId,
    thread_id: &ThreadId,
    invocation_id: InvocationId,
) -> ResourceScope {
    ResourceScope {
        tenant_id: tenant_id.clone(),
        user_id: user_id.clone(),
        agent_id: Some(agent_id.clone()),
        project_id: None,
        mission_id: None,
        thread_id: Some(thread_id.clone()),
        invocation_id,
    }
}

fn contains_run_status(
    events: &[ProductOutboundEnvelope],
    invocation_id: InvocationId,
    expected_status: &str,
) -> bool {
    let expected_run_id = TurnRunId::from_uuid(invocation_id.as_uuid());
    events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionSnapshot { state }
        | ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus { run_id, status, .. }
                    if *run_id == expected_run_id && status == expected_status
            )
        }),
        _ => false,
    })
}

struct FakeTurnEventSource {
    events: Vec<TurnLifecycleEvent>,
}

#[async_trait]
impl TurnEventProjectionSource for FakeTurnEventSource {
    async fn read_turn_events_after(
        &self,
        scope: &TurnScope,
        owner_user_id: Option<&UserId>,
        after: Option<TurnEventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        let after = after.unwrap_or_default();
        let mut events = self
            .events
            .iter()
            .filter(|event| {
                &event.scope == scope
                    && event.cursor > after
                    && owner_user_id.is_none_or(|owner| event.owner_user_id.as_ref() == Some(owner))
            })
            .cloned()
            .collect::<Vec<_>>();
        events.sort_by_key(|event| event.cursor);
        let truncated = events.len() > limit;
        if truncated {
            events.truncate(limit);
        }
        let next_cursor = events.last().map(|event| event.cursor).unwrap_or(after);
        Ok(TurnEventPage {
            entries: events,
            next_cursor,
            truncated,
            rebase_required: None,
        })
    }
}

struct RebaseTurnEventSource {
    cursor: TurnEventCursor,
}

struct FailingApprovalRequestStore;

#[async_trait]
impl ApprovalRequestStore for FailingApprovalRequestStore {
    async fn save_pending(
        &self,
        _scope: ResourceScope,
        _request: ApprovalRequest,
    ) -> Result<ApprovalRecord, RunStateError> {
        Err(RunStateError::Backend(
            "approval store unavailable".to_string(),
        ))
    }

    async fn get(
        &self,
        _scope: &ResourceScope,
        _request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, RunStateError> {
        Err(RunStateError::Backend(
            "approval store unavailable".to_string(),
        ))
    }

    async fn approve(
        &self,
        _scope: &ResourceScope,
        _request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        Err(RunStateError::Backend(
            "approval store unavailable".to_string(),
        ))
    }

    async fn deny(
        &self,
        _scope: &ResourceScope,
        _request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        Err(RunStateError::Backend(
            "approval store unavailable".to_string(),
        ))
    }

    async fn records_for_scope(
        &self,
        _scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, RunStateError> {
        Err(RunStateError::Backend(
            "approval store unavailable".to_string(),
        ))
    }
}

#[async_trait]
impl TurnEventProjectionSource for RebaseTurnEventSource {
    async fn read_turn_events_after(
        &self,
        _scope: &TurnScope,
        _owner_user_id: Option<&UserId>,
        _after: Option<TurnEventCursor>,
        _limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        Ok(TurnEventPage {
            entries: Vec::new(),
            next_cursor: self.cursor,
            truncated: false,
            rebase_required: Some(self.cursor),
        })
    }
}

struct FakeFailureExplainer {
    explanation: String,
}

#[async_trait]
impl FailureExplanationProvider for FakeFailureExplainer {
    async fn explain_failure(&self, input: FailureExplanationInput) -> Option<String> {
        assert!(
            !input.failure_category.is_empty(),
            "failure category should be available to the explainer"
        );
        assert!(
            !input.fallback_summary.is_empty(),
            "fallback summary should be available to the explainer"
        );
        Some(self.explanation.clone())
    }
}

struct CountingFailureExplainer {
    explanation: String,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl FailureExplanationProvider for CountingFailureExplainer {
    async fn explain_failure(&self, _input: FailureExplanationInput) -> Option<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Some(self.explanation.clone())
    }
}

struct RecordingFailureGateway {
    response: Mutex<Result<SystemInferenceResponse, SystemInferenceError>>,
    requests: Mutex<Vec<SystemInferenceRequest>>,
}

#[async_trait]
impl SystemInferencePort for RecordingFailureGateway {
    async fn call_system_inference(
        &self,
        request: SystemInferenceRequest,
    ) -> Result<SystemInferenceResponse, SystemInferenceError> {
        self.requests.lock().await.push(request);
        self.response.lock().await.clone()
    }
}

struct SlowSystemInference;

#[async_trait]
impl SystemInferencePort for SlowSystemInference {
    async fn call_system_inference(
        &self,
        request: SystemInferenceRequest,
    ) -> Result<SystemInferenceResponse, SystemInferenceError> {
        tokio::time::sleep(Duration::from_millis(2000)).await;
        Ok(SystemInferenceResponse {
            task_id: request.task_id,
            output_text: "too late".to_string(),
            elapsed_ms: 2000,
        })
    }
}

struct FakeTurnCoordinator {
    state: TurnRunState,
}

#[async_trait]
impl TurnCoordinator for FakeTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        unreachable!("projection tests only read run state")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        unreachable!("projection tests only read run state")
    }

    async fn retry_turn(
        &self,
        _request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        unreachable!("projection tests only read run state")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        unreachable!("projection tests only read run state")
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        if request.scope == self.state.scope && request.run_id == self.state.run_id {
            Ok(self.state.clone())
        } else {
            Err(TurnError::ScopeNotFound)
        }
    }
}

struct FakeAuthChallengeProvider {
    expected_owner_user_id: UserId,
    expected_run_id: TurnRunId,
    expected_gate_ref: String,
}

struct FailingAuthChallengeProvider;

#[async_trait]
impl AuthChallengeProvider for FakeAuthChallengeProvider {
    async fn challenge_for_gate(
        &self,
        _scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: TurnRunId,
        gate_ref: &str,
        _credential_requirements: &[ironclaw_host_api::RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, ironclaw_auth::AuthProductError> {
        if owner_user_id != &self.expected_owner_user_id
            || run_id != self.expected_run_id
            || gate_ref != self.expected_gate_ref
        {
            return Ok(None);
        }
        Ok(Some(AuthChallengeView {
            kind: AuthPromptChallengeKind::OAuthUrl,
            provider: AuthProviderId::new("github".to_string()).unwrap(),
            account_label: None,
            authorization_url: Some(
                OAuthAuthorizationUrl::new("https://github.com/login/oauth/authorize".to_string())
                    .unwrap(),
            ),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::minutes(10)),
        }))
    }
}

#[async_trait]
impl AuthChallengeProvider for FailingAuthChallengeProvider {
    async fn challenge_for_gate(
        &self,
        _scope: &TurnScope,
        _owner_user_id: &UserId,
        _run_id: TurnRunId,
        _gate_ref: &str,
        _credential_requirements: &[ironclaw_host_api::RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, ironclaw_auth::AuthProductError> {
        Err(ironclaw_auth::AuthProductError::BackendUnavailable)
    }
}

fn turn_run_state(
    scope: &TurnScope,
    user_id: &UserId,
    run_id: TurnRunId,
    cursor: TurnEventCursor,
) -> TurnRunState {
    TurnRunState {
        scope: scope.clone(),
        actor: Some(TurnActor::new(user_id.clone())),
        turn_id: ironclaw_turns::TurnId::new(),
        run_id,
        status: TurnStatus::BlockedAuth,
        accepted_message_ref: AcceptedMessageRef::new("message:auth-required").unwrap(),
        source_binding_ref: SourceBindingRef::new("source:auth-required").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply:auth-required").unwrap(),
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        received_at: chrono::Utc::now(),
        checkpoint_id: None,
        gate_ref: Some(GateRef::new("gate:auth-required").unwrap()),
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: cursor,
    }
}
