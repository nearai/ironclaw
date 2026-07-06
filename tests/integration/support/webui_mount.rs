//! Shared axum mounting + request helpers for the real `webui_v2_router`
//! over a real `RebornServices` facade (W5-WEBUI-API-1). Mirrors
//! `webui_v2_router_smoke.rs::smoke_router`'s shape; auth bypassed by
//! injecting `WebUiAuthenticatedCaller` directly instead of the bearer middleware.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use ironclaw_product_workflow::{RebornServicesApi, ResolvedBinding, WebUiAuthenticatedCaller};
use ironclaw_webui_v2::{
    DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2Capabilities, WebUiV2State, webui_v2_router,
};
use serde_json::Value;
use tower::ServiceExt;

/// Build the `WebUiAuthenticatedCaller` resolving to the SAME
/// `TurnScope`/`ThreadScope` owner a harness turn ran under.
/// `subject_user_id` is the execution-scope user; falls back to
/// `actor_user_id` for legacy bindings without one.
pub(crate) fn webui_caller_for(binding: &ResolvedBinding) -> WebUiAuthenticatedCaller {
    let user_id = binding
        .subject_user_id
        .clone()
        .unwrap_or_else(|| binding.actor_user_id.clone());
    WebUiAuthenticatedCaller::new(
        binding.tenant_id.clone(),
        user_id,
        binding.agent_id.clone(),
        binding.project_id.clone(),
    )
}

/// Mount the real WebUI v2 router over `services`, with `caller` injected as
/// the authenticated-caller `Extension` (mirrors production's bearer
/// middleware output — bypassed here since this tier tests the
/// facade/router contract, not HTTP auth).
pub(crate) fn mount_webui_v2_router(
    services: Arc<dyn RebornServicesApi>,
    caller: WebUiAuthenticatedCaller,
) -> Router {
    webui_v2_router(WebUiV2State::new(
        services,
        DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
    ))
    .layer(axum::Extension(caller))
    .layer(axum::Extension(WebUiV2Capabilities::default()))
}

/// `GET path` against `router`, returning the status and parsed JSON body
/// (`Value::Null` for an empty body).
pub(crate) async fn get_json(router: Router, path: &str) -> (StatusCode, Value) {
    let (status, _headers, bytes) = get_raw(router, path).await;
    (status, parse_json_or_null(&bytes))
}

/// `POST path` with a JSON `body` against `router`, returning the status and
/// parsed JSON response body.
pub(crate) async fn post_json(router: Router, path: &str, body: Value) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, parse_json_or_null(&bytes))
}

/// `GET path` against `router`, returning the status, response headers, and
/// raw body bytes — for non-JSON responses (e.g. served attachment bytes).
pub(crate) async fn get_raw(router: Router, path: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, headers, bytes.to_vec())
}

fn parse_json_or_null(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(bytes).unwrap_or_else(|err| {
            panic!(
                "response body is not valid JSON ({err}): {}",
                String::from_utf8_lossy(bytes)
            )
        })
    }
}
