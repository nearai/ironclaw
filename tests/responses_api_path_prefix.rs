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

const AUTH_TOKEN: &str = "test-responses-api-token";

async fn start_test_server() -> (SocketAddr, Arc<GatewayState>) {
    let state = TestGatewayBuilder::new().user_id("test-user").build();
    let auth = MultiAuthState::single(AUTH_TOKEN.to_string(), "test-user".to_string());
    let addr: SocketAddr = "127.0.0.1:0"
        .parse()
        .expect("hard-coded address must parse");
    let bound = start_server(addr, state.clone(), auth.into())
        .await
        .expect("start gateway test server");
    (bound, state)
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
    let (addr, _state) = start_test_server().await;
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
    let (addr, _state) = start_test_server().await;
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
    let (addr, _state) = start_test_server().await;
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
    let (addr, _state) = start_test_server().await;
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
    let (addr, _state) = start_test_server().await;

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

/// Both GET item paths (`/api/v1/responses/{id}` and `/v1/responses/{id}`)
/// must also enforce bearer-token auth. A missing token should return 401,
/// not 404 — the auth middleware has to apply to legacy aliases as well.
#[tokio::test]
async fn both_get_paths_require_auth() {
    let (addr, _state) = start_test_server().await;

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
