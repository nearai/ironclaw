//! Caller-level contract for static security headers and sanitized
//! errors on the WebChat v2 surface.
//!
//! The broader WebUI serve tests already assert that a
//! successful (200) response carries `X-Content-Type-Options: nosniff`,
//! `X-Frame-Options: DENY`, and a `content-security-policy` header
//! (presence only). This file covers what that test does NOT:
//!
//! 1. The static headers are present on an **error** response too — an
//!    unauthenticated 401 still carries them (the `SetResponseHeaderLayer`
//!    is outermost, so error pages are not frameable / sniffable), and
//!    includes `Referrer-Policy: no-referrer` (the `?token=` leak
//!    defense, untested upstream).
//! 2. The **CSP directive content** is locked, not just its presence —
//!    a regression that widened `object-src` or dropped
//!    `frame-ancestors 'none'` would pass a presence-only check.
//! 3. A malformed request body yields a **sanitized client error**
//!    (400) without reaching the facade or leaking internal detail.
//! 4. The per-caller SSE concurrency cap is enforced end-to-end (the
//!    connection-limit row of `02-network-limits.md` previously had only
//!    unit coverage in `sse_capacity.rs`): holding the cap open makes the
//!    next stream open return 429, and releasing a stream frees a slot.
//!
//! Supports the static-security-header + sanitized-error slice of the
//! #3615 WebUI security parity audit, plus the connection-limit backfill
//! for `02-network-limits.md`.

#![cfg(feature = "test-support")]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_webui::EnvBearerAuthenticator;
use ironclaw_webui::{WebuiServeConfig, webui_v2_app};
use secrecy::SecretString;
use tower::ServiceExt;

#[path = "support/harness.rs"]
mod harness;
use harness::{AGENT, PROJECT, StubServices, TENANT, with_peer};

const ENV_TOKEN: &str = "operator-secret-token";

// ─── harness ──────────────────────────────────────────────────────────

fn build_app() -> (axum::Router, Arc<StubServices>) {
    build_app_from(StubServices::default())
}

fn build_app_from(services: StubServices) -> (axum::Router, Arc<StubServices>) {
    let services = Arc::new(services);
    let authenticator = Arc::new(
        EnvBearerAuthenticator::new(
            SecretString::from(ENV_TOKEN.to_string()),
            UserId::new("operator").expect("user"),
        )
        .expect("env bearer authenticator"),
    );
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        authenticator,
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"));
    (
        webui_v2_app(services.clone(), config).expect("webui v2 app"),
        services,
    )
}

fn create_thread_request(bearer: Option<&str>, body: &'static str) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/api/webchat/v2/threads")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    with_peer(builder.body(Body::from(body)).expect("request"))
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

// ─── tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn static_security_headers_present_on_error_response() {
    // The `SetResponseHeaderLayer` stack is outermost, so an
    // unauthenticated 401 must still carry every static header — an error
    // page must not be sniffable, frameable, or referer-leaking just
    // because auth failed.
    let (app, _services) = build_app();
    let response = app
        .oneshot(create_thread_request(None, "{}"))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let headers = response.headers();
    assert_eq!(
        headers
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|v| v.to_str().ok()),
        Some("nosniff"),
    );
    assert_eq!(
        headers
            .get(header::X_FRAME_OPTIONS)
            .and_then(|v| v.to_str().ok()),
        Some("DENY"),
    );
    assert_eq!(
        headers.get("referrer-policy").and_then(|v| v.to_str().ok()),
        Some("no-referrer"),
        "Referrer-Policy must be present even on errors (the `?token=` leak defense)",
    );
    assert!(
        headers.contains_key("content-security-policy"),
        "CSP must be present on error responses too",
    );
}

#[tokio::test]
async fn csp_directives_are_locked() {
    // Presence is not enough — lock the directive content so a
    // regression that widened `object-src` or dropped
    // `frame-ancestors 'none'` fails here rather than silently shipping.
    let (app, _services) = build_app();
    let response = app
        .oneshot(create_thread_request(Some(ENV_TOKEN), "{}"))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let csp = response
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .expect("CSP header present")
        .to_string();
    for directive in [
        "default-src 'self'",
        "object-src 'none'",
        "frame-ancestors 'none'",
        "base-uri 'self'",
    ] {
        assert!(
            csp.contains(directive),
            "CSP must contain `{directive}`; got `{csp}`",
        );
    }
}

#[tokio::test]
async fn malformed_request_body_returns_sanitized_client_error() {
    // A malformed JSON body must yield a clean 4xx (not a 500 / panic),
    // must not reach the facade, and must not leak internal detail
    // (paths, type names, tracebacks) per `.claude/rules/error-handling.md`.
    let (app, services) = build_app();
    let response = app
        .oneshot(create_thread_request(
            Some(ENV_TOKEN),
            "{ this is not valid json",
        ))
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "a malformed body must be a sanitized 400, not a 500",
    );
    assert!(
        services
            .create_thread_callers
            .lock()
            .expect("lock")
            .is_empty(),
        "a malformed body must be rejected before the facade",
    );
    // The body is axum's standard `JsonRejection` text — it does include
    // serde's structural parse position (`line`/`column`), which is not
    // sensitive (see 03-headers-errors.md row 7). What it must NOT carry
    // is any genuinely internal detail: filesystem paths, Rust type or
    // field names, panic/traceback markers, or the configured token.
    let body = body_string(response).await;
    for leak in [
        "/Users/",
        "/home/",
        "src/",
        "panicked",
        "::",
        "ProductCreateThreadRequest",
        "client_action_id",
        ENV_TOKEN,
    ] {
        assert!(
            !body.contains(leak),
            "validation error must not leak `{leak}`; body was `{body}`",
        );
    }
}

#[tokio::test]
async fn panic_boundary_returns_sanitized_500() {
    // Row 9: a handler panic must unwind into `CatchPanicLayer::custom`
    // and return a generic 500 with the detail logged, not echoed. Drive
    // a facade that panics with a sensitive-looking message and assert
    // the response body is exactly the opaque string — no path, SQL,
    // token, or `::` from the panic payload reaches the client — and that
    // the static security headers still ride the 500.
    let sensitive = "/Users/secret/db SELECT token=operator-secret-token";
    let (app, _services) = build_app_from(StubServices {
        create_thread_panic: Some(sensitive),
        ..StubServices::default()
    });
    let response = app
        .oneshot(create_thread_request(Some(ENV_TOKEN), "{}"))
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "a panicking handler must be caught and return 500, not crash the connection",
    );
    // Security headers are applied outside the panic boundary, so the 500
    // is still nosniff / DENY / no-referrer.
    let headers = response.headers().clone();
    assert_eq!(
        headers
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|v| v.to_str().ok()),
        Some("nosniff"),
        "the sanitized 500 must still carry nosniff",
    );
    assert_eq!(
        headers.get("referrer-policy").and_then(|v| v.to_str().ok()),
        Some("no-referrer"),
        "the sanitized 500 must still carry Referrer-Policy",
    );
    let body = body_string(response).await;
    assert_eq!(
        body, "Internal Server Error",
        "the 500 body must be the fixed opaque string, not the panic payload",
    );
    for leak in [
        "/Users/",
        "SELECT",
        "operator-secret-token",
        "panicked",
        "::",
    ] {
        assert!(
            !body.contains(leak),
            "the 500 body must not leak the panic detail `{leak}`; body was `{body}`",
        );
    }
}

fn events_request() -> Request<Body> {
    with_peer(
        Request::builder()
            .method(Method::GET)
            .uri("/api/webchat/v2/threads/t1/events")
            .header(header::AUTHORIZATION, format!("Bearer {ENV_TOKEN}"))
            .body(Body::empty())
            .expect("request"),
    )
}

#[tokio::test]
async fn sse_streams_are_capped_per_caller() {
    // Connection-limit backfill for 02-network-limits.md: the per-caller
    // SSE concurrency cap (default 3 streams per (tenant,user)) is
    // enforced at the route layer. Slots are RAII — a held response keeps
    // its `SseSlot` alive, so holding the cap open forces the next open to
    // 429, and dropping a held stream frees the slot again.
    let (app, _services) = build_app();

    let mut held = Vec::new();
    for i in 0..3 {
        let response = app
            .clone()
            .oneshot(events_request())
            .await
            .expect("oneshot");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "stream {i} within the per-caller cap must open",
        );
        held.push(response);
    }

    let over_cap = app
        .clone()
        .oneshot(events_request())
        .await
        .expect("oneshot");
    assert_eq!(
        over_cap.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the 4th concurrent stream from one caller must be refused with 429",
    );

    // Releasing the held streams frees their slots; a new open succeeds.
    drop(held);
    drop(over_cap);
    let after_release = app.oneshot(events_request()).await.expect("oneshot");
    assert_eq!(
        after_release.status(),
        StatusCode::OK,
        "a slot must free once a held stream is dropped",
    );
}
