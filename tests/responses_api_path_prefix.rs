//! Regression tests for the OpenAI Responses API route prefix
//! (see ironclaw#2201).
//!
//! The canonical path is `/api/v1/responses`; the legacy `/v1/responses`
//! path is retained as an alias for backward compatibility. Both must
//! reach `create_response_handler` / `get_response_handler` and produce
//! identical behavior.
//!
//! These tests drive the full router via `start_server` rather than
//! calling the handler in isolation — per `.claude/rules/testing.md`
//! ("Test Through the Caller, Not Just the Helper"), the regression
//! coverage has to exercise the router wiring, otherwise a future
//! rename / removal of one path silently loses the coverage.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ironclaw::channels::web::auth::MultiAuthState;
use ironclaw::channels::web::platform::router::start_server;
use ironclaw::channels::web::platform::state::GatewayState;
use ironclaw::channels::web::test_helpers::TestGatewayBuilder;
use tokio::sync::oneshot;

const AUTH_TOKEN: &str = "test-responses-api-token";

/// RAII guard that shuts the gateway test server down when dropped,
/// even on early returns or panics. Without this, every `#[tokio::test]`
/// would leak its spawned `axum::serve` task for the remainder of the
/// test process.
struct ServerGuard {
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            // Best-effort: the receiver may already be gone if the
            // serve task exited for another reason. Either way, we've
            // released our half of the channel.
            let _ = tx.send(());
        }
    }
}

async fn start_test_server() -> (SocketAddr, Arc<GatewayState>, ServerGuard) {
    let state = TestGatewayBuilder::new().user_id("test-user").build();
    let auth = MultiAuthState::single(AUTH_TOKEN.to_string(), "test-user".to_string());
    let addr: SocketAddr = "127.0.0.1:0"
        .parse()
        .expect("hard-coded address must parse");
    let bound = start_server(addr, state.clone(), auth.into())
        .await
        .expect("start gateway test server");
    let shutdown = state.shutdown_tx.write().await.take();
    (bound, state, ServerGuard { shutdown })
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("build test http client")
}

/// POST `/api/v1/responses` must route to `create_response_handler` —
/// not 404. We send a deliberately invalid `model` so the handler
/// short-circuits with 400 before touching the agent loop; the important
/// assertion is "the route exists".
#[tokio::test]
async fn canonical_post_path_routes_to_handler() {
    let (addr, _state, _guard) = start_test_server().await;
    let url = format!("http://{}/api/v1/responses", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "not-a-real-model",
            "input": "hello",
        }))
        .send()
        .await
        .expect("POST /api/v1/responses");

    // The handler rejects non-"default" models with 400, which proves the
    // request reached `create_response_handler` rather than the router's
    // fallback 404. A 404 here would mean the route isn't registered.
    assert_eq!(
        resp.status(),
        400,
        "expected 400 from handler, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
}

/// Legacy alias `POST /v1/responses` must still route to the same
/// handler (backward compatibility with clients that were configured
/// against the pre-#2201 path).
#[tokio::test]
async fn legacy_post_path_still_routes_to_handler() {
    let (addr, _state, _guard) = start_test_server().await;
    let url = format!("http://{}/v1/responses", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "not-a-real-model",
            "input": "hello",
        }))
        .send()
        .await
        .expect("POST /v1/responses");

    assert_eq!(
        resp.status(),
        400,
        "legacy path must reach handler, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
}

/// GET `/api/v1/responses/{id}` with a malformed id must return 400
/// from the handler (invalid response ID) — proving the route is
/// registered and the path parameter is reaching the handler.
#[tokio::test]
async fn canonical_get_path_routes_to_handler() {
    let (addr, _state, _guard) = start_test_server().await;
    let url = format!("http://{}/api/v1/responses/not_a_valid_id", addr);

    let resp = client()
        .get(&url)
        .bearer_auth(AUTH_TOKEN)
        .send()
        .await
        .expect("GET /api/v1/responses/{id}");

    assert_eq!(
        resp.status(),
        400,
        "expected 400 from handler for malformed id, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
}

/// GET `/v1/responses/{id}` (legacy alias) must also route to the same
/// handler.
#[tokio::test]
async fn legacy_get_path_still_routes_to_handler() {
    let (addr, _state, _guard) = start_test_server().await;
    let url = format!("http://{}/v1/responses/not_a_valid_id", addr);

    let resp = client()
        .get(&url)
        .bearer_auth(AUTH_TOKEN)
        .send()
        .await
        .expect("GET /v1/responses/{id}");

    assert_eq!(
        resp.status(),
        400,
        "legacy path must reach handler, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
}

/// Both paths must enforce bearer-token auth. A missing token should
/// return 401, not 404 (which would indicate the route is missing).
#[tokio::test]
async fn both_paths_require_auth() {
    let (addr, _state, _guard) = start_test_server().await;

    for path in ["/api/v1/responses", "/v1/responses"] {
        let url = format!("http://{}{}", addr, path);
        let resp = client()
            .post(&url)
            .json(&serde_json::json!({ "model": "default", "input": "hi" }))
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST {path}: {e}"));
        assert_eq!(
            resp.status(),
            401,
            "{path} should return 401 without a token, got {}",
            resp.status()
        );
    }
}

/// `tools: [{type: "function", ...}]` is the externally-provided-tools
/// surface. POST handler must accept the field instead of rejecting with
/// 400 — the agent's reply is delivered asynchronously, but the request
/// validation has to clear. We use an obviously bad tool definition
/// (missing `name`) to assert the dedicated 400 path: this proves both
/// "the tools field is parsed" and "validation kicks in".
#[tokio::test]
async fn missing_tool_name_returns_validation_error() {
    let (addr, _state, _guard) = start_test_server().await;
    let url = format!("http://{}/api/v1/responses", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "default",
            "input": "hi",
            "tools": [
                {"type": "function", "description": "nameless"}
            ]
        }))
        .send()
        .await
        .expect("POST /api/v1/responses with malformed tool");

    assert_eq!(
        resp.status(),
        400,
        "expected 400 from external-tool validator, got {}",
        resp.status()
    );
    let body = resp.text().await.unwrap_or_default();
    assert!(
        body.contains("name"),
        "validation error should mention the missing 'name' field, got: {body}"
    );
}

/// Unsupported tool types (e.g. `web_search`) must be rejected by the
/// validator with a clear 400 — not silently accepted, since the engine
/// doesn't honour them.
#[tokio::test]
async fn unsupported_tool_type_returns_validation_error() {
    let (addr, _state, _guard) = start_test_server().await;
    let url = format!("http://{}/api/v1/responses", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "default",
            "input": "hi",
            "tools": [
                {"type": "web_search", "name": "search"}
            ]
        }))
        .send()
        .await
        .expect("POST /api/v1/responses with unsupported tool type");

    assert_eq!(resp.status(), 400);
    let body = resp.text().await.unwrap_or_default();
    assert!(
        body.contains("web_search"),
        "validation error should mention the unsupported tool type, got: {body}"
    );
}

/// `instructions` is a per-request system/developer message (OpenAI Responses
/// API spec). The handler used to reject it with 400; it must now accept it
/// and route the request into the agent loop. We assert the request clears
/// the synchronous validation gate by asking for a malformed `model` so the
/// handler short-circuits with a 400 whose message is about `model`, not
/// about `instructions`. A 400 mentioning `instructions` would mean the
/// rejection regressed.
#[tokio::test]
async fn instructions_field_is_accepted() {
    let (addr, _state, _guard) = start_test_server().await;
    let url = format!("http://{}/api/v1/responses", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "not-a-real-model",
            "input": "hi",
            "instructions": "You are a terse assistant. Always reply in one sentence.",
        }))
        .send()
        .await
        .expect("POST /api/v1/responses with instructions");

    assert_eq!(resp.status(), 400);
    let body = resp.text().await.unwrap_or_default();
    assert!(
        !body.contains("instructions"),
        "instructions must no longer be rejected, got: {body}"
    );
    assert!(
        body.contains("Model selection"),
        "expected the model rejection to be the reason for 400, got: {body}"
    );
}

/// External tools require engine v2: when the global flag is off the
/// handler must reject the request with a clear 4xx instead of silently
/// degrading. The test gateway boots without `ENGINE_V2=true`, so a
/// well-formed `tools[]` payload exercises this branch.
#[tokio::test]
async fn external_tools_rejected_when_engine_v2_disabled() {
    // Belt-and-braces: explicitly clear the env var so this test passes
    // even if some earlier test left it set in the same process.
    // SAFETY: `set_var` is unsafe because it mutates process global state
    // shared across threads. This test is a #[tokio::test] and the env
    // var is only consulted synchronously by `is_engine_v2_enabled`, so
    // racing with other tests' reads is acceptable here.
    unsafe {
        std::env::remove_var("ENGINE_V2");
    }

    let (addr, _state, _guard) = start_test_server().await;
    let url = format!("http://{}/api/v1/responses", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "default",
            "input": "hello",
            "tools": [
                {"type": "function", "name": "lookup", "parameters": {"type": "object"}}
            ]
        }))
        .send()
        .await
        .expect("POST /api/v1/responses with tools and ENGINE_V2 off");

    assert_eq!(
        resp.status(),
        400,
        "expected 400 when ENGINE_V2 is off, got {}",
        resp.status()
    );
    let body = resp.text().await.unwrap_or_default();
    assert!(
        body.contains("ENGINE_V2"),
        "rejection should mention ENGINE_V2, got: {body}"
    );
}

/// `function_call_output` items are a resume signal: they must be
/// matched against a pending external-tool gate for the resolved
/// thread. Without one (e.g. because the caller fabricates a
/// `previous_response_id` or the gate already expired), the handler
/// must reject with 400 instead of silently sending the resume into a
/// fresh thread.
#[tokio::test]
async fn resume_without_pending_gate_returns_400() {
    let (addr, _state, _guard) = start_test_server().await;
    let url = format!("http://{}/api/v1/responses", addr);

    // Synthesize a wire-valid previous_response_id (resp_<32hex><32hex>)
    // that names a thread the gateway has never seen. The handler
    // accepts the format, looks for a pending gate, finds none, and
    // must respond 400 — not silently drop the function_call_output
    // and start a fresh turn against the thread.
    let fake_prev = format!("resp_{}{}", "0".repeat(32), "1".repeat(32));

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "default",
            "previous_response_id": fake_prev,
            "input": [
                {
                    "type": "function_call_output",
                    "call_id": "call_made_up",
                    "output": "irrelevant"
                }
            ]
        }))
        .send()
        .await
        .expect("POST /api/v1/responses resume w/o pending gate");

    assert_eq!(
        resp.status(),
        400,
        "expected 400 for resume without pending gate, got {}",
        resp.status()
    );
    let body = resp.text().await.unwrap_or_default();
    assert!(
        body.contains("pending"),
        "rejection should mention the missing pending gate, got: {body}"
    );
}

/// Both GET item paths (`/api/v1/responses/{id}` and `/v1/responses/{id}`)
/// must also enforce bearer-token auth. A missing token should return 401,
/// not 404 — the auth middleware has to apply to legacy aliases as well.
#[tokio::test]
async fn both_get_paths_require_auth() {
    let (addr, _state, _guard) = start_test_server().await;

    for path in [
        "/api/v1/responses/not_a_valid_id",
        "/v1/responses/not_a_valid_id",
    ] {
        let url = format!("http://{}{}", addr, path);
        let resp = client()
            .get(&url)
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert_eq!(
            resp.status(),
            401,
            "{path} should return 401 without a token, got {}",
            resp.status()
        );
    }
}
