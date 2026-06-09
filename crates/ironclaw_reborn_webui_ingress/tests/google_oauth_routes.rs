//! Caller-level tests for the WebChat v2 Google OAuth login surface.
//!
//! Drives the unauthenticated `Router` returned by
//! [`webui_v2_auth_router`] through `tower::ServiceExt::oneshot` so
//! the assertions cover the full HTTP shape, not just the helper
//! types underneath. Per `.claude/rules/testing.md` "Test Through
//! the Caller, Not Just the Helper", the side effect we care about
//! (session creation, redirect target, error code mapping) is
//! end-of-pipeline; testing the Google provider's `exchange_code`
//! alone wouldn't catch a wrapper that drops the verifier.
//!
//! Gated on `dev-in-memory-session` because the test wires
//! `InMemorySessionStore` + `EmailUserDirectory`, both of which only
//! exist behind that feature. Matches `session_round_trip.rs`.

#![cfg(feature = "dev-in-memory-session")]

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chrono::Duration as ChronoDuration;
use http_body_util::BodyExt;
use ironclaw_host_api::TenantId;
use ironclaw_reborn_webui_ingress::{
    EmailUserDirectory, InMemorySessionStore, OAuthError, OAuthProvider, OAuthProviderName,
    OAuthRouterConfig, OAuthUserProfile, SessionStore, webui_v2_auth_router,
};
use parking_lot::Mutex;
use serde::Deserialize;
use tower::ServiceExt;

/// Stub provider that captures the args the router hands it and
/// returns whichever canned profile the test installed. Lets us
/// test the route handlers without owning a mock Google token
/// endpoint.
struct StubProvider {
    name: OAuthProviderName,
    auth_url_template: String,
    next_profile: Mutex<Option<Result<OAuthUserProfile, OAuthError>>>,
    captured: Mutex<Option<CapturedExchange>>,
}

#[derive(Clone, Debug)]
struct CapturedExchange {
    code: String,
    callback_url: String,
    code_verifier: String,
}

impl StubProvider {
    fn for_provider(name: &str, profile: OAuthUserProfile) -> Arc<Self> {
        Arc::new(Self {
            name: OAuthProviderName::new(name).expect("name"),
            // Use a single stable mock authorization host across
            // every stub provider — the route-level tests assert
            // against the fixed `accounts.google.test` prefix
            // returned from `Location`, and varying the host per
            // provider would mask a real change in the redirect
            // construction.
            auth_url_template: "https://accounts.google.test/o/oauth2/v2/auth".to_string(),
            next_profile: Mutex::new(Some(Ok(profile))),
            captured: Mutex::new(None),
        })
    }

    fn google_with_profile(profile: OAuthUserProfile) -> Arc<Self> {
        Self::for_provider("google", profile)
    }

    fn google_with_error(err: OAuthError) -> Arc<Self> {
        Arc::new(Self {
            name: OAuthProviderName::new("google").expect("name"),
            auth_url_template: "https://accounts.google.test/o/oauth2/v2/auth".to_string(),
            next_profile: Mutex::new(Some(Err(err))),
            captured: Mutex::new(None),
        })
    }
}

#[async_trait]
impl OAuthProvider for StubProvider {
    fn name(&self) -> &OAuthProviderName {
        &self.name
    }

    fn authorization_url(&self, callback_url: &str, state: &str, code_challenge: &str) -> String {
        format!(
            "{}?redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256",
            self.auth_url_template,
            urlencoding::encode(callback_url),
            urlencoding::encode(state),
            urlencoding::encode(code_challenge),
        )
    }

    async fn exchange_code(
        &self,
        code: &str,
        callback_url: &str,
        code_verifier: &str,
    ) -> Result<OAuthUserProfile, OAuthError> {
        *self.captured.lock() = Some(CapturedExchange {
            code: code.to_string(),
            callback_url: callback_url.to_string(),
            code_verifier: code_verifier.to_string(),
        });
        self.next_profile
            .lock()
            .take()
            .unwrap_or(Err(OAuthError::ProfileFetch(
                "stub already consumed".into(),
            )))
    }
}

fn tenant() -> TenantId {
    TenantId::new("tenant-a").expect("tenant")
}

fn alice_profile() -> OAuthUserProfile {
    OAuthUserProfile {
        provider_user_id: "google-sub-123".to_string(),
        email: Some("alice@example.com".to_string()),
        email_verified: true,
        verified_emails: vec!["alice@example.com".to_string()],
        display_name: Some("Alice".to_string()),
    }
}

fn build_router(
    providers: Vec<Arc<dyn OAuthProvider>>,
    session_store: Arc<dyn SessionStore>,
) -> axum::Router {
    // These tests drive the route table directly via `oneshot` and
    // deliberately bypass the descriptor-driven rate-limit /
    // body-limit middleware that lives in the composition layer.
    // The `.router` field is sufficient for the per-handler
    // assertions below; end-to-end coverage of the descriptor-
    // mounted shape lives in `tests/session_round_trip.rs`.
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

// ─── providers ────────────────────────────────────────────────────────

#[tokio::test]
async fn providers_lists_configured_google() {
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router(
        vec![StubProvider::google_with_profile(alice_profile()) as Arc<dyn OAuthProvider>],
        store,
    );
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
    let body = body_string(resp.into_body()).await;
    let payload: ProvidersResponse = serde_json::from_str(&body).expect("json");
    assert_eq!(payload.providers, vec!["google".to_string()]);
}

#[tokio::test]
async fn providers_returns_empty_when_none_configured() {
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router(Vec::new(), store);
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
    let body = body_string(resp.into_body()).await;
    let payload: ProvidersResponse = serde_json::from_str(&body).expect("json");
    assert!(
        payload.providers.is_empty(),
        "expected empty providers, got {:?}",
        payload.providers
    );
}

// ─── login redirect ───────────────────────────────────────────────────

#[tokio::test]
async fn login_redirects_to_provider_with_state_and_callback_url() {
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let provider = StubProvider::google_with_profile(alice_profile());
    let router = build_router(vec![provider.clone() as Arc<dyn OAuthProvider>], store);

    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/google?redirect_after=%2Fv2")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = resp
        .headers()
        .get(header::LOCATION)
        .expect("Location header")
        .to_str()
        .expect("utf-8");
    assert!(location.starts_with("https://accounts.google.test/"));
    // Callback URL the provider received must reflect the config base_url.
    assert!(
        location.contains("redirect_uri=https%3A%2F%2Fgateway.example%2Fauth%2Fcallback%2Fgoogle")
    );
    assert!(location.contains("state="));
    assert!(location.contains("code_challenge="));
    assert!(location.contains("code_challenge_method=S256"));
}

#[tokio::test]
async fn login_unknown_provider_returns_404() {
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router(Vec::new(), store);
    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/github")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn login_invalid_provider_slug_returns_404() {
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let router = build_router(
        vec![StubProvider::google_with_profile(alice_profile()) as Arc<dyn OAuthProvider>],
        store,
    );
    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/Google")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ─── callback success ─────────────────────────────────────────────────

/// Extract the CSRF state token from a Location URL returned by
/// `/auth/login/google`. The stub provider builds the auth URL with
/// `state=<value>` as a query param.
fn state_from_location(location: &str) -> String {
    let query = location.split_once('?').expect("query").1;
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("state=") {
            return urlencoding::decode(value).expect("urldecode").into_owned();
        }
    }
    panic!("no state in {location}");
}

#[tokio::test]
async fn callback_success_creates_session_and_redirects_with_login_ticket() {
    let store_inner: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let session_store: Arc<dyn SessionStore> = store_inner.clone();
    let provider = StubProvider::google_with_profile(alice_profile());
    let router = build_router(
        vec![provider.clone() as Arc<dyn OAuthProvider>],
        session_store,
    );

    // 1. Login → capture state.
    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/google?redirect_after=%2Fv2")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let location = login
        .headers()
        .get(header::LOCATION)
        .expect("Location")
        .to_str()
        .expect("utf-8")
        .to_string();
    let state = state_from_location(&location);

    // 2. Callback with that state — must succeed and redirect with
    //    a one-time `login_ticket`, and a session must exist in the
    //    store without leaking the bearer in the Location header.
    let callback = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/google?code=auth-code&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    let landing = callback
        .headers()
        .get(header::LOCATION)
        .expect("Location")
        .to_str()
        .expect("utf-8")
        .to_string();
    assert!(landing.starts_with("/v2?login_ticket="), "got {landing}",);
    assert!(
        !landing.contains("#token="),
        "callback Location must not carry the bearer: {landing}",
    );
    assert!(
        callback.headers().get(header::SET_COOKIE).is_none(),
        "v2 OAuth callback must not set a session cookie — transport is the one-time \
         login_ticket only (#4116 cookie-transport beta-break)",
    );
    assert_eq!(store_inner.len(), 1, "session should be persisted");

    // The provider should have received the original PKCE verifier
    // the login step generated — captured by the stub.
    let captured = provider
        .captured
        .lock()
        .clone()
        .expect("provider captured exchange");
    assert_eq!(captured.code, "auth-code");
    assert_eq!(
        captured.callback_url,
        "https://gateway.example/auth/callback/google"
    );
    assert!(!captured.code_verifier.is_empty());

    // The one-time ticket must exchange for a bearer that actually
    // authenticates against the session store (locks in the
    // round-trip), and then fail on replay.
    let ticket = ticket_from_landing(&landing);
    let decoded = redeem_ticket(router.clone(), &ticket).await;
    let replay = exchange_ticket(router, &ticket).await;
    assert_eq!(
        replay.status(),
        StatusCode::UNAUTHORIZED,
        "login ticket must be single-use",
    );
    let session = store_inner
        .lookup(&decoded)
        .await
        .expect("lookup")
        .expect("session");
    assert_eq!(session.user_id.as_str(), "alice@example.com");
}

fn ticket_from_landing(landing: &str) -> String {
    let query = landing.split_once('?').expect("query").1;
    let query = query.split_once('#').map(|(q, _)| q).unwrap_or(query);
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("login_ticket=") {
            return urlencoding::decode(value).expect("urldecode").into_owned();
        }
    }
    panic!("no login_ticket in {landing}");
}

async fn exchange_ticket(router: axum::Router, ticket: &str) -> axum::response::Response {
    router
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
        .expect("oneshot")
}

async fn redeem_ticket(router: axum::Router, ticket: &str) -> String {
    let resp = exchange_ticket(router, ticket).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    let payload: SessionExchangeResponse = serde_json::from_str(&body).expect("json");
    assert!(!payload.token.is_empty());
    payload.token
}

// ─── callback failure paths ───────────────────────────────────────────

#[tokio::test]
async fn callback_with_unknown_state_redirects_with_error_code() {
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let provider = StubProvider::google_with_profile(alice_profile());
    let router = build_router(vec![provider as Arc<dyn OAuthProvider>], store);

    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/callback/google?code=c&state=does-not-exist")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let location = resp
        .headers()
        .get(header::LOCATION)
        .expect("Location")
        .to_str()
        .expect("utf-8");
    assert_eq!(location, "/v2?login_error=invalid_state");
}

#[tokio::test]
async fn callback_with_state_replay_fails_closed() {
    // The bug class being locked: a successfully-consumed state
    // token must not be re-usable on the SAME router. The earlier
    // version of this test built a fresh router for the replay,
    // which only exercised "unknown state" — already covered by
    // `callback_with_unknown_state_redirects_with_error_code`.
    let store_inner: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let session_store: Arc<dyn SessionStore> = store_inner.clone();
    let provider = StubProvider::google_with_profile(alice_profile());
    let router = build_router(
        vec![provider as Arc<dyn OAuthProvider>],
        session_store.clone(),
    );

    // 1. Login → capture state.
    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/google")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let state = state_from_location(
        login
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );

    // 2. First callback consumes the state, mints a session, and
    //    redirects with a one-time login ticket.
    let first = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/google?code=auth-code&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(first.status(), StatusCode::SEE_OTHER);
    let first_location = first
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        first_location.starts_with("/v2?login_ticket="),
        "first callback must succeed; got {first_location}"
    );
    assert_eq!(store_inner.len(), 1, "first callback must mint a session");

    // 3. Replay the SAME state token against the SAME router. The
    //    pending-flow store's single-use semantics must drop the
    //    second `take` to `None`, so the callback hits the
    //    `invalid_state` branch — no new session, no provider call.
    let replay = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/google?code=auth-code&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(replay.status(), StatusCode::SEE_OTHER);
    let replay_location = replay
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(replay_location, "/v2?login_error=invalid_state");
    assert_eq!(
        store_inner.len(),
        1,
        "replay must NOT mint a second session",
    );
}

#[tokio::test]
async fn callback_with_provider_error_param_redirects_with_denied() {
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let provider = StubProvider::google_with_profile(alice_profile());
    let router = build_router(vec![provider as Arc<dyn OAuthProvider>], store);

    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/callback/google?error=access_denied&error_description=User+denied")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let location = resp
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(location, "/v2?login_error=denied");
}

#[tokio::test]
async fn callback_when_provider_rejects_hosted_domain_yields_unauthorized() {
    let store_inner: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let session_store: Arc<dyn SessionStore> = store_inner.clone();
    let provider = StubProvider::google_with_error(OAuthError::Denied("hd mismatch".into()));
    let router = build_router(vec![provider as Arc<dyn OAuthProvider>], session_store);

    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/google")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let state = state_from_location(
        login
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );

    let callback = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/google?code=c&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    let location = callback
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(location, "/v2?login_error=unauthorized");
    assert_eq!(store_inner.len(), 0, "no session must be created");
}

#[tokio::test]
async fn login_open_redirect_attempt_falls_back_to_default() {
    let store_inner: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let session_store: Arc<dyn SessionStore> = store_inner.clone();
    let provider = StubProvider::google_with_profile(alice_profile());
    let router = build_router(
        vec![provider as Arc<dyn OAuthProvider>],
        session_store.clone(),
    );

    // Protocol-relative redirect target: sanitize_redirect must
    // strip it, and the callback must land on the default `/v2`.
    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/google?redirect_after=%2F%2Fevil.example%2Fpath")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let state = state_from_location(
        login
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );

    let callback = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/google?code=c&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    let location = callback
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(location.starts_with("/v2?login_ticket="));
}

// ─── logout ───────────────────────────────────────────────────────────

#[tokio::test]
async fn logout_with_bearer_revokes_session() {
    let store_inner: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let session_store: Arc<dyn SessionStore> = store_inner.clone();
    let provider = StubProvider::google_with_profile(alice_profile());
    let router = build_router(vec![provider as Arc<dyn OAuthProvider>], session_store);

    // Drive a successful callback to mint a real session token.
    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/google")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let state = state_from_location(
        login
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );
    let callback = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/google?code=c&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let landing = callback
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    let ticket = ticket_from_landing(landing);
    let bearer = redeem_ticket(router.clone(), &ticket).await;
    assert_eq!(store_inner.len(), 1);

    let logout = router
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
    assert_eq!(
        store_inner.len(),
        0,
        "session must be revoked from the store",
    );
    let probe = store_inner.lookup(&bearer).await.expect("lookup");
    assert!(probe.is_none(), "lookup after revoke must return None");
}

#[tokio::test]
async fn logout_without_bearer_returns_no_content() {
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let provider = StubProvider::google_with_profile(alice_profile());
    let router = build_router(vec![provider as Arc<dyn OAuthProvider>], store);
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/logout")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

// `state_from_location` is a helper used across every callback test
// above. Locking its behavior with a unit assertion here makes
// failures in the helper distinguishable from failures in the
// flow it's supposed to inspect.
#[test]
fn state_extraction_handles_urlencoded_value() {
    let url = "https://accounts.google.test/x?state=foo%2Bbar&code_challenge=z";
    assert_eq!(state_from_location(url), "foo+bar");
}

// ─── callback error-path regression tests (reviewer-requested) ────────

// Finding #5: the callback maps missing/empty `code` and `state`
// to `?login_error=invalid_request`, but only the unknown-state
// branch had test coverage. Lock both missing and empty shapes.
#[tokio::test]
async fn callback_missing_code_or_state_redirects_invalid_request() {
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let provider = StubProvider::google_with_profile(alice_profile());
    let router = build_router(vec![provider as Arc<dyn OAuthProvider>], store);

    for uri in [
        "/auth/callback/google",                 // both missing
        "/auth/callback/google?code=&state=abc", // empty code
        "/auth/callback/google?code=abc&state=", // empty state
        "/auth/callback/google?state=abc",       // no code
        "/auth/callback/google?code=abc",        // no state
    ] {
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(resp.status(), StatusCode::SEE_OTHER, "uri={uri}");
        let location = resp
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(location, "/v2?login_error=invalid_request", "uri={uri}");
    }
}

// Finding #6 (High): cross-provider state replay. The callback
// rejects when `flow.provider != provider_name`. Mint state for
// `google` (login) then drive `/auth/callback/github` with that
// state on the SAME router. Must redirect to
// `?login_error=provider_mismatch` and NOT mint a session.
#[tokio::test]
async fn callback_with_state_for_different_provider_redirects_provider_mismatch() {
    let store_inner: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let session_store: Arc<dyn SessionStore> = store_inner.clone();
    let google = StubProvider::for_provider("google", alice_profile());
    let github = StubProvider::for_provider("github", alice_profile());
    let router = build_router(
        vec![
            google as Arc<dyn OAuthProvider>,
            github as Arc<dyn OAuthProvider>,
        ],
        session_store,
    );

    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/google")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let state = state_from_location(
        login
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );

    // Cross-provider replay: same router, same state, different
    // `{provider}` URL segment.
    let replay = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/github?code=c&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(replay.status(), StatusCode::SEE_OTHER);
    let location = replay
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(location, "/v2?login_error=provider_mismatch");
    assert_eq!(
        store_inner.len(),
        0,
        "cross-provider replay must NOT mint a session",
    );
}

// Finding #7: provider exchange failures map to
// `?login_error=exchange_failed`. Drive a provider whose
// `exchange_code` returns `OAuthError::CodeExchange` (network
// failure / non-2xx Google response shape) — distinct from the
// `Denied` branch that already had coverage.
#[tokio::test]
async fn callback_when_provider_exchange_fails_redirects_exchange_failed() {
    let store_inner: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let session_store: Arc<dyn SessionStore> = store_inner.clone();
    let provider = StubProvider::google_with_error(OAuthError::CodeExchange(
        "simulated token-endpoint 500".into(),
    ));
    let router = build_router(vec![provider as Arc<dyn OAuthProvider>], session_store);

    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/google")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let state = state_from_location(
        login
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );

    let callback = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/google?code=c&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    let location = callback
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(location, "/v2?login_error=exchange_failed");
    assert_eq!(store_inner.len(), 0);
}

// Same as above, but the provider returns `ProfileFetch` (the
// other error variant that also maps to `exchange_failed`).
#[tokio::test]
async fn callback_when_profile_fetch_fails_redirects_exchange_failed() {
    let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
    let provider = StubProvider::google_with_error(OAuthError::ProfileFetch(
        "simulated malformed id_token".into(),
    ));
    let router = build_router(vec![provider as Arc<dyn OAuthProvider>], store);

    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/login/google")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let state = state_from_location(
        login
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );

    let callback = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/auth/callback/google?code=c&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(
        callback
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
        "/v2?login_error=exchange_failed",
    );
}

// Finding #8: `UserDirectory::resolve` distinguishes
// `Unknown` -> `unauthorized` vs `Backend` -> `server_error`.
// Drive a stub directory that returns each variant.
mod user_directory_branches {
    use super::*;
    use async_trait::async_trait;
    use ironclaw_host_api::UserId;
    use ironclaw_reborn_webui_ingress::{UserDirectory, UserDirectoryError};

    struct AlwaysUnknown;

    #[async_trait]
    impl UserDirectory for AlwaysUnknown {
        async fn resolve(
            &self,
            _provider: &OAuthProviderName,
            _profile: &OAuthUserProfile,
        ) -> Result<UserId, UserDirectoryError> {
            Err(UserDirectoryError::Unknown)
        }
    }

    struct AlwaysBackendFail;

    #[async_trait]
    impl UserDirectory for AlwaysBackendFail {
        async fn resolve(
            &self,
            _provider: &OAuthProviderName,
            _profile: &OAuthUserProfile,
        ) -> Result<UserId, UserDirectoryError> {
            Err(UserDirectoryError::Backend("db unreachable".into()))
        }
    }

    fn build_router_with_directory(
        directory: Arc<dyn UserDirectory>,
    ) -> (axum::Router, Arc<InMemorySessionStore>) {
        let store_inner: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
        let session_store: Arc<dyn SessionStore> = store_inner.clone();
        let provider = StubProvider::google_with_profile(alice_profile());
        let config = OAuthRouterConfig::new(
            tenant(),
            session_store,
            directory,
            vec![provider as Arc<dyn OAuthProvider>],
            "https://gateway.example",
        )
        .with_session_lifetime(ChronoDuration::hours(1));
        (webui_v2_auth_router(config).router, store_inner)
    }

    async fn drive_callback(router: axum::Router) -> String {
        let login = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/auth/login/google")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        let state = state_from_location(
            login
                .headers()
                .get(header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap(),
        );
        let callback = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!(
                        "/auth/callback/google?code=c&state={}",
                        urlencoding::encode(&state)
                    ))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(callback.status(), StatusCode::SEE_OTHER);
        callback
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn unknown_user_redirects_unauthorized() {
        let (router, store) = build_router_with_directory(Arc::new(AlwaysUnknown));
        let location = drive_callback(router).await;
        assert_eq!(location, "/v2?login_error=unauthorized");
        assert_eq!(store.len(), 0);
    }

    #[tokio::test]
    async fn backend_failure_redirects_server_error() {
        let (router, store) = build_router_with_directory(Arc::new(AlwaysBackendFail));
        let location = drive_callback(router).await;
        assert_eq!(location, "/v2?login_error=server_error");
        assert_eq!(store.len(), 0);
    }
}

// Finding #9: `SessionStore::create_session` failures map to
// `server_error`. Drive a stub store that always errors on
// `create_session` (the in-memory store can't naturally fail).
mod session_store_failure {
    use super::*;
    use async_trait::async_trait;
    use chrono::Duration as ChronoDuration;
    use ironclaw_host_api::{TenantId, UserId};
    use ironclaw_reborn_webui_ingress::{
        SessionRecord, SessionStore, SessionStoreError, UserDirectory,
    };
    use secrecy::SecretString;

    struct AlwaysFailCreate;

    #[async_trait]
    impl SessionStore for AlwaysFailCreate {
        async fn create_session(
            &self,
            _tenant_id: TenantId,
            _user_id: UserId,
            _lifetime: ChronoDuration,
        ) -> Result<SecretString, SessionStoreError> {
            Err(SessionStoreError::Backend("simulated outage".into()))
        }
        async fn lookup(
            &self,
            _candidate: &str,
        ) -> Result<Option<SessionRecord>, SessionStoreError> {
            Ok(None)
        }
    }

    fn build_router_with_session_store(
        store: Arc<dyn SessionStore>,
        directory: Arc<dyn UserDirectory>,
    ) -> axum::Router {
        let provider = StubProvider::google_with_profile(alice_profile());
        let config = OAuthRouterConfig::new(
            tenant(),
            store,
            directory,
            vec![provider as Arc<dyn OAuthProvider>],
            "https://gateway.example",
        )
        .with_session_lifetime(ChronoDuration::hours(1));
        webui_v2_auth_router(config).router
    }

    #[tokio::test]
    async fn callback_when_session_store_create_fails_redirects_server_error() {
        let router = build_router_with_session_store(
            Arc::new(AlwaysFailCreate),
            Arc::new(EmailUserDirectory),
        );

        let login = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/auth/login/google")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        let state = state_from_location(
            login
                .headers()
                .get(header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap(),
        );
        let callback = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!(
                        "/auth/callback/google?code=c&state={}",
                        urlencoding::encode(&state)
                    ))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(callback.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            callback
                .headers()
                .get(header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "/v2?login_error=server_error",
        );
    }
}

// Reviewer finding #2: logout returns 500 when revoke fails. The
// SPA still clears local state, but the response status now
// truthfully reflects that the server-side bearer may live on.
mod logout_revoke_failure {
    use super::*;
    use async_trait::async_trait;
    use chrono::Duration as ChronoDuration;
    use ironclaw_host_api::{TenantId, UserId};
    use ironclaw_reborn_webui_ingress::{
        SessionRecord, SessionStore, SessionStoreError, UserDirectory,
    };
    use secrecy::SecretString;

    struct RevokeAlwaysFails;

    #[async_trait]
    impl SessionStore for RevokeAlwaysFails {
        async fn create_session(
            &self,
            _tenant_id: TenantId,
            _user_id: UserId,
            _lifetime: ChronoDuration,
        ) -> Result<SecretString, SessionStoreError> {
            unreachable!("test does not drive create_session")
        }
        async fn lookup(
            &self,
            _candidate: &str,
        ) -> Result<Option<SessionRecord>, SessionStoreError> {
            Ok(None)
        }
        async fn revoke(&self, _candidate: &str) -> Result<(), SessionStoreError> {
            Err(SessionStoreError::Backend("simulated outage".into()))
        }
    }

    fn build_router_with_session_store(
        store: Arc<dyn SessionStore>,
        directory: Arc<dyn UserDirectory>,
    ) -> axum::Router {
        let provider = StubProvider::google_with_profile(alice_profile());
        let config = OAuthRouterConfig::new(
            tenant(),
            store,
            directory,
            vec![provider as Arc<dyn OAuthProvider>],
            "https://gateway.example",
        )
        .with_session_lifetime(ChronoDuration::hours(1));
        webui_v2_auth_router(config).router
    }

    #[tokio::test]
    async fn logout_returns_500_when_revoke_fails() {
        let router = build_router_with_session_store(
            Arc::new(RevokeAlwaysFails),
            Arc::new(EmailUserDirectory),
        );

        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/logout")
                    .header(header::AUTHORIZATION, "Bearer some-token")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "logout MUST return 5xx when SessionStore::revoke fails — \
             returning 204 would lie about the server-side state",
        );
    }
}
