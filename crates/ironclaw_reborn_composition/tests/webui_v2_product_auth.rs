//! Caller-level tests for Reborn WebUI v2 product-auth OAuth routes.

#![cfg(feature = "webui-v2-beta")]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthContinuationEvent, AuthProductError, AuthProviderClient, InMemoryAuthProductServices,
    OAuthProviderCallbackRequest, OAuthProviderExchange,
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
        _request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        Err(AuthProductError::TokenExchangeFailed)
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
    let expires_at = (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339();
    let mut body = json!({
        "provider": "github",
        "authorization_url": "https://provider.example/oauth?client_id=reborn",
        "opaque_state": state,
        "pkce_verifier": pkce,
        "expires_at": expires_at
    });
    merge_json_object(&mut body, extra_fields);

    let response = app
        .clone()
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
        .expect("oneshot");
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

    let callback_response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(callback_uri(
                    &started.flow_id,
                    &started.invocation_id,
                    USER,
                    "route-state-secret",
                    "&thread_id=thread-auth-1&session_id=web-session-1&provider=github&account_label=work%20github&code=route-auth-code&scopes=repo",
                ))
                .header("x-reborn-pkce-verifier", "route-pkce-secret")
                .body(Body::empty())
                .expect("request"),
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
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(callback_uri(
                    &started.flow_id,
                    &started.invocation_id,
                    USER,
                    "provider-denied-state",
                    "&error=access_denied",
                ))
                .body(Body::empty())
                .expect("request"),
        )
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
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(callback_uri(
                    &flow_id,
                    &invocation_id,
                    USER,
                    "unknown-flow-state",
                    "&error=access_denied",
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"unknown_or_expired_flow\""));
    assert!(!body.contains("unknown-flow-state"));
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn product_auth_callback_rejects_request_body() {
    let (app, dispatcher) = build_app_with_product_auth();
    let flow_id = uuid::Uuid::new_v4().to_string();
    let invocation_id = ironclaw_host_api::InvocationId::new().to_string();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(callback_uri(
                    &flow_id,
                    &invocation_id,
                    USER,
                    "callback-body-state",
                    "&error=access_denied",
                ))
                .body(Body::from("body-not-allowed"))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn product_auth_callback_has_route_scoped_rate_limit() {
    let (app, dispatcher) = build_app_with_product_auth();
    let make_request = || {
        let flow_id = uuid::Uuid::new_v4().to_string();
        let invocation_id = ironclaw_host_api::InvocationId::new().to_string();
        Request::builder()
            .method(Method::GET)
            .uri(callback_uri(
                &flow_id,
                &invocation_id,
                USER,
                "callback-rate-state",
                "&error=access_denied",
            ))
            .body(Body::empty())
            .expect("request")
    };

    for _ in 0..120 {
        let response = app.clone().oneshot(make_request()).await.expect("oneshot");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let response = app.oneshot(make_request()).await.expect("oneshot");
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
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(callback_uri(
                    &started.flow_id,
                    &started.invocation_id,
                    USER,
                    "exchange-failed-state",
                    "&provider=github&account_label=work%20github&code=exchange-failed-code&scopes=repo",
                ))
                .header("x-reborn-pkce-verifier", "exchange-failed-pkce")
                .body(Body::empty())
                .expect("request"),
        )
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
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(callback_uri(
                    &started.flow_id,
                    &started.invocation_id,
                    "bob",
                    "wrong-scope-state",
                    "&provider=github&account_label=work%20github&code=wrong-scope-code",
                ))
                .header("x-reborn-pkce-verifier", "wrong-scope-pkce")
                .body(Body::empty())
                .expect("request"),
        )
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
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(callback_uri(
                    "not-a-flow-id",
                    &invocation_id,
                    USER,
                    "malformed-flow-state",
                    "&provider=github&account_label=work%20github&code=malformed-flow-code",
                ))
                .header("x-reborn-pkce-verifier", "malformed-flow-pkce")
                .body(Body::empty())
                .expect("request"),
        )
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
