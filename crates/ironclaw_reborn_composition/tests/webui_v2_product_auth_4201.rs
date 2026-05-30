//! Caller-level tests for issue #4201: product-facing HTTP surfaces for
//! manual-token setup/secret-submit, credential account list/select/recovery,
//! refresh, and lifecycle cleanup.
//!
//! These tests drive the HTTP routes end-to-end through `webui_v2_app` so the
//! caller path (auth layer + body limit + rate limit + handler +
//! `RebornProductAuthServices`) is exercised, not just the facade helpers.

#![cfg(feature = "webui-v2-beta")]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use ironclaw_auth::{
    AuthContinuationEvent, AuthProductError, AuthProductScope, AuthSurface, CredentialAccountLabel,
    CredentialAccountStatus, CredentialOwnership, InMemoryAuthProductServices,
    NewCredentialAccount,
};
use ironclaw_auth::{AuthProviderId, CredentialAccountId, CredentialAccountService};
use ironclaw_host_api::{AgentId, InvocationId, ProjectId, ResourceScope, TenantId, UserId};
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
use serde_json::{Value, json};
use tower::ServiceExt;

const TENANT: &str = "tenant-4201";
const USER: &str = "user-4201";
const AGENT: &str = "agent-4201";
const PROJECT: &str = "project-4201";
const VALID_TOKEN: &str = "valid-bearer-token-4201";

struct OnlyValidToken;

#[async_trait]
impl WebuiAuthenticator for OnlyValidToken {
    async fn authenticate(&self, token: &str) -> Option<UserId> {
        (token == VALID_TOKEN).then(|| UserId::new(USER).expect("user id"))
    }
}

#[derive(Default)]
struct NoopAuthDispatcher {
    events: Mutex<Vec<AuthContinuationEvent>>,
}

#[async_trait]
impl RebornAuthContinuationDispatcher for NoopAuthDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.events.lock().expect("auth events lock").push(event);
        Ok(())
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

struct AppFixture {
    app: axum::Router,
    shared: Arc<InMemoryAuthProductServices>,
}

fn build_fixture() -> AppFixture {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        shared.clone(),
        Arc::new(NoopAuthDispatcher::default()),
    ));
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
    let app = webui_v2_app(bundle, config).expect("webui v2 app");
    AppFixture { app, shared }
}

fn caller_scope_with_invocation(invocation_id: InvocationId) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            user_id: UserId::new(USER).expect("user"),
            agent_id: Some(AgentId::new(AGENT).expect("agent")),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            mission_id: None,
            thread_id: None,
            invocation_id,
        },
        AuthSurface::Callback,
    )
}

async fn seed_configured_account(
    shared: &InMemoryAuthProductServices,
    invocation_id: InvocationId,
    provider: &str,
    label: &str,
) -> ironclaw_auth::CredentialAccountId {
    let account = shared
        .create_account(NewCredentialAccount {
            scope: caller_scope_with_invocation(invocation_id),
            provider: AuthProviderId::new(provider.to_string()).expect("provider"),
            label: CredentialAccountLabel::new(label.to_string()).expect("label"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: None,
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("seeded account");
    account.id
}

async fn read_body_string(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body bytes");
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn post_authenticated(
    app: &axum::Router,
    uri: &str,
    body: Value,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(uri)
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot")
}

async fn post_unauthenticated(
    app: &axum::Router,
    uri: &str,
    body: Value,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot")
}

const PATHS: &[&str] = &[
    "/api/reborn/product-auth/manual-token/setup",
    "/api/reborn/product-auth/manual-token/secret-submit",
    "/api/reborn/product-auth/accounts/list",
    "/api/reborn/product-auth/accounts/select",
    "/api/reborn/product-auth/accounts/recovery",
    "/api/reborn/product-auth/accounts/refresh",
    "/api/reborn/product-auth/lifecycle/cleanup",
];

#[tokio::test]
async fn product_auth_new_routes_require_bearer_auth() {
    let fixture = build_fixture();
    for path in PATHS {
        let response = post_unauthenticated(&fixture.app, path, json!({})).await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{path} must require bearer auth"
        );
    }
}

#[tokio::test]
async fn manual_token_setup_then_secret_submit_returns_redacted_projection() {
    let fixture = build_fixture();
    let raw_token = "ghp_routing_through_4201_secret";

    let setup_response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/manual-token/setup",
        json!({
            "provider": "github",
            "account_label": "work github 4201",
            "run_id": "22222222-2222-2222-2222-222222222222",
            "gate_ref": "gate:auth-github-4201",
            "thread_id": "thread-auth-4201"
        }),
    )
    .await;
    assert_eq!(setup_response.status(), StatusCode::OK);
    let setup_body = read_body_string(setup_response).await;
    let setup_json: Value = serde_json::from_str(&setup_body).expect("setup json");
    let interaction_id = setup_json["interaction_id"]
        .as_str()
        .expect("interaction id")
        .to_string();
    let invocation_id = setup_json["invocation_id"]
        .as_str()
        .expect("invocation id from setup response")
        .to_string();
    assert_eq!(setup_json["provider"].as_str(), Some("github"));
    assert_eq!(setup_json["label"].as_str(), Some("work github 4201"));

    let submit_response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/manual-token/secret-submit",
        json!({
            "interaction_id": interaction_id,
            "token": raw_token,
            "thread_id": "thread-auth-4201",
            "invocation_id": invocation_id
        }),
    )
    .await;
    assert_eq!(submit_response.status(), StatusCode::OK);
    let submit_body = read_body_string(submit_response).await;
    assert!(
        !submit_body.contains(raw_token),
        "secret-submit response must not echo raw token: {submit_body}"
    );
    assert!(
        !submit_body.contains("interaction_id"),
        "secret-submit response must not echo interaction_id: {submit_body}"
    );
    let submit_json: Value = serde_json::from_str(&submit_body).expect("submit json");
    assert!(submit_json["credential_ref"].as_str().is_some());
    assert_eq!(submit_json["status"].as_str(), Some("configured"));
    assert_eq!(
        submit_json["continuation"]["type"].as_str(),
        Some("turn_gate_resume")
    );
    assert_eq!(
        submit_json["continuation"]["gate_ref"].as_str(),
        Some("gate:auth-github-4201")
    );
}

#[tokio::test]
async fn manual_token_setup_rejects_partial_continuation_inputs() {
    let fixture = build_fixture();
    let invalid_bodies = [
        json!({
            "provider": "github",
            "account_label": "label-only-run",
            "run_id": "22222222-2222-2222-2222-222222222222"
        }),
        json!({
            "provider": "github",
            "account_label": "label-only-gate",
            "gate_ref": "gate:auth-github"
        }),
    ];
    for body in invalid_bodies {
        let response = post_authenticated(
            &fixture.app,
            "/api/reborn/product-auth/manual-token/setup",
            body,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = read_body_string(response).await;
        assert!(body.contains("\"code\":\"invalid_request\""));
    }
}

#[tokio::test]
async fn manual_token_secret_submit_invalid_interaction_is_sanitized() {
    let fixture = build_fixture();
    let raw_token = "ghp_invalid_interaction_secret";

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/manual-token/secret-submit",
        json!({
            "interaction_id": "not-a-uuid",
            "token": raw_token
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"invalid_request\""));
    assert!(!body.contains(raw_token));
}

#[tokio::test]
async fn accounts_list_returns_only_seeded_provider_accounts() {
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();
    let github_id =
        seed_configured_account(&fixture.shared, invocation_id, "github", "work github").await;
    let _slack_id =
        seed_configured_account(&fixture.shared, invocation_id, "slack", "work slack").await;

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/list",
        json!({
            "provider": "github",
            "invocation_id": invocation_id.to_string()
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    let json: Value = serde_json::from_str(&body).expect("list json");
    let accounts = json["accounts"].as_array().expect("accounts array");
    assert_eq!(accounts.len(), 1);
    assert_eq!(
        accounts[0]["id"].as_str(),
        Some(github_id.to_string().as_str())
    );
    assert_eq!(accounts[0]["provider"].as_str(), Some("github"));
    assert_eq!(accounts[0]["status"].as_str(), Some("configured"));
    // Redacted projection must never carry secret handle names.
    assert!(!body.contains("access_secret"));
    assert!(!body.contains("refresh_secret"));
}

#[tokio::test]
async fn accounts_list_invalid_limit_is_sanitized() {
    let fixture = build_fixture();
    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/list",
        json!({ "provider": "github", "limit": 0 }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"invalid_request\""));
}

#[tokio::test]
async fn accounts_select_returns_redacted_projection() {
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();
    let account_id =
        seed_configured_account(&fixture.shared, invocation_id, "github", "work github").await;

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/select",
        json!({
            "provider": "github",
            "account_id": account_id.to_string(),
            "invocation_id": invocation_id.to_string()
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    let json: Value = serde_json::from_str(&body).expect("select json");
    assert_eq!(json["id"].as_str(), Some(account_id.to_string().as_str()));
    assert_eq!(json["status"].as_str(), Some("configured"));
    assert!(!body.contains("access_secret"));
    assert!(!body.contains("refresh_secret"));
}

#[tokio::test]
async fn accounts_recovery_projects_setup_required_when_no_account_exists() {
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/recovery",
        json!({ "provider": "github", "invocation_id": invocation_id.to_string() }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    let json: Value = serde_json::from_str(&body).expect("recovery json");
    assert_eq!(json["provider"].as_str(), Some("github"));
    assert_eq!(json["kind"].as_str(), Some("setup_required"));
    assert_eq!(json["reason"].as_str(), Some("no_account"));
    assert!(!body.contains("access_secret"));
    assert!(!body.contains("refresh_secret"));
}

#[tokio::test]
async fn lifecycle_cleanup_redacts_report_and_reaches_service() {
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();
    // Seed an unrelated account so cleanup has scope to walk but no extension owns it.
    let _account_id =
        seed_configured_account(&fixture.shared, invocation_id, "github", "work github").await;

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/lifecycle/cleanup",
        json!({
            "extension_id": "ext-no-grant-4201",
            "action": "deactivate",
            "invocation_id": invocation_id.to_string()
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    let json: Value = serde_json::from_str(&body).expect("cleanup json");
    // No matching extension grant: report must be empty but well-formed.
    assert_eq!(
        json,
        json!({}),
        "cleanup report must omit empty arrays via skip_serializing_if"
    );
}

#[tokio::test]
async fn lifecycle_cleanup_rejects_invalid_extension_id() {
    let fixture = build_fixture();

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/lifecycle/cleanup",
        json!({ "extension_id": "", "action": "deactivate" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"invalid_request\""));
}

#[tokio::test]
async fn accounts_refresh_returns_report_for_seeded_account() {
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();
    let account_id =
        seed_configured_account(&fixture.shared, invocation_id, "github", "refresh-test").await;

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/refresh",
        json!({
            "provider": "github",
            "account_id": account_id.to_string(),
            "invocation_id": invocation_id.to_string()
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    let json: Value = serde_json::from_str(&body).expect("refresh json");
    assert_eq!(
        json["account"]["id"].as_str(),
        Some(account_id.to_string().as_str())
    );
    assert!(json["recovery"].is_object(), "recovery must be present");
    assert!(json["refreshed"].is_boolean(), "refreshed must be present");
    // Redacted projection must never carry secret handle names.
    assert!(!body.contains("access_secret"));
    assert!(!body.contains("refresh_secret"));
}

#[tokio::test]
async fn manual_token_secret_submit_requires_invocation_id() {
    // Omitting invocation_id means the host cannot re-derive the setup scope;
    // the route must reject with invalid_request rather than minting a fresh
    // invocation that will never match the pending interaction.
    let fixture = build_fixture();
    let raw_token = "ghp_should_not_be_echoed_invocation_required";

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/manual-token/secret-submit",
        json!({
            "interaction_id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            "token": raw_token
            // invocation_id intentionally absent
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"invalid_request\""));
    assert!(
        !body.contains(raw_token),
        "raw token must not be echoed: {body}"
    );
}

#[tokio::test]
async fn accounts_list_requires_invocation_id() {
    // Omitting invocation_id would cause a fresh scope to be minted, silently
    // returning an empty page instead of scoped results.
    let fixture = build_fixture();

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/list",
        json!({ "provider": "github" /* invocation_id absent */ }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"invalid_request\""));
}

#[tokio::test]
async fn new_routes_reject_malformed_invocation_id() {
    // All new routes that accept invocation_id must return 400 on a non-UUID
    // value so audit tooling can confirm the validation path is live.
    let fixture = build_fixture();
    let cases: &[(&str, Value)] = &[
        (
            "/api/reborn/product-auth/manual-token/secret-submit",
            json!({
                "interaction_id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "token": "tok",
                "invocation_id": "not-a-uuid"
            }),
        ),
        (
            "/api/reborn/product-auth/accounts/list",
            json!({ "provider": "github", "invocation_id": "not-a-uuid" }),
        ),
        (
            "/api/reborn/product-auth/accounts/select",
            json!({
                "provider": "github",
                "account_id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "invocation_id": "not-a-uuid"
            }),
        ),
        (
            "/api/reborn/product-auth/accounts/recovery",
            json!({ "provider": "github", "invocation_id": "not-a-uuid" }),
        ),
        (
            "/api/reborn/product-auth/accounts/refresh",
            json!({
                "provider": "github",
                "account_id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "invocation_id": "not-a-uuid"
            }),
        ),
        (
            "/api/reborn/product-auth/lifecycle/cleanup",
            json!({
                "extension_id": "ext-test",
                "action": "deactivate",
                "invocation_id": "not-a-uuid"
            }),
        ),
    ];
    for (path, body) in cases {
        let response = post_authenticated(&fixture.app, path, body.clone()).await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{path} must reject malformed invocation_id"
        );
        let body_str = read_body_string(response).await;
        assert!(
            body_str.contains("\"code\":\"invalid_request\""),
            "{path} must return invalid_request for malformed invocation_id: {body_str}"
        );
    }
}

#[tokio::test]
async fn accounts_select_rejects_malformed_account_id() {
    let fixture = build_fixture();

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/select",
        json!({
            "provider": "github",
            "account_id": "not-a-uuid",
            "invocation_id": InvocationId::new().to_string()
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"invalid_request\""));
}

// --- finding t1: body-limit enforcement ---

#[tokio::test]
async fn new_product_auth_routes_enforce_body_limit() {
    // PROTECTED_MUTATION policy sets BodyLimitPolicy::Limited{16 KiB}.
    // Any route receiving a body that exceeds this cap must be rejected
    // with 413 before auth or the handler runs.
    let fixture = build_fixture();
    // Construct a JSON payload that exceeds 16 KiB.
    let padding = "x".repeat(16 * 1024 + 1);
    let oversized_body = format!("{{\"provider\":\"github\",\"padding\":\"{padding}\"}}");
    assert!(oversized_body.len() > 16 * 1024);

    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/reborn/product-auth/manual-token/setup")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(oversized_body))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "oversized body must be rejected before the handler"
    );
    let body = read_body_string(response).await;
    assert!(
        body.contains("body limit"),
        "413 body should reference the body limit, got: {body}"
    );
}

// --- finding t2: rate-limit enforcement ---

#[tokio::test]
async fn new_product_auth_routes_enforce_per_caller_rate_limit() {
    // PROTECTED_MUTATION declares RateLimitPolicy::Limited{PerCaller, 20 req/60s}.
    // Send 20 requests from the same bearer token and confirm the 21st returns 429.
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();
    let body = json!({
        "provider": "github",
        "invocation_id": invocation_id.to_string()
    })
    .to_string();

    let make_request = || {
        Request::builder()
            .method(Method::POST)
            .uri("/api/reborn/product-auth/accounts/recovery")
            .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.clone()))
            .expect("request")
    };

    for i in 0..20 {
        let response = fixture
            .app
            .clone()
            .oneshot(make_request())
            .await
            .expect("oneshot");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "request {i} should be within the per-caller rate-limit budget"
        );
    }

    let response = fixture
        .app
        .clone()
        .oneshot(make_request())
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "21st request must exceed the per-caller rate-limit window"
    );
    let body_str = read_body_string(response).await;
    assert!(
        body_str.contains("Rate limit exceeded") || body_str.contains("rate limit"),
        "429 body should explain the limit, got: {body_str}"
    );
}

// --- finding t3: accounts_refresh error path ---

#[tokio::test]
async fn accounts_refresh_returns_error_for_unknown_account_id() {
    // Refreshing a syntactically valid but unseeded account_id must return
    // a sanitized error (not 500, no secret-handle leakage).
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();
    let unknown_id = CredentialAccountId::from_uuid(uuid::Uuid::new_v4());

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/refresh",
        json!({
            "provider": "github",
            "account_id": unknown_id.to_string(),
            "invocation_id": invocation_id.to_string()
        }),
    )
    .await;
    // Backend returns CredentialMissing which projects to 409 CONFLICT.
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "unknown account_id must produce a sanitized error, not 500"
    );
    let body = read_body_string(response).await;
    assert!(!body.contains("access_secret"), "no secret leakage: {body}");
    assert!(
        !body.contains("refresh_secret"),
        "no secret leakage: {body}"
    );
}

// --- finding t4: accounts_recovery existing-account projection ---

#[tokio::test]
async fn accounts_recovery_projects_configured_for_existing_account() {
    // With a seeded Configured account the recovery projection must not be
    // setup_required; it should project a non-empty configured or selection
    // state, and the response must never carry secret handles.
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();
    let _account_id =
        seed_configured_account(&fixture.shared, invocation_id, "github", "work github").await;

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/recovery",
        json!({
            "provider": "github",
            "invocation_id": invocation_id.to_string()
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    let json: Value = serde_json::from_str(&body).expect("recovery json");
    assert_ne!(
        json["kind"].as_str(),
        Some("setup_required"),
        "seeded account must not project setup_required: {body}"
    );
    assert!(!body.contains("access_secret"), "no secret leakage: {body}");
    assert!(
        !body.contains("refresh_secret"),
        "no secret leakage: {body}"
    );
}

// --- finding t5: manual_token_setup validation error paths ---

#[tokio::test]
async fn manual_token_setup_rejects_empty_provider_and_label() {
    let fixture = build_fixture();
    let cases = [
        // empty provider
        json!({ "provider": "", "account_label": "my-account" }),
        // empty account_label
        json!({ "provider": "github", "account_label": "" }),
    ];
    for body in cases {
        let response = post_authenticated(
            &fixture.app,
            "/api/reborn/product-auth/manual-token/setup",
            body,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body_str = read_body_string(response).await;
        assert!(
            body_str.contains("\"code\":\"invalid_request\""),
            "empty provider/label must return invalid_request, got: {body_str}"
        );
    }
}

// --- finding t6: accounts_refresh malformed account_id ---

#[tokio::test]
async fn accounts_refresh_rejects_malformed_account_id() {
    let fixture = build_fixture();

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/refresh",
        json!({
            "provider": "github",
            "account_id": "not-a-uuid",
            "invocation_id": InvocationId::new().to_string()
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"invalid_request\""));
}

// --- finding b1 regression: select/recovery/refresh require invocation_id ---

#[tokio::test]
async fn follow_up_routes_require_invocation_id() {
    // accounts/select, accounts/recovery, and accounts/refresh are follow-up
    // routes and must reject requests that omit invocation_id (same as
    // accounts/list and secret-submit) so all five routes share the same scope
    // derivation contract.
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();
    let account_id =
        seed_configured_account(&fixture.shared, invocation_id, "github", "work github").await;

    let cases: &[(&str, Value)] = &[
        (
            "/api/reborn/product-auth/accounts/select",
            json!({ "provider": "github", "account_id": account_id.to_string() /* no invocation_id */ }),
        ),
        (
            "/api/reborn/product-auth/accounts/recovery",
            json!({ "provider": "github" /* no invocation_id */ }),
        ),
        (
            "/api/reborn/product-auth/accounts/refresh",
            json!({ "provider": "github", "account_id": account_id.to_string() /* no invocation_id */ }),
        ),
    ];
    for (path, body) in cases {
        let response = post_authenticated(&fixture.app, path, body.clone()).await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{path} must reject missing invocation_id"
        );
        let body_str = read_body_string(response).await;
        assert!(
            body_str.contains("\"code\":\"invalid_request\""),
            "{path}: expected invalid_request, got: {body_str}"
        );
    }
}
