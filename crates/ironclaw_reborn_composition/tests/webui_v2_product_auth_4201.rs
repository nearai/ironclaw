//! Caller-level tests for issue #4201: product-facing HTTP surfaces for
//! manual-token setup/secret-submit, credential account list/select/recovery,
//! refresh, and lifecycle cleanup.
//!
//! These tests drive the HTTP routes end-to-end through `webui_v2_app` so the
//! caller path (auth layer + body limit + rate limit + handler +
//! `RebornProductAuthServices`) is exercised, not just the facade helpers.

// arch-exempt: large_file, product-auth route contracts stay in one caller-level suite until the WebUI route split lands, plan #5985

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
use ironclaw_host_api::{
    AgentId, InvocationId, ProductSurfaceCaller, ProductSurfaceError, ProjectId, ResourceScope,
    TenantId, UserId,
};
use ironclaw_product::rejecting_product_surface_error;
use ironclaw_reborn_composition::{
    ProductAuthRouteState, RebornAuthContinuationDispatcher, RebornProductAuthServices,
    product_auth_route_mount,
};
use ironclaw_webui::{WebuiAuthentication, WebuiAuthenticator, WebuiServeConfig, webui_v2_app};
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
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        (token == VALID_TOKEN)
            .then(|| WebuiAuthentication::user(UserId::new(USER).expect("user id")))
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
    async fn dispatch_canceled_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

struct UnusedServices;

#[async_trait]
impl ironclaw_host_api::ProductSurface for UnusedServices {
    async fn invoke(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ironclaw_host_api::ProductSurfaceInvokeRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceInvokeResponse, ProductSurfaceError> {
        Err(rejecting_product_surface_error())
    }

    async fn query(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ironclaw_host_api::ProductSurfaceQueryRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceQueryPage, ProductSurfaceError> {
        Err(rejecting_product_surface_error())
    }

    async fn stream_events(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ironclaw_host_api::ProductSurfaceStreamRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceStreamResponse, ProductSurfaceError> {
        Err(rejecting_product_surface_error())
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
    let product_surface = Arc::new(UnusedServices);
    let product_auth_mount = product_auth_route_mount(
        ProductAuthRouteState::new(
            product_auth,
            TenantId::new(TENANT).expect("tenant"),
            Some(AgentId::new(AGENT).expect("agent")),
            Some(ProjectId::new(PROJECT).expect("project")),
        )
        .with_product_surface(product_surface.clone()),
    );
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(OnlyValidToken),
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
    .with_split_route_mount(product_auth_mount);
    let app = webui_v2_app(product_surface, config).expect("webui v2 app");
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

fn caller_scope_with_invocation_and_thread(
    invocation_id: InvocationId,
    thread_id: ironclaw_host_api::ThreadId,
) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            user_id: UserId::new(USER).expect("user"),
            agent_id: Some(AgentId::new(AGENT).expect("agent")),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            mission_id: None,
            thread_id: Some(thread_id),
            invocation_id,
        },
        AuthSurface::Callback,
    )
}

async fn seed_account_with_status(
    shared: &InMemoryAuthProductServices,
    invocation_id: InvocationId,
    provider: &str,
    label: &str,
    status: CredentialAccountStatus,
) -> ironclaw_auth::CredentialAccountId {
    let account = shared
        .create_account(NewCredentialAccount {
            scope: caller_scope_with_invocation(invocation_id),
            provider: AuthProviderId::new(provider.to_string()).expect("provider"),
            label: CredentialAccountLabel::new(label.to_string()).expect("label"),
            status,
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

async fn seed_configured_account(
    shared: &InMemoryAuthProductServices,
    invocation_id: InvocationId,
    provider: &str,
    label: &str,
) -> ironclaw_auth::CredentialAccountId {
    seed_account_with_status(
        shared,
        invocation_id,
        provider,
        label,
        CredentialAccountStatus::Configured,
    )
    .await
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
    let invocation_id = InvocationId::new();
    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/list",
        json!({ "provider": "github", "limit": 0, "invocation_id": invocation_id.to_string() }),
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
async fn accounts_select_rejects_account_from_different_invocation_scope() {
    let fixture = build_fixture();
    let owner_invocation_id = InvocationId::new();
    let caller_invocation_id = InvocationId::new();
    let account_id = seed_configured_account(
        &fixture.shared,
        owner_invocation_id,
        "github",
        "foreign github",
    )
    .await;

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/select",
        json!({
            "provider": "github",
            "account_id": account_id.to_string(),
            "invocation_id": caller_invocation_id.to_string()
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "accounts/select must reject ids outside the caller's auth scope"
    );
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"cross_scope_denied\""));
}

#[tokio::test]
async fn accounts_select_rejects_wrong_provider_as_missing() {
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();
    let account_id =
        seed_configured_account(&fixture.shared, invocation_id, "github", "work github").await;

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/select",
        json!({
            "provider": "slack",
            "account_id": account_id.to_string(),
            "invocation_id": invocation_id.to_string()
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "wrong provider must not reveal that the account id exists"
    );
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"credential_missing\""));
}

#[tokio::test]
async fn accounts_select_rejects_unconfigured_account() {
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();
    let account_id = seed_account_with_status(
        &fixture.shared,
        invocation_id,
        "github",
        "expired github",
        CredentialAccountStatus::Expired,
    )
    .await;

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
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"credential_missing\""));
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
async fn accounts_refresh_enforces_tighter_per_caller_rate_limit() {
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();
    let account_id =
        seed_configured_account(&fixture.shared, invocation_id, "github", "refresh-limit").await;
    let body = json!({
        "provider": "github",
        "account_id": account_id.to_string(),
        "invocation_id": invocation_id.to_string()
    });

    for i in 1..=5 {
        let response = post_authenticated(
            &fixture.app,
            "/api/reborn/product-auth/accounts/refresh",
            body.clone(),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "refresh request {i} should be inside the tighter per-caller budget"
        );
    }

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/refresh",
        body,
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "6th refresh request must exceed the 5/min accounts-refresh budget"
    );
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

#[tokio::test]
async fn new_product_auth_routes_enforce_body_limit() {
    let fixture = build_fixture();
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

#[tokio::test]
async fn new_product_auth_routes_enforce_per_caller_rate_limit() {
    let fixture = build_fixture();
    let mut invocation_ids = (0..21).map(|_| InvocationId::new());

    for i in 1..=20 {
        let response = post_authenticated(
            &fixture.app,
            "/api/reborn/product-auth/accounts/list",
            json!({
                "provider": "github",
                "invocation_id": invocation_ids.next().expect("id").to_string()
            }),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "request {i} should be within the per-caller rate-limit budget"
        );
    }

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/list",
        json!({
            "provider": "github",
            "invocation_id": invocation_ids.next().expect("id").to_string()
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "21st request must exceed the per-caller rate-limit window"
    );
    let body = read_body_string(response).await;
    assert!(
        body.contains("Rate limit exceeded") || body.contains("rate limit"),
        "429 body should explain the limit, got: {body}"
    );
}

#[tokio::test]
async fn accounts_refresh_returns_error_for_unknown_account_id() {
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

#[tokio::test]
async fn accounts_recovery_projects_configured_for_existing_account() {
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

#[tokio::test]
async fn manual_token_setup_rejects_empty_provider_and_label() {
    let fixture = build_fixture();
    let cases = [
        json!({ "provider": "", "account_label": "work" }),
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
        let body = read_body_string(response).await;
        assert!(body.contains("\"code\":\"invalid_request\""));
    }
}

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

#[tokio::test]
async fn follow_up_routes_require_invocation_id() {
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();
    let account_id =
        seed_configured_account(&fixture.shared, invocation_id, "github", "work github").await;

    let cases: &[(&str, Value)] = &[
        (
            "/api/reborn/product-auth/accounts/select",
            json!({ "provider": "github", "account_id": account_id.to_string() }),
        ),
        (
            "/api/reborn/product-auth/accounts/recovery",
            json!({ "provider": "github" }),
        ),
        (
            "/api/reborn/product-auth/accounts/refresh",
            json!({ "provider": "github", "account_id": account_id.to_string() }),
        ),
    ];
    for (path, body) in cases {
        let response = post_authenticated(&fixture.app, path, body.clone()).await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{path} must reject missing invocation_id"
        );
        let body = read_body_string(response).await;
        assert!(
            body.contains("\"code\":\"invalid_request\""),
            "{path}: expected invalid_request, got: {body}"
        );
    }
}

// ── Wire-shape enrichment tests (issue #4112) ────────────────────────────────

#[test]
fn auth_prompt_view_serialises_optional_fields_when_present() {
    use ironclaw_product::{AuthPromptChallengeKind, AuthPromptView};
    use ironclaw_turns::TurnRunId;

    let view = AuthPromptView {
        turn_run_id: TurnRunId::new(),
        auth_request_ref: "gate-ref-001".to_string(),
        invocation_id: Some(InvocationId::new()),
        headline: "Authentication required".to_string(),
        body: "Authenticate to continue.".to_string(),
        challenge_kind: Some(AuthPromptChallengeKind::OAuthUrl),
        provider: Some("google".to_string()),
        account_label: Some("work@example.com".to_string()),
        authorization_url: Some(
            "https://accounts.google.com/o/oauth2/auth?scope=calendar".to_string(),
        ),
        expires_at: Some(
            chrono::DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        ),
        connection: None,
        pairing: None,
    };
    let json = serde_json::to_value(&view).expect("serialise");
    assert_eq!(json["challenge_kind"], "oauth_url");
    let invocation_id = view
        .invocation_id
        .expect("invocation id present")
        .to_string();
    assert_eq!(json["invocation_id"].as_str(), Some(invocation_id.as_str()));
    assert_eq!(json["provider"], "google");
    assert_eq!(json["account_label"], "work@example.com");
    assert!(
        json["authorization_url"]
            .as_str()
            .unwrap()
            .starts_with("https://")
    );
    assert!(json["expires_at"].is_string());
}

#[test]
fn auth_prompt_view_omits_optional_fields_when_absent() {
    use ironclaw_product::AuthPromptView;
    use ironclaw_turns::TurnRunId;

    let view = AuthPromptView {
        turn_run_id: TurnRunId::new(),
        auth_request_ref: "gate-ref-002".to_string(),
        invocation_id: None,
        headline: "Authentication required".to_string(),
        body: "Authenticate to continue.".to_string(),
        challenge_kind: None,
        provider: None,
        account_label: None,
        authorization_url: None,
        expires_at: None,
        connection: None,
        pairing: None,
    };
    let json = serde_json::to_value(&view).expect("serialise");
    assert!(
        json.get("challenge_kind").is_none(),
        "challenge_kind should be absent when None"
    );
    assert!(
        json.get("connection").is_none(),
        "connection should be absent when None"
    );
    assert!(
        json.get("provider").is_none(),
        "provider should be absent when None"
    );
    assert!(
        json.get("account_label").is_none(),
        "account_label should be absent when None"
    );
    assert!(
        json.get("authorization_url").is_none(),
        "authorization_url should be absent when None"
    );
    assert!(
        json.get("expires_at").is_none(),
        "expires_at should be absent when None"
    );
    assert!(
        json.get("invocation_id").is_none(),
        "invocation_id should be absent when None"
    );
}

#[test]
fn auth_prompt_view_deserialises_without_optional_fields() {
    // Simulate a legacy serialised row (no new fields) — must round-trip as None.
    use ironclaw_product::AuthPromptView;

    let legacy_json = r#"{
        "turn_run_id": "11111111-1111-1111-1111-111111111111",
        "auth_request_ref": "gate-legacy",
        "headline": "Auth required",
        "body": "Authenticate."
    }"#;
    let view: AuthPromptView = serde_json::from_str(legacy_json).expect("deserialise legacy");
    assert!(view.challenge_kind.is_none());
    assert!(view.provider.is_none());
    assert!(view.account_label.is_none());
    assert!(view.authorization_url.is_none());
    assert!(view.expires_at.is_none());
    assert!(view.invocation_id.is_none());
}

#[tokio::test]
async fn challenge_for_gate_returns_oauth_url_view_for_seeded_flow() {
    use chrono::Utc;
    use ironclaw_auth::AuthProviderId;
    use ironclaw_auth::{
        AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthFlowManager, AuthGateRef,
        InMemoryAuthProductServices, NewAuthFlow, OAuthAuthorizationUrl, TurnRunRef,
    };
    use ironclaw_product::AuthPromptChallengeKind;
    use std::sync::Arc;

    let shared = Arc::new(InMemoryAuthProductServices::new());
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(
            shared.clone(),
            Arc::new(NoopAuthDispatcher::default()),
        )
        .with_flow_record_source(shared.clone()),
    );

    let gate_ref_str = "aaaabbbb-cccc-dddd-eeee-111111111111";
    let auth_url = OAuthAuthorizationUrl::new(
        "https://accounts.google.com/o/oauth2/auth?scope=calendar".to_string(),
    )
    .unwrap();
    let expires_at = Utc::now() + chrono::Duration::hours(1);

    use ironclaw_host_api::ThreadId;
    use ironclaw_turns::{TurnRunId, TurnScope};
    let thread_id = ThreadId::new("thread-4112".to_string()).expect("thread id");
    let turn_run_id = TurnRunId::new();

    shared
        .create_flow(NewAuthFlow {
            id: None,
            // Flow must carry the same thread_id as the TurnScope — thread_id matching
            // is fail-closed: a flow with None thread_id does not match any scoped request.
            scope: caller_scope_with_invocation_and_thread(InvocationId::new(), thread_id.clone()),
            kind: AuthFlowKind::IntegrationCredential,
            provider: AuthProviderId::new("google".to_string()).unwrap(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: auth_url,
                expires_at,
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(turn_run_id.to_string()).unwrap(),
                gate_ref: AuthGateRef::new(gate_ref_str.to_string()).unwrap(),
            },
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .expect("create flow");

    let provider = product_auth.as_auth_challenge_provider().expect("provider");
    // Build a TurnScope matching the flow's tenant/agent/project/thread.
    let turn_scope = TurnScope::new(
        TenantId::new(TENANT).expect("tenant"),
        Some(AgentId::new(AGENT).expect("agent")),
        Some(ProjectId::new(PROJECT).expect("project")),
        thread_id,
    );
    let view = provider
        .challenge_for_gate(
            &turn_scope,
            &UserId::new(USER).expect("user"),
            turn_run_id,
            gate_ref_str,
            &[],
        )
        .await
        .expect("lookup")
        .expect("found");
    assert!(matches!(view.kind, AuthPromptChallengeKind::OAuthUrl));
    assert_eq!(view.provider.as_str(), "google");
    assert!(
        view.authorization_url
            .as_ref()
            .map(|url| url.as_str())
            .unwrap()
            .contains("accounts.google.com")
    );
    assert!(view.account_label.is_none());
    assert!(
        provider
            .challenge_for_gate(
                &turn_scope,
                &UserId::new("other-user-4201").expect("user"),
                turn_run_id,
                gate_ref_str,
                &[],
            )
            .await
            .expect("lookup")
            .is_none(),
        "challenge lookup must reject the wrong owner user"
    );
    assert!(
        provider
            .challenge_for_gate(
                &turn_scope,
                &UserId::new(USER).expect("user"),
                TurnRunId::new(),
                gate_ref_str,
                &[],
            )
            .await
            .expect("lookup")
            .is_none(),
        "challenge lookup must reject the wrong turn run"
    );
}

#[test]
fn auth_challenge_provider_absent_when_no_flow_record_source() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        shared,
        Arc::new(NoopAuthDispatcher::default()),
    ));
    assert!(
        product_auth.as_auth_challenge_provider().is_none(),
        "no flow_record_source → no AuthChallengeProvider"
    );
}

// ── Security fix tests (issue #4257 review) ──────────────────────────────────

#[tokio::test]
async fn challenge_for_gate_cancelled_flow_returns_none() {
    // Fix #1/#2: verify that terminal-status flows are not surfaced.
    use chrono::Utc;
    use ironclaw_auth::AuthProviderId;
    use ironclaw_auth::{
        AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthFlowManager, AuthGateRef,
        InMemoryAuthProductServices, NewAuthFlow, OAuthAuthorizationUrl, TurnRunRef,
    };
    use ironclaw_host_api::ThreadId;
    use ironclaw_turns::{TurnRunId, TurnScope};
    use std::sync::Arc;

    let shared = Arc::new(InMemoryAuthProductServices::new());
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(
            shared.clone(),
            Arc::new(NoopAuthDispatcher::default()),
        )
        .with_flow_record_source(shared.clone()),
    );

    let gate_ref_str = "bbbbbbbb-cccc-dddd-eeee-222222222222";
    let auth_url =
        OAuthAuthorizationUrl::new("https://accounts.google.com/o/oauth2/auth".to_string())
            .unwrap();
    let expires_at = Utc::now() + chrono::Duration::hours(1);
    let scope = caller_scope_with_invocation(InvocationId::new());
    let turn_run_id = TurnRunId::new();

    let flow = shared
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: AuthProviderId::new("google".to_string()).unwrap(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: auth_url,
                expires_at,
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(turn_run_id.to_string()).unwrap(),
                gate_ref: AuthGateRef::new(gate_ref_str.to_string()).unwrap(),
            },
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .expect("create flow");

    // Cancel the flow — it becomes terminal.
    shared
        .cancel_flow(&scope, flow.id)
        .await
        .expect("cancel flow");

    let provider = product_auth.as_auth_challenge_provider().expect("provider");
    let turn_scope = TurnScope::new(
        TenantId::new(TENANT).expect("tenant"),
        Some(AgentId::new(AGENT).expect("agent")),
        Some(ProjectId::new(PROJECT).expect("project")),
        ThreadId::new("thread-4112b".to_string()).expect("thread id"),
    );
    let result = provider
        .challenge_for_gate(
            &turn_scope,
            &UserId::new(USER).expect("user"),
            turn_run_id,
            gate_ref_str,
            &[],
        )
        .await
        .expect("lookup");
    assert!(
        result.is_none(),
        "cancelled flow must not be surfaced by challenge_for_gate"
    );
}

#[tokio::test]
async fn challenge_for_gate_threadless_flow_returns_none_for_thread_scope() {
    use chrono::Utc;
    use ironclaw_auth::AuthProviderId;
    use ironclaw_auth::{
        AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthFlowManager, AuthGateRef,
        InMemoryAuthProductServices, NewAuthFlow, OAuthAuthorizationUrl, TurnRunRef,
    };
    use ironclaw_host_api::ThreadId;
    use ironclaw_turns::{TurnRunId, TurnScope};
    use std::sync::Arc;

    let shared = Arc::new(InMemoryAuthProductServices::new());
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(
            shared.clone(),
            Arc::new(NoopAuthDispatcher::default()),
        )
        .with_flow_record_source(shared.clone()),
    );

    let gate_ref_str = "bbbbbbbb-cccc-dddd-eeee-222222222223";
    let expires_at = Utc::now() + chrono::Duration::hours(1);
    let turn_run_id = TurnRunId::new();
    shared
        .create_flow(NewAuthFlow {
            id: None,
            scope: caller_scope_with_invocation(InvocationId::new()),
            kind: AuthFlowKind::IntegrationCredential,
            provider: AuthProviderId::new("google".to_string()).unwrap(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new(
                    "https://accounts.google.com/o/oauth2/auth".to_string(),
                )
                .unwrap(),
                expires_at,
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(turn_run_id.to_string()).unwrap(),
                gate_ref: AuthGateRef::new(gate_ref_str.to_string()).unwrap(),
            },
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .expect("create flow");

    let provider = product_auth.as_auth_challenge_provider().expect("provider");
    let turn_scope = TurnScope::new(
        TenantId::new(TENANT).expect("tenant"),
        Some(AgentId::new(AGENT).expect("agent")),
        Some(ProjectId::new(PROJECT).expect("project")),
        ThreadId::new("thread-4112c".to_string()).expect("thread id"),
    );
    let result = provider
        .challenge_for_gate(
            &turn_scope,
            &UserId::new(USER).expect("user"),
            turn_run_id,
            gate_ref_str,
            &[],
        )
        .await
        .expect("lookup");
    assert!(
        result.is_none(),
        "thread-scoped lookup must reject matching flows that lack thread_id"
    );
}

#[tokio::test]
async fn challenge_for_gate_wrong_tenant_returns_none() {
    // Fix #1: verify that a flow from a different tenant cannot be retrieved
    // by a caller with a different scope, even with the correct gate_ref UUID.
    use chrono::Utc;
    use ironclaw_auth::AuthProviderId;
    use ironclaw_auth::{
        AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthFlowManager, AuthGateRef,
        InMemoryAuthProductServices, NewAuthFlow, OAuthAuthorizationUrl, TurnRunRef,
    };
    use ironclaw_host_api::ThreadId;
    use ironclaw_turns::{TurnRunId, TurnScope};
    use std::sync::Arc;

    let shared = Arc::new(InMemoryAuthProductServices::new());
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(
            shared.clone(),
            Arc::new(NoopAuthDispatcher::default()),
        )
        .with_flow_record_source(shared.clone()),
    );

    let gate_ref_str = "cccccccc-dddd-eeee-ffff-333333333333";
    let auth_url =
        OAuthAuthorizationUrl::new("https://accounts.google.com/o/oauth2/auth".to_string())
            .unwrap();
    let expires_at = Utc::now() + chrono::Duration::hours(1);
    let turn_run_id = TurnRunId::new();

    // Create the flow under TENANT (the test tenant).
    shared
        .create_flow(NewAuthFlow {
            id: None,
            scope: caller_scope_with_invocation(InvocationId::new()),
            kind: AuthFlowKind::IntegrationCredential,
            provider: AuthProviderId::new("google".to_string()).unwrap(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: auth_url,
                expires_at,
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(turn_run_id.to_string()).unwrap(),
                gate_ref: AuthGateRef::new(gate_ref_str.to_string()).unwrap(),
            },
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .expect("create flow");

    let provider = product_auth.as_auth_challenge_provider().expect("provider");

    // Query with a DIFFERENT tenant — must return None even with same gate_ref.
    let other_turn_scope = TurnScope::new(
        TenantId::new("other-tenant-4257").expect("tenant"),
        Some(AgentId::new(AGENT).expect("agent")),
        Some(ProjectId::new(PROJECT).expect("project")),
        ThreadId::new("thread-other".to_string()).expect("thread id"),
    );
    let result = provider
        .challenge_for_gate(
            &other_turn_scope,
            &UserId::new(USER).expect("user"),
            turn_run_id,
            gate_ref_str,
            &[],
        )
        .await
        .expect("lookup");
    assert!(
        result.is_none(),
        "different-tenant caller must not receive another tenant's challenge"
    );
}

#[tokio::test]
async fn challenge_for_gate_returns_manual_token_view_for_seeded_flow() {
    // Covers the ManualTokenRequired arm of auth_challenge_to_view.
    use chrono::Utc;
    use ironclaw_auth::AuthProviderId;
    use ironclaw_auth::{
        AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthFlowManager, AuthGateRef,
        AuthInteractionId, CredentialAccountLabel, InMemoryAuthProductServices, NewAuthFlow,
        TurnRunRef,
    };
    use ironclaw_host_api::ThreadId;
    use ironclaw_product::AuthPromptChallengeKind;
    use ironclaw_turns::{TurnRunId, TurnScope};
    use std::sync::Arc;

    let shared = Arc::new(InMemoryAuthProductServices::new());
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(
            shared.clone(),
            Arc::new(NoopAuthDispatcher::default()),
        )
        .with_flow_record_source(shared.clone()),
    );

    let gate_ref_str = "dddddddd-eeee-ffff-aaaa-444444444444";
    let expires_at = Utc::now() + chrono::Duration::hours(1);
    let thread_id = ThreadId::new("thread-manual-token".to_string()).expect("thread id");
    let turn_run_id = TurnRunId::new();

    shared
        .create_flow(NewAuthFlow {
            id: None,
            scope: caller_scope_with_invocation_and_thread(InvocationId::new(), thread_id.clone()),
            kind: AuthFlowKind::IntegrationCredential,
            provider: AuthProviderId::new("slack".to_string()).unwrap(),
            challenge: AuthChallenge::ManualTokenRequired {
                interaction_id: AuthInteractionId::new(),
                provider: AuthProviderId::new("slack".to_string()).unwrap(),
                label: CredentialAccountLabel::new("work slack token".to_string()).unwrap(),
                expires_at,
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(turn_run_id.to_string()).unwrap(),
                gate_ref: AuthGateRef::new(gate_ref_str.to_string()).unwrap(),
            },
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .expect("create flow");

    let provider = product_auth.as_auth_challenge_provider().expect("provider");
    let turn_scope = TurnScope::new(
        TenantId::new(TENANT).expect("tenant"),
        Some(AgentId::new(AGENT).expect("agent")),
        Some(ProjectId::new(PROJECT).expect("project")),
        thread_id,
    );
    let view = provider
        .challenge_for_gate(
            &turn_scope,
            &UserId::new(USER).expect("user"),
            turn_run_id,
            gate_ref_str,
            &[],
        )
        .await
        .expect("lookup")
        .expect("found");
    assert!(matches!(view.kind, AuthPromptChallengeKind::ManualToken));
    assert_eq!(view.provider.as_str(), "slack");
    assert_eq!(
        view.account_label.as_ref().map(|label| label.as_str()),
        Some("work slack token")
    );
    assert!(view.authorization_url.is_none());
}

#[tokio::test]
async fn challenge_for_gate_returns_other_kind_view_for_setup_required_flow() {
    // Covers the AccountSelectionRequired / ReauthorizeRequired / SetupRequired
    // arms of auth_challenge_to_view (all map to AuthPromptChallengeKind::Other).
    use chrono::Utc;
    use ironclaw_auth::AuthProviderId;
    use ironclaw_auth::{
        AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthFlowManager, AuthGateRef,
        InMemoryAuthProductServices, NewAuthFlow, TurnRunRef,
    };
    use ironclaw_host_api::ThreadId;
    use ironclaw_product::AuthPromptChallengeKind;
    use ironclaw_turns::{TurnRunId, TurnScope};
    use std::sync::Arc;

    let shared = Arc::new(InMemoryAuthProductServices::new());
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(
            shared.clone(),
            Arc::new(NoopAuthDispatcher::default()),
        )
        .with_flow_record_source(shared.clone()),
    );

    let gate_ref_str = "eeeeeeee-ffff-aaaa-bbbb-555555555555";
    let expires_at = Utc::now() + chrono::Duration::hours(1);
    let thread_id = ThreadId::new("thread-setup-required".to_string()).expect("thread id");
    let turn_run_id = TurnRunId::new();

    shared
        .create_flow(NewAuthFlow {
            id: None,
            scope: caller_scope_with_invocation_and_thread(InvocationId::new(), thread_id.clone()),
            kind: AuthFlowKind::IntegrationCredential,
            provider: AuthProviderId::new("github".to_string()).unwrap(),
            challenge: AuthChallenge::SetupRequired {
                provider: AuthProviderId::new("github".to_string()).unwrap(),
                message: "GitHub app not installed".to_string(),
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(turn_run_id.to_string()).unwrap(),
                gate_ref: AuthGateRef::new(gate_ref_str.to_string()).unwrap(),
            },
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .expect("create flow");

    let provider = product_auth.as_auth_challenge_provider().expect("provider");
    let turn_scope = TurnScope::new(
        TenantId::new(TENANT).expect("tenant"),
        Some(AgentId::new(AGENT).expect("agent")),
        Some(ProjectId::new(PROJECT).expect("project")),
        thread_id,
    );
    let view = provider
        .challenge_for_gate(
            &turn_scope,
            &UserId::new(USER).expect("user"),
            turn_run_id,
            gate_ref_str,
            &[],
        )
        .await
        .expect("lookup")
        .expect("found");
    assert!(matches!(view.kind, AuthPromptChallengeKind::Other));
    assert_eq!(view.provider.as_str(), "github");
    assert!(view.account_label.is_none());
    assert!(view.authorization_url.is_none());
    assert!(view.expires_at.is_none());
}

#[tokio::test]
async fn accounts_select_requires_invocation_id() {
    // Fix #4: accounts_select must reject requests without invocation_id.
    let fixture = build_fixture();
    let invocation_id = InvocationId::new();
    let account_id =
        seed_configured_account(&fixture.shared, invocation_id, "github", "work github").await;

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/select",
        json!({
            "provider": "github",
            "account_id": account_id.to_string()
            // invocation_id intentionally absent
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "accounts/select must require invocation_id"
    );
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"invalid_request\""));
}

#[tokio::test]
async fn accounts_recovery_requires_invocation_id() {
    // Fix #4: accounts_recovery must reject requests without invocation_id.
    let fixture = build_fixture();

    let response = post_authenticated(
        &fixture.app,
        "/api/reborn/product-auth/accounts/recovery",
        json!({ "provider": "github" /* invocation_id absent */ }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "accounts/recovery must require invocation_id"
    );
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"invalid_request\""));
}
