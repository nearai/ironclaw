//! Doc-fact contract: `docs/api/responses.mdx` matches the real request
//! policy.
//!
//! The page carries a machine-readable `doc-fact:responses-request-policy`
//! marker block (invisible when rendered); this test parses it and drives
//! every claim through the same route-level seam as the sibling
//! `*_contract.rs` suites. The marker's values parameterize the assertions
//! (e.g. accept `temperature_max`, reject just above it), so editing the
//! doc's claims without matching code behavior fails, and vice versa.

use std::collections::BTreeMap;
use std::sync::Arc;

mod support;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_host_api::product_adapter::auth::{AuthRequirement, ProtocolAuthEvidence};
use ironclaw_openai_compat::{
    OpenAiCompatActorScope, OpenAiCompatAuthenticatedCaller, OpenAiCompatExternalToolResume,
    OpenAiCompatExternalToolResumeRequest, OpenAiCompatExternalToolSpec,
    OpenAiCompatExternalToolStore, OpenAiCompatHttpError, OpenAiCompatInternalRefs,
    OpenAiCompatProductActionRef, OpenAiCompatProjectionRef, OpenAiCompatRouterState,
    OpenAiCompatTurnRunRef, OpenAiResponseId, OpenAiResponseObject, OpenAiResponseOutputItem,
    OpenAiResponseOutputItemStatus, OpenAiResponseProjection, OpenAiResponseReadRequest,
    OpenAiResponseStatus, OpenAiResponseUsage, OpenAiResponseWaitRequest,
    OpenAiResponsesMessageRole, OpenAiResponsesProjectionReader, OpenAiResponsesWorkflow,
    openai_compat_router_with_state,
};
use ironclaw_turns::TurnRunId;
use serde_json::{Value, json};
use std::sync::Mutex;
use support::{FakeProductSurface, in_memory_openai_compat_ref_store};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Marker parsing
// ---------------------------------------------------------------------------

const MARKER_NAME: &str = "doc-fact:responses-request-policy";

fn repo_root() -> std::path::PathBuf {
    let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if dir.join("docs/api/responses.mdx").is_file() {
            return dir;
        }
        assert!(
            dir.pop(),
            "walked out of the filesystem without finding docs/api/responses.mdx"
        );
    }
}

fn doc_facts() -> BTreeMap<String, String> {
    let page = repo_root().join("docs/api/responses.mdx");
    let text = std::fs::read_to_string(&page)
        .unwrap_or_else(|error| panic!("read {}: {error}", page.display()));
    let start = text
        .find(&format!("{{/* {MARKER_NAME}"))
        .unwrap_or_else(|| panic!("{} lost its `{MARKER_NAME}` marker block", page.display()));
    let block = &text[start..];
    let end = block
        .find("*/}")
        .unwrap_or_else(|| panic!("unterminated `{MARKER_NAME}` marker block"));
    let mut facts = BTreeMap::new();
    for line in block[..end].lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        facts.insert(key.trim().to_string(), value.trim().to_string());
    }
    assert!(
        !facts.is_empty(),
        "`{MARKER_NAME}` marker block parsed to zero facts"
    );
    facts
}

fn fact(facts: &BTreeMap<String, String>, key: &str) -> String {
    facts
        .get(key)
        .unwrap_or_else(|| panic!("marker block is missing `{key}`"))
        .clone()
}

// ---------------------------------------------------------------------------
// The behavioral checks, parameterized by the marker's values
// ---------------------------------------------------------------------------

/// Every field the doc's table documents must be accepted in one request.
/// The sample map is this test's half of the contract: a field added to the
/// doc without a sample here fails loudly, and a sample the router rejects
/// fails the request.
#[tokio::test]
async fn every_documented_request_field_is_accepted() {
    let facts = doc_facts();
    let conditional: Vec<String> = fact(&facts, "rejected_without_external_tools")
        .split(',')
        .map(|field| field.trim().to_string())
        .collect();

    let workflow = Arc::new(FakeProductSurface::new());
    let router = test_router(workflow.clone());

    // `previous_response_id` is resolved at create, so its sample must be a
    // real prior response, seeded through the same router.
    let seed = router
        .clone()
        .oneshot(response_create_request(
            json!({"model": "default", "input": "seed"}),
        ))
        .await
        .expect("seed response");
    assert_eq!(seed.status(), StatusCode::OK);
    let seed_id = json_body(seed).await["id"]
        .as_str()
        .expect("seed response id")
        .to_string();

    let samples: BTreeMap<&str, Value> = [
        ("model", json!("default")),
        ("input", json!("hello")),
        ("stream", json!(false)),
        ("temperature", json!(1.0)),
        ("instructions", json!("be brief")),
        ("previous_response_id", json!(seed_id)),
        ("metadata", json!({"k": "v"})),
        ("tools", json!([])),
        ("tool_choice", Value::Null),
        ("x_context", json!({"env": {"k": "v"}})),
    ]
    .into_iter()
    .collect();

    let mut body = serde_json::Map::new();
    for field in fact(&facts, "request_fields").split(',') {
        let field = field.trim();
        let sample = samples.get(field).unwrap_or_else(|| {
            panic!(
                "doc marker lists request field `{field}` with no sample in this \
                 test — add one (and make sure the router really accepts it)"
            )
        });
        // Conditionally-rejected fields stay out; the dedicated tests below
        // prove their acceptance with external tools wired.
        if sample.is_null() || conditional.iter().any(|c| c == field) {
            continue;
        }
        body.insert(field.to_string(), sample.clone());
    }

    let response = router
        .oneshot(response_create_request(Value::Object(body)))
        .await
        .expect("response");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a request carrying every documented field must be accepted"
    );
    assert_eq!(workflow.accepted_count(), 2);
}

/// `tool_choice` behaves like `tools`: 400 on the plain router, accepted
/// and ignored when external tools are wired (there is no per-request
/// tool-choice surface).
#[tokio::test]
async fn tool_choice_rejects_without_external_wiring_and_is_ignored_with_it() {
    let facts = doc_facts();
    let conditional: Vec<String> = fact(&facts, "rejected_without_external_tools")
        .split(',')
        .map(|field| field.trim().to_string())
        .collect();
    assert_eq!(
        conditional,
        vec!["tools".to_string(), "tool_choice".to_string()],
        "the doc's conditionally-rejected set changed; update the dedicated \
         behavioral tests to cover the new set"
    );

    // Without external-tool wiring: rejected, naming the param.
    let workflow = Arc::new(FakeProductSurface::new());
    let router = test_router(workflow.clone());
    let response = router
        .oneshot(response_create_request(json!({
            "model": "default",
            "input": "hello",
            "tool_choice": "auto",
        })))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["error"]["param"], "tool_choice");
    assert_eq!(workflow.accepted_count(), 0);

    // With external-tool wiring: accepted and ignored — the request submits,
    // and nothing registers because no tools were supplied.
    let workflow = Arc::new(FakeProductSurface::new());
    let store = Arc::new(RecordingExternalToolStore::default());
    let router = test_router_with_external_tools(workflow.clone(), store.clone());
    let response = router
        .oneshot(response_create_request(json!({
            "model": "default",
            "input": "hello",
            "tool_choice": "auto",
        })))
        .await
        .expect("response");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "tool_choice must be accepted (and ignored) when external tools are wired"
    );
    assert_eq!(workflow.accepted_count(), 1);
    assert_eq!(store.registered_count(), 0);
}

#[tokio::test]
async fn tools_reject_without_external_wiring_and_accept_with_it() {
    let facts = doc_facts();
    assert!(
        fact(&facts, "rejected_without_external_tools")
            .split(',')
            .any(|field| field.trim() == "tools"),
        "the doc no longer lists `tools` as conditionally rejected"
    );
    let tools_body = json!({
        "model": "default",
        "input": "hello",
        "tools": [{
            "type": "function",
            "name": "lookup",
            "description": "Look something up.",
            "parameters": {"type": "object", "properties": {}},
        }],
    });

    // Without external-tool wiring: rejected, naming the param.
    let workflow = Arc::new(FakeProductSurface::new());
    let router = test_router(workflow.clone());
    let response = router
        .oneshot(response_create_request(tools_body.clone()))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["error"]["param"], "tools");
    assert_eq!(workflow.accepted_count(), 0);

    // An empty tools array is treated as omitted on the same router.
    let workflow = Arc::new(FakeProductSurface::new());
    let router = test_router(workflow.clone());
    let response = router
        .oneshot(response_create_request(json!({
            "model": "default",
            "input": "hello",
            "tools": [],
        })))
        .await
        .expect("response");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "empty tools array must be treated as omitted"
    );

    // With external-tool wiring: accepted and registered.
    let workflow = Arc::new(FakeProductSurface::new());
    let store = Arc::new(RecordingExternalToolStore::default());
    let router = test_router_with_external_tools(workflow.clone(), store.clone());
    let response = router
        .oneshot(response_create_request(tools_body))
        .await
        .expect("response");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "non-empty tools must be accepted when external tools are wired"
    );
    assert_eq!(workflow.accepted_count(), 1);
    assert_eq!(
        store.registered_count(),
        1,
        "accepted caller tools must be registered for the run"
    );
}

#[tokio::test]
async fn temperature_boundary_matches_the_documented_range() {
    let facts = doc_facts();
    let min: f64 = fact(&facts, "temperature_min").parse().expect("min");
    let max: f64 = fact(&facts, "temperature_max").parse().expect("max");

    for accepted in [min, max] {
        let workflow = Arc::new(FakeProductSurface::new());
        let router = test_router(workflow.clone());
        let response = router
            .oneshot(response_create_request(json!({
                "model": "default",
                "input": "hello",
                "temperature": accepted,
            })))
            .await
            .expect("response");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "documented boundary temperature {accepted} must be accepted"
        );
    }
    for rejected in [min - 0.1, max + 0.1] {
        let workflow = Arc::new(FakeProductSurface::new());
        let router = test_router(workflow.clone());
        let response = router
            .oneshot(response_create_request(json!({
                "model": "default",
                "input": "hello",
                "temperature": rejected,
            })))
            .await
            .expect("response");
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "temperature {rejected} outside the documented range must reject"
        );
        assert_eq!(json_body(response).await["error"]["param"], "temperature");
    }
}

#[tokio::test]
async fn model_accepts_any_well_formed_name_up_to_the_documented_byte_cap() {
    let facts = doc_facts();
    let max_bytes: usize = fact(&facts, "model_max_bytes").parse().expect("cap");

    let at_cap = "m".repeat(max_bytes);
    for accepted in ["my-custom-model", at_cap.as_str()] {
        let workflow = Arc::new(FakeProductSurface::new());
        let router = test_router(workflow.clone());
        let response = router
            .oneshot(response_create_request(json!({
                "model": accepted,
                "input": "hello",
            })))
            .await
            .expect("response");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "well-formed model name of {} bytes must be accepted",
            accepted.len()
        );
    }

    let over_cap = "m".repeat(max_bytes + 1);
    let workflow = Arc::new(FakeProductSurface::new());
    let router = test_router(workflow.clone());
    let response = router
        .oneshot(response_create_request(json!({
            "model": over_cap,
            "input": "hello",
        })))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["error"]["param"], "model");
}

#[tokio::test]
async fn unknown_fields_are_accepted_and_ignored() {
    let facts = doc_facts();
    assert_eq!(
        fact(&facts, "ignored_unknown_fields"),
        "true",
        "the doc claims unknown fields are tolerated; if that policy changes, \
         change the DTO and this marker together"
    );

    let workflow = Arc::new(FakeProductSurface::new());
    let router = test_router(workflow.clone());
    let response = router
        .oneshot(response_create_request(json!({
            "model": "default",
            "input": "hello",
            "max_output_tokens": 12345,
        })))
        .await
        .expect("response");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "unknown OpenAI fields (e.g. max_output_tokens) must be accepted and ignored"
    );
    assert_eq!(workflow.accepted_count(), 1);
}

// ---------------------------------------------------------------------------
// Router wiring (the same seam as the sibling *_contract.rs suites)
// ---------------------------------------------------------------------------

fn test_router(workflow: Arc<FakeProductSurface>) -> axum::Router {
    let service = OpenAiResponsesWorkflow::new(
        workflow,
        in_memory_openai_compat_ref_store(),
        Arc::new(StaticResponsesReader),
    );
    openai_compat_router_with_state(OpenAiCompatRouterState::with_responses(Arc::new(service)))
        .layer(axum::Extension(caller()))
}

fn test_router_with_external_tools(
    workflow: Arc<FakeProductSurface>,
    store: Arc<RecordingExternalToolStore>,
) -> axum::Router {
    let service = OpenAiResponsesWorkflow::new(
        workflow,
        in_memory_openai_compat_ref_store(),
        Arc::new(StaticResponsesReader),
    )
    .with_external_tools(store, Arc::new(NoopExternalToolResume));
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

#[derive(Default)]
struct RecordingExternalToolStore {
    registered: Mutex<usize>,
}

impl RecordingExternalToolStore {
    fn registered_count(&self) -> usize {
        *self.registered.lock().expect("registered lock")
    }
}

#[async_trait]
impl OpenAiCompatExternalToolStore for RecordingExternalToolStore {
    async fn register_tools(
        &self,
        _run_ref: OpenAiCompatTurnRunRef,
        _specs: Vec<OpenAiCompatExternalToolSpec>,
    ) -> Result<(), OpenAiCompatHttpError> {
        *self.registered.lock().expect("registered lock") += 1;
        Ok(())
    }

    async fn submit_tool_output(
        &self,
        _run_ref: OpenAiCompatTurnRunRef,
        _call_id: String,
        _output: Value,
    ) -> Result<(), OpenAiCompatHttpError> {
        Ok(())
    }
}

struct NoopExternalToolResume;

#[async_trait]
impl OpenAiCompatExternalToolResume for NoopExternalToolResume {
    async fn resume_external_tool_run(
        &self,
        _request: OpenAiCompatExternalToolResumeRequest,
    ) -> Result<(), OpenAiCompatHttpError> {
        Ok(())
    }
}

struct StaticResponsesReader;

#[async_trait]
impl OpenAiResponsesProjectionReader for StaticResponsesReader {
    async fn wait_for_response_completion(
        &self,
        request: OpenAiResponseWaitRequest,
    ) -> Result<OpenAiResponseProjection, OpenAiCompatHttpError> {
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
    ) -> Result<OpenAiResponseObject, OpenAiCompatHttpError> {
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
            input_tokens_details: None,
            cost: None,
            input_tokens: 1,
            output_tokens: 1,
            total_tokens: 2,
        }),
    }
}
