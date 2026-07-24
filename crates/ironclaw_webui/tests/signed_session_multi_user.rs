//! Caller-level production-chain test for multi-user WebChat v2 SSO.
//!
//! This drives the REAL production session path the `ironclaw-reborn serve`
//! binary wires: `build_signed_session_login` → `SignedTokenSessionStore`
//! (stateless HMAC) → `CompositeAuthenticator` → composed `webui_v2_app`.
//!
//! It logs in TWO distinct OAuth identities through the SAME app, mints two
//! signed session bearers, and asserts each bearer reaches the protected v2
//! surface as its OWN `ProductSurfaceCaller.user_id` — never the other's
//! and never the env operator. That per-user identity is exactly what the
//! facade's owner-scoped thread isolation builds on, so a regression that
//! collapsed both logins onto one user (or onto the operator) would fail
//! here.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use ironclaw_host_api::{
    AgentId, ProductSurface, ProductSurfaceCaller, ProductSurfaceError, ProjectId, TenantId,
    ThreadId, UserId,
};
use ironclaw_product::RebornCreateThreadResponse;
use ironclaw_threads::{SessionThreadRecord, ThreadScope};
use ironclaw_webui::{
    EnvBearerAuthenticator, OAuthProvider, OAuthProviderName, OAuthUserProfile,
    SignedSessionLoginConfig, UserDirectory, UserDirectoryError, build_signed_session_login,
};
use ironclaw_webui::{WebuiServeConfig, webui_v2_app};
use parking_lot::Mutex as PlMutex;
use secrecy::SecretString;
use serde::Deserialize;
use tower::ServiceExt;

const TENANT: &str = "tenant-a";
const AGENT: &str = "agent-default";
const PROJECT: &str = "project-default";

// ─── facade stub: records the caller per create_thread ───────────────────

#[derive(Default)]
struct RecordingServices {
    create_thread_callers: Mutex<Vec<ProductSurfaceCaller>>,
}

#[async_trait]
impl ProductSurface for RecordingServices {
    async fn invoke(
        &self,
        caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceInvokeRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceInvokeResponse, ProductSurfaceError> {
        if request.operation_id.as_str() != "thread.create" {
            return Err(ProductSurfaceError::service_unavailable(false));
        }
        // Return a thread owned by the calling user, mirroring the real
        // facade's `owner = caller.user_id` rule.
        let owner = caller.user_id.clone();
        self.create_thread_callers
            .lock()
            .expect("lock")
            .push(caller);
        let output = serde_json::to_value(RebornCreateThreadResponse {
            thread: SessionThreadRecord {
                thread_id: ThreadId::new("thread.fake").expect("thread"),
                scope: ThreadScope {
                    tenant_id: TenantId::new(TENANT).expect("tenant"),
                    agent_id: AgentId::new("agent.fake").expect("agent"),
                    project_id: Some(ProjectId::new("project.fake").expect("project")),
                    owner_user_id: Some(owner),
                    mission_id: None,
                },
                created_by_actor_id: "actor".to_string(),
                title: None,
                metadata_json: None,
                goal: None,
                created_at: None,
                updated_at: None,
            },
        })
        .map_err(ProductSurfaceError::internal_from)?;
        Ok(ironclaw_host_api::ProductSurfaceInvokeResponse { output })
    }

    async fn query(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ironclaw_host_api::ProductSurfaceQueryRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceQueryPage, ProductSurfaceError> {
        Err(ProductSurfaceError::service_unavailable(false))
    }

    async fn stream_events(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ironclaw_host_api::ProductSurfaceStreamRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceStreamResponse, ProductSurfaceError> {
        Err(ProductSurfaceError::service_unavailable(false))
    }
}

// ─── distinct-user directory: one user per provider subject ──────────────

/// Maps each provider subject to its own `UserId`, mirroring the real
/// reborn-owned store's distinct-user behavior without a database.
struct DistinctUserDirectory;

#[async_trait]
impl UserDirectory for DistinctUserDirectory {
    async fn resolve(
        &self,
        _provider: &OAuthProviderName,
        profile: &OAuthUserProfile,
    ) -> Result<UserId, UserDirectoryError> {
        UserId::new(format!("user-{}", profile.provider_user_id))
            .map_err(|err| UserDirectoryError::Backend(err.to_string()))
    }
}

// ─── OAuth provider that yields a queue of profiles ──────────────────────

struct QueueProvider {
    name: OAuthProviderName,
    profiles: PlMutex<VecDeque<OAuthUserProfile>>,
}

impl QueueProvider {
    fn new(profiles: Vec<OAuthUserProfile>) -> Arc<Self> {
        Arc::new(Self {
            name: OAuthProviderName::new("google").expect("name"),
            profiles: PlMutex::new(profiles.into()),
        })
    }
}

#[async_trait]
impl OAuthProvider for QueueProvider {
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
    ) -> Result<OAuthUserProfile, ironclaw_webui::OAuthError> {
        Ok(self
            .profiles
            .lock()
            .pop_front()
            .expect("a queued profile per login"))
    }
}

fn profile(sub: &str, email: &str) -> OAuthUserProfile {
    OAuthUserProfile {
        provider_user_id: sub.to_string(),
        email: Some(email.to_string()),
        email_verified: true,
        verified_emails: vec![email.to_string()],
        display_name: None,
    }
}

fn with_peer(mut req: Request<Body>) -> Request<Body> {
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))));
    req
}

fn build_app(profiles: Vec<OAuthUserProfile>) -> (axum::Router, Arc<RecordingServices>) {
    let env_authenticator = Arc::new(
        EnvBearerAuthenticator::new(
            SecretString::from("env-operator-token".to_string()),
            UserId::new("operator").expect("operator"),
        )
        .expect("env authenticator"),
    );
    let wiring = build_signed_session_login(SignedSessionLoginConfig {
        tenant_id: TenantId::new(TENANT).expect("tenant"),
        user_directory: Arc::new(DistinctUserDirectory),
        operator_secret: SecretString::from("operator-secret".to_string()),
        base_url: "https://gateway.example".to_string(),
        providers: vec![QueueProvider::new(profiles) as Arc<dyn OAuthProvider>],
        env_authenticator,
    })
    .expect("login wiring");

    let services = Arc::new(RecordingServices::default());
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        wiring.authenticator,
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
    .with_public_route_mount(wiring.mount);
    let app = webui_v2_app(services.clone(), config).expect("webui v2 app");
    (app, services)
}

// ─── login helpers ───────────────────────────────────────────────────────

fn state_from_location(location: &str) -> String {
    let query = location.split_once('?').expect("query").1;
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("state=") {
            return urlencoding::decode(value).expect("decode").into_owned();
        }
    }
    panic!("no state in {location}");
}

fn ticket_from_landing(landing: &str) -> String {
    let query = landing.split_once('?').expect("query").1;
    let query = query.split_once('#').map(|(q, _)| q).unwrap_or(query);
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("login_ticket=") {
            return urlencoding::decode(value).expect("decode").into_owned();
        }
    }
    panic!("no login_ticket in {landing}");
}

#[derive(Deserialize)]
struct SessionExchangeResponse {
    token: String,
}

/// Drive one full login → callback → ticket-exchange and return the bearer.
async fn login(app: &axum::Router) -> String {
    let login = app
        .clone()
        .oneshot(with_peer(
            Request::builder()
                .method(Method::GET)
                .uri("/auth/login/google?redirect_after=%2F")
                .body(Body::empty())
                .expect("request"),
        ))
        .await
        .expect("oneshot");
    assert_eq!(login.status(), StatusCode::TEMPORARY_REDIRECT);
    let auth_url = login
        .headers()
        .get(header::LOCATION)
        .expect("Location")
        .to_str()
        .expect("utf-8")
        .to_string();
    let state = state_from_location(&auth_url);

    let callback = app
        .clone()
        .oneshot(with_peer(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/auth/callback/google?code=auth-code&state={}",
                    urlencoding::encode(&state)
                ))
                .body(Body::empty())
                .expect("request"),
        ))
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
    let ticket = ticket_from_landing(&landing);

    let response = app
        .clone()
        .oneshot(with_peer(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/session/exchange")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "ticket": ticket }).to_string(),
                ))
                .expect("request"),
        ))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let payload: SessionExchangeResponse = serde_json::from_slice(&bytes).expect("json");
    payload.token
}

async fn create_thread(app: &axum::Router, bearer: &str) -> StatusCode {
    app.clone()
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
        .expect("oneshot")
        .status()
}

async fn session_payload(app: &axum::Router, bearer: &str) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(with_peer(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/session")
                .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                .body(Body::empty())
                .expect("request"),
        ))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("session json")
}

async fn llm_providers_status(app: &axum::Router, bearer: &str, method: Method) -> StatusCode {
    app.clone()
        .oneshot(with_peer(
            Request::builder()
                .method(method)
                .uri("/api/webchat/v2/llm/providers")
                .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                .body(Body::empty())
                .expect("request"),
        ))
        .await
        .expect("oneshot")
        .status()
}

// ─── test ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn two_oauth_users_reach_protected_routes_as_distinct_callers() {
    // alice and bob log in through the real signed-session production
    // chain; each must reach the protected v2 surface as its own user.
    let (app, services) = build_app(vec![
        profile("alice-sub", "alice@example.com"),
        profile("bob-sub", "bob@example.com"),
    ]);

    let alice_bearer = login(&app).await;
    let bob_bearer = login(&app).await;
    assert_ne!(
        alice_bearer, bob_bearer,
        "two distinct logins must mint two distinct session bearers"
    );

    assert_eq!(create_thread(&app, &alice_bearer).await, StatusCode::OK);
    assert_eq!(create_thread(&app, &bob_bearer).await, StatusCode::OK);

    let callers = services.create_thread_callers.lock().expect("lock").clone();
    assert_eq!(callers.len(), 2, "facade reached once per user");
    assert_eq!(
        callers[0].user_id.as_str(),
        "user-alice-sub",
        "alice's bearer must reach the facade as alice"
    );
    assert_eq!(
        callers[1].user_id.as_str(),
        "user-bob-sub",
        "bob's bearer must reach the facade as bob — never collapsed onto one user or the operator"
    );
    // Both callers carry the host-trusted tenant, never a browser value.
    assert!(callers.iter().all(|c| c.tenant_id.as_str() == TENANT));
}

#[tokio::test]
async fn sso_sessions_stay_non_operator_while_env_token_can_configure_operator_routes() {
    // This mirrors the Railway deployment shape: SSO login is enabled,
    // but the env bearer remains the separate operator credential.
    let (app, _services) = build_app(vec![profile("alice-sub", "alice@example.com")]);
    let sso_bearer = login(&app).await;

    let sso_session = session_payload(&app, &sso_bearer).await;
    assert_eq!(sso_session["user_id"], "user-alice-sub");
    assert_eq!(
        sso_session["capabilities"]["operator_webui_config"], false,
        "SSO session tokens must not inherit operator privileges"
    );
    assert_eq!(
        llm_providers_status(&app, &sso_bearer, Method::GET).await,
        StatusCode::FORBIDDEN,
        "SSO session tokens must be denied on operator LLM config routes"
    );
    assert_eq!(
        llm_providers_status(&app, &sso_bearer, Method::HEAD).await,
        StatusCode::FORBIDDEN,
        "SSO session tokens must be denied on operator LLM config routes before Axum routes HEAD through GET"
    );

    let operator_session = session_payload(&app, "env-operator-token").await;
    assert_eq!(operator_session["user_id"], "operator");
    assert_eq!(
        operator_session["capabilities"]["operator_webui_config"], true,
        "the env bearer token must keep operator capability when SSO is mounted"
    );
    let operator_status = llm_providers_status(&app, "env-operator-token", Method::GET).await;
    assert_ne!(operator_status, StatusCode::UNAUTHORIZED);
    assert_ne!(operator_status, StatusCode::FORBIDDEN);
    assert_ne!(
        operator_status,
        StatusCode::NOT_FOUND,
        "operator routes must be mounted when the composite authenticator contains an env operator token"
    );
}

#[tokio::test]
async fn one_users_bearer_is_rejected_after_tampering() {
    // A signed session bearer must not be malleable into another identity:
    // flipping a byte breaks the HMAC and the protected route rejects it,
    // so a user cannot forge a token for a different user.
    let (app, _services) = build_app(vec![profile("alice-sub", "alice@example.com")]);
    let bearer = login(&app).await;

    let mut tampered = bearer.clone();
    let last = tampered.pop().expect("non-empty");
    tampered.push(if last == 'A' { 'B' } else { 'A' });

    let status = create_thread(&app, &tampered).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a tampered session bearer must not authenticate"
    );
}
