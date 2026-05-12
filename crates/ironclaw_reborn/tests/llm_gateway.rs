use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_llm::{
    CompletionRequest, CompletionResponse, FinishReason, LlmError, LlmProvider,
    ToolCompletionRequest, ToolCompletionResponse,
};
use ironclaw_loop_support::{
    HostManagedModelErrorKind, HostManagedModelGateway, HostManagedModelMessage,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelRouteSnapshot,
};
use ironclaw_reborn::{
    LlmModelProfilePolicy, LlmProviderModelGateway, ModelRoute, ModelRoutePolicy,
    ModelSelectionMode, ModelSlot, RoutedLlmProviderModelGateway, StaticModelRouteProviderPool,
    StaticModelRouteResolver, ThreadBackedLoopModelGateway,
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

    let request = model_request(interactive_model());
    let expected_run_id = request.run_id.to_string();
    let expected_turn_id = request.turn_id.to_string();

    let response = gateway.stream_model(request).await.unwrap();

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
    assert_eq!(
        requests[0].metadata.get("run_id").map(String::as_str),
        Some(expected_run_id.as_str())
    );
    assert_eq!(
        requests[0].metadata.get("turn_id").map(String::as_str),
        Some(expected_turn_id.as_str())
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
    assert_eq!(milestone_kinds, vec!["model_started", "model_failed"]);
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

#[tokio::test]
async fn routed_gateway_uses_provider_pool_route_not_request_model_override() {
    let route = ModelRoute::new("rig-openai", "gpt-4.1").unwrap();
    let provider = Arc::new(RecordingLlmProvider::reply_for_model(
        "gpt-4.1",
        "routed response",
    ));
    let pool = provider_pool_for_route(route.clone(), provider.clone());
    let gateway = RoutedLlmProviderModelGateway::new(pool, route_resolver_for_route(route));

    let response = gateway
        .stream_model(model_request_with_route(
            interactive_model(),
            "rig-openai",
            "gpt-4.1",
        ))
        .await
        .unwrap();

    assert_eq!(
        response.safe_text_deltas,
        vec!["routed response".to_string()]
    );
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model.as_deref(), Some("gpt-4.1"));
    assert_eq!(
        requests[0]
            .metadata
            .get("model_route_provider_id")
            .map(String::as_str),
        Some("rig-openai")
    );
    assert_eq!(
        requests[0]
            .metadata
            .get("model_route_model_id")
            .map(String::as_str),
        Some("gpt-4.1")
    );
}

#[tokio::test]
async fn provider_pool_rejects_wrong_provider_identity_with_same_model() {
    let route = ModelRoute::new("rig-openai", "gpt-4.1").unwrap();
    let provider = Arc::new(RecordingLlmProvider::reply_for_model("gpt-4.1", "unused"));
    let key = ironclaw_reborn::ModelRouteProviderKey::for_route(route);

    let error =
        match StaticModelRouteProviderPool::new().with_provider_identity("nearai", key, provider) {
            Ok(_) => panic!("wrong provider identity should be rejected"),
            Err(error) => error,
        };

    assert_eq!(error.kind, HostManagedModelErrorKind::InvalidRequest);
}

#[tokio::test]
async fn provider_pool_rejects_route_bound_to_wrong_active_model() {
    let route = ModelRoute::new("rig-openai", "gpt-4.1").unwrap();
    let provider = Arc::new(IgnoresModelOverrideProvider::new("gpt-4o", "unused"));

    let error = match StaticModelRouteProviderPool::new().with_provider(route, provider) {
        Ok(_) => panic!("route/provider mismatch should be rejected"),
        Err(error) => error,
    };

    assert_eq!(error.kind, HostManagedModelErrorKind::InvalidRequest);
}

#[tokio::test]
async fn routed_gateway_rejects_provider_that_ignores_route_model_override_at_call_time() {
    let route = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let provider = Arc::new(IgnoresModelOverrideProvider::new("qwen3-coder", "unused"));
    let pool = provider_pool_for_route(route.clone(), provider.clone());
    let gateway = RoutedLlmProviderModelGateway::new(pool, route_resolver_for_route(route));
    provider.set_active_model("other-model");

    let error = gateway
        .stream_model(model_request_with_route(
            interactive_model(),
            "nearai",
            "qwen3-coder",
        ))
        .await
        .unwrap_err();

    assert_eq!(error.kind, HostManagedModelErrorKind::InvalidRequest);
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn routed_gateway_rejects_missing_route_snapshot_before_provider_call() {
    let route = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let provider = Arc::new(RecordingLlmProvider::reply_for_model(
        "qwen3-coder",
        "unused",
    ));
    let pool = provider_pool_for_route(route.clone(), provider.clone());
    let gateway = RoutedLlmProviderModelGateway::new(pool, route_resolver_for_route(route));

    let error = gateway
        .stream_model(model_request(interactive_model()))
        .await
        .unwrap_err();

    assert_eq!(error.kind, HostManagedModelErrorKind::PolicyDenied);
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn routed_gateway_reports_configuration_error_for_missing_provider_pool_entry() {
    let route = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let pool = Arc::new(StaticModelRouteProviderPool::new());
    let gateway = RoutedLlmProviderModelGateway::new(pool, route_resolver_for_route(route));

    let error = gateway
        .stream_model(model_request_with_route(
            interactive_model(),
            "nearai",
            "qwen3-coder",
        ))
        .await
        .unwrap_err();

    assert_eq!(error.kind, HostManagedModelErrorKind::ConfigurationError);
    assert_eq!(error.safe_summary, "model route provider is not configured");
}

#[tokio::test]
async fn routed_gateway_reports_configuration_error_for_missing_resolver_slot() {
    let route = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let provider = Arc::new(RecordingLlmProvider::reply_for_model(
        "qwen3-coder",
        "unused",
    ));
    let pool = provider_pool_for_route(route.clone(), provider.clone());
    let resolver = Arc::new(StaticModelRouteResolver::new(
        ModelRoutePolicy::new(ModelSelectionMode::ManagedOnly).with_approved_route(route.clone()),
    ));
    let gateway = RoutedLlmProviderModelGateway::new(pool, resolver);

    let error = gateway
        .stream_model(model_request_with_route(
            interactive_model(),
            "nearai",
            "qwen3-coder",
        ))
        .await
        .unwrap_err();

    assert_eq!(error.kind, HostManagedModelErrorKind::ConfigurationError);
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn routed_gateway_rejects_route_snapshot_denied_by_policy() {
    let allowed_route = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let denied_route = ModelRoute::new("openrouter", "anthropic/claude-sonnet-4").unwrap();
    let provider = Arc::new(RecordingLlmProvider::reply_for_model(
        "anthropic/claude-sonnet-4",
        "unused",
    ));
    let pool = provider_pool_for_route(denied_route, provider.clone());
    let gateway = RoutedLlmProviderModelGateway::new(pool, route_resolver_for_route(allowed_route));

    let error = gateway
        .stream_model(model_request_with_route(
            interactive_model(),
            "openrouter",
            "anthropic/claude-sonnet-4",
        ))
        .await
        .unwrap_err();

    assert_eq!(error.kind, HostManagedModelErrorKind::PolicyDenied);
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn routed_gateway_uses_request_route_snapshot() {
    let route = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let provider = Arc::new(RecordingLlmProvider::reply_for_model(
        "qwen3-coder",
        "snapshot response",
    ));
    let pool = provider_pool_for_route(route.clone(), provider.clone());
    let gateway = RoutedLlmProviderModelGateway::new(pool, route_resolver_for_route(route));

    let response = gateway
        .stream_model(model_request_with_route(
            interactive_model(),
            "nearai",
            "qwen3-coder",
        ))
        .await
        .unwrap();

    assert_eq!(
        response.safe_text_deltas,
        vec!["snapshot response".to_string()]
    );
}

#[tokio::test]
async fn routed_gateway_accepts_mission_model_profile_when_slot_route_configured() {
    let route = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let provider = Arc::new(RecordingLlmProvider::reply_for_model(
        "qwen3-coder",
        "mission response",
    ));
    let pool = provider_pool_for_route(route.clone(), provider);
    let gateway = RoutedLlmProviderModelGateway::new(
        pool,
        route_resolver_for_slot(ModelSlot::Mission, route),
    );

    let response = gateway
        .stream_model(model_request_with_route(
            ModelProfileId::new("mission_model").unwrap(),
            "nearai",
            "qwen3-coder",
        ))
        .await
        .unwrap();

    assert_eq!(
        response.safe_text_deltas,
        vec!["mission response".to_string()]
    );
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

fn provider_pool_for_route<P>(
    route: ModelRoute,
    provider: Arc<P>,
) -> Arc<StaticModelRouteProviderPool>
where
    P: LlmProvider + 'static,
{
    Arc::new(
        StaticModelRouteProviderPool::new()
            .with_provider(route, provider)
            .unwrap(),
    )
}

fn route_resolver_for_route(route: ModelRoute) -> Arc<StaticModelRouteResolver> {
    route_resolver_for_slot(ModelSlot::Default, route)
}

fn route_resolver_for_slot(slot: ModelSlot, route: ModelRoute) -> Arc<StaticModelRouteResolver> {
    Arc::new(
        StaticModelRouteResolver::new(
            ModelRoutePolicy::new(ModelSelectionMode::ManagedOnly)
                .with_approved_route(route.clone()),
        )
        .with_route(slot, route),
    )
}

#[test]
fn host_managed_model_request_accepts_legacy_string_identity_wire_shape() {
    let wire = serde_json::json!({
        "model_profile_id": "interactive_model",
        "messages": [
            {
                "role": "system",
                "content": "system instructions",
                "content_ref": "msg:11111111-1111-1111-1111-111111111111"
            }
        ],
        "surface_version": null,
        "run_id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
        "turn_id": "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
    });

    let decoded = serde_json::from_value::<HostManagedModelRequest>(wire).unwrap();
    assert_eq!(
        decoded.model_profile_id,
        ModelProfileId::new("interactive_model").unwrap()
    );
    assert_eq!(
        decoded.run_id.to_string(),
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
    );
    assert_eq!(
        decoded.turn_id.to_string(),
        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
    );

    let encoded = serde_json::to_value(&decoded).unwrap();
    assert_eq!(encoded["run_id"], "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    assert_eq!(encoded["turn_id"], "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
}

#[test]
fn host_managed_model_request_rejects_invalid_legacy_identity_strings() {
    let wire = serde_json::json!({
        "model_profile_id": "interactive_model",
        "messages": [],
        "surface_version": null,
        "run_id": "not-a-uuid",
        "turn_id": "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
    });

    assert!(serde_json::from_value::<HostManagedModelRequest>(wire).is_err());
}

fn model_request_with_route(
    model_profile_id: ModelProfileId,
    provider_id: &str,
    model_id: &str,
) -> HostManagedModelRequest {
    let mut request = model_request(model_profile_id);
    request.resolved_model_route = Some(HostManagedModelRouteSnapshot::new(
        provider_id,
        model_id,
        "config:default",
        "auth:default",
    ));
    request
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
        resolved_model_route: None,
        run_id: TurnRunId::new(),
        turn_id: TurnId::new(),
    }
}

struct IgnoresModelOverrideProvider {
    model_name: Mutex<String>,
    content: String,
    requests: Mutex<Vec<CompletionRequest>>,
}

impl IgnoresModelOverrideProvider {
    fn new(model_name: &str, content: &str) -> Self {
        Self {
            model_name: Mutex::new(model_name.to_string()),
            content: content.to_string(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn set_active_model(&self, model_name: &str) {
        *self.model_name.lock().unwrap() = model_name.to_string();
    }
}

#[async_trait]
impl LlmProvider for IgnoresModelOverrideProvider {
    fn model_name(&self) -> &str {
        "ignores-model-override-provider"
    }

    fn active_model_name(&self) -> String {
        self.model_name.lock().unwrap().clone()
    }

    fn effective_model_name(&self, _requested_model: Option<&str>) -> String {
        self.active_model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.requests.lock().unwrap().push(request);
        Ok(CompletionResponse {
            content: self.content.clone(),
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Err(LlmError::RequestFailed {
            provider: "mutable".to_string(),
            reason: "tool completion is not used by the loop support gateway".to_string(),
        })
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
    model_name: String,
    requests: Mutex<Vec<CompletionRequest>>,
    response: Mutex<Option<Result<CompletionResponse, LlmError>>>,
}

impl RecordingLlmProvider {
    fn reply(content: &str) -> Self {
        Self::reply_for_model("recording-model", content)
    }

    fn reply_for_model(model_name: &str, content: &str) -> Self {
        Self::reply_for_model_with_finish_reason(model_name, content, FinishReason::Stop)
    }

    fn reply_with_finish_reason(content: &str, finish_reason: FinishReason) -> Self {
        Self::reply_for_model_with_finish_reason("recording-model", content, finish_reason)
    }

    fn reply_for_model_with_finish_reason(
        model_name: &str,
        content: &str,
        finish_reason: FinishReason,
    ) -> Self {
        Self {
            model_name: model_name.to_string(),
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
            model_name: "recording-model".to_string(),
            requests: Mutex::new(Vec::new()),
            response: Mutex::new(Some(Err(error))),
        }
    }
}

#[async_trait]
impl LlmProvider for RecordingLlmProvider {
    fn model_name(&self) -> &str {
        &self.model_name
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
