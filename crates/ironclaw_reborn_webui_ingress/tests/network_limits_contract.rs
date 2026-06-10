//! Caller-level network-control contract for the WebChat v2 surface,
//! focused on the gaps that the composition crate's `webui_v2_serve.rs`
//! and the OAuth-route tests do NOT already cover — the rules that ride
//! on the **host-owned public SSO mount** (`webui_v2_auth_router`) plus
//! the CORS fail-closed default.
//!
//! Already locked elsewhere (cross-referenced, not duplicated here):
//! CORS allow / reject-with-configured-origin, descriptor body-limit 413
//! and rate-limit 429 on the v2 facade routes, and WebSocket
//! same-origin 403 all live in
//! `ironclaw_reborn_composition/tests/webui_v2_serve.rs`; OAuth CSRF
//! state single-use, cross-provider replay, and redirect sanitization
//! live in `google_oauth_routes.rs` / `pending.rs`.
//!
//! This file adds, by driving the composed `webui_v2_app` through
//! `tower::ServiceExt::oneshot`:
//!
//! 1. The public SSO routes inherit the descriptor-driven **per-IP**
//!    rate limit (`/auth/login/{provider}` → 429 after the 60/60s
//!    budget) — a distinct scope from the facade's per-caller limiter —
//!    and that the `PerIp` scope keys each distinct peer IP to its own
//!    independent budget (one IP's flood cannot deny another).
//! 2. The SSO body caps: `POST /auth/session/exchange` and
//!    `POST /auth/logout` both reject oversized bodies (→ 413 before the
//!    handler runs) while a body exactly at the 1 KiB cap is accepted
//!    (guarding the `>=` vs `>` boundary).
//! 3. An empty CORS allow-list fails closed — no `Access-Control-Allow-
//!    Origin` is echoed for either a cross-origin preflight or a simple
//!    (non-preflighted) request.
//!
//! Deliberately NOT covered here: rate-limit **window reset** after the
//! budget is exhausted. The limiter reads wall-clock `SystemTime::now()`
//! with no injectable clock seam, so verifying recovery would require
//! either a 60-second sleep or a production refactor — both out of scope
//! for this test slice. Tracked as a follow-up.
//!
//! Supports the CSRF/origin/CORS + body/rate/connection-limit slice of
//! the #3615 WebUI security parity audit.

#![cfg(feature = "dev-in-memory-session")]

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use ironclaw_host_api::{AgentId, ProjectId, TenantId};
use ironclaw_reborn_composition::{
    RebornReadiness, RebornWebuiBundle, WebuiServeConfig, webui_v2_app,
};
use ironclaw_reborn_webui_ingress::{
    EmailUserDirectory, InMemorySessionStore, OAuthError, OAuthProvider, OAuthProviderName,
    OAuthRouterConfig, OAuthUserProfile, SessionAuthenticator, SessionStore, webui_v2_auth_router,
};
use tower::ServiceExt;

#[path = "support/harness.rs"]
mod harness;
use harness::{AGENT, PROJECT, StubServices, TENANT, with_peer, with_peer_addr};

const PROVIDER: &str = "google";

// ─── stub OAuth provider ──────────────────────────────────────────────

/// Minimal provider so `/auth/login/{provider}` resolves and mints a
/// pending flow + redirect. `exchange_code` is never reached by these
/// tests. Mirrors the stub in `session_round_trip.rs`.
struct StubProvider {
    name: OAuthProviderName,
}

impl StubProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            name: OAuthProviderName::new(PROVIDER).expect("name"),
        })
    }
}

#[async_trait]
impl OAuthProvider for StubProvider {
    fn name(&self) -> &OAuthProviderName {
        &self.name
    }
    fn authorization_url(&self, callback_url: &str, state: &str, _challenge: &str) -> String {
        format!(
            "https://accounts.google.test/o/oauth2/v2/auth?redirect_uri={}&state={}",
            urlencoding::encode(callback_url),
            urlencoding::encode(state),
        )
    }
    async fn exchange_code(
        &self,
        _code: &str,
        _callback_url: &str,
        _verifier: &str,
    ) -> Result<OAuthUserProfile, OAuthError> {
        unreachable!("network-limit tests do not drive the OAuth callback")
    }
}

// ─── harness ──────────────────────────────────────────────────────────

/// Compose `webui_v2_app` with a session authenticator plus the public
/// SSO mount, parameterized on the CORS allow-list so the fail-closed
/// case can pass an empty list.
fn build_app(allowed_origins: Vec<HeaderValue>) -> axum::Router {
    let session_store: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let authenticator = Arc::new(SessionAuthenticator::new(session_store.clone()));

    let oauth_mount = webui_v2_auth_router(OAuthRouterConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        session_store as Arc<dyn SessionStore>,
        Arc::new(EmailUserDirectory),
        vec![StubProvider::new() as Arc<dyn OAuthProvider>],
        "https://gateway.example",
    ));

    let bundle = RebornWebuiBundle {
        api: Arc::new(StubServices::default()),
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        authenticator,
        allowed_origins,
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
    .with_public_route_mount(oauth_mount);
    webui_v2_app(bundle, config).expect("webui v2 app")
}

fn default_origins() -> Vec<HeaderValue> {
    vec![HeaderValue::from_static("http://localhost:1234")]
}

fn login_builder() -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(format!("/auth/login/{PROVIDER}?redirect_after=%2Fv2"))
        .body(Body::empty())
        .expect("request")
}

fn login_request() -> Request<Body> {
    with_peer(login_builder())
}

fn login_request_from(addr: SocketAddr) -> Request<Body> {
    with_peer_addr(login_builder(), addr)
}

// ─── tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn sso_login_enforces_per_ip_rate_limit() {
    // The public SSO mount declares `RateLimitScope::PerIp` at 60 req /
    // 60s on `/auth/login/{provider}` (a different scope from the v2
    // facade's per-caller limiter). A single IP must be cut off after
    // the budget so an unauthenticated login flood is bounded.
    //
    // Two implementation properties this test relies on, both held by
    // `webui_rate_limit::build_rate_limit_state` (`shards: Arc::new(..)`)
    // and verified by the 429 below: (1) the rate-limit counter is
    // Arc-backed inside the router, so the 60 `app.clone()` calls share
    // ONE window — if `Router::clone` reset the counter, all 61 would
    // redirect and this test would never reach 429; (2) the state is
    // built per `webui_v2_app` call (not a process-global), so this
    // test's budget is independent of other tests / run order. `i` is
    // bound into `login_request` via a fresh builder each iteration, so
    // the only shared state is the limiter itself.
    let app = build_app(default_origins());

    for i in 0..60 {
        let response = app.clone().oneshot(login_request()).await.expect("oneshot");
        assert_eq!(
            response.status(),
            StatusCode::TEMPORARY_REDIRECT,
            "login {i} within budget must redirect to the provider",
        );
    }

    // The 61st request through a fresh `app.clone()` is the explicit
    // assertion that the counter survives clone: it can only be 429 if
    // the prior 60 clones incremented the same Arc-backed window.
    let blocked = app.oneshot(login_request()).await.expect("oneshot");
    assert_eq!(
        blocked.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the 61st login from the same IP must be rate-limited",
    );
}

#[tokio::test]
async fn sso_login_per_ip_budgets_are_independent() {
    // The `PerIp` scope must give each distinct peer IP its own 60/60s
    // budget. Exhaust IP-A entirely, then prove IP-B is still admitted —
    // a regression to a global counter (or PerRoute keying) would let
    // one IP's flood deny everyone else, and the single-IP exhaustion
    // test above would not catch it.
    let app = build_app(default_origins());
    let ip_a = SocketAddr::from(([10, 0, 0, 1], 1111));
    let ip_b = SocketAddr::from(([10, 0, 0, 2], 2222));

    for i in 0..60 {
        let response = app
            .clone()
            .oneshot(login_request_from(ip_a))
            .await
            .expect("oneshot");
        assert_eq!(
            response.status(),
            StatusCode::TEMPORARY_REDIRECT,
            "IP-A login {i} within budget must redirect",
        );
    }
    let blocked = app
        .clone()
        .oneshot(login_request_from(ip_a))
        .await
        .expect("oneshot");
    assert_eq!(
        blocked.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "IP-A must be exhausted after its 60-request budget",
    );

    // IP-B has touched nothing yet; its first request must succeed.
    let other = app
        .oneshot(login_request_from(ip_b))
        .await
        .expect("oneshot");
    assert_eq!(
        other.status(),
        StatusCode::TEMPORARY_REDIRECT,
        "a distinct IP must have its own independent rate-limit budget",
    );
}

#[tokio::test]
async fn sso_session_exchange_enforces_body_limit() {
    // `POST /auth/session/exchange` declares a 1 KiB body cap. An
    // oversized payload must be rejected with 413 before the handler
    // parses it (defense against unbounded request bodies on a public
    // route).
    let app = build_app(default_origins());

    let oversized = "x".repeat(2048);
    let request = with_peer(
        Request::builder()
            .method(Method::POST)
            .uri("/auth/session/exchange")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(oversized))
            .expect("request"),
    );
    let response = app.oneshot(request).await.expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "an oversized exchange body must be rejected with 413",
    );
}

#[tokio::test]
async fn sso_session_exchange_at_limit_body_accepted() {
    // A body exactly at the 1 KiB cap must NOT be rejected by the
    // body-limit layer. The exchange handler will still reject the
    // payload as a bad ticket, but the status must be anything other
    // than 413 — an off-by-one `>= 1024` guard would silently reject a
    // legitimate 1024-byte body, which the oversized test cannot detect.
    let app = build_app(default_origins());

    let at_limit = "x".repeat(1024);
    let request = with_peer(
        Request::builder()
            .method(Method::POST)
            .uri("/auth/session/exchange")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(at_limit))
            .expect("request"),
    );
    let response = app.oneshot(request).await.expect("oneshot");
    assert_ne!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "a body exactly at the 1 KiB cap must not be rejected by the body-limit layer",
    );
}

#[tokio::test]
async fn sso_logout_enforces_body_limit() {
    // `POST /auth/logout` shares the same 1 KiB `BodyLimitPolicy` as
    // session exchange (it reads no body, but the cap still bounds
    // oversized POSTs before the handler runs). It can regress
    // independently of the exchange route, so it gets its own 413 test.
    let app = build_app(default_origins());

    let oversized = "x".repeat(2048);
    let request = with_peer(
        Request::builder()
            .method(Method::POST)
            .uri("/auth/logout")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(oversized))
            .expect("request"),
    );
    let response = app.oneshot(request).await.expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "an oversized logout body must be rejected with 413",
    );
}

#[tokio::test]
async fn empty_cors_allowlist_fails_closed() {
    // An empty allow-list must reject every cross-origin request by
    // never echoing `Access-Control-Allow-Origin` — the gateway must
    // not reflect an attacker-supplied origin.
    let app = build_app(Vec::new());

    let preflight = with_peer(
        Request::builder()
            .method(Method::OPTIONS)
            .uri("/api/webchat/v2/threads")
            .header(header::ORIGIN, "http://evil.example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .body(Body::empty())
            .expect("request"),
    );
    let response = app.oneshot(preflight).await.expect("oneshot");
    assert!(
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "an empty CORS allow-list must not echo any origin",
    );
}

#[tokio::test]
async fn empty_cors_allowlist_fails_closed_on_simple_request() {
    // Browsers send "simple" requests (a plain GET/POST with an `Origin`
    // header) without a preflight, so the OPTIONS test above does not
    // cover them. An empty allow-list must also withhold
    // `Access-Control-Allow-Origin` on the actual response — a
    // regression where `CorsLayer` echoes the origin on non-preflight
    // responses would let an attacker page read cross-origin data.
    let app = build_app(Vec::new());

    let simple = with_peer(
        Request::builder()
            .method(Method::GET)
            .uri("/api/webchat/v2/threads")
            .header(header::ORIGIN, "http://evil.example.com")
            .body(Body::empty())
            .expect("request"),
    );
    let response = app.oneshot(simple).await.expect("oneshot");
    assert!(
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "an empty CORS allow-list must not echo any origin on a simple request",
    );
}
