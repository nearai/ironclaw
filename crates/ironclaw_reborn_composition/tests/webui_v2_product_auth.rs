//! Caller-level tests for Reborn WebUI v2 product-auth OAuth routes.

#![cfg(feature = "webui-v2-beta")]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationEvent, AuthFlowManager, AuthInteractionId,
    AuthInteractionService, AuthProductError, AuthProductScope, AuthProviderClient,
    CredentialAccountService, CredentialSetupService, InMemoryAuthProductServices,
    ManualTokenSetupRequest, OAuthProviderCallbackRequest, OAuthProviderExchange,
    OAuthProviderExchangeContext, OAuthProviderRefresh, OAuthProviderRefreshRequest,
    SecretCleanupService, SecretSubmitRequest, SecretSubmitResult,
};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_workflow::{
    ExtensionName, RebornCancelRunResponse, RebornCreateThreadResponse, RebornGetRunStateRequest,
    RebornGetRunStateResponse, RebornListThreadsResponse, RebornResolveGateResponse,
    RebornServicesApi, RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
    RebornSetupExtensionResponse, RebornStreamEventsRequest, RebornStreamEventsResponse,
    RebornSubmitTurnResponse, RebornTimelineRequest, RebornTimelineResponse,
    WebUiAuthenticatedCaller, WebUiCancelRunRequest, WebUiCreateThreadRequest,
    WebUiListThreadsRequest, WebUiResolveGateRequest, WebUiSendMessageRequest,
    WebUiSetupExtensionRequest,
};
use ironclaw_reborn_composition::{
    RebornAuthContinuationDispatcher, RebornProductAuthServices, RebornReadiness,
    RebornWebuiBundle, WebuiAuthenticator, WebuiServeConfig, webui_v2_app,
};
use serde_json::json;
use tower::ServiceExt;

const TENANT: &str = "tenant-alpha";
const USER: &str = "user-alpha";
const AGENT: &str = "agent-default";
const PROJECT: &str = "project-default";
const VALID_TOKEN: &str = "valid-bearer-token";

struct OnlyValidToken;

#[async_trait]
impl WebuiAuthenticator for OnlyValidToken {
    async fn authenticate(&self, token: &str) -> Option<UserId> {
        (token == VALID_TOKEN).then(|| UserId::new(USER).expect("user id"))
    }
}

#[derive(Default)]
struct RecordingAuthDispatcher {
    events: Mutex<Vec<AuthContinuationEvent>>,
}

impl RecordingAuthDispatcher {
    fn events(&self) -> Vec<AuthContinuationEvent> {
        self.events.lock().expect("auth events lock").clone()
    }
}

#[async_trait]
impl RebornAuthContinuationDispatcher for RecordingAuthDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.events.lock().expect("auth events lock").push(event);
        Ok(())
    }
}

struct FailingProviderClient;

#[async_trait]
impl AuthProviderClient for FailingProviderClient {
    async fn exchange_callback(
        &self,
        _context: OAuthProviderExchangeContext,
        _request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        Err(AuthProductError::TokenExchangeFailed)
    }

    async fn refresh_token(
        &self,
        _request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        Err(AuthProductError::RefreshFailed)
    }
}

#[derive(Debug, Default)]
struct SubmitFailingManualTokenInteractions {
    interaction_id: AuthInteractionId,
    abandoned: Mutex<Vec<(AuthProductScope, AuthInteractionId)>>,
}

impl SubmitFailingManualTokenInteractions {
    fn abandoned(&self) -> Vec<(AuthProductScope, AuthInteractionId)> {
        self.abandoned
            .lock()
            .expect("abandoned interactions lock")
            .clone()
    }
}

#[async_trait]
impl AuthInteractionService for SubmitFailingManualTokenInteractions {
    async fn request_secret_input(
        &self,
        request: ManualTokenSetupRequest,
    ) -> Result<AuthChallenge, AuthProductError> {
        Ok(AuthChallenge::ManualTokenRequired {
            interaction_id: self.interaction_id,
            provider: request.provider,
            label: request.label,
            expires_at: request.expires_at,
        })
    }

    async fn submit_manual_token(
        &self,
        _scope: &AuthProductScope,
        _request: SecretSubmitRequest,
    ) -> Result<SecretSubmitResult, AuthProductError> {
        Err(AuthProductError::InvalidRequest {
            reason: "provider rejected token".to_string(),
        })
    }

    async fn abandon_manual_token(
        &self,
        scope: &AuthProductScope,
        interaction_id: AuthInteractionId,
    ) -> Result<bool, AuthProductError> {
        self.abandoned
            .lock()
            .expect("abandoned interactions lock")
            .push((scope.clone(), interaction_id));
        Ok(true)
    }
}

#[derive(Debug)]
struct SetupFailingManualTokenInteractions;

#[async_trait]
impl AuthInteractionService for SetupFailingManualTokenInteractions {
    async fn request_secret_input(
        &self,
        _request: ManualTokenSetupRequest,
    ) -> Result<AuthChallenge, AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }

    async fn submit_manual_token(
        &self,
        _scope: &AuthProductScope,
        _request: SecretSubmitRequest,
    ) -> Result<SecretSubmitResult, AuthProductError> {
        unreachable!("setup-failure test does not submit manual tokens")
    }

    async fn abandon_manual_token(
        &self,
        _scope: &AuthProductScope,
        _interaction_id: AuthInteractionId,
    ) -> Result<bool, AuthProductError> {
        unreachable!("setup-failure test does not abandon manual tokens")
    }
}

struct UnusedServices;

#[async_trait]
impl RebornServicesApi for UnusedServices {
    async fn create_thread(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
        Err(unused_service_error())
    }

    async fn submit_turn(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiSendMessageRequest,
    ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
        Err(unused_service_error())
    }

    async fn get_timeline(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, RebornServicesError> {
        Err(unused_service_error())
    }

    async fn stream_events(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        Err(unused_service_error())
    }

    async fn get_run_state(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
        Err(unused_service_error())
    }

    async fn cancel_run(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, RebornServicesError> {
        Err(unused_service_error())
    }

    async fn resolve_gate(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
        Err(unused_service_error())
    }

    async fn list_threads(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiListThreadsRequest,
    ) -> Result<RebornListThreadsResponse, RebornServicesError> {
        Err(unused_service_error())
    }

    async fn setup_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _extension_name: ExtensionName,
        _request: WebUiSetupExtensionRequest,
    ) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
        Err(unused_service_error())
    }
}

fn unused_service_error() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Internal,
        kind: RebornServicesErrorKind::Internal,
        status_code: 500,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

fn build_app_with_product_auth() -> (axum::Router, Arc<RecordingAuthDispatcher>) {
    let dispatcher = Arc::new(RecordingAuthDispatcher::default());
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        Arc::new(InMemoryAuthProductServices::new()),
        dispatcher.clone(),
    ));
    (
        build_app_with_product_auth_service(product_auth),
        dispatcher,
    )
}

fn build_app_with_product_auth_service(
    product_auth: Arc<RebornProductAuthServices>,
) -> axum::Router {
    let bundle = RebornWebuiBundle {
        api: Arc::new(UnusedServices),
        product_auth: Some(product_auth),
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(OnlyValidToken),
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"));
    webui_v2_app(bundle, config).expect("webui v2 app")
}

fn product_auth_with_interaction_service(
    interaction_service: Arc<dyn AuthInteractionService>,
) -> Arc<RebornProductAuthServices> {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let flow_manager: Arc<dyn AuthFlowManager> = shared.clone();
    let credential_setup_service: Arc<dyn CredentialSetupService> = shared.clone();
    let credential_account_service: Arc<dyn CredentialAccountService> = shared.clone();
    let provider_client: Arc<dyn AuthProviderClient> = shared.clone();
    let cleanup_service: Arc<dyn SecretCleanupService> = shared;

    Arc::new(RebornProductAuthServices::new(
        flow_manager,
        interaction_service,
        credential_setup_service,
        credential_account_service,
        provider_client,
        cleanup_service,
        Arc::new(RecordingAuthDispatcher::default()),
    ))
}

#[derive(Debug)]
struct StartedFlow {
    flow_id: String,
    invocation_id: String,
    body: String,
}

async fn start_oauth_flow(
    app: &axum::Router,
    state: &str,
    pkce: &str,
    extra_fields: serde_json::Value,
) -> StartedFlow {
    let response = post_oauth_start(app, oauth_start_body(state, pkce, extra_fields)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    let json: serde_json::Value = serde_json::from_str(&body).expect("start json");
    StartedFlow {
        flow_id: json["flow_id"].as_str().expect("flow id").to_string(),
        invocation_id: json["callback_scope"]["invocation_id"]
            .as_str()
            .expect("invocation id")
            .to_string(),
        body,
    }
}

fn oauth_start_body(state: &str, pkce: &str, extra_fields: serde_json::Value) -> serde_json::Value {
    let expires_at = (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339();
    let mut body = json!({
        "provider": "github",
        "authorization_url": "https://provider.example/oauth",
        "opaque_state": state,
        "pkce_verifier": pkce,
        "expires_at": expires_at
    });
    merge_json_object(&mut body, extra_fields);
    body
}

async fn post_oauth_start(app: &axum::Router, body: serde_json::Value) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/reborn/product-auth/oauth/start")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot")
}

async fn post_manual_token_submit(
    app: &axum::Router,
    body: serde_json::Value,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/reborn/product-auth/manual-token/submit")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot")
}

fn manual_token_body(token: &str, extra_fields: serde_json::Value) -> serde_json::Value {
    let mut body = json!({
        "provider": "github",
        "account_label": "work github",
        "token": token,
        "run_id": "11111111-1111-1111-1111-111111111111",
        "gate_ref": "gate:auth-github",
        "thread_id": "thread-auth-1"
    });
    merge_json_object(&mut body, extra_fields);
    body
}

fn merge_json_object(target: &mut serde_json::Value, source: serde_json::Value) {
    let Some(target) = target.as_object_mut() else {
        return;
    };
    if let Some(source) = source.as_object() {
        target.extend(source.clone());
    }
}

fn callback_uri(
    flow_id: &str,
    invocation_id: &str,
    user_id: &str,
    state: &str,
    extra_query: &str,
) -> String {
    format!(
        "/api/reborn/product-auth/oauth/callback/{flow_id}\
         ?user_id={user_id}\
         &agent_id={AGENT}\
         &project_id={PROJECT}\
         &invocation_id={invocation_id}\
         &state={state}{extra_query}"
    )
    .replace(' ', "")
}

fn callback_peer(last_octet: u8) -> SocketAddr {
    SocketAddr::from(([203, 0, 113, last_octet], 443))
}

fn callback_request(uri: String) -> Request<Body> {
    callback_request_with_options(uri, Body::empty(), callback_peer(10), None)
}

fn callback_request_with_body(uri: String, body: Body) -> Request<Body> {
    callback_request_with_options(uri, body, callback_peer(10), None)
}

fn callback_request_from_peer(uri: String, peer: SocketAddr) -> Request<Body> {
    callback_request_with_options(uri, Body::empty(), peer, None)
}

fn callback_request_from_peer_with_xff(
    uri: String,
    peer: SocketAddr,
    x_forwarded_for: &'static str,
) -> Request<Body> {
    callback_request_with_options(uri, Body::empty(), peer, Some(x_forwarded_for))
}

fn callback_request_with_options(
    uri: String,
    body: Body,
    peer: SocketAddr,
    x_forwarded_for: Option<&'static str>,
) -> Request<Body> {
    let mut builder = Request::builder().method(Method::GET).uri(uri);
    if let Some(value) = x_forwarded_for {
        builder = builder.header("x-forwarded-for", value);
    }
    let mut request = builder.body(body).expect("request");
    request.extensions_mut().insert(ConnectInfo(peer));
    request
}

async fn read_body_string(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body bytes");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn product_auth_oauth_start_requires_bearer_auth() {
    let (app, _) = build_app_with_product_auth();

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/reborn/product-auth/oauth/start")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({}).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn product_auth_manual_token_submit_requires_bearer_auth() {
    let (app, _) = build_app_with_product_auth();

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/reborn/product-auth/manual-token/submit")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    manual_token_body("ghp_secret", json!({})).to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn product_auth_manual_token_submit_returns_credential_ref_without_exposing_pat() {
    let (app, dispatcher) = build_app_with_product_auth();
    let raw_pat = "ghp_super_secret_manual_pat";

    let response = post_manual_token_submit(&app, manual_token_body(raw_pat, json!({}))).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    assert!(
        !body.contains(raw_pat),
        "manual token response must not expose the raw PAT: {body}"
    );

    let json: serde_json::Value = serde_json::from_str(&body).expect("manual token json");
    assert!(json["credential_ref"].as_str().is_some());
    assert_eq!(json["status"].as_str(), Some("configured"));
    assert_eq!(
        json["continuation"]["type"].as_str(),
        Some("turn_gate_resume")
    );
    assert_eq!(
        json["continuation"]["gate_ref"].as_str(),
        Some("gate:auth-github")
    );
    assert_eq!(
        json["continuation"]["turn_run_ref"].as_str(),
        Some("11111111-1111-1111-1111-111111111111")
    );
    assert!(
        dispatcher.events().is_empty(),
        "manual token submit should return credential_ref; resolve_gate owns turn resumption"
    );
}

#[tokio::test]
async fn product_auth_manual_token_submit_rejects_invalid_secret_without_echoing_it() {
    let (app, _) = build_app_with_product_auth();
    let raw_pat = " padded-ghp-secret ";

    let response = post_manual_token_submit(&app, manual_token_body(raw_pat, json!({}))).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(!body.contains(raw_pat));
    assert!(body.contains("invalid_request"));
}

#[tokio::test]
async fn product_auth_manual_token_submit_abandons_interaction_on_submit_failure() {
    let interactions = Arc::new(SubmitFailingManualTokenInteractions::default());
    let expected_interaction_id = interactions.interaction_id;
    let app = build_app_with_product_auth_service(product_auth_with_interaction_service(
        interactions.clone(),
    ));

    let response = post_manual_token_submit(
        &app,
        manual_token_body(
            "ghp_submit_fails_after_interaction",
            json!({ "thread_id": "thread-cleanup-1" }),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(!body.contains("ghp_submit_fails_after_interaction"));
    assert!(!body.contains("credential_ref"));
    assert!(!body.contains("interaction_id"));

    let abandoned = interactions.abandoned();
    assert_eq!(abandoned.len(), 1);
    assert_eq!(abandoned[0].1, expected_interaction_id);
    assert_eq!(
        abandoned[0].0.resource.tenant_id,
        TenantId::new(TENANT).unwrap()
    );
    assert_eq!(abandoned[0].0.resource.user_id, UserId::new(USER).unwrap());
    assert_eq!(
        abandoned[0]
            .0
            .resource
            .thread_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("thread-cleanup-1")
    );
}

#[tokio::test]
async fn product_auth_manual_token_submit_handles_setup_service_error() {
    let app = build_app_with_product_auth_service(product_auth_with_interaction_service(Arc::new(
        SetupFailingManualTokenInteractions,
    )));
    let raw_pat = "ghp_setup_fails_before_submit";

    let response = post_manual_token_submit(&app, manual_token_body(raw_pat, json!({}))).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"backend_unavailable\""));
    assert!(!body.contains(raw_pat));
    assert!(!body.contains("credential_ref"));
    assert!(!body.contains("interaction_id"));
}

#[tokio::test]
async fn product_auth_manual_token_submit_oversized_body_rejects_before_auth() {
    let (app, _) = build_app_with_product_auth();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/reborn/product-auth/manual-token/submit")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("x".repeat(17 * 1024)))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn product_auth_manual_token_submit_has_per_caller_rate_limit() {
    let (app, _) = build_app_with_product_auth();

    for index in 0..20 {
        let response = post_manual_token_submit(
            &app,
            manual_token_body(&format!("ghp_secret_{index}"), json!({})),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    let response =
        post_manual_token_submit(&app, manual_token_body("ghp_secret_over", json!({}))).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn product_auth_manual_token_submit_invalid_fields_are_sanitized() {
    let (app, _) = build_app_with_product_auth();

    let invalid_requests = [
        manual_token_body("ghp_invalid_provider_secret", json!({ "provider": "" })),
        manual_token_body("ghp_invalid_label_secret", json!({ "account_label": "" })),
        manual_token_body("ghp_invalid_run_secret", json!({ "run_id": "" })),
        manual_token_body("ghp_invalid_gate_secret", json!({ "gate_ref": "" })),
    ];

    for body in invalid_requests {
        let response = post_manual_token_submit(&app, body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = read_body_string(response).await;
        assert!(body.contains("\"code\":\"invalid_request\""));
        assert!(!body.contains("ghp_invalid_provider_secret"));
        assert!(!body.contains("ghp_invalid_label_secret"));
        assert!(!body.contains("ghp_invalid_run_secret"));
        assert!(!body.contains("ghp_invalid_gate_secret"));
    }
}

#[tokio::test]
async fn product_auth_manual_token_submit_invalid_scope_fields_are_sanitized() {
    let (app, _) = build_app_with_product_auth();

    let invalid_requests = [
        manual_token_body(
            "ghp_invalid_thread_secret",
            json!({ "thread_id": "bad/thread" }),
        ),
        manual_token_body(
            "ghp_invalid_session_secret",
            json!({ "session_id": "bad\u{0}session" }),
        ),
    ];

    for body in invalid_requests {
        let response = post_manual_token_submit(&app, body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = read_body_string(response).await;
        assert!(body.contains("\"code\":\"invalid_request\""));
        assert!(!body.contains("ghp_invalid_thread_secret"));
        assert!(!body.contains("ghp_invalid_session_secret"));
        assert!(!body.contains("credential_ref"));
    }
}

#[tokio::test]
async fn product_auth_oauth_start_oversized_body_rejects_before_auth() {
    let (app, _) = build_app_with_product_auth();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/reborn/product-auth/oauth/start")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("x".repeat(17 * 1024)))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn product_auth_oauth_start_has_per_caller_rate_limit() {
    let (app, _) = build_app_with_product_auth();

    for index in 0..20 {
        let response = post_oauth_start(
            &app,
            oauth_start_body(
                &format!("start-rate-state-{index}"),
                &format!("start-rate-pkce-{index}"),
                json!({}),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    let response = post_oauth_start(
        &app,
        oauth_start_body("start-rate-state-over", "start-rate-pkce-over", json!({})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn product_auth_oauth_start_invalid_requests_are_sanitized() {
    let (app, _) = build_app_with_product_auth();

    let invalid_requests = [
        oauth_start_body(
            "expired-start-state",
            "expired-start-pkce",
            json!({ "expires_at": (Utc::now() - ChronoDuration::minutes(1)).to_rfc3339() }),
        ),
        oauth_start_body(
            "far-future-start-state",
            "far-future-start-pkce",
            json!({ "expires_at": (Utc::now() + ChronoDuration::hours(1)).to_rfc3339() }),
        ),
        oauth_start_body(
            "bad-provider-state",
            "bad-provider-pkce",
            json!({ "provider": "" }),
        ),
        oauth_start_body(
            "bad-url-state",
            "bad-url-pkce",
            json!({ "authorization_url": "http://provider.example/oauth" }),
        ),
        oauth_start_body(
            "precomposed-url-state",
            "precomposed-url-pkce",
            json!({ "authorization_url": "https://provider.example/oauth?state=precomposed-url-state&code_challenge=precomposed-url-pkce" }),
        ),
        oauth_start_body(" padded-start-state ", "padded-start-pkce", json!({})),
        oauth_start_body("bad-pkce-state", " padded-start-pkce ", json!({})),
        oauth_start_body(
            "bad-thread-state",
            "bad-thread-pkce",
            json!({ "thread_id": "" }),
        ),
    ];

    for body in invalid_requests {
        let response = post_oauth_start(&app, body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = read_body_string(response).await;
        assert!(body.contains("\"code\":\"invalid_request\""));
        assert!(!body.contains("expired-start-state"));
        assert!(!body.contains("far-future-start-pkce"));
        assert!(!body.contains("bad-provider-pkce"));
        assert!(!body.contains("precomposed-url-state"));
        assert!(!body.contains("precomposed-url-pkce"));
        assert!(!body.contains("padded-start-pkce"));
        assert!(!body.contains("bad-thread-state"));
    }
}

#[tokio::test]
async fn product_auth_oauth_routes_create_flow_and_complete_callback() {
    let (app, dispatcher) = build_app_with_product_auth();
    let started = start_oauth_flow(
        &app,
        "route-state-secret",
        "route-pkce-secret",
        json!({
            "session_id": "web-session-1",
            "thread_id": "thread-auth-1"
        }),
    )
    .await;
    assert!(!started.body.contains("route-state-secret"));
    assert!(!started.body.contains("route-pkce-secret"));
    let start_json: serde_json::Value = serde_json::from_str(&started.body).expect("start json");
    let callback_scope = &start_json["callback_scope"];
    assert_eq!(callback_scope["user_id"], USER);
    assert_eq!(callback_scope["agent_id"], AGENT);
    assert_eq!(callback_scope["project_id"], PROJECT);
    assert_eq!(start_json["continuation"]["type"], "setup_only");
    let authorization_url = start_json["authorization_url"]
        .as_str()
        .expect("authorization url");
    assert!(authorization_url.contains(&started.flow_id));
    assert!(authorization_url.contains(&started.invocation_id));
    assert!(!authorization_url.contains("route-state-secret"));
    assert!(!authorization_url.contains("route-pkce-secret"));

    let callback_response = app
        .oneshot(
            callback_request(callback_uri(
                &started.flow_id,
                &started.invocation_id,
                USER,
                "route-state-secret",
                "&thread_id=thread-auth-1&session_id=web-session-1&provider=github&account_label=work%20github&code=route-auth-code&scopes=repo",
            )),
        )
        .await
        .expect("oneshot");
    assert_eq!(callback_response.status(), StatusCode::OK);
    let callback_body = read_body_string(callback_response).await;
    assert!(!callback_body.contains("route-state-secret"));
    assert!(!callback_body.contains("route-pkce-secret"));
    assert!(!callback_body.contains("route-auth-code"));
    assert!(!callback_body.contains("oauth-access"));
    assert!(!callback_body.contains("oauth-refresh"));

    let callback_json: serde_json::Value =
        serde_json::from_str(&callback_body).expect("callback json");
    assert_eq!(callback_json["flow_id"], started.flow_id);
    assert_eq!(callback_json["status"], "completed");
    assert_eq!(dispatcher.events().len(), 1);
}

#[tokio::test]
async fn product_auth_callback_provider_denial_is_sanitized() {
    let (app, dispatcher) = build_app_with_product_auth();
    let started = start_oauth_flow(
        &app,
        "provider-denied-state",
        "provider-denied-pkce",
        json!({}),
    )
    .await;

    let response = app
        .oneshot(callback_request(callback_uri(
            &started.flow_id,
            &started.invocation_id,
            USER,
            "provider-denied-state",
            "&error=access_denied",
        )))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"provider_denied\""));
    assert!(!body.contains("provider-denied-state"));
    assert!(!body.contains("access_denied"));
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn product_auth_callback_unknown_flow_is_sanitized() {
    let (app, dispatcher) = build_app_with_product_auth();
    let flow_id = uuid::Uuid::new_v4().to_string();
    let invocation_id = ironclaw_host_api::InvocationId::new().to_string();
    let response = app
        .oneshot(callback_request(callback_uri(
            &flow_id,
            &invocation_id,
            USER,
            "unknown-flow-state",
            "&error=access_denied",
        )))
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"unknown_or_expired_flow\""));
    assert!(!body.contains("unknown-flow-state"));
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn product_auth_authorized_callback_unknown_flow_is_sanitized() {
    let (app, dispatcher) = build_app_with_product_auth();
    let flow_id = uuid::Uuid::new_v4().to_string();
    let invocation_id = ironclaw_host_api::InvocationId::new().to_string();
    let response = app
        .oneshot(callback_request(callback_uri(
            &flow_id,
            &invocation_id,
            USER,
            "unknown-authorized-state",
            "&provider=github&account_label=work%20github&code=unknown-authorized-code",
        )))
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"unknown_or_expired_flow\""));
    assert!(!body.contains("unknown-authorized-state"));
    assert!(!body.contains("unknown-authorized-code"));
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn product_auth_callback_malformed_fields_are_sanitized() {
    let (app, dispatcher) = build_app_with_product_auth();
    let started = start_oauth_flow(
        &app,
        "malformed-field-state",
        "malformed-field-pkce",
        json!({}),
    )
    .await;

    let malformed_uris = [
        format!(
            "/api/reborn/product-auth/oauth/callback/{}?user_id={USER}&agent_id={AGENT}&project_id={PROJECT}&invocation_id={}&provider=github&account_label=work&code=missing-state-code",
            started.flow_id, started.invocation_id
        ),
        callback_uri(
            &started.flow_id,
            &started.invocation_id,
            USER,
            "malformed-field-state",
            "&account_label=work&code=missing-provider-code",
        ),
        callback_uri(
            &started.flow_id,
            &started.invocation_id,
            USER,
            "malformed-field-state",
            "&provider=github&code=missing-label-code",
        ),
        callback_uri(
            &started.flow_id,
            &started.invocation_id,
            USER,
            "malformed-field-state",
            "&provider=github&account_label=work",
        ),
        callback_uri(
            &started.flow_id,
            &started.invocation_id,
            USER,
            "malformed-field-state",
            "&provider=&account_label=work&code=empty-provider-code",
        ),
        callback_uri(
            &started.flow_id,
            &started.invocation_id,
            USER,
            "malformed-field-state",
            "&provider=github&account_label=%20work&code=bad-label-code",
        ),
        callback_uri(
            &started.flow_id,
            &started.invocation_id,
            USER,
            "malformed-field-state",
            "&provider=github&account_label=work&code=bad-scopes-code&scopes=repo,,gist",
        ),
        callback_uri(
            &started.flow_id,
            &started.invocation_id,
            USER,
            "malformed-field-state",
            "&provider=github&account_label=work&code=missing-scopes-code",
        ),
        callback_uri(
            &started.flow_id,
            &started.invocation_id,
            USER,
            "malformed-field-state",
            "&provider=github&account_label=work&code=empty-scopes-code&scopes=",
        ),
    ];

    for uri in malformed_uris {
        let response = app
            .clone()
            .oneshot(callback_request(uri))
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = read_body_string(response).await;
        assert!(body.contains("\"code\":\"malformed_callback\""));
        assert!(!body.contains("malformed-field-state"));
        assert!(!body.contains("malformed-field-pkce"));
    }
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn product_auth_callback_rejects_request_body() {
    let (app, dispatcher) = build_app_with_product_auth();
    let flow_id = uuid::Uuid::new_v4().to_string();
    let invocation_id = ironclaw_host_api::InvocationId::new().to_string();
    let response = app
        .oneshot(callback_request_with_body(
            callback_uri(
                &flow_id,
                &invocation_id,
                USER,
                "callback-body-state",
                "&error=access_denied",
            ),
            Body::from("body-not-allowed"),
        ))
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn product_auth_callback_has_peer_ip_scoped_rate_limit() {
    let (app, dispatcher) = build_app_with_product_auth();
    let make_request = |peer: SocketAddr| {
        let flow_id = uuid::Uuid::new_v4().to_string();
        let invocation_id = ironclaw_host_api::InvocationId::new().to_string();
        callback_request_from_peer(
            callback_uri(
                &flow_id,
                &invocation_id,
                USER,
                "callback-rate-state",
                "&error=access_denied",
            ),
            peer,
        )
    };
    let first_peer = callback_peer(10);
    let second_peer = callback_peer(11);

    for _ in 0..120 {
        let response = app
            .clone()
            .oneshot(make_request(first_peer))
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let response = app
        .clone()
        .oneshot(make_request(first_peer))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let response = app
        .oneshot(make_request(second_peer))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn product_auth_callback_rate_limit_ignores_spoofed_forwarded_headers() {
    let (app, dispatcher) = build_app_with_product_auth();
    let peer = callback_peer(20);
    let make_request = |xff: &'static str| {
        let flow_id = uuid::Uuid::new_v4().to_string();
        let invocation_id = ironclaw_host_api::InvocationId::new().to_string();
        callback_request_from_peer_with_xff(
            callback_uri(
                &flow_id,
                &invocation_id,
                USER,
                "callback-rate-state",
                "&error=access_denied",
            ),
            peer,
            xff,
        )
    };

    for index in 0..120 {
        let response = app
            .clone()
            .oneshot(make_request(if index % 2 == 0 {
                "198.51.100.10"
            } else {
                "198.51.100.11"
            }))
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let response = app
        .oneshot(make_request("198.51.100.12"))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn product_auth_callback_provider_exchange_failure_is_sanitized() {
    let dispatcher = Arc::new(RecordingAuthDispatcher::default());
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(
            Arc::new(InMemoryAuthProductServices::new()),
            dispatcher.clone(),
        )
        .with_provider_client(Arc::new(FailingProviderClient)),
    );
    let app = build_app_with_product_auth_service(product_auth);
    let started = start_oauth_flow(
        &app,
        "exchange-failed-state",
        "exchange-failed-pkce",
        json!({}),
    )
    .await;

    let response = app
        .oneshot(callback_request(callback_uri(
            &started.flow_id,
            &started.invocation_id,
            USER,
            "exchange-failed-state",
            "&provider=github&account_label=work%20github&code=exchange-failed-code&scopes=repo",
        )))
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"token_exchange_failed\""));
    assert!(!body.contains("exchange-failed-state"));
    assert!(!body.contains("exchange-failed-pkce"));
    assert!(!body.contains("exchange-failed-code"));
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn product_auth_callback_cross_scope_failure_is_sanitized() {
    let (app, dispatcher) = build_app_with_product_auth();
    let started = start_oauth_flow(&app, "wrong-scope-state", "wrong-scope-pkce", json!({})).await;

    let callback_response = app
        .oneshot(callback_request(callback_uri(
            &started.flow_id,
            &started.invocation_id,
            "bob",
            "wrong-scope-state",
            "&provider=github&account_label=work%20github&code=wrong-scope-code",
        )))
        .await
        .expect("oneshot");
    assert_eq!(callback_response.status(), StatusCode::FORBIDDEN);
    let body = read_body_string(callback_response).await;
    assert!(body.contains("\"code\":\"cross_scope_denied\""));
    assert!(!body.contains("wrong-scope-state"));
    assert!(!body.contains("wrong-scope-pkce"));
    assert!(!body.contains("wrong-scope-code"));
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn product_auth_callback_malformed_flow_id_uses_sanitized_error() {
    let (app, dispatcher) = build_app_with_product_auth();
    let invocation_id = ironclaw_host_api::InvocationId::new().to_string();

    let response = app
        .oneshot(callback_request(callback_uri(
            "not-a-flow-id",
            &invocation_id,
            USER,
            "malformed-flow-state",
            "&provider=github&account_label=work%20github&code=malformed-flow-code",
        )))
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"malformed_callback\""));
    assert!(!body.contains("malformed-flow-state"));
    assert!(!body.contains("malformed-flow-code"));
    assert!(!body.contains("malformed-flow-pkce"));
    assert!(dispatcher.events().is_empty());
}
