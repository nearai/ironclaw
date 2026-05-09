use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw::llm::{
    CompletionRequest, CompletionResponse, FinishReason, LlmError, LlmProvider,
    ToolCompletionRequest, ToolCompletionResponse,
};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_loop_support::{
    HostManagedModelErrorKind, HostManagedModelGateway, HostManagedModelMessage,
    HostManagedModelMessageRole, HostManagedModelRequest,
};
use ironclaw_reborn::{
    LlmModelProfilePolicy, LlmProviderModelGateway, ThreadBackedLoopModelGateway,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    SessionThreadService, ThreadScope,
};
use ironclaw_turns::{
    LoopMessageRef, RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
    run_profile::{
        AgentLoopHostErrorKind, HostManagedLoopModelPort, InMemoryLoopHostMilestoneSink,
        InMemoryRunProfileResolver, LoopHostMilestoneKind, LoopModelMessage, LoopModelPort,
        LoopModelRequest, LoopRunContext, ModelProfileId,
    },
};
use rust_decimal::Decimal;

#[tokio::test]
async fn gateway_calls_llm_provider_for_allowed_model_profile() {
    let provider = Arc::new(RecordingLlmProvider::reply("assistant response"));
    let policy = LlmModelProfilePolicy::new()
        .allow_model_profile(interactive_model(), Some("host-selected-model".to_string()));
    let gateway = LlmProviderModelGateway::new(provider.clone(), policy);

    let response = gateway
        .stream_model(model_request(interactive_model()))
        .await
        .unwrap();

    assert_eq!(
        response.safe_text_deltas,
        vec!["assistant response".to_string()]
    );
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model.as_deref(), Some("host-selected-model"));
    assert_eq!(
        requests[0]
            .metadata
            .get("model_profile_id")
            .map(String::as_str),
        Some("interactive_model")
    );
    assert_eq!(requests[0].messages.len(), 2);
    assert_eq!(requests[0].messages[0].content, "system instructions");
    assert_eq!(requests[0].messages[1].content, "hello model");
}

#[tokio::test]
async fn gateway_rejects_unknown_model_profile_without_calling_provider() {
    let provider = Arc::new(RecordingLlmProvider::reply("unused"));
    let gateway = LlmProviderModelGateway::new(
        provider.clone(),
        LlmModelProfilePolicy::new()
            .allow_model_profile(interactive_model(), Some("host-selected-model".to_string())),
    );

    let error = gateway
        .stream_model(model_request(ModelProfileId::new("unknown_model").unwrap()))
        .await
        .unwrap_err();

    assert_eq!(error.kind, HostManagedModelErrorKind::PolicyDenied);
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn gateway_rejects_unpinned_model_profile_without_calling_provider() {
    let provider = Arc::new(RecordingLlmProvider::reply("unused"));
    let gateway = LlmProviderModelGateway::new(
        provider.clone(),
        LlmModelProfilePolicy::new().allow_model_profile(interactive_model(), None),
    );

    let error = gateway
        .stream_model(model_request(interactive_model()))
        .await
        .unwrap_err();

    assert_eq!(error.kind, HostManagedModelErrorKind::PolicyDenied);
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn gateway_rejects_truncated_provider_responses() {
    let provider = Arc::new(RecordingLlmProvider::reply_with_finish_reason(
        "partial response",
        FinishReason::Length,
    ));
    let gateway = LlmProviderModelGateway::new(
        provider,
        LlmModelProfilePolicy::new()
            .allow_model_profile(interactive_model(), Some("host-selected-model".to_string())),
    );

    let error = gateway
        .stream_model(model_request(interactive_model()))
        .await
        .unwrap_err();

    assert_eq!(error.kind, HostManagedModelErrorKind::BudgetExceeded);
}

#[tokio::test]
async fn gateway_rejects_content_filtered_provider_responses() {
    let provider = Arc::new(RecordingLlmProvider::reply_with_finish_reason(
        "filtered response",
        FinishReason::ContentFilter,
    ));
    let gateway = LlmProviderModelGateway::new(
        provider,
        LlmModelProfilePolicy::new()
            .allow_model_profile(interactive_model(), Some("host-selected-model".to_string())),
    );

    let error = gateway
        .stream_model(model_request(interactive_model()))
        .await
        .unwrap_err();

    assert_eq!(error.kind, HostManagedModelErrorKind::PolicyDenied);
}

#[tokio::test]
async fn gateway_rejects_tool_use_provider_responses() {
    let provider = Arc::new(RecordingLlmProvider::reply_with_finish_reason(
        "tool call requested",
        FinishReason::ToolUse,
    ));
    let gateway = LlmProviderModelGateway::new(
        provider,
        LlmModelProfilePolicy::new()
            .allow_model_profile(interactive_model(), Some("host-selected-model".to_string())),
    );

    let error = gateway
        .stream_model(model_request(interactive_model()))
        .await
        .unwrap_err();

    assert_eq!(error.kind, HostManagedModelErrorKind::InvalidRequest);
}

#[tokio::test]
async fn gateway_rejects_unknown_finish_reason_provider_responses() {
    let provider = Arc::new(RecordingLlmProvider::reply_with_finish_reason(
        "unknown completion",
        FinishReason::Unknown,
    ));
    let gateway = LlmProviderModelGateway::new(
        provider,
        LlmModelProfilePolicy::new()
            .allow_model_profile(interactive_model(), Some("host-selected-model".to_string())),
    );

    let error = gateway
        .stream_model(model_request(interactive_model()))
        .await
        .unwrap_err();

    assert_eq!(error.kind, HostManagedModelErrorKind::Unavailable);
}

#[tokio::test]
async fn production_loop_model_gateway_resolves_thread_refs_and_emits_milestones() {
    let fixture = ThreadFixture::new().await;
    let provider = Arc::new(RecordingLlmProvider::reply("production response"));
    let provider_gateway = Arc::new(LlmProviderModelGateway::new(
        provider.clone(),
        LlmModelProfilePolicy::new()
            .allow_model_profile(interactive_model(), Some("host-selected-model".to_string())),
    ));
    let model_gateway = Arc::new(ThreadBackedLoopModelGateway::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        provider_gateway,
        16,
    ));
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let port = HostManagedLoopModelPort::new(
        fixture.run_context.clone(),
        model_gateway,
        milestones.clone(),
    );

    let response = port
        .stream_model(LoopModelRequest {
            messages: vec![LoopModelMessage {
                role: "user".to_string(),
                content_ref: LoopMessageRef::new(format!("msg:{}", fixture.user_message_id))
                    .unwrap(),
            }],
            surface_version: None,
            model_preference: None,
        })
        .await
        .unwrap();

    assert_eq!(response.chunks[0].safe_text_delta, "production response");
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model.as_deref(), Some("host-selected-model"));
    assert_eq!(requests[0].messages[0].content, "hello production gateway");
    let milestone_kinds = milestones
        .milestones()
        .into_iter()
        .map(|milestone| milestone.kind)
        .collect::<Vec<_>>();
    assert!(matches!(
        milestone_kinds.as_slice(),
        [
            LoopHostMilestoneKind::ModelStarted {
                requested_model_profile_id: None
            },
            LoopHostMilestoneKind::ModelCompleted {
                effective_model_profile_id
            }
        ] if effective_model_profile_id.as_str() == "interactive_model"
    ));
}

#[tokio::test]
async fn production_loop_model_gateway_fails_closed_before_provider_call() {
    let fixture = ThreadFixture::new().await;
    let provider = Arc::new(RecordingLlmProvider::reply("unused"));
    let provider_gateway = Arc::new(LlmProviderModelGateway::new(
        provider.clone(),
        LlmModelProfilePolicy::new()
            .allow_model_profile(interactive_model(), Some("host-selected-model".to_string())),
    ));
    let model_gateway = Arc::new(ThreadBackedLoopModelGateway::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        provider_gateway,
        16,
    ));
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let port = HostManagedLoopModelPort::new(
        fixture.run_context.clone(),
        model_gateway,
        milestones.clone(),
    );

    let error = port
        .stream_model(LoopModelRequest {
            messages: vec![LoopModelMessage {
                role: "user".to_string(),
                content_ref: LoopMessageRef::new(format!("msg:{}", fixture.user_message_id))
                    .unwrap(),
            }],
            surface_version: None,
            model_preference: Some(ModelProfileId::new("mission_model").unwrap()),
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    assert!(provider.requests.lock().unwrap().is_empty());
    let milestone_kinds = milestones
        .milestones()
        .into_iter()
        .map(|milestone| milestone.kind.kind_name())
        .collect::<Vec<_>>();
    assert_eq!(milestone_kinds, vec!["model_started"]);
}

#[tokio::test]
async fn production_loop_model_gateway_preserves_error_kind_when_summary_is_resanitized() {
    let fixture = ThreadFixture::new().await;
    let invalid_summary_gateway = Arc::new(InvalidSummaryModelGateway {
        kind: HostManagedModelErrorKind::PolicyDenied,
        safe_summary: "RAW_PROVIDER_SECRET".to_string(),
    });
    let model_gateway = Arc::new(ThreadBackedLoopModelGateway::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        invalid_summary_gateway,
        16,
    ));
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let port =
        HostManagedLoopModelPort::new(fixture.run_context.clone(), model_gateway, milestones);

    let error = port
        .stream_model(LoopModelRequest {
            messages: vec![LoopModelMessage {
                role: "user".to_string(),
                content_ref: LoopMessageRef::new(format!("msg:{}", fixture.user_message_id))
                    .unwrap(),
            }],
            surface_version: None,
            model_preference: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    assert_eq!(error.safe_summary, "model gateway failed");
}

#[tokio::test]
async fn gateway_sanitizes_provider_errors() {
    let provider = Arc::new(RecordingLlmProvider::fail(LlmError::RequestFailed {
        provider: "raw-provider".to_string(),
        reason: "RAW_PROVIDER_SECRET".to_string(),
    }));
    let gateway = LlmProviderModelGateway::new(
        provider,
        LlmModelProfilePolicy::new()
            .allow_model_profile(interactive_model(), Some("host-selected-model".to_string())),
    );

    let error = gateway
        .stream_model(model_request(interactive_model()))
        .await
        .unwrap_err();

    assert_eq!(error.kind, HostManagedModelErrorKind::Unavailable);
    assert!(!error.safe_summary.contains("RAW_PROVIDER_SECRET"));
    assert!(!format!("{error:?}").contains("RAW_PROVIDER_SECRET"));
}

struct ThreadFixture {
    thread_service: Arc<InMemorySessionThreadService>,
    thread_scope: ThreadScope,
    user_message_id: ironclaw_threads::ThreadMessageId,
    run_context: LoopRunContext,
}

impl ThreadFixture {
    async fn new() -> Self {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let tenant_id = TenantId::new("tenant-production-gateway").unwrap();
        let agent_id = AgentId::new("agent-production-gateway").unwrap();
        let project_id = ProjectId::new("project-production-gateway").unwrap();
        let user_id = UserId::new("user-production-gateway").unwrap();
        let thread_id = ThreadId::new("thread-production-gateway").unwrap();
        let thread_scope = ThreadScope {
            tenant_id: tenant_id.clone(),
            agent_id: agent_id.clone(),
            project_id: Some(project_id.clone()),
            owner_user_id: Some(user_id.clone()),
            mission_id: None,
        };
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: user_id.as_str().to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        let accepted = thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
                actor_id: user_id.as_str().to_string(),
                source_binding_id: Some("source-web".to_string()),
                reply_target_binding_id: Some("reply-web".to_string()),
                external_event_id: Some("event-production-gateway-1".to_string()),
                content: MessageContent::text("hello production gateway"),
            })
            .await
            .unwrap();
        let turn_scope = TurnScope::new(
            tenant_id,
            Some(agent_id),
            Some(project_id),
            thread_id.clone(),
        );
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        let run_context =
            LoopRunContext::new(turn_scope, TurnId::new(), TurnRunId::new(), resolved);
        Self {
            thread_service,
            thread_scope,
            user_message_id: accepted.message_id,
            run_context,
        }
    }
}

fn interactive_model() -> ModelProfileId {
    ModelProfileId::new("interactive_model").unwrap()
}

fn model_request(model_profile_id: ModelProfileId) -> HostManagedModelRequest {
    HostManagedModelRequest {
        model_profile_id,
        messages: vec![
            HostManagedModelMessage {
                role: HostManagedModelMessageRole::System,
                content: "system instructions".to_string(),
                content_ref: LoopMessageRef::new("msg:11111111-1111-1111-1111-111111111111")
                    .unwrap(),
            },
            HostManagedModelMessage {
                role: HostManagedModelMessageRole::User,
                content: "hello model".to_string(),
                content_ref: LoopMessageRef::new("msg:22222222-2222-2222-2222-222222222222")
                    .unwrap(),
            },
        ],
        surface_version: None,
        run_id: "run-1".to_string(),
        turn_id: "turn-1".to_string(),
    }
}

struct InvalidSummaryModelGateway {
    kind: HostManagedModelErrorKind,
    safe_summary: String,
}

#[async_trait]
impl HostManagedModelGateway for InvalidSummaryModelGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<
        ironclaw_loop_support::HostManagedModelResponse,
        ironclaw_loop_support::HostManagedModelError,
    > {
        Err(ironclaw_loop_support::HostManagedModelError::safe(
            self.kind,
            self.safe_summary.clone(),
        ))
    }
}

struct RecordingLlmProvider {
    requests: Mutex<Vec<CompletionRequest>>,
    response: Mutex<Option<Result<CompletionResponse, LlmError>>>,
}

impl RecordingLlmProvider {
    fn reply(content: &str) -> Self {
        Self::reply_with_finish_reason(content, FinishReason::Stop)
    }

    fn reply_with_finish_reason(content: &str, finish_reason: FinishReason) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            response: Mutex::new(Some(Ok(CompletionResponse {
                content: content.to_string(),
                input_tokens: 1,
                output_tokens: 1,
                finish_reason,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            }))),
        }
    }

    fn fail(error: LlmError) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            response: Mutex::new(Some(Err(error))),
        }
    }
}

#[async_trait]
impl LlmProvider for RecordingLlmProvider {
    fn model_name(&self) -> &str {
        "recording-model"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.requests.lock().unwrap().push(request);
        self.response
            .lock()
            .unwrap()
            .take()
            .expect("test provider response is configured once")
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Err(LlmError::RequestFailed {
            provider: "recording".to_string(),
            reason: "tool completion is not used by the loop support gateway".to_string(),
        })
    }
}
