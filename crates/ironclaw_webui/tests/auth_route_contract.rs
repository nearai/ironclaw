//! Caller-level auth contract for the WebChat v2 surface.
//!
//! `session_round_trip.rs` already locks the OAuth → session-mint →
//! protected-route → logout path. This file covers the gaps that path
//! does not exercise, all by driving the composed `webui_v2_app`
//! `Router` through `tower::ServiceExt::oneshot` (per
//! `.claude/rules/testing.md` — test through the caller, not just the
//! authenticator's `authenticate()` in isolation):
//!
//! 1. `EnvBearerAuthenticator` (the standalone CLI / single-operator
//!    deployment) accept/reject on a protected v2 route.
//! 2. Missing / malformed `Authorization` headers collapse to `401`
//!    without ever reaching the facade.
//! 3. `Bearer` prefix parsing is case-insensitive — parity with v1's
//!    `auth.rs` extractor (documented as a KEEP in
//!    `docs/reborn/security-parity/01-auth.md`).
//! 4. A session revoked directly through `SessionStore::revoke` stops
//!    authenticating, isolated from the OAuth round-trip.
//! 5. An expired session is rejected at the route layer (the
//!    `session.rs` unit test only covers `authenticate()` in isolation).
//! 6. `?token=` is rejected on the WebSocket upgrade route — the
//!    WS-specific half of the v1 query-token narrowing, locked directly
//!    rather than inferred from the POST-mutation rejection.
//! 7. An `ironclaw_session` cookie carrying a valid bearer does not
//!    authenticate — the v2 cookie-transport beta-break (#4116).
//!
//! Supports the authentication slice of the #3615 WebUI security parity
//! audit.

#![cfg(feature = "dev-in-memory-session")]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use chrono::Duration as ChronoDuration;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_reborn_composition::{RebornReadiness, RebornWebuiBundle};
use ironclaw_webui::{
    EnvBearerAuthenticator, InMemorySessionStore, OidcAuthenticator, OidcAuthenticatorConfig,
    SessionAuthenticator, SessionStore,
};
use ironclaw_webui::{WebuiAuthenticator, WebuiServeConfig, webui_v2_app};
use secrecy::{ExposeSecret, SecretString};
use tower::ServiceExt;

// OIDC route-layer test scaffolding (loopback JWKS server + signed JWTs).
use axum::Json;
use axum::routing::get;
use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rsa::pkcs8::{EncodePrivateKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde_json::json;

#[path = "support/harness.rs"]
mod harness;
use harness::{AGENT, PROJECT, StubServices, TENANT, with_peer};

const ENV_TOKEN: &str = "operator-secret-token";
const ENV_USER: &str = "operator-user";

// ─── harness ──────────────────────────────────────────────────────────

/// Compose `webui_v2_app` with the supplied authenticator and no public
/// route mount (these tests never exercise the OAuth login surface —
/// they inject bearers / sessions directly). Returns the router plus the
/// facade stub so callers can assert whether a request reached the
/// handler.
fn compose(authenticator: Arc<dyn WebuiAuthenticator>) -> (axum::Router, Arc<StubServices>) {
    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        api: services.clone(),
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        authenticator,
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"));
    let app = webui_v2_app(bundle, config).expect("webui v2 app");
    (app, services)
}

fn env_bearer_app() -> (axum::Router, Arc<StubServices>) {
    let authenticator = Arc::new(
        EnvBearerAuthenticator::new(
            SecretString::from(ENV_TOKEN.to_string()),
            UserId::new(ENV_USER).expect("user"),
        )
        .expect("env bearer authenticator"),
    );
    compose(authenticator)
}

fn session_app() -> (axum::Router, Arc<StubServices>, Arc<InMemorySessionStore>) {
    let store: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let authenticator = Arc::new(SessionAuthenticator::new(store.clone()));
    let (app, services) = compose(authenticator);
    (app, services, store)
}

/// `POST /api/webchat/v2/threads` with an optional raw `Authorization`
/// header value, so a test can supply a malformed value verbatim
/// (`"Bearer "`, `"bearer <tok>"`, a bare token, …) rather than only a
/// well-formed bearer.
fn create_thread_request(raw_authorization: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/api/webchat/v2/threads")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(value) = raw_authorization {
        builder = builder.header(header::AUTHORIZATION, value);
    }
    with_peer(
        builder
            .body(Body::from(r#"{"client_action_id":"act-1"}"#))
            .expect("request"),
    )
}

// ─── env-bearer tests ─────────────────────────────────────────────────

#[tokio::test]
async fn env_bearer_valid_token_authenticates_protected_route() {
    let (app, services) = env_bearer_app();
    let response = app
        .oneshot(create_thread_request(Some(&format!("Bearer {ENV_TOKEN}"))))
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "valid env bearer must authenticate on the v2 surface",
    );
    let callers = services.create_thread_callers.lock().expect("lock");
    assert_eq!(callers.len(), 1, "facade reached exactly once");
    assert_eq!(
        callers[0].tenant_id.as_str(),
        TENANT,
        "tenant comes from trusted host config, not the request",
    );
    assert_eq!(
        callers[0].user_id.as_str(),
        ENV_USER,
        "authenticator-resolved user_id must be stamped on the caller",
    );
}

#[tokio::test]
async fn env_bearer_wrong_token_rejected() {
    let (app, services) = env_bearer_app();
    let response = app
        .oneshot(create_thread_request(Some("Bearer not-the-token")))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        services
            .create_thread_callers
            .lock()
            .expect("lock")
            .is_empty(),
        "facade must not be reached when the bearer is wrong",
    );
}

#[tokio::test]
async fn missing_authorization_header_rejected() {
    let (app, services) = env_bearer_app();
    let response = app
        .oneshot(create_thread_request(None))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        services
            .create_thread_callers
            .lock()
            .expect("lock")
            .is_empty()
    );
}

#[tokio::test]
async fn bearer_with_empty_token_rejected() {
    // `Authorization: Bearer ` (prefix only, no token) extracts an empty
    // token, which the constant-time compare rejects. This is the case
    // `EnvBearerAuthenticator::new` guards against by forbidding an empty
    // configured token — verified here end-to-end at the route layer.
    let (app, services) = env_bearer_app();
    let response = app
        .oneshot(create_thread_request(Some("Bearer ")))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        services
            .create_thread_callers
            .lock()
            .expect("lock")
            .is_empty()
    );
}

#[tokio::test]
async fn bearer_prefix_is_case_insensitive_parity_with_v1() {
    // v1's `extract_token` matches the `Bearer ` prefix case-insensitively
    // (auth.rs); v2's `extract_bearer_token` does the same via
    // `eq_ignore_ascii_case("Bearer ")`. Locking the lowercase form keeps
    // this a documented KEEP in 01-auth.md rather than silent drift.
    let (app, services) = env_bearer_app();
    let response = app
        .oneshot(create_thread_request(Some(&format!("bearer {ENV_TOKEN}"))))
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "lowercase `bearer` prefix must be accepted (v1 parity)",
    );
    assert_eq!(
        services.create_thread_callers.lock().expect("lock").len(),
        1
    );
}

#[tokio::test]
async fn bearer_without_prefix_rejected() {
    // A bare token with no `Bearer ` prefix is not a valid credential —
    // it falls through to the `?token=` shim, which is not honored on a
    // POST mutation, so the request is unauthenticated.
    let (app, services) = env_bearer_app();
    let response = app
        .oneshot(create_thread_request(Some(ENV_TOKEN)))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        services
            .create_thread_callers
            .lock()
            .expect("lock")
            .is_empty()
    );
}

// ─── session revoke / expiry tests ────────────────────────────────────

#[tokio::test]
async fn revoked_session_bearer_rejected() {
    // Isolates revoke-then-reject from the OAuth round-trip: mint a
    // session directly, confirm it authenticates, revoke it, confirm the
    // SAME bearer no longer authenticates.
    let (app, services, store) = session_app();
    let bearer = store
        .create_session(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new("session-user").expect("user"),
            ChronoDuration::hours(1),
        )
        .await
        .expect("create_session")
        .expose_secret()
        .to_string();

    let authed = app
        .clone()
        .oneshot(create_thread_request(Some(&format!("Bearer {bearer}"))))
        .await
        .expect("oneshot");
    assert_eq!(
        authed.status(),
        StatusCode::OK,
        "freshly minted session must authenticate",
    );
    assert_eq!(callers_len(&services), 1);

    store.revoke(&bearer).await.expect("revoke");
    assert_eq!(store.len(), 0, "revoke must drop the session");

    let after_revoke = app
        .oneshot(create_thread_request(Some(&format!("Bearer {bearer}"))))
        .await
        .expect("oneshot");
    assert_eq!(
        after_revoke.status(),
        StatusCode::UNAUTHORIZED,
        "revoked session bearer must NOT authenticate",
    );
    assert_eq!(
        callers_len(&services),
        1,
        "facade must not be reached after revoke",
    );
}

#[tokio::test]
async fn expired_session_bearer_rejected_on_route() {
    // The `session.rs` unit test checks expiry inside `authenticate()`;
    // this drives the full route to confirm an expired session yields a
    // 401 at the gateway, not just inside the authenticator.
    let (app, services, store) = session_app();
    let bearer = store
        .create_session(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new("session-user").expect("user"),
            // Already expired: `SessionRecord::is_expired` is `now >=
            // expires_at`, so a negative lifetime is unambiguously past.
            ChronoDuration::seconds(-1),
        )
        .await
        .expect("create_session")
        .expose_secret()
        .to_string();

    let response = app
        .oneshot(create_thread_request(Some(&format!("Bearer {bearer}"))))
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "expired session bearer must be rejected at the route layer",
    );
    assert!(
        services
            .create_thread_callers
            .lock()
            .expect("lock")
            .is_empty()
    );
}

fn callers_len(services: &Arc<StubServices>) -> usize {
    services.create_thread_callers.lock().expect("lock").len()
}

#[tokio::test]
async fn session_minted_for_one_tenant_does_not_authenticate_another_deployment() {
    // v2 isolates tenants two ways: each deployment owns a separate
    // `SessionStore`, and `caller.tenant_id` is always stamped from host
    // config — never from the bearer. A session minted against tenant-a's
    // store must therefore fail on a tenant-b deployment backed by its own
    // (different) store: the lookup misses and the bearer is rejected. If
    // the tenant binding were ever loosened to trust a shared store, this
    // would catch it.
    let tenant_a_store: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let bearer = tenant_a_store
        .create_session(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new("session-user").expect("user"),
            ChronoDuration::hours(1),
        )
        .await
        .expect("create_session")
        .expose_secret()
        .to_string();

    // A distinct deployment: tenant-b, its own empty store.
    let tenant_b_store: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let authenticator = Arc::new(SessionAuthenticator::new(tenant_b_store.clone()));
    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        api: services.clone(),
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new("tenant-b").expect("tenant"),
        authenticator,
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"));
    let app = webui_v2_app(bundle, config).expect("webui v2 app");

    let response = app
        .oneshot(with_peer(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"client_action_id":"act-1"}"#))
                .expect("request"),
        ))
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a tenant-a session must not authenticate on a tenant-b deployment's store",
    );
    assert!(
        services
            .create_thread_callers
            .lock()
            .expect("lock")
            .is_empty(),
        "a cross-deployment bearer must never reach the facade",
    );
    assert_eq!(
        tenant_a_store.len(),
        1,
        "the tenant-a session stays intact in its own store",
    );
}

// ─── query-token (`?token=`) shim ─────────────────────────────────────

/// `GET /api/webchat/v2/threads/{id}/events` with an optional `?token=`
/// query param and no `Authorization` header. The SSE event stream is
/// the one route where the browser's `EventSource` (which cannot set
/// headers) is allowed to authenticate via the query string.
fn sse_events_request(token: Option<&str>) -> Request<Body> {
    let uri = match token {
        Some(token) => format!("/api/webchat/v2/threads/t1/events?token={token}"),
        None => "/api/webchat/v2/threads/t1/events".to_string(),
    };
    with_peer(
        Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::empty())
            .expect("request"),
    )
}

#[tokio::test]
async fn query_token_honored_on_sse_events_route() {
    // v1 allowed `?token=` on three GET routes; v2 keeps the escape
    // hatch on exactly one — the SSE event stream — because
    // `EventSource` cannot set an `Authorization` header.
    let (app, services, store) = session_app();
    let bearer = store
        .create_session(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new("session-user").expect("user"),
            ChronoDuration::hours(1),
        )
        .await
        .expect("create_session")
        .expose_secret()
        .to_string();

    let no_token = app
        .clone()
        .oneshot(sse_events_request(None))
        .await
        .expect("oneshot");
    assert_eq!(
        no_token.status(),
        StatusCode::UNAUTHORIZED,
        "the SSE route still requires a credential when none is supplied",
    );

    let with_token = app
        .oneshot(sse_events_request(Some(&bearer)))
        .await
        .expect("oneshot");
    assert_eq!(
        with_token.status(),
        StatusCode::OK,
        "a valid `?token=` must authenticate the SSE event stream",
    );

    // Identity binding: the `?token=` value must be consumed as the
    // session credential and stamped as that user — not merely yield a
    // 200. A mis-wire that left the route auth-gated but stamped a
    // default/empty caller would still 200. The SSE body is a lazy
    // stream, so the facade's `stream_events` only runs once the body is
    // polled — drive frames briefly so it runs at least once (the
    // drain-poll loop may call it several times; assert on identity, not
    // count).
    use http_body_util::BodyExt;
    let mut body = with_token.into_body();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while body.frame().await.is_some() {}
    })
    .await;

    let callers = services.stream_events_callers.lock().expect("lock");
    assert!(
        !callers.is_empty(),
        "the authenticated SSE request must reach the facade",
    );
    assert_eq!(
        callers[0].user_id.as_str(),
        "session-user",
        "the `?token=` session must be resolved to its owning user_id",
    );
    assert_eq!(
        callers[0].tenant_id.as_str(),
        TENANT,
        "tenant comes from trusted host config, not the query token",
    );
}

#[tokio::test]
async fn query_token_wrong_token_rejected_on_sse_route() {
    // The `?token=` shim must reject a wrong-but-non-empty token with a
    // 401, exactly like the bearer header path. A regression that
    // short-circuited an unknown query token to an unauthenticated
    // default caller (instead of 401) would 200 with an empty stream and
    // silently leak thread-id existence. Pairs with the no-token and
    // valid-token cases in `query_token_honored_on_sse_events_route`.
    let (app, services, _store) = session_app();
    let response = app
        .oneshot(sse_events_request(Some("wrong-but-non-empty-token")))
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a wrong `?token=` must 401, not fall through to an unauthenticated 200",
    );
    assert!(
        services
            .stream_events_callers
            .lock()
            .expect("lock")
            .is_empty(),
        "a rejected `?token=` must never reach the facade",
    );
}

#[tokio::test]
async fn expired_query_token_rejected_on_sse_route() {
    // The `?token=` shim must honor session expiry at the route layer,
    // not just the `Authorization: Bearer` path. If the shim were ever
    // widened or the expiry check skipped on the query path, an expired
    // session could keep streaming over `?token=` while bearer-header
    // tests still pass. Mint a session that is already expired and
    // present it through the query string.
    let (app, services, store) = session_app();
    let expired_bearer = store
        .create_session(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new("session-user").expect("user"),
            ChronoDuration::seconds(-1),
        )
        .await
        .expect("create_session")
        .expose_secret()
        .to_string();

    let response = app
        .oneshot(sse_events_request(Some(&expired_bearer)))
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "an expired session presented via `?token=` must be rejected at the route layer",
    );
    assert!(
        services
            .stream_events_callers
            .lock()
            .expect("lock")
            .is_empty(),
        "an expired `?token=` must never reach the facade",
    );
}

#[tokio::test]
async fn query_token_rejected_on_mutation_route() {
    // The security-critical half of the narrowing: `?token=` must NOT
    // authenticate a state-changing route, so a stale referer/URL leak
    // cannot drive a mutation. The token rides the query string and no
    // `Authorization` header is sent.
    let (app, services, store) = session_app();
    let bearer = store
        .create_session(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new("session-user").expect("user"),
            ChronoDuration::hours(1),
        )
        .await
        .expect("create_session")
        .expose_secret()
        .to_string();

    let request = with_peer(
        Request::builder()
            .method(Method::POST)
            .uri(format!("/api/webchat/v2/threads?token={bearer}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"client_action_id":"act-1"}"#))
            .expect("request"),
    );
    let response = app.oneshot(request).await.expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "`?token=` must not authenticate a mutation route",
    );
    assert_eq!(callers_len(&services), 0, "facade must not be reached");
}

/// `GET /api/webchat/v2/threads/{id}/ws` (the WebSocket upgrade route)
/// with an optional `?token=` and no `Authorization` header. A matching
/// `Origin` is supplied so the same-origin middleware (which runs before
/// auth) passes and the verdict isolates to the auth layer. `t1` matches
/// the SSE helper's thread id.
fn ws_events_request(token: Option<&str>) -> Request<Body> {
    let uri = match token {
        Some(token) => format!("/api/webchat/v2/threads/t1/ws?token={token}"),
        None => "/api/webchat/v2/threads/t1/ws".to_string(),
    };
    with_peer(
        Request::builder()
            .method(Method::GET)
            .uri(uri)
            // Origin matches Host so the WS same-origin middleware (which
            // runs before auth, and requires a Host/canonical_host to
            // compare against) passes — isolating the verdict to auth.
            .header(header::HOST, "localhost:1234")
            .header(header::ORIGIN, "http://localhost:1234")
            .body(Body::empty())
            .expect("request"),
    )
}

#[tokio::test]
async fn query_token_rejected_on_websocket_route() {
    // v1 allowed `?token=` on three GET routes including the WS stream
    // (`/api/chat/ws`); v2 drops it everywhere except the SSE event
    // stream. This is a WS-specific beta-break, so lock it directly
    // rather than inferring it from the POST-mutation rejection: a VALID
    // session token on the query string of the WS upgrade, with no
    // `Authorization` header, must NOT authenticate.
    let (app, _services, store) = session_app();
    let bearer = store
        .create_session(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new("session-user").expect("user"),
            ChronoDuration::hours(1),
        )
        .await
        .expect("create_session")
        .expose_secret()
        .to_string();

    let response = app
        .oneshot(ws_events_request(Some(&bearer)))
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "`?token=` must not authenticate the WebSocket route (v1 WS query-token exception dropped)",
    );
}

// ─── cookie session is not a v2 credential ────────────────────────────

#[tokio::test]
async fn cookie_session_not_honored_on_protected_route() {
    // v1 accepted an `ironclaw_session` cookie as a credential and set
    // one on the OAuth callback; v2 never reads or writes cookies (#4116).
    // A request carrying a VALID session bearer in the `ironclaw_session`
    // cookie — and no `Authorization` header — must be rejected, proving
    // the cookie *transport* is dropped (not merely an invalid value).
    let (app, services, store) = session_app();
    let bearer = store
        .create_session(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new("session-user").expect("user"),
            ChronoDuration::hours(1),
        )
        .await
        .expect("create_session")
        .expose_secret()
        .to_string();

    let request = with_peer(
        Request::builder()
            .method(Method::POST)
            .uri("/api/webchat/v2/threads")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, format!("ironclaw_session={bearer}"))
            .body(Body::from(r#"{"client_action_id":"act-1"}"#))
            .expect("request"),
    );
    let response = app.oneshot(request).await.expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "an `ironclaw_session` cookie must not authenticate a v2 route (cookie transport dropped)",
    );
    assert_eq!(callers_len(&services), 0, "facade must not be reached");
}

// ─── operator-config mounting boundary ────────────────────────────────

/// `GET /api/webchat/v2/llm/providers` with no auth. Whether this route
/// exists at all depends on the authenticator's
/// `allows_operator_webui_config()`; sending no credential lets us read
/// the verdict as 401 (mounted, behind bearer auth) vs 404 (not
/// mounted) without invoking the facade.
fn llm_providers_unauthenticated() -> Request<Body> {
    with_peer(
        Request::builder()
            .method(Method::GET)
            .uri("/api/webchat/v2/llm/providers")
            .body(Body::empty())
            .expect("request"),
    )
}

#[tokio::test]
async fn operator_config_route_mounted_for_operator_authenticator() {
    // `EnvBearerAuthenticator` is a single trusted operator
    // (`allows_operator_webui_config() == true`), so the operator-only
    // LLM config routes are mounted and sit behind bearer auth.
    let (app, _services) = env_bearer_app();
    let response = app
        .oneshot(llm_providers_unauthenticated())
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "operator route must be mounted (behind auth) for an operator authenticator",
    );
}

#[tokio::test]
async fn operator_config_route_absent_for_multi_user_authenticator() {
    // `SessionAuthenticator` is multi-user
    // (`allows_operator_webui_config() == false`), so operator-only LLM
    // config routes must NOT be mounted — there is no admin boundary yet.
    let (app, _services, _store) = session_app();
    let response = app
        .oneshot(llm_providers_unauthenticated())
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "operator route must be absent for a multi-user authenticator",
    );
}

// ─── OIDC at the route layer ──────────────────────────────────────────
//
// `oidc_e2e.rs` exercises `OidcAuthenticator::authenticate()` in
// isolation. This composes the authenticator INTO `webui_v2_app` and
// drives a protected route, so a regression in the route-layer wiring
// (not just the verifier) is caught: a JWKS-signed token authenticates,
// and tampered claims / expiry collapse to 401 at the gateway.

const OIDC_ISSUER: &str = "https://issuer.test";
const OIDC_AUDIENCE: &str = "test-audience";
const OIDC_KID: &str = "route-test-key";

/// Fresh RSA keypair: PKCS#8 PEM (for signing) + public key (for the JWK).
fn generate_oidc_key() -> (String, RsaPublicKey) {
    let mut rng = rand_core::OsRng;
    // 2048-bit is required, not just preferred: `jsonwebtoken` rejects
    // smaller RSA keys at sign time with `InvalidRsaKey("TooSmall")`, so
    // a faster 1024-bit test key is not an option here.
    let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa gen");
    let pem = private
        .to_pkcs8_pem(LineEnding::LF)
        .expect("pkcs8 pem")
        .to_string();
    (pem, RsaPublicKey::from(&private))
}

fn oidc_jwk(public: &RsaPublicKey, kid: &str) -> serde_json::Value {
    let n = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(public.n().to_bytes_be());
    let e = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(public.e().to_bytes_be());
    json!({ "kty": "RSA", "alg": "RS256", "use": "sig", "kid": kid, "n": n, "e": e })
}

/// Serve a static JWKS document on loopback. Returns the URL and a join
/// handle the test aborts when done.
async fn spawn_jwks_server(jwk: serde_json::Value) -> (String, tokio::task::JoinHandle<()>) {
    let keys = json!({ "keys": [jwk] });
    let router = axum::Router::new().route(
        "/jwks",
        get(move || {
            let keys = keys.clone();
            async move { Json(keys) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{addr}/jwks"), handle)
}

fn sign_oidc_token(pem: &str, kid: &str, issuer: &str, audience: &str, expires_in: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = json!({
        "iss": issuer,
        "sub": "alice",
        "aud": audience,
        "exp": now + expires_in,
        "iat": now,
    });
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.to_string());
    encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(pem.as_bytes()).expect("encoding key"),
    )
    .expect("sign jwt")
}

fn oidc_app(jwks_url: String) -> (axum::Router, Arc<StubServices>) {
    let config = OidcAuthenticatorConfig::new(OIDC_ISSUER, OIDC_AUDIENCE, jwks_url);
    let authenticator = Arc::new(
        OidcAuthenticator::new(config, OidcAuthenticator::sub_claim_mapper())
            .expect("oidc authenticator"),
    );
    compose(authenticator)
}

#[tokio::test]
async fn oidc_signed_token_authenticates_protected_route_and_bad_claims_rejected() {
    let (pem, public) = generate_oidc_key();
    let (jwks_url, server) = spawn_jwks_server(oidc_jwk(&public, OIDC_KID)).await;
    let (app, services) = oidc_app(jwks_url);

    // Valid JWKS-signed token → authenticates; `sub` maps to the caller.
    let valid = sign_oidc_token(&pem, OIDC_KID, OIDC_ISSUER, OIDC_AUDIENCE, 600);
    let ok = app
        .clone()
        .oneshot(create_thread_request(Some(&format!("Bearer {valid}"))))
        .await
        .expect("oneshot");
    assert_eq!(
        ok.status(),
        StatusCode::OK,
        "valid OIDC token must authenticate on the v2 surface",
    );
    {
        let callers = services.create_thread_callers.lock().expect("lock");
        assert_eq!(callers.len(), 1);
        assert_eq!(
            callers[0].user_id.as_str(),
            "alice",
            "the `sub` claim must map onto the caller's user_id",
        );
    }

    // Wrong issuer → 401.
    let wrong_iss = sign_oidc_token(&pem, OIDC_KID, "https://attacker.test", OIDC_AUDIENCE, 600);
    let iss = app
        .clone()
        .oneshot(create_thread_request(Some(&format!("Bearer {wrong_iss}"))))
        .await
        .expect("oneshot");
    assert_eq!(iss.status(), StatusCode::UNAUTHORIZED, "wrong iss → 401");

    // Wrong audience → 401.
    let wrong_aud = sign_oidc_token(&pem, OIDC_KID, OIDC_ISSUER, "wrong-audience", 600);
    let aud = app
        .clone()
        .oneshot(create_thread_request(Some(&format!("Bearer {wrong_aud}"))))
        .await
        .expect("oneshot");
    assert_eq!(aud.status(), StatusCode::UNAUTHORIZED, "wrong aud → 401");

    // Expired → 401.
    let expired = sign_oidc_token(&pem, OIDC_KID, OIDC_ISSUER, OIDC_AUDIENCE, -60);
    let exp = app
        .oneshot(create_thread_request(Some(&format!("Bearer {expired}"))))
        .await
        .expect("oneshot");
    assert_eq!(exp.status(), StatusCode::UNAUTHORIZED, "expired → 401");

    assert_eq!(
        callers_len(&services),
        1,
        "only the valid token may reach the facade",
    );
    server.abort();
}

#[tokio::test]
async fn oidc_hs256_token_rejected_on_route() {
    // Algorithm-confusion (CVE-class JWT bypass): sign an HS256 token
    // using the RSA *public* modulus as the HMAC secret, with a `kid`
    // matching the JWKS key. A verifier that doesn't pin the algorithm
    // would verify the MAC with the public key and forge a valid caller.
    // The RS/ES-only allowlist must reject it at the gateway, and the
    // facade must never be reached. (row 5 — locks the highest-value
    // OIDC control, which RS256-only tests cannot exercise.)
    let (_pem, public) = generate_oidc_key();
    let (jwks_url, server) = spawn_jwks_server(oidc_jwk(&public, OIDC_KID)).await;
    let (app, services) = oidc_app(jwks_url);

    let now = chrono::Utc::now().timestamp();
    let claims = json!({
        "iss": OIDC_ISSUER,
        "sub": "attacker",
        "aud": OIDC_AUDIENCE,
        "exp": now + 600,
        "iat": now,
    });
    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some(OIDC_KID.to_string());
    let forged = encode(
        &header,
        &claims,
        &EncodingKey::from_secret(public.n().to_bytes_be().as_slice()),
    )
    .expect("sign hs256");

    let response = app
        .oneshot(create_thread_request(Some(&format!("Bearer {forged}"))))
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "HS256 (alg-confusion) token must be rejected at the route layer",
    );
    assert_eq!(
        callers_len(&services),
        0,
        "the forged token must never reach the facade",
    );
    server.abort();
}

#[tokio::test]
async fn oidc_not_yet_valid_nbf_token_rejected_on_route() {
    // row 5 lists `nbf` as a validated claim. A correctly-signed token
    // with valid iss/aud/exp but a not-before in the future must collapse
    // to 401 at the gateway (not just inside `authenticate()`), and the
    // facade must not be reached.
    let (pem, public) = generate_oidc_key();
    let (jwks_url, server) = spawn_jwks_server(oidc_jwk(&public, OIDC_KID)).await;
    let (app, services) = oidc_app(jwks_url);

    let now = chrono::Utc::now().timestamp();
    let claims = json!({
        "iss": OIDC_ISSUER,
        "sub": "alice",
        "aud": OIDC_AUDIENCE,
        "exp": now + 600,
        "iat": now,
        "nbf": now + 3600,
    });
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(OIDC_KID.to_string());
    let token = encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(pem.as_bytes()).expect("encoding key"),
    )
    .expect("sign jwt");

    let response = app
        .oneshot(create_thread_request(Some(&format!("Bearer {token}"))))
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a token whose nbf is in the future must be rejected at the route layer",
    );
    assert_eq!(callers_len(&services), 0, "facade must not be reached");
    server.abort();
}

// ─── 401 body sanitization (no reason leak) ───────────────────────────

#[tokio::test]
async fn unauthorized_body_is_generic_and_leaks_no_reason() {
    // row 9 records failure sanitization as a KEEP: every auth failure
    // collapses to a fixed generic 401 and the reason is never echoed.
    // Every other 401 test asserts only the status; this collects the
    // body and asserts it is the fixed string with no leaked detail
    // (token value, configured user, expiry/backend cause), so a
    // regression that interpolated the rejection reason fails loudly.
    use axum::body::to_bytes;

    let (app, _services) = env_bearer_app();
    let response = app
        .oneshot(create_thread_request(Some("Bearer not-the-token")))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&body);
    assert_eq!(
        text, "Invalid or missing auth token",
        "401 body must be the fixed sanitized string",
    );
    for leak in [
        "not-the-token",
        ENV_TOKEN,
        ENV_USER,
        "expired",
        "Database",
        "session",
    ] {
        assert!(
            !text.contains(leak),
            "401 body must not leak `{leak}`; got: {text}",
        );
    }
}
