#![cfg(feature = "openai-compat-beta")]

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use http::Request;
use http_body_util::BodyExt;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_adapters::{AuthRequirement, FakeProductWorkflow, ProtocolAuthEvidence};
use ironclaw_reborn_openai_compat::{
    InMemoryOpenAiCompatRefStore, OpenAiChatCompletionProjection, OpenAiChatCompletionWaitRequest,
    OpenAiChatCompletionWaiter, OpenAiChatCompletionsWorkflow, OpenAiChatFinishReason,
    OpenAiChatToolCall, OpenAiChatToolCallFunction, OpenAiChatToolKind, OpenAiCompatActorScope,
    OpenAiCompatAuthenticatedCaller, OpenAiCompatInternalRefs, OpenAiCompatProductActionRef,
    OpenAiCompatProjectionRef, OpenAiCompatRouterState, OpenAiCompatTurnRunRef, OpenAiUsage,
    openai_compat_router_with_state,
};
use serde_json::{Value, json};
use tower::ServiceExt;

#[tokio::test]
async fn chat_completion_route_submits_product_workflow_and_returns_projection() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let waiter = Arc::new(StaticChatWaiter::text("hello from reborn"));
    let router = test_router(workflow.clone(), waiter);

    let response = router
        .oneshot(chat_request(
            json!({
                "model": "gpt-reborn",
                "messages": [{"role": "user", "content": "hello"}]
            }),
            Some("same-key"),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["model"], "gpt-reborn");
    assert_eq!(body["choices"][0]["message"]["role"], "assistant");
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "hello from reborn"
    );
    assert!(body["id"].as_str().expect("id").starts_with("chatcmpl-"));

    let envelopes = workflow.accepted_envelopes();
    assert_eq!(envelopes.len(), 1);
    assert_eq!(envelopes[0].adapter_id().as_str(), "openai_compat");
    assert_eq!(
        envelopes[0].external_event_id().as_str(),
        body["id"].as_str().expect("id")
    );
    let rendered = serde_json::to_string(envelopes[0].payload()).expect("payload json");
    assert!(rendered.contains("user: hello"));
    assert!(!rendered.contains("model: gpt-reborn"));
}

#[tokio::test]
async fn chat_completion_idempotency_replays_same_id_and_conflicts_on_different_body() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let waiter = Arc::new(StaticChatWaiter::text("ok"));
    let router = test_router(workflow.clone(), waiter);
    let body = json!({
        "model": "gpt-reborn",
        "messages": [{"role": "user", "content": "hello"}]
    });

    let first = json_body(
        router
            .clone()
            .oneshot(chat_request(body.clone(), Some("same-key")))
            .await
            .expect("first"),
    )
    .await;
    let replay = json_body(
        router
            .clone()
            .oneshot(chat_request(body, Some("same-key")))
            .await
            .expect("replay"),
    )
    .await;

    assert_eq!(first["id"], replay["id"]);
    assert_eq!(workflow.accepted_count(), 1);

    let conflict = router
        .oneshot(chat_request(
            json!({
                "model": "gpt-reborn",
                "messages": [{"role": "user", "content": "different"}]
            }),
            Some("same-key"),
        ))
        .await
        .expect("conflict");

    assert_eq!(conflict.status(), http::StatusCode::CONFLICT);
    let body = json_body(conflict).await;
    assert_eq!(body["error"]["code"], "conflict");
    assert_eq!(workflow.accepted_count(), 1);
}

#[tokio::test]
async fn wired_chat_completion_requires_authenticated_caller_before_product_workflow() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let service = OpenAiChatCompletionsWorkflow::new(
        workflow.clone(),
        Arc::new(InMemoryOpenAiCompatRefStore::new()),
        Arc::new(StaticChatWaiter::text("unused")),
    );
    let router = openai_compat_router_with_state(OpenAiCompatRouterState::with_chat_completions(
        Arc::new(service),
    ));

    let response = router
        .oneshot(chat_request(
            json!({
                "model": "gpt-reborn",
                "messages": [{"role": "user", "content": "hello"}]
            }),
            None,
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
    assert_eq!(workflow.accepted_count(), 0);
}

#[test]
fn authenticated_caller_rejects_auth_subject_scope_mismatch() {
    let result = OpenAiCompatAuthenticatedCaller::new(
        OpenAiCompatActorScope::new(
            TenantId::new("tenant-a").expect("tenant"),
            UserId::new("user-a").expect("user"),
            None,
            None,
        ),
        ProtocolAuthEvidence::test_verified(AuthRequirement::BearerToken, "user-b"),
    );

    assert!(result.is_err());
}

#[tokio::test]
async fn streaming_chat_completion_is_rejected_before_product_workflow() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let router = test_router(workflow.clone(), Arc::new(StaticChatWaiter::text("unused")));

    let response = router
        .oneshot(chat_request(
            json!({
                "model": "gpt-reborn",
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
            None,
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
    assert_eq!(workflow.accepted_count(), 0);
}

#[tokio::test]
async fn chat_completion_wait_timeout_returns_retryable_error_without_resubmitting() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let service = OpenAiChatCompletionsWorkflow::new(
        workflow.clone(),
        Arc::new(InMemoryOpenAiCompatRefStore::new()),
        Arc::new(NeverChatWaiter),
    )
    .with_wait_timeout(Duration::from_millis(1));
    let router = openai_compat_router_with_state(OpenAiCompatRouterState::with_chat_completions(
        Arc::new(service),
    ))
    .layer(axum::Extension(caller()));

    let response = router
        .oneshot(chat_request(
            json!({
                "model": "gpt-reborn",
                "messages": [{"role": "user", "content": "hello"}]
            }),
            Some("timeout-key"),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(workflow.accepted_count(), 1);
}

#[tokio::test]
async fn model_only_tool_call_output_shape_is_preserved() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let waiter = Arc::new(StaticChatWaiter::projection(
        OpenAiChatCompletionProjection {
            assistant_content: None,
            tool_calls: Some(vec![OpenAiChatToolCall {
                id: "call_1".to_string(),
                kind: OpenAiChatToolKind::Function,
                function: OpenAiChatToolCallFunction {
                    name: "lookup_order".to_string(),
                    arguments: "{\"id\":\"123\"}".to_string(),
                },
            }]),
            finish_reason: OpenAiChatFinishReason::ToolCalls,
            usage: Some(OpenAiUsage {
                prompt_tokens: 3,
                completion_tokens: 5,
                total_tokens: 8,
            }),
            effective_model: Some("gpt-reborn-effective".to_string()),
            internal_refs: Some(
                OpenAiCompatInternalRefs::new(
                    OpenAiCompatProductActionRef::new("product-action:1").expect("action ref"),
                )
                .with_turn_run_ref(OpenAiCompatTurnRunRef::new("turn-run:1").expect("run ref"))
                .with_projection_ref(
                    OpenAiCompatProjectionRef::new("projection:1").expect("projection ref"),
                ),
            ),
        },
    ));
    let router = test_router(workflow, waiter);

    let response = router
        .oneshot(chat_request(
            json!({
                "model": "gpt-reborn",
                "messages": [{"role": "user", "content": "call tool if needed"}],
                "tools": [{
                    "type": "function",
                    "function": {"name": "lookup_order", "parameters": {"type": "object"}}
                }]
            }),
            None,
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["model"], "gpt-reborn-effective");
    assert_eq!(body["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(
        body["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
        "lookup_order"
    );
    assert_eq!(body["usage"]["total_tokens"], 8);
}

#[tokio::test]
async fn requested_model_is_forwarded_as_waiter_hint() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let waiter = Arc::new(RecordingChatWaiter::new(
        OpenAiChatCompletionProjection::text("ok"),
    ));
    let router = test_router(workflow, waiter.clone());

    let response = router
        .oneshot(chat_request(
            json!({
                "model": "gpt-reborn-model-hint",
                "messages": [{"role": "user", "content": "hello"}]
            }),
            None,
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), http::StatusCode::OK);
    let wait_request = waiter.last_request();
    assert_eq!(wait_request.requested_model, "gpt-reborn-model-hint");
}

fn test_router(
    workflow: Arc<FakeProductWorkflow>,
    waiter: Arc<dyn OpenAiChatCompletionWaiter>,
) -> axum::Router {
    let service = OpenAiChatCompletionsWorkflow::new(
        workflow,
        Arc::new(InMemoryOpenAiCompatRefStore::new()),
        waiter,
    );
    openai_compat_router_with_state(OpenAiCompatRouterState::with_chat_completions(Arc::new(
        service,
    )))
    .layer(axum::Extension(caller()))
}

fn chat_request(body: Value, idempotency_key: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json");
    if let Some(idempotency_key) = idempotency_key {
        builder = builder.header("idempotency-key", idempotency_key);
    }
    builder.body(Body::from(body.to_string())).expect("request")
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json")
}

fn caller() -> OpenAiCompatAuthenticatedCaller {
    OpenAiCompatAuthenticatedCaller::new(
        OpenAiCompatActorScope::new(
            TenantId::new("tenant-a").expect("tenant"),
            UserId::new("user-a").expect("user"),
            Some(AgentId::new("agent-a").expect("agent")),
            Some(ProjectId::new("project-a").expect("project")),
        ),
        ProtocolAuthEvidence::test_verified(AuthRequirement::BearerToken, "user-a"),
    )
    .expect("caller")
}

struct StaticChatWaiter {
    projection: OpenAiChatCompletionProjection,
}

impl StaticChatWaiter {
    fn text(content: &str) -> Self {
        Self::projection(OpenAiChatCompletionProjection::text(content))
    }

    fn projection(projection: OpenAiChatCompletionProjection) -> Self {
        Self { projection }
    }
}

#[async_trait]
impl OpenAiChatCompletionWaiter for StaticChatWaiter {
    async fn wait_for_chat_completion(
        &self,
        _request: OpenAiChatCompletionWaitRequest,
    ) -> Result<OpenAiChatCompletionProjection, ironclaw_reborn_openai_compat::OpenAiCompatHttpError>
    {
        Ok(self.projection.clone())
    }
}

struct NeverChatWaiter;

#[async_trait]
impl OpenAiChatCompletionWaiter for NeverChatWaiter {
    async fn wait_for_chat_completion(
        &self,
        _request: OpenAiChatCompletionWaitRequest,
    ) -> Result<OpenAiChatCompletionProjection, ironclaw_reborn_openai_compat::OpenAiCompatHttpError>
    {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(OpenAiChatCompletionProjection::text("late"))
    }
}

struct RecordingChatWaiter {
    projection: OpenAiChatCompletionProjection,
    last_request: Mutex<Option<OpenAiChatCompletionWaitRequest>>,
}

impl RecordingChatWaiter {
    fn new(projection: OpenAiChatCompletionProjection) -> Self {
        Self {
            projection,
            last_request: Mutex::new(None),
        }
    }

    fn last_request(&self) -> OpenAiChatCompletionWaitRequest {
        self.last_request
            .lock()
            .expect("waiter request lock")
            .clone()
            .expect("waiter request captured")
    }
}

#[async_trait]
impl OpenAiChatCompletionWaiter for RecordingChatWaiter {
    async fn wait_for_chat_completion(
        &self,
        request: OpenAiChatCompletionWaitRequest,
    ) -> Result<OpenAiChatCompletionProjection, ironclaw_reborn_openai_compat::OpenAiCompatHttpError>
    {
        *self.last_request.lock().expect("waiter request lock") = Some(request);
        Ok(self.projection.clone())
    }
}
