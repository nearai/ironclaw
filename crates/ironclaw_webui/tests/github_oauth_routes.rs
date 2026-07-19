//! Caller-level tests for the WebChat v2 GitHub OAuth login surface.
//!
//! Unlike `google_oauth_routes.rs`, which uses a stub provider to
//! exercise the route handlers, these tests drive the REAL
//! [`GitHubProvider`] against a local mock GitHub HTTP server wired in
//! through `GitHubProvider::with_endpoints`. That covers the
//! github-specific behavior the route handlers can't see on their own
//! — the token→user→emails sequence and the verified-email
//! preference — through the full HTTP pipeline rather than the
//! provider in isolation. Per `.claude/rules/testing.md` "Test
//! Through the Caller, Not Just the Helper", the side effect we care
//! about (a session minted for the *verified* email, exchanged for a
//! usable bearer, then revoked) is end-of-pipeline.
//!
//! Gated on `dev-in-memory-session` because the test wires
//! `InMemorySessionStore` + `EmailUserDirectory` + the test-only
//! `GitHubProvider::with_endpoints` constructor, all of which only
//! exist behind that feature.

#![cfg(feature = "dev-in-memory-session")]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::Form;
use axum::http::{Request, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use chrono::Duration as ChronoDuration;
use http_body_util::BodyExt;
use ironclaw_host_api::TenantId;
use ironclaw_webui::{
    EmailUserDirectory, GitHubOAuthConfig, GitHubProvider, InMemorySessionStore, OAuthProvider,
    OAuthRouterConfig, SessionStore, webui_v2_auth_router,
};
use secrecy::SecretString;
use serde::Deserialize;
use tower::ServiceExt;

mod support;
use support::{AbortOnDrop, MockEmail, spawn_router};

// ── mock GitHub HTTP server ───────────────────────────────────────────

/// Fixed happy-path GitHub mock (token always succeeds, `/user` +
/// `/user/emails` return canned bodies). Named `Stub*` to distinguish
/// it from the richer failure-injecting `MockGitHub` in the `github.rs`
/// unit tests — they are different types in different compilation units.
#[derive(Clone)]
struct StubGitHub {
    user_body: serde_json::Value,
    emails: Vec<MockEmail>,
}

impl StubGitHub {
    fn octocat() -> Self {
        Self {
            user_body: serde_json::json!({
                "id": 90210,
                "login": "octocat",
                "name": "The Octocat",
                "email": null,
            }),
            emails: vec![
                MockEmail {
                    email: "noreply@example.com",
                    verified: false,
                    primary: false,
                },
                MockEmail {
                    email: "octocat@example.com",
                    verified: true,
                    primary: true,
                },
            ],
        }
    }
}

/// Spawn the stub GitHub token / user / emails endpoints and return
/// the bound address plus a guard that aborts the server on drop.
async fn spawn_stub_github(stub: StubGitHub) -> (SocketAddr, AbortOnDrop) {
    let user_stub = stub.clone();
    let emails_stub = stub.clone();

    let router = axum::Router::new()
        .route(
            "/token",
            post(|_: Form<HashMap<String, String>>| async {
                Json(serde_json::json!({
                    "access_token": "gho_mock_token",
                    "token_type": "bearer",
                    "scope": "read:user,user:email",
                }))
            }),
        )
        .route(
            "/user",
            get(move || {
                let body = user_stub.user_body.clone();
                async move { Json(body).into_response() }
            }),
        )
        .route(
            "/emails",
            get(move || {
                let emails = emails_stub.emails.clone();
                async move { Json(emails).into_response() }
            }),
        );

    spawn_router(router).await
}

fn github_provider(addr: SocketAddr) -> Arc<dyn OAuthProvider> {
    Arc::new(
        GitHubProvider::with_endpoints(
            GitHubOAuthConfig {
                client_id: "gh-client-id".to_string(),
                client_secret: SecretString::from("gh-client-secret".to_string()),
                http_timeout: None,
            },
            "https://github.test/login/oauth/authorize",
            format!("http://{addr}/token"),
            format!("http://{addr}/user"),
            format!("http://{addr}/emails"),
        )
        .expect("build provider"),
    )
}

fn tenant() -> TenantId {
    TenantId::new("tenant-a").expect("tenant")
}

fn build_router(
    providers: Vec<Arc<dyn OAuthProvider>>,
    session_store: Arc<dyn SessionStore>,
) -> axum::Router {
    let config = OAuthRouterConfig::new(
        tenant(),
        session_store,
        Arc::new(EmailUserDirectory),
        providers,
        "https://gateway.example",
    )
    .with_session_lifetime(ChronoDuration::hours(1));
    webui_v2_auth_router(config).router
}

async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.expect("collect body").to_bytes();
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

#[derive(Deserialize)]
struct ProvidersResponse {
    providers: Vec<String>,
}

#[derive(Deserialize)]
struct SessionExchangeResponse {
    token: String,
}

fn header_str(resp: &axum::response::Response, name: header::HeaderName) -> &str {
    resp.headers()
        .get(name)
        .expect("header present")
        .to_str()
        .expect("utf-8")
}

fn state_from_location(location: &str) -> String {
    let query = location.split_once('?').expect("query").1;
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("state=") {
            return urlencoding::decode(value).expect("urldecode").into_owned();
        }
    }
    panic!("no state in {location}");
}

fn ticket_from_landing(landing: &str) -> String {
    let query = landing.split_once('?').expect("query").1;
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("login_ticket=") {
            return urlencoding::decode(value).expect("urldecode").into_owned();
        }
    }
    panic!("no login_ticket in {landing}");
}

/// Drive login → callback and return `(login_ticket, landing_url)`.
async fn login_and_callback(router: &axum::Router) -> String {
    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/github?redirect_after=%2F")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(login.status(), StatusCode::TEMPORARY_REDIRECT);
    let state = state_from_location(header_str(&login, header::LOCATION));

    let callback = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/github?code=gh-auth-code&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    header_str(&callback, header::LOCATION).to_string()
}

async fn redeem_ticket(router: &axum::Router, ticket: &str) -> String {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/session/exchange")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "ticket": ticket }).to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);
    let payload: SessionExchangeResponse =
        serde_json::from_str(&body_string(resp.into_body()).await).expect("json");
    payload.token
}

// ─── discovery ─────────────────────────────────────────────────────────

#[tokio::test]
async fn providers_lists_configured_github() {
    let (addr, _server) = spawn_stub_github(StubGitHub::octocat()).await;
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router(vec![github_provider(addr)], store);

    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/providers")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);
    let payload: ProvidersResponse =
        serde_json::from_str(&body_string(resp.into_body()).await).expect("json");
    assert_eq!(payload.providers, vec!["github".to_string()]);
}

// ─── login redirect ──────────────────────────────────────────────────

#[tokio::test]
async fn login_redirects_to_github_with_state_and_scope_and_no_pkce() {
    let (addr, _server) = spawn_stub_github(StubGitHub::octocat()).await;
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router(vec![github_provider(addr)], store);

    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/github?redirect_after=%2F")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = header_str(&resp, header::LOCATION);
    assert!(location.starts_with("https://github.test/login/oauth/authorize"));
    assert!(
        location.contains("redirect_uri=https%3A%2F%2Fgateway.example%2Fauth%2Fcallback%2Fgithub")
    );
    assert!(location.contains("scope=read%3Auser%20user%3Aemail"));
    assert!(location.contains("state="));
    // GitHub does not support PKCE — the challenge must never leak
    // into the authorization URL even though the router computes one.
    assert!(
        !location.contains("code_challenge"),
        "PKCE challenge must not appear in the GitHub auth URL: {location}",
    );
}

// ─── callback success + email selection + session use ──────────────────

#[tokio::test]
async fn callback_success_mints_session_for_primary_verified_email() {
    let (addr, _server) = spawn_stub_github(StubGitHub::octocat()).await;
    let store_inner: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let session_store: Arc<dyn SessionStore> = store_inner.clone();
    let router = build_router(vec![github_provider(addr)], session_store);

    let landing = login_and_callback(&router).await;
    assert!(landing.starts_with("/?login_ticket="), "got {landing}");
    assert!(
        !landing.contains("#token="),
        "callback Location must not carry the bearer: {landing}",
    );
    assert_eq!(store_inner.len(), 1, "session should be persisted");

    // Exchange the one-time ticket for a usable bearer, then confirm
    // the session was minted for the PRIMARY VERIFIED email (not the
    // unverified one and not the empty profile email).
    let ticket = ticket_from_landing(&landing);
    let bearer = redeem_ticket(&router, &ticket).await;
    let session = store_inner
        .lookup(&bearer)
        .await
        .expect("lookup")
        .expect("session present");
    assert_eq!(session.user_id.as_str(), "octocat@example.com");

    // Ticket is single-use.
    let replay = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/session/exchange")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "ticket": ticket }).to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn callback_with_unverified_emails_mints_session_for_provider_sub() {
    // When GitHub returns NO verified address, the provider falls back
    // to the unverified profile email flagged `email_verified = false`.
    // The whole point of that flag is that the session identity must NOT
    // be the email — `EmailUserDirectory` (and any fail-closed
    // production directory) must mint the session for the provider-sub
    // (`github:<id>`) instead. This drives the full login→callback→
    // exchange pipeline and asserts the resulting `user_id`, so a
    // regression decoupling `email_verified` from the minted identity —
    // invisible to the provider-level unit test — fails here.
    let mut stub = StubGitHub::octocat();
    stub.user_body = serde_json::json!({
        "id": 90210,
        "login": "octocat",
        "name": "The Octocat",
        "email": "octocat@example.com",
    });
    // Every address unverified — including the primary.
    stub.emails = vec![
        MockEmail {
            email: "octocat@example.com",
            verified: false,
            primary: true,
        },
        MockEmail {
            email: "alt@example.com",
            verified: false,
            primary: false,
        },
    ];
    let (addr, _server) = spawn_stub_github(stub).await;
    let store_inner: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let session_store: Arc<dyn SessionStore> = store_inner.clone();
    let router = build_router(vec![github_provider(addr)], session_store);

    let landing = login_and_callback(&router).await;
    assert!(landing.starts_with("/?login_ticket="), "got {landing}");
    let ticket = ticket_from_landing(&landing);
    let bearer = redeem_ticket(&router, &ticket).await;
    let session = store_inner
        .lookup(&bearer)
        .await
        .expect("lookup")
        .expect("session present");
    assert_eq!(
        session.user_id.as_str(),
        "github:90210",
        "an unverified email must mint a provider-sub session, never an email identity",
    );
}

// ─── callback failure ─────────────────────────────────────────────────

#[tokio::test]
async fn callback_with_provider_error_redirects_with_denied() {
    let (addr, _server) = spawn_stub_github(StubGitHub::octocat()).await;
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router(vec![github_provider(addr)], store);

    // GitHub appends `?error=access_denied` when the user rejects the
    // consent screen. No state is consumed; the SPA must see a
    // generic `denied` code.
    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/callback/github?error=access_denied&error_description=The+user+denied")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(header_str(&resp, header::LOCATION), "/?login_error=denied");
}

#[tokio::test]
async fn callback_exchange_failure_redirects_with_exchange_failed() {
    // Point the provider at a server that 500s the token endpoint so
    // the real GitHubProvider's `exchange_code` returns CodeExchange,
    // which the router maps to `exchange_failed`.
    let failing = axum::Router::new().route(
        "/token",
        post(|_: Form<HashMap<String, String>>| async { StatusCode::INTERNAL_SERVER_ERROR }),
    );
    let (addr, _server) = spawn_router(failing).await;

    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let provider: Arc<dyn OAuthProvider> = Arc::new(
        GitHubProvider::with_endpoints(
            GitHubOAuthConfig {
                client_id: "gh-client-id".to_string(),
                client_secret: SecretString::from("gh-client-secret".to_string()),
                http_timeout: None,
            },
            "https://github.test/login/oauth/authorize",
            format!("http://{addr}/token"),
            format!("http://{addr}/user"),
            format!("http://{addr}/emails"),
        )
        .expect("build provider"),
    );
    let router = build_router(vec![provider], store);

    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/github")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let state = state_from_location(header_str(&login, header::LOCATION));

    let callback = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/github?code=gh-auth-code&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_str(&callback, header::LOCATION),
        "/?login_error=exchange_failed",
    );
}

#[tokio::test]
async fn callback_with_unknown_state_redirects_with_invalid_state_error() {
    // A callback whose state was never minted (expired out of the
    // pending-flow store, or fabricated) must fail closed with the
    // opaque `invalid_state` code and never reach the provider.
    let (addr, _server) = spawn_stub_github(StubGitHub::octocat()).await;
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router(vec![github_provider(addr)], store);

    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/callback/github?code=gh-code&state=does-not-exist")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_str(&resp, header::LOCATION),
        "/?login_error=invalid_state"
    );
}

#[tokio::test]
async fn callback_with_state_replay_fails_closed() {
    // The pending-flow store's single-use `take` is a documented
    // security property (CLAUDE.md §Security invariants). A state token
    // consumed by a successful callback must not mint a second session
    // when replayed against the same router.
    let (addr, _server) = spawn_stub_github(StubGitHub::octocat()).await;
    let store_inner: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router(vec![github_provider(addr)], store_inner.clone());

    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/github")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let state = state_from_location(header_str(&login, header::LOCATION));

    // First callback consumes the state and mints a session.
    let first = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/github?code=gh-code&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(first.status(), StatusCode::SEE_OTHER);
    assert!(header_str(&first, header::LOCATION).starts_with("/?login_ticket="));
    assert_eq!(store_inner.len(), 1);

    // Replaying the SAME state must fail closed — no second session.
    let replay = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/github?code=gh-code&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(replay.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_str(&replay, header::LOCATION),
        "/?login_error=invalid_state"
    );
    assert_eq!(
        store_inner.len(),
        1,
        "replayed state must NOT mint a second session"
    );
}

#[tokio::test]
async fn callback_profile_fetch_failure_redirects_with_exchange_failed() {
    // Token exchange succeeds but the `/user` read fails — the real
    // GitHubProvider returns `OAuthError::ProfileFetch`, which the
    // router must map to the same opaque `exchange_failed` code as a
    // token-exchange failure. Covers the ProfileFetch translation path
    // that the token-500 test (CodeExchange) does not reach.
    let server = axum::Router::new()
        .route(
            "/token",
            post(|_: Form<HashMap<String, String>>| async {
                Json(serde_json::json!({
                    "access_token": "gho_mock_token",
                    "token_type": "bearer",
                }))
            }),
        )
        .route("/user", get(|| async { StatusCode::UNAUTHORIZED }));
    let (addr, _server) = spawn_router(server).await;

    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let provider: Arc<dyn OAuthProvider> = Arc::new(
        GitHubProvider::with_endpoints(
            GitHubOAuthConfig {
                client_id: "gh-client-id".to_string(),
                client_secret: SecretString::from("gh-client-secret".to_string()),
                http_timeout: None,
            },
            "https://github.test/login/oauth/authorize",
            format!("http://{addr}/token"),
            format!("http://{addr}/user"),
            format!("http://{addr}/emails"),
        )
        .expect("build provider"),
    );
    let router = build_router(vec![provider], store);

    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/github")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let state = state_from_location(header_str(&login, header::LOCATION));

    let callback = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/github?code=gh-code&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_str(&callback, header::LOCATION),
        "/?login_error=exchange_failed",
    );
}

#[tokio::test]
async fn callback_exchange_timeout_redirects_with_exchange_failed() {
    // The token endpoint accepts the request but never responds. With a
    // tiny exchange budget the provider's overall `tokio::time::timeout`
    // fires, returning `OAuthError::CodeExchange("...timed out")`, which
    // the router must map to the opaque `exchange_failed` code — not a
    // 500 or a hung request. Locks the budget's behavior at the route
    // level (the unit test covers the provider in isolation).
    let blackhole = axum::Router::new().route(
        "/token",
        post(|_: Form<HashMap<String, String>>| async {
            std::future::pending::<axum::Json<serde_json::Value>>().await
        }),
    );
    let (addr, _server) = spawn_router(blackhole).await;

    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let provider: Arc<dyn OAuthProvider> = Arc::new(
        GitHubProvider::with_endpoints(
            GitHubOAuthConfig {
                client_id: "gh-client-id".to_string(),
                client_secret: SecretString::from("gh-client-secret".to_string()),
                http_timeout: None,
            },
            "https://github.test/login/oauth/authorize",
            format!("http://{addr}/token"),
            format!("http://{addr}/user"),
            format!("http://{addr}/emails"),
        )
        .expect("build provider")
        .with_exchange_budget(Duration::from_millis(150)),
    );
    let router = build_router(vec![provider], store);

    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/github")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let state = state_from_location(header_str(&login, header::LOCATION));

    let callback = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/github?code=gh-code&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_str(&callback, header::LOCATION),
        "/?login_error=exchange_failed",
    );
}

// ─── logout revokes the session ────────────────────────────────────────

#[tokio::test]
async fn logout_revokes_the_minted_session() {
    let (addr, _server) = spawn_stub_github(StubGitHub::octocat()).await;
    let store_inner: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let session_store: Arc<dyn SessionStore> = store_inner.clone();
    let router = build_router(vec![github_provider(addr)], session_store);

    let landing = login_and_callback(&router).await;
    let ticket = ticket_from_landing(&landing);
    let bearer = redeem_ticket(&router, &ticket).await;

    // Sanity: the bearer authenticates before logout.
    assert!(
        store_inner.lookup(&bearer).await.expect("lookup").is_some(),
        "session must exist before logout",
    );

    let logout = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/logout")
                .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);

    // After logout the bearer must no longer resolve to a session —
    // subsequent API / SSE / WebSocket access is denied.
    assert!(
        store_inner.lookup(&bearer).await.expect("lookup").is_none(),
        "session must be revoked after logout",
    );
}
