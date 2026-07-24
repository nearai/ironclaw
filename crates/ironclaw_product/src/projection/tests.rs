use super::turn_events::{
    FailureExplanationInput, FailureExplanationProvider, ModelFailureExplanationProvider,
    WEBUI_TURN_EVENT_PAGE_LIMIT, bounded_failure_explanation, failure_explanation_user_prompt,
};
use super::*;

use crate::{
    CapabilityActivityStatusView, ProductGateKind, ProductOutboundEnvelope, ProductOutboundPayload,
    ProductProjectionItem,
};
use async_trait::async_trait;
use ironclaw_event_projections::{
    CapabilityActivityProjection, ProjectionSnapshot, ThreadTimeline,
};
use ironclaw_events::{InMemoryDurableEventLog, RuntimeEvent};
use ironclaw_host_api::{
    Action, AgentId, ApprovalRequest, ApprovalRequestId, CapabilityId, CorrelationId, ExtensionId,
    InvocationId, NetworkMethod, NetworkScheme, NetworkTarget, Principal, ProcessId,
    ResourceEstimate, ResourceScope, RuntimeKind, ScopedPath, TenantId, ThreadId, UserId,
};
use ironclaw_run_state::{ApprovalRecord, ApprovalRequestStorePort, RunStateError};
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

mod cursor_validation;
mod display_preview;
mod display_preview_runtime;
mod failure_explanation;
mod live_progress_stream;
mod runtime_stream;
mod turn_stream;

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

#[tokio::test]
async fn projection_outbound_store_mount_is_tenant_user_scoped() {
    let scoped = outbound_scoped(Arc::new(InMemoryBackend::new()));
    let agent_id = AgentId::new("projection-outbound-agent").unwrap();
    let thread_id = ThreadId::new("projection-outbound-thread").unwrap();
    let scope_a = resource_scope(
        &TenantId::new("projection-outbound-tenant-a").unwrap(),
        &UserId::new("projection-outbound-user-a").unwrap(),
        &agent_id,
        &thread_id,
        InvocationId::new(),
    );
    let scope_b = resource_scope(
        &TenantId::new("projection-outbound-tenant-b").unwrap(),
        &UserId::new("projection-outbound-user-b").unwrap(),
        &agent_id,
        &thread_id,
        InvocationId::new(),
    );
    let path = ScopedPath::new("/outbound/subscriptions/shared-key.json").unwrap();

    scoped
        .write_bytes(&scope_a, &path, br#"{"owner":"a"}"#.to_vec())
        .await
        .unwrap();

    let owner_a = scoped.read_bytes(&scope_a, &path).await.unwrap();
    assert_eq!(owner_a, br#"{"owner":"a"}"#);
    assert!(
        scoped.read_bytes(&scope_b, &path).await.is_err(),
        "same outbound alias path must not cross tenant/user mount roots"
    );
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

fn run_status_failure_summary(
    events: &[ProductOutboundEnvelope],
    invocation_id: InvocationId,
) -> Option<String> {
    let expected_run_id = TurnRunId::from_uuid(invocation_id.as_uuid());
    events.iter().find_map(|event| match event.payload() {
        ProductOutboundPayload::ProjectionSnapshot { state }
        | ProductOutboundPayload::ProjectionUpdate { state } => {
            state.items.iter().find_map(|item| match item {
                ProductProjectionItem::RunStatus {
                    run_id,
                    failure_summary,
                    ..
                } if *run_id == expected_run_id => failure_summary.clone(),
                _ => None,
            })
        }
        _ => None,
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

    async fn read_turn_event_log_after(
        &self,
        after: Option<TurnEventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        let after = after.unwrap_or_default();
        let mut events = self
            .events
            .iter()
            .filter(|event| event.cursor > after)
            .cloned()
            .collect::<Vec<_>>();
        events.sort_by_key(|event| event.cursor);
        let truncated = events.len() > limit;
        if truncated {
            events.truncate(limit);
        }
        let next_cursor = events.last().map_or(after, |event| event.cursor);
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
impl ApprovalRequestStorePort for FailingApprovalRequestStore {
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

    async fn read_turn_event_log_after(
        &self,
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
        model_usage: None,
        received_at: chrono::Utc::now(),
        checkpoint_id: None,
        gate_ref: Some(GateRef::new("gate:auth-required").unwrap()),
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: cursor,
        product_context: None,
        resume_disposition: None,
    }
}
