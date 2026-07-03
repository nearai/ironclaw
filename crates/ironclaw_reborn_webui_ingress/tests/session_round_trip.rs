//! End-to-end caller-level test: a session minted via the WebChat
//! v2 OAuth callback is accepted as a bearer on a protected v2 route
//! and stops working after `POST /auth/logout`.
//!
//! Per the issue #4116 acceptance criteria — "session use on a
//! protected WebChat v2 route" — this test composes the full
//! `webui_v2_app` (`ironclaw_reborn_composition`) with:
//!
//! - the OAuth public router from `webui_v2_auth_router` (mints
//!   sessions),
//! - a `SessionAuthenticator` backed by the SAME
//!   `InMemorySessionStore` (validates bearers),
//! - a minimal `RebornServicesApi` stub that only implements
//!   `create_thread` for the round-trip assertion.
//!
//! The chain it locks: OAuth callback → SessionStore::create_session
//! → one-time `login_ticket` exchange → SessionAuthenticator → v2
//! route handler → facade call. A regression that loses any link
//! (e.g. store mismatch, bearer exchange drift, missing user_id
//! stamp) would break exactly the path users hit when they sign in
//! with Google.

#![cfg(feature = "dev-in-memory-session")]

use std::sync::{Arc, Mutex};

use std::net::SocketAddr;

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use chrono::Duration as ChronoDuration;
use http_body_util::BodyExt;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_workflow::{
    LifecyclePackageRef, RebornCancelRunResponse, RebornCreateThreadResponse,
    RebornDeleteThreadRequest, RebornDeleteThreadResponse, RebornExtensionActionResponse,
    RebornExtensionListResponse, RebornExtensionRegistryResponse, RebornGetRunStateRequest,
    RebornGetRunStateResponse, RebornListAutomationsResponse, RebornListThreadsResponse,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundPreferencesResponse,
    RebornResolveGateResponse, RebornServicesApi, RebornServicesError,
    RebornSetOutboundPreferencesRequest, RebornSetupExtensionResponse, RebornSkillActionResponse,
    RebornSkillContentResponse, RebornSkillListResponse, RebornSkillSearchResponse,
    RebornStreamEventsRequest, RebornStreamEventsResponse, RebornSubmitTurnResponse,
    RebornTimelineRequest, RebornTimelineResponse, WebUiAuthenticatedCaller, WebUiCancelRunRequest,
    WebUiCreateThreadRequest, WebUiListAutomationsRequest, WebUiListThreadsRequest,
    WebUiResolveGateRequest, WebUiSendMessageRequest, WebUiSetupExtensionRequest,
    rejecting_reborn_services_error,
};
use ironclaw_reborn_composition::{
    RebornReadiness, RebornWebuiBundle, WebuiServeConfig, webui_v2_app,
};
use ironclaw_reborn_webui_ingress::{
    EmailUserDirectory, InMemorySessionStore, OAuthProvider, OAuthProviderName, OAuthRouterConfig,
    OAuthUserProfile, SessionAuthenticator, SessionStore, webui_v2_auth_router,
};
use ironclaw_threads::{SessionThreadRecord, ThreadScope};
use parking_lot::Mutex as PlMutex;
use serde::Deserialize;
use tower::ServiceExt;

const TENANT: &str = "tenant-a";
const AGENT: &str = "agent-default";
const PROJECT: &str = "project-default";

// ─── stub facade ──────────────────────────────────────────────────────

/// `RebornServicesApi` stub — only `create_thread` returns Ok with a
/// fake thread (the protected route this test exercises). Every
/// other method panics with `unreachable!()` because the test
/// deliberately drives a single mutation; a future expansion that
/// hits another route would fail loudly here, locking the test
/// scope.
#[derive(Default)]
struct StubServices {
    create_thread_callers: Mutex<Vec<WebUiAuthenticatedCaller>>,
}

#[async_trait]
impl RebornServicesApi for StubServices {
    async fn create_thread(
        &self,
        caller: WebUiAuthenticatedCaller,
        _request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
        self.create_thread_callers
            .lock()
            .expect("lock")
            .push(caller);
        Ok(RebornCreateThreadResponse {
            thread: SessionThreadRecord {
                thread_id: ThreadId::new("thread.fake").expect("thread"),
                scope: ThreadScope {
                    tenant_id: TenantId::new(TENANT).expect("tenant"),
                    agent_id: AgentId::new("agent.fake").expect("agent"),
                    project_id: Some(ProjectId::new("project.fake").expect("project")),
                    owner_user_id: Some(UserId::new("alice@example.com").expect("user")),
                    mission_id: None,
                },
                created_by_actor_id: "alice@example.com".to_string(),
                title: None,
                metadata_json: None,
                goal: None,
                created_at: None,
                updated_at: None,
            },
        })
    }

    async fn submit_turn(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiSendMessageRequest,
    ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
        unreachable!("test does not drive submit_turn")
    }

    async fn get_timeline(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, RebornServicesError> {
        unreachable!("test does not drive get_timeline")
    }

    async fn stream_events(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        unreachable!("test does not drive stream_events")
    }

    async fn get_run_state(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
        unreachable!("test does not drive get_run_state")
    }

    async fn cancel_run(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, RebornServicesError> {
        unreachable!("test does not drive cancel_run")
    }

    async fn resolve_gate(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
        unreachable!("test does not drive resolve_gate")
    }

    async fn list_threads(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiListThreadsRequest,
    ) -> Result<RebornListThreadsResponse, RebornServicesError> {
        // Defensive: a future composition layer that pre-warms by
        // listing threads would fall here rather than into the
        // `create_thread` arm. Return an empty page instead of
        // panicking so the test is robust to incidental calls.
        Ok(RebornListThreadsResponse {
            threads: Vec::new(),
            next_cursor: None,
        })
    }

    async fn delete_thread(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornDeleteThreadRequest,
    ) -> Result<RebornDeleteThreadResponse, RebornServicesError> {
        unreachable!("test does not drive delete_thread")
    }

    async fn list_automations(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiListAutomationsRequest,
    ) -> Result<RebornListAutomationsResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn get_outbound_preferences(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn set_outbound_preferences(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn list_outbound_delivery_targets(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn list_extensions(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornExtensionListResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn list_skills(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornSkillListResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn search_skills(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _query: String,
    ) -> Result<RebornSkillSearchResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn install_skill(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _name: String,
        _content: Option<String>,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn read_skill_content(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _name: String,
    ) -> Result<RebornSkillContentResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn update_skill(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _name: String,
        _content: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn remove_skill(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _name: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn list_extension_registry(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornExtensionRegistryResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn install_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn activate_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn remove_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn setup_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _package_ref: LifecyclePackageRef,
        _request: WebUiSetupExtensionRequest,
    ) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }
}

// ─── stub OAuth provider ──────────────────────────────────────────────

struct StubProvider {
    name: OAuthProviderName,
    profile: PlMutex<Option<OAuthUserProfile>>,
}

impl StubProvider {
    fn new(profile: OAuthUserProfile) -> Arc<Self> {
        Arc::new(Self {
            name: OAuthProviderName::new("google").expect("name"),
            profile: PlMutex::new(Some(profile)),
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
    ) -> Result<OAuthUserProfile, ironclaw_reborn_webui_ingress::OAuthError> {
        Ok(self
            .profile
            .lock()
            .take()
            .expect("profile already consumed"))
    }
}

// ─── harness ──────────────────────────────────────────────────────────

fn alice_profile() -> OAuthUserProfile {
    OAuthUserProfile {
        provider_user_id: "google-sub-123".to_string(),
        email: Some("alice@example.com".to_string()),
        email_verified: true,
        verified_emails: vec!["alice@example.com".to_string()],
        display_name: Some("Alice".to_string()),
    }
}

fn state_from_location(location: &str) -> String {
    let query = location.split_once('?').expect("query").1;
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("state=") {
            return urlencoding::decode(value).expect("decode").into_owned();
        }
    }
    panic!("no state in {location}");
}

fn build_app() -> (axum::Router, Arc<StubServices>, Arc<InMemorySessionStore>) {
    let session_store: Arc<InMemorySessionStore> = Arc::new(InMemorySessionStore::new());
    let bearer_authenticator = Arc::new(SessionAuthenticator::new(session_store.clone()));

    let oauth_mount = webui_v2_auth_router(
        OAuthRouterConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            session_store.clone() as Arc<dyn SessionStore>,
            Arc::new(EmailUserDirectory),
            vec![StubProvider::new(alice_profile()) as Arc<dyn OAuthProvider>],
            "https://gateway.example",
        )
        .with_session_lifetime(ChronoDuration::hours(1)),
    );

    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        api: services.clone(),
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        bearer_authenticator,
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
    .with_public_route_mount(oauth_mount);
    let app = webui_v2_app(bundle, config).expect("webui v2 app");
    (app, services, session_store)
}

/// Helper: tag a request with `ConnectInfo` so the descriptor-
/// driven PerIp rate-limit middleware can resolve a peer address.
/// In production, host composition injects this through
/// `into_make_service_with_connect_info`; the `oneshot` harness
/// has to do it explicitly.
fn with_peer(mut req: Request<Body>) -> Request<Body> {
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))));
    req
}

// ─── test ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn session_minted_via_oauth_callback_authenticates_protected_v2_route() {
    let (app, services, session_store) = build_app();

    // 1. Login → capture CSRF state from the stub provider's
    //    authorization URL.
    let login = app
        .clone()
        .oneshot(with_peer(
            Request::builder()
                .method(Method::GET)
                .uri("/auth/login/google?redirect_after=%2Fv2")
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

    // 2. Callback mints a session — extract the one-time ticket from
    //    the SPA-bound redirect, then exchange it for the bearer.
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
    assert!(
        !landing.contains("#token="),
        "callback Location must not carry the bearer: {landing}",
    );
    let ticket = ticket_from_landing(&landing);
    let bearer = redeem_ticket(app.clone(), &ticket).await;
    assert_eq!(session_store.len(), 1, "session must be persisted");

    // 3. Use the bearer on a protected WebChat v2 route. This is
    //    the contract the issue #4116 acceptance criterion calls
    //    out by name: "session use on a protected WebChat v2
    //    route". The stub `create_thread` records the caller so we
    //    can also assert the `UserId` resolved through
    //    `EmailUserDirectory` made it onto the
    //    `WebUiAuthenticatedCaller` stamped by the bearer
    //    middleware.
    let create = app
        .clone()
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
        create.status(),
        StatusCode::OK,
        "OAuth-issued bearer must authenticate on the v2 surface",
    );
    let callers = services.create_thread_callers.lock().expect("lock").clone();
    assert_eq!(callers.len(), 1, "facade reached exactly once");
    assert_eq!(callers[0].tenant_id.as_str(), TENANT);
    assert_eq!(callers[0].user_id.as_str(), "alice@example.com");
    assert_eq!(
        callers[0].agent_id.as_ref().map(|id| id.as_str()),
        Some(AGENT),
        "host-installation default agent_id must be stamped",
    );

    // 4. Logout revokes the session — the SAME bearer must stop
    //    working on the protected v2 route. This is the third
    //    bullet of the acceptance criterion: "Logout revokes the
    //    active server-side session... and prevents subsequent v2
    //    API access with that session."
    let logout = app
        .clone()
        .oneshot(with_peer(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/logout")
                .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                .body(Body::empty())
                .expect("request"),
        ))
        .await
        .expect("oneshot");
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert_eq!(session_store.len(), 0, "session must be revoked");

    let post_logout = app
        .oneshot(with_peer(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"client_action_id":"act-2"}"#))
                .expect("request"),
        ))
        .await
        .expect("oneshot");
    assert_eq!(
        post_logout.status(),
        StatusCode::UNAUTHORIZED,
        "post-logout bearer must NOT authenticate",
    );
    assert_eq!(
        services.create_thread_callers.lock().expect("lock").len(),
        1,
        "facade must not be reached after revoke",
    );
}

#[derive(Deserialize)]
struct SessionExchangeResponse {
    token: String,
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

async fn redeem_ticket(app: axum::Router, ticket: &str) -> String {
    let response = app
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
    assert!(!payload.token.is_empty());
    payload.token
}
