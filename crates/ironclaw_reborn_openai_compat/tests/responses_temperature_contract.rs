#![cfg(feature = "openai-compat-beta")]

//! Caller-level regression tests for per-request `temperature` on the
//! Responses API (PR #3641, serrrfirat's Medium-severity follow-up).
//!
//! The current Responses API surface is owned by the Reborn
//! OpenAI-compatible router, not the retired v1 gateway. These tests drive the
//! route-level workflow and assert that `temperature` is validated at the API
//! boundary and preserved in the submitted product payload.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{
    AuthRequirement, FakeProductWorkflow, ProductInboundEnvelope, ProductInboundPayload,
    ProjectionReadRequest, ProtocolAuthEvidence,
};
use ironclaw_reborn_openai_compat::{
    InMemoryOpenAiCompatRefStore, OpenAiCompatActorScope, OpenAiCompatAuthenticatedCaller,
    OpenAiCompatInternalRefs, OpenAiCompatProductActionRef, OpenAiCompatProjectionRef,
    OpenAiCompatRouterState, OpenAiCompatTurnRunRef, OpenAiResponseId, OpenAiResponseObject,
    OpenAiResponseOutputItem, OpenAiResponseOutputItemStatus, OpenAiResponseProjection,
    OpenAiResponseReadRequest, OpenAiResponseStatus, OpenAiResponseUsage,
    OpenAiResponseWaitRequest, OpenAiResponsesMessageRole, OpenAiResponsesProjectionReader,
    OpenAiResponsesWorkflow, openai_compat_router_with_state,
};
use ironclaw_turns::{TurnActor, TurnRunId, TurnScope};
use serde_json::{Value, json};
use tower::ServiceExt;

/// POST `/v1/responses` with `temperature` set must preserve it in the
/// openai_compat product payload. That is the Reborn equivalent of the old v1
/// gateway writing `IncomingMessage.metadata["temperature"]`.
#[tokio::test]
async fn responses_request_temperature_lands_in_submitted_payload() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let router = test_router(workflow.clone());

    let response = router
        .oneshot(response_create_request(json!({
            "model": "default",
            "input": "hello",
            "temperature": 0.42,
        })))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let submitted = submitted_user_message_json(
        &workflow
            .accepted_envelopes()
            .into_iter()
            .next()
            .expect("accepted envelope"),
    );
    let temperature = submitted["temperature"]
        .as_f64()
        .unwrap_or_else(|| panic!("submitted payload missing numeric temperature: {submitted}"));
    assert!(
        (temperature - 0.42).abs() < f64::EPSILON,
        "expected temperature 0.42, got {temperature}"
    );
}

/// POST `/v1/responses` with `temperature` outside the OpenAI-compatible
/// `[0, 2]` range must reject with 400 before submitting to ProductWorkflow.
#[tokio::test]
async fn responses_request_temperature_out_of_range_rejects_and_does_not_submit() {
    for bad_temperature in [-0.5_f64, 2.5_f64] {
        let workflow = Arc::new(FakeProductWorkflow::new());
        let router = test_router(workflow.clone());

        let response = router
            .oneshot(response_create_request(json!({
                "model": "default",
                "input": "hello",
                "temperature": bad_temperature,
            })))
            .await
            .expect("response");

        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "temperature {bad_temperature} must be rejected with 400",
        );
        let body = json_body(response).await;
        assert_eq!(
            body["error"]["type"], "invalid_request_error",
            "bad temperature should return invalid_request_error, body={body}",
        );
        assert_eq!(
            body["error"]["param"], "temperature",
            "bad temperature should name the temperature param, body={body}",
        );
        assert_eq!(
            workflow.accepted_count(),
            0,
            "bad temperature {bad_temperature} must not submit to ProductWorkflow"
        );
    }
}

/// POST `/v1/responses` without `temperature` must not fabricate a payload
/// field. Downstream code treats presence as the per-request override signal.
#[tokio::test]
async fn responses_request_without_temperature_omits_payload_field() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let router = test_router(workflow.clone());

    let response = router
        .oneshot(response_create_request(json!({
            "model": "default",
            "input": "hello",
        })))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let submitted = submitted_user_message_json(
        &workflow
            .accepted_envelopes()
            .into_iter()
            .next()
            .expect("accepted envelope"),
    );
    assert!(
        submitted.get("temperature").is_none(),
        "payload must not carry a fabricated temperature when the request body has none: {submitted}"
    );
}

fn test_router(workflow: Arc<FakeProductWorkflow>) -> axum::Router {
    workflow.program_projection_read_resolution(sample_projection_read_request());
    let service = OpenAiResponsesWorkflow::new(
        workflow,
        Arc::new(InMemoryOpenAiCompatRefStore::new()),
        Arc::new(StaticResponsesReader),
    );
    openai_compat_router_with_state(OpenAiCompatRouterState::with_responses(Arc::new(service)))
        .layer(axum::Extension(caller()))
}

fn response_create_request(body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
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

fn submitted_user_message_json(envelope: &ProductInboundEnvelope) -> Value {
    let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
        panic!("expected user message payload");
    };
    serde_json::from_str(&payload.text).expect("submitted payload json")
}

fn caller() -> OpenAiCompatAuthenticatedCaller {
    OpenAiCompatAuthenticatedCaller::new(
        OpenAiCompatActorScope::new(
            TenantId::new("tenant-a").expect("tenant"),
            UserId::new("test-user").expect("user"),
            Some(AgentId::new("agent-a").expect("agent")),
            Some(ProjectId::new("project-a").expect("project")),
        ),
        ProtocolAuthEvidence::test_verified_for_tenant(
            AuthRequirement::BearerToken,
            "test-user",
            TenantId::new("tenant-a").expect("tenant"),
        ),
    )
    .expect("caller")
}

fn sample_projection_read_request() -> ProjectionReadRequest {
    ProjectionReadRequest {
        actor: TurnActor::new(UserId::new("test-user").expect("user")),
        scope: TurnScope::new_with_owner(
            TenantId::new("tenant-a").expect("tenant"),
            Some(AgentId::new("agent-a").expect("agent")),
            Some(ProjectId::new("project-a").expect("project")),
            ThreadId::new("thread-openai-response").expect("thread"),
            Some(UserId::new("test-user").expect("user")),
        ),
        after_cursor: None,
        limit: None,
    }
}

struct StaticResponsesReader;

#[async_trait]
impl OpenAiResponsesProjectionReader for StaticResponsesReader {
    async fn wait_for_response_completion(
        &self,
        request: OpenAiResponseWaitRequest,
    ) -> Result<OpenAiResponseProjection, ironclaw_reborn_openai_compat::OpenAiCompatHttpError>
    {
        Ok(OpenAiResponseProjection::new(completed_response(
            request.public_id,
            request.requested_model,
        ))
        .with_internal_refs(
            OpenAiCompatInternalRefs::new(
                OpenAiCompatProductActionRef::new("product-action:response").expect("action"),
            )
            .with_turn_run_ref(
                OpenAiCompatTurnRunRef::new(TurnRunId::new().to_string()).expect("run"),
            )
            .with_projection_ref(
                OpenAiCompatProjectionRef::new("projection:response").expect("projection"),
            ),
        ))
    }

    async fn read_response(
        &self,
        request: OpenAiResponseReadRequest,
    ) -> Result<OpenAiResponseObject, ironclaw_reborn_openai_compat::OpenAiCompatHttpError> {
        Ok(completed_response(
            request.public_id,
            request
                .requested_model
                .unwrap_or_else(|| "default".to_string()),
        ))
    }
}

fn completed_response(id: OpenAiResponseId, model: String) -> OpenAiResponseObject {
    OpenAiResponseObject {
        id,
        object: "response".to_string(),
        created_at: 1_777_777_777,
        status: OpenAiResponseStatus::Completed,
        model,
        output: vec![OpenAiResponseOutputItem::Message {
            id: "msg_1".to_string(),
            status: Some(OpenAiResponseOutputItemStatus::Completed),
            role: OpenAiResponsesMessageRole::Assistant,
            content: json!([{"type": "output_text", "text": "ok"}]),
        }],
        error: None,
        incomplete_details: None,
        usage: Some(OpenAiResponseUsage {
            input_tokens: 1,
            output_tokens: 1,
            total_tokens: 2,
        }),
    }
}
