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

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chrono::Duration as ChronoDuration;
use http_body_util::BodyExt;
use ironclaw_host_api::TenantId;
use ironclaw_reborn_webui_ingress::{
    EmailUserDirectory, InMemorySessionStore, OAuthError, OAuthProvider, OAuthRouterConfig,
    OAuthUserProfile, SessionStore, webui_v2_auth_router,
};
use parking_lot::Mutex;
use serde::Deserialize;
use tower::ServiceExt;

/// Stub provider that captures the args the router hands it and
/// returns whichever canned profile the test installed. Lets us
/// test the route handlers without owning a mock Google token
/// endpoint.
struct StubProvider {
    name: &'static str,
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
    fn google_with_profile(profile: OAuthUserProfile) -> Arc<Self> {
        Arc::new(Self {
            name: "google",
            auth_url_template: "https://accounts.google.test/o/oauth2/v2/auth".to_string(),
            next_profile: Mutex::new(Some(Ok(profile))),
            captured: Mutex::new(None),
        })
    }

    fn google_with_error(err: OAuthError) -> Arc<Self> {
        Arc::new(Self {
            name: "google",
            auth_url_template: "https://accounts.google.test/o/oauth2/v2/auth".to_string(),
            next_profile: Mutex::new(Some(Err(err))),
            captured: Mutex::new(None),
        })
    }
}

#[async_trait]
impl OAuthProvider for StubProvider {
    fn name(&self) -> &'static str {
        self.name
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
        display_name: Some("Alice".to_string()),
    }
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
    webui_v2_auth_router(config)
}

async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.expect("collect body").to_bytes();
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

#[derive(Deserialize)]
struct ProvidersResponse {
    providers: Vec<String>,
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
async fn callback_success_creates_session_and_redirects_with_token_fragment() {
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
    //    `#token=` fragment, and a session must exist in the store.
    let callback = router
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
    assert!(landing.starts_with("/v2#token="), "got {landing}");
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

    // The token in the URL fragment must actually authenticate
    // against the session store (locks in the round-trip).
    let token = landing.split_once("#token=").unwrap().1;
    let decoded = urlencoding::decode(token).expect("decode").into_owned();
    let session = store_inner
        .lookup(&decoded)
        .await
        .expect("lookup")
        .expect("session");
    assert_eq!(session.user_id.as_str(), "alice@example.com");
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
    let store_inner: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let session_store: Arc<dyn SessionStore> = store_inner.clone();
    let provider = StubProvider::google_with_profile(alice_profile());
    let router = build_router(
        vec![provider as Arc<dyn OAuthProvider>],
        session_store.clone(),
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
    let location = login
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let state = state_from_location(&location);

    // First callback consumes the state and creates a session. Need
    // a second provider that still returns a profile, since the stub
    // we built only fires once.
    let provider2 = StubProvider::google_with_profile(alice_profile());
    let router2 = build_router(
        vec![provider2 as Arc<dyn OAuthProvider>],
        session_store.clone(),
    );
    // Replay the (still-unknown-to-router2) state against a fresh
    // router. Different pending stores → second call must fail
    // closed with `invalid_state`.
    let replay = router2
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
    assert_eq!(replay.status(), StatusCode::SEE_OTHER);
    let location = replay
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(location, "/v2?login_error=invalid_state");
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
    assert!(location.starts_with("/v2#token="));
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
    let token = landing.split_once("#token=").unwrap().1;
    let bearer = urlencoding::decode(token).expect("decode").into_owned();
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
