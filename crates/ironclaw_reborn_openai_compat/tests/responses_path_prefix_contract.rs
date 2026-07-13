#![cfg(feature = "openai-compat-beta")]

//! Regression tests for the OpenAI Responses API route prefix
//! (see ironclaw#2201).
//!
//! The canonical path is `/api/v1/responses`; the legacy `/v1/responses`
//! path is retained as an alias for backward compatibility. Both must reach
//! the Reborn OpenAI-compatible router. These tests intentionally drive the
//! router rather than the retired v1 gateway router, which no longer owns the
//! Responses API surface.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use axum::response::Response;
use http_body_util::BodyExt;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_adapters::{AuthRequirement, FakeProductWorkflow, ProtocolAuthEvidence};
use ironclaw_reborn_openai_compat::{
    InMemoryOpenAiCompatRefStore, OpenAiCompatActorScope, OpenAiCompatAuthenticatedCaller,
    OpenAiCompatInternalRefs, OpenAiCompatProductActionRef, OpenAiCompatProjectionRef,
    OpenAiCompatRouterState, OpenAiCompatTurnRunRef, OpenAiResponseId, OpenAiResponseObject,
    OpenAiResponseOutputItem, OpenAiResponseOutputItemStatus, OpenAiResponseProjection,
    OpenAiResponseReadRequest, OpenAiResponseStatus, OpenAiResponseUsage,
    OpenAiResponseWaitRequest, OpenAiResponsesMessageRole, OpenAiResponsesProjectionReader,
    OpenAiResponsesWorkflow, openai_compat_router_with_state,
};
use ironclaw_turns::TurnRunId;
use serde_json::{Value, json};
use tower::ServiceExt;

const AUTH_TOKEN: &str = "test-responses-api-token";

/// POST `/api/v1/responses` must route to the Reborn Responses handler, not
/// the router fallback. An empty model is rejected by the handler with a JSON
/// OpenAI-compatible validation error, proving the route exists.
#[tokio::test]
async fn canonical_post_path_routes_to_handler() {
    let response = send(
        Method::POST,
        "/api/v1/responses",
        Some(AUTH_TOKEN),
        Some(json!({
            "model": "",
            "input": "hello",
        })),
    )
    .await;

    assert_json_error(response, StatusCode::BAD_REQUEST, Some("model")).await;
}

/// Legacy alias `POST /v1/responses` must still route to the same handler.
#[tokio::test]
async fn legacy_post_path_still_routes_to_handler() {
    let response = send(
        Method::POST,
        "/v1/responses",
        Some(AUTH_TOKEN),
        Some(json!({
            "model": "",
            "input": "hello",
        })),
    )
    .await;

    assert_json_error(response, StatusCode::BAD_REQUEST, Some("model")).await;
}

/// GET `/api/v1/responses/{id}` with a malformed id must return the handler's
/// JSON error, not an empty router fallback 404.
#[tokio::test]
async fn canonical_get_path_routes_to_handler() {
    let response = send(
        Method::GET,
        "/api/v1/responses/not_a_valid_id",
        Some(AUTH_TOKEN),
        None,
    )
    .await;

    assert_json_error(response, StatusCode::NOT_FOUND, Some("response_id")).await;
}

/// GET `/v1/responses/{id}` (legacy alias) must also route to the handler.
#[tokio::test]
async fn legacy_get_path_still_routes_to_handler() {
    let response = send(
        Method::GET,
        "/v1/responses/not_a_valid_id",
        Some(AUTH_TOKEN),
        None,
    )
    .await;

    assert_json_error(response, StatusCode::NOT_FOUND, Some("response_id")).await;
}

/// Both create paths must enforce an authenticated caller before processing.
#[tokio::test]
async fn both_paths_require_auth() {
    for path in ["/api/v1/responses", "/v1/responses"] {
        let response = send(
            Method::POST,
            path,
            None,
            Some(json!({ "model": "default", "input": "hi" })),
        )
        .await;
        assert_json_error(response, StatusCode::UNAUTHORIZED, None).await;
    }
}

/// A malformed external-tool declaration must be rejected by request
/// validation. The important prefix regression is that the request reaches the
/// Responses validator rather than a route fallback.
#[tokio::test]
async fn missing_tool_name_returns_validation_error() {
    let response = send(
        Method::POST,
        "/api/v1/responses",
        Some(AUTH_TOKEN),
        Some(json!({
            "model": "default",
            "input": "hi",
            "tools": [
                {"type": "function", "description": "nameless"}
            ]
        })),
    )
    .await;

    assert_json_error(response, StatusCode::BAD_REQUEST, Some("tools")).await;
}

/// Unsupported tool types must also be rejected by the Responses validator.
#[tokio::test]
async fn unsupported_tool_type_returns_validation_error() {
    let response = send(
        Method::POST,
        "/api/v1/responses",
        Some(AUTH_TOKEN),
        Some(json!({
            "model": "default",
            "input": "hi",
            "tools": [
                {"type": "web_search", "name": "search"}
            ]
        })),
    )
    .await;

    assert_json_error(response, StatusCode::BAD_REQUEST, Some("tools")).await;
}

/// `instructions` is part of the Responses API request shape. It must not be
/// rejected as an unknown field; the empty model should be the validation error.
#[tokio::test]
async fn instructions_field_is_accepted() {
    let response = send(
        Method::POST,
        "/api/v1/responses",
        Some(AUTH_TOKEN),
        Some(json!({
            "model": "",
            "input": "hi",
            "instructions": "You are a terse assistant. Always reply in one sentence.",
        })),
    )
    .await;

    let body = assert_json_error(response, StatusCode::BAD_REQUEST, Some("model")).await;
    assert!(
        !body.to_string().contains("instructions"),
        "instructions must not be the rejection reason, got: {body}"
    );
}

/// A `function_call_output` continuation without an authorized previous
/// response must be rejected by the handler, not missed by the router.
#[tokio::test]
async fn resume_without_pending_gate_returns_400() {
    let fake_prev = format!("resp_{}{}", "0".repeat(32), "1".repeat(32));
    let response = send(
        Method::POST,
        "/api/v1/responses",
        Some(AUTH_TOKEN),
        Some(json!({
            "model": "default",
            "previous_response_id": fake_prev,
            "input": [
                {
                    "type": "function_call_output",
                    "call_id": "call_made_up",
                    "output": "irrelevant"
                }
            ]
        })),
    )
    .await;

    assert_json_error(response, StatusCode::BAD_REQUEST, Some("input")).await;
}

/// Caller-supplied external tools are fail-closed until the host wires the
/// external-tool store/resume ports. Shadowing against host capabilities is
/// enforced later by composition when that surface is enabled.
#[tokio::test]
async fn caller_supplied_tools_are_rejected_when_external_tools_are_unwired() {
    let response = send(
        Method::POST,
        "/api/v1/responses",
        Some(AUTH_TOKEN),
        Some(json!({
            "model": "default",
            "input": "hi",
            "tools": [
                {
                    "type": "function",
                    "name": "stand_in_tool",
                    "description": "shadow attempt",
                    "parameters": {"type": "object"}
                }
            ]
        })),
    )
    .await;

    assert_json_error(response, StatusCode::BAD_REQUEST, Some("tools")).await;
}

/// Both GET item paths must also enforce authentication before id parsing.
#[tokio::test]
async fn both_get_paths_require_auth() {
    for path in [
        "/api/v1/responses/not_a_valid_id",
        "/v1/responses/not_a_valid_id",
    ] {
        let response = send(Method::GET, path, None, None).await;
        assert_json_error(response, StatusCode::UNAUTHORIZED, None).await;
    }
}

async fn send(
    method: Method,
    path: &str,
    auth_token: Option<&str>,
    body: Option<Value>,
) -> Response {
    let mut router = test_router();
    // Host auth middleware injects this extension before the route fragment.
    // The composition mount test covers the bearer-token middleware itself.
    if auth_token == Some(AUTH_TOKEN) {
        router = router.layer(axum::Extension(caller()));
    }

    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = auth_token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let body = if let Some(body) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };

    router
        .oneshot(builder.body(body).expect("request"))
        .await
        .expect("route response")
}

fn test_router() -> axum::Router {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let service = OpenAiResponsesWorkflow::new(
        workflow,
        Arc::new(InMemoryOpenAiCompatRefStore::new()),
        Arc::new(StaticResponsesReader),
    );
    openai_compat_router_with_state(OpenAiCompatRouterState::with_responses(Arc::new(service)))
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

async fn assert_json_error(
    response: Response,
    expected_status: StatusCode,
    expected_param: Option<&str>,
) -> Value {
    assert_eq!(
        response.status(),
        expected_status,
        "expected {expected_status}, got {}",
        response.status()
    );
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body: Value = serde_json::from_slice(&body).expect("json error body");
    assert_eq!(
        body["error"]["param"].as_str(),
        expected_param,
        "unexpected error body: {body}"
    );
    body
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
