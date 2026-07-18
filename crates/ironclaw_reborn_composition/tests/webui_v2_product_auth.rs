//! Caller-level tests for Reborn WebUI v2 product-auth OAuth routes.

// arch-exempt: large_file, caller-level product-auth route regression coverage, plan #5905

#![cfg(feature = "webui-v2-beta")]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationEvent, AuthContinuationRef, AuthFlowId, AuthFlowKind,
    AuthFlowManager, AuthInteractionId, AuthInteractionService, AuthProductError, AuthProductScope,
    AuthProviderClient, AuthProviderId, AuthSurface, CredentialAccountLabel,
    CredentialAccountService, CredentialAccountStatus, CredentialOwnership, CredentialSetupService,
    GOOGLE_CALENDAR_READONLY_SCOPE, GOOGLE_GMAIL_READONLY_SCOPE, InMemoryAuthProductServices,
    ManualTokenSetupRequest, NewAuthFlow, NewCredentialAccount, OAuthAuthorizationUrl,
    OAuthProviderCallbackRequest, OAuthProviderExchange, OAuthProviderExchangeContext,
    OAuthProviderRefresh, OAuthProviderRefreshRequest, ProviderScope, SecretCleanupService,
    SecretSubmitRequest, SecretSubmitResult,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, SecretHandle, TenantId, UserId,
};
use ironclaw_product_workflow::{
    LifecyclePackageKind, LifecyclePackageRef, RebornCancelRunResponse, RebornCreateThreadResponse,
    RebornDeleteThreadRequest, RebornDeleteThreadResponse, RebornExtensionActionResponse,
    RebornExtensionInfo, RebornExtensionListResponse, RebornExtensionRegistryResponse,
    RebornGetRunStateRequest, RebornGetRunStateResponse, RebornListAutomationsResponse,
    RebornListThreadsResponse, RebornOutboundDeliveryTargetListResponse,
    RebornOutboundPreferencesResponse, RebornResolveGateResponse, RebornRetryRunResponse,
    RebornServicesApi, RebornServicesError, RebornSetOutboundPreferencesRequest,
    RebornSetupExtensionResponse, RebornSkillActionResponse, RebornSkillContentResponse,
    RebornSkillListResponse, RebornSkillSearchResponse, RebornStreamEventsRequest,
    RebornStreamEventsResponse, RebornSubmitTurnResponse, RebornTimelineRequest,
    RebornTimelineResponse, WebUiAuthenticatedCaller, WebUiCancelRunRequest,
    WebUiCreateThreadRequest, WebUiListAutomationsRequest, WebUiListThreadsRequest,
    WebUiResolveGateRequest, WebUiRetryRunRequest, WebUiSendMessageRequest,
    WebUiSetupExtensionRequest, rejecting_reborn_services_error,
};
use ironclaw_reborn_composition::{
    GoogleOAuthRouteConfig, RebornAuthContinuationDispatcher, RebornProductAuthServices,
    RebornReadiness, RebornWebuiBundle,
};
use ironclaw_webui::{WebuiAuthentication, WebuiAuthenticator, WebuiServeConfig, webui_v2_app};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

const TENANT: &str = "tenant-alpha";
const USER: &str = "user-alpha";
const AGENT: &str = "agent-default";
const PROJECT: &str = "project-default";
const VALID_TOKEN: &str = "valid-bearer-token";
const DISALLOWED_GOOGLE_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

struct OnlyValidToken;

#[async_trait]
impl WebuiAuthenticator for OnlyValidToken {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        (token == VALID_TOKEN)
            .then(|| WebuiAuthentication::user(UserId::new(USER).expect("user id")))
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
struct RecordingProviderClient {
    exchanged_scopes: Mutex<Vec<Vec<String>>>,
}

impl RecordingProviderClient {
    fn exchanged_scopes(&self) -> Vec<Vec<String>> {
        self.exchanged_scopes
            .lock()
            .expect("exchanged scopes lock")
            .clone()
    }
}

#[async_trait]
impl AuthProviderClient for RecordingProviderClient {
    async fn exchange_callback(
        &self,
        _context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        let scopes = request
            .scopes
            .iter()
            .map(|scope| scope.as_str().to_string())
            .collect::<Vec<_>>();
        self.exchanged_scopes
            .lock()
            .expect("exchanged scopes lock")
            .push(scopes);
        Ok(OAuthProviderExchange {
            provider: request.provider,
            account_label: request.account_label,
            authorization_code_hash: request.authorization_code_hash,
            pkce_verifier_hash: request.pkce_verifier_hash,
            access_secret: SecretHandle::new("recorded-google-access").expect("secret handle"),
            refresh_secret: Some(
                SecretHandle::new("recorded-google-refresh").expect("secret handle"),
            ),
            scopes: request.scopes,
            account_id: None,
            provider_identity: None,
        })
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

#[derive(Default)]
struct UnusedServices {
    installed_extensions: Vec<RebornExtensionInfo>,
}

impl UnusedServices {
    fn with_installed_extensions(package_ids: &[&str]) -> Self {
        Self {
            installed_extensions: package_ids
                .iter()
                .map(|package_id| RebornExtensionInfo {
                    package_ref: LifecyclePackageRef::new(
                        LifecyclePackageKind::Extension,
                        *package_id,
                    )
                    .expect("installed extension package ref"),
                    display_name: (*package_id).to_string(),
                    kind: "wasm_tool".to_string(),
                    description: "test installed extension".to_string(),
                    authenticated: false,
                    active: false,
                    tools: Vec::new(),
                    needs_setup: true,
                    has_auth: true,
                    activation_status: None,
                    activation_error: None,
                    version: None,
                    install_scope: None,
                    onboarding_state: None,
                    onboarding: None,
                })
                .collect(),
        }
    }
}

#[async_trait]
impl RebornServicesApi for UnusedServices {
    async fn create_thread(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn submit_turn(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiSendMessageRequest,
    ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn get_timeline(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn delete_thread(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornDeleteThreadRequest,
    ) -> Result<RebornDeleteThreadResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn stream_events(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn get_run_state(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn cancel_run(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn retry_run(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiRetryRunRequest,
    ) -> Result<RebornRetryRunResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn resolve_gate(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
    }

    async fn list_threads(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiListThreadsRequest,
    ) -> Result<RebornListThreadsResponse, RebornServicesError> {
        Err(rejecting_reborn_services_error())
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
        Ok(RebornExtensionListResponse {
            extensions: self.installed_extensions.clone(),
        })
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

fn build_app_with_product_auth() -> (axum::Router, Arc<RecordingAuthDispatcher>) {
    build_app_with_product_auth_and_installed_extensions(&[])
}

fn build_app_with_product_auth_and_installed_extensions(
    installed_package_ids: &[&str],
) -> (axum::Router, Arc<RecordingAuthDispatcher>) {
    let dispatcher = Arc::new(RecordingAuthDispatcher::default());
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        Arc::new(InMemoryAuthProductServices::new()),
        dispatcher.clone(),
    ));
    (
        build_app_with_product_auth_service_config_and_extensions(
            product_auth,
            None,
            installed_package_ids,
        ),
        dispatcher,
    )
}

fn build_app_with_product_auth_service(
    product_auth: Arc<RebornProductAuthServices>,
) -> axum::Router {
    build_app_with_product_auth_service_and_config(product_auth, None)
}

fn build_app_with_product_auth_service_and_config(
    product_auth: Arc<RebornProductAuthServices>,
    google_oauth: Option<GoogleOAuthRouteConfig>,
) -> axum::Router {
    build_app_with_product_auth_service_config_and_extensions(product_auth, google_oauth, &[])
}

fn build_app_with_product_auth_service_config_and_extensions(
    product_auth: Arc<RebornProductAuthServices>,
    google_oauth: Option<GoogleOAuthRouteConfig>,
    installed_package_ids: &[&str],
) -> axum::Router {
    let bundle = RebornWebuiBundle {
        api: Arc::new(UnusedServices::with_installed_extensions(
            installed_package_ids,
        )),
        product_auth: Some(product_auth),
        readiness: RebornReadiness::disabled(),
    };
    let mut config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(OnlyValidToken),
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"));
    if let Some(google_oauth) = google_oauth {
        config = config.with_google_oauth(google_oauth);
    }
    webui_v2_app(bundle, config).expect("webui v2 app")
}

fn google_oauth_route_config() -> GoogleOAuthRouteConfig {
    GoogleOAuthRouteConfig::new(
        "google-client.apps.googleusercontent.com",
        "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
    )
    .expect("google oauth route config")
}

fn build_app_with_google_oauth() -> (axum::Router, Arc<RecordingAuthDispatcher>) {
    let dispatcher = Arc::new(RecordingAuthDispatcher::default());
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        Arc::new(InMemoryAuthProductServices::new()),
        dispatcher.clone(),
    ));
    (
        build_app_with_product_auth_service_and_config(
            product_auth,
            Some(google_oauth_route_config()),
        ),
        dispatcher,
    )
}

fn build_app_with_google_oauth_provider(
    provider_client: Arc<dyn AuthProviderClient>,
) -> (axum::Router, Arc<RecordingAuthDispatcher>) {
    let dispatcher = Arc::new(RecordingAuthDispatcher::default());
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let flow_manager: Arc<dyn AuthFlowManager> = shared.clone();
    let interaction_service: Arc<dyn AuthInteractionService> = shared.clone();
    let credential_setup_service: Arc<dyn CredentialSetupService> = shared.clone();
    let credential_account_service: Arc<dyn CredentialAccountService> = shared.clone();
    let cleanup_service: Arc<dyn SecretCleanupService> = shared;
    let product_auth = Arc::new(RebornProductAuthServices::new(
        flow_manager,
        interaction_service,
        credential_setup_service,
        credential_account_service,
        provider_client,
        cleanup_service,
        dispatcher.clone(),
    ));
    (
        build_app_with_product_auth_service_and_config(
            product_auth,
            Some(google_oauth_route_config()),
        ),
        dispatcher,
    )
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

async fn get_oauth_flow_status(
    app: &axum::Router,
    flow_id: &str,
    query: &str,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/reborn/product-auth/oauth/flow/{flow_id}/status{query}"
                ))
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot")
}

fn google_oauth_start_body(extra_fields: serde_json::Value) -> serde_json::Value {
    let expires_at = (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339();
    let mut body = json!({
        "account_label": "work google",
        "scopes": [GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_CALENDAR_READONLY_SCOPE],
        "expires_at": expires_at
    });
    merge_json_object(&mut body, extra_fields);
    body
}

async fn post_google_oauth_start(
    app: &axum::Router,
    body: serde_json::Value,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/reborn/product-auth/oauth/google/start")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot")
}

async fn post_extension_oauth_start(
    app: &axum::Router,
    package_id: &str,
    body: serde_json::Value,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/api/webchat/v2/extensions/{package_id}/setup/oauth/start"
                ))
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot")
}

async fn start_google_oauth_flow(app: &axum::Router) -> (serde_json::Value, String) {
    let start_response = post_google_oauth_start(
        app,
        google_oauth_start_body(json!({
            "session_id": "web-session-google",
            "thread_id": "thread-auth-google"
        })),
    )
    .await;
    assert_eq!(start_response.status(), StatusCode::OK);
    let start_body = read_body_string(start_response).await;
    let start_json: serde_json::Value = serde_json::from_str(&start_body).expect("start json");
    let authorization_url = start_json["authorization_url"]
        .as_str()
        .expect("authorization url");
    let parsed = url::Url::parse(authorization_url).expect("google authorization url");
    let state = parsed
        .query_pairs()
        .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
        .expect("state");
    (start_json, state)
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

fn callback_request_accept(uri: String, accept: HeaderValue) -> Request<Body> {
    let mut request = callback_request_with_options(uri, Body::empty(), callback_peer(10), None);
    request.headers_mut().insert(header::ACCEPT, accept);
    request
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
async fn product_auth_google_oauth_start_requires_bearer_auth() {
    let (app, _) = build_app_with_google_oauth();

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/reborn/product-auth/oauth/google/start")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(google_oauth_start_body(json!({})).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn product_auth_google_oauth_start_fails_closed_without_config() {
    let (app, _) = build_app_with_product_auth();

    let response = post_google_oauth_start(&app, google_oauth_start_body(json!({}))).await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"backend_unavailable\""));
}

#[tokio::test]
async fn product_auth_google_oauth_start_rejects_disallowed_scopes() {
    let (app, _) = build_app_with_google_oauth();

    let invalid_requests = [
        google_oauth_start_body(json!({ "scopes": [] })),
        google_oauth_start_body(json!({ "scopes": [DISALLOWED_GOOGLE_SCOPE] })),
    ];

    for body in invalid_requests {
        let response = post_google_oauth_start(&app, body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = read_body_string(response).await;
        assert!(body.contains("\"code\":\"invalid_request\""));
        assert!(!body.contains(DISALLOWED_GOOGLE_SCOPE));
    }
}

#[tokio::test]
async fn product_auth_google_oauth_start_rejects_invalid_expiry() {
    let (app, _) = build_app_with_google_oauth();

    let invalid_requests = [
        google_oauth_start_body(
            json!({ "expires_at": (Utc::now() - ChronoDuration::minutes(1)).to_rfc3339() }),
        ),
        google_oauth_start_body(
            json!({ "expires_at": (Utc::now() + ChronoDuration::hours(1)).to_rfc3339() }),
        ),
    ];

    for body in invalid_requests {
        let response = post_google_oauth_start(&app, body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = read_body_string(response).await;
        assert!(body.contains("\"code\":\"invalid_request\""));
    }
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
    assert_eq!(
        dispatcher.events().len(),
        1,
        "manual token submit should dispatch the completed turn-gate continuation"
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
            manual_token_body(
                &format!("ghp_secret_{index}"),
                json!({
                    "account_label": format!("work github {index}"),
                    "run_id": Uuid::from_u128((index + 1) as u128).to_string(),
                    "gate_ref": format!("gate:auth-github-{index}"),
                    "thread_id": format!("thread-auth-{index}")
                }),
            ),
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

// The origin-independent reconnect backstop: after the callback marks the flow
// completed, the caller-scoped flow-status poll reports "completed" so the
// reconnect modal can close even when the same-origin browser signal never
// arrived. Also locks the read's error surface: malformed id → 400, unknown id
// → 404, and no secret material ever crosses the read.
#[tokio::test]
async fn product_auth_oauth_flow_status_reports_completed_without_secrets() {
    let (app, _dispatcher) = build_app_with_product_auth();
    // Start WITHOUT session/thread so the flow scope is thread/session-free —
    // exactly what the caller-scoped poll re-derives from the invocation id.
    let started =
        start_oauth_flow(&app, "status-state-secret", "status-pkce-secret", json!({})).await;
    let callback_response = app
        .clone()
        .oneshot(callback_request(callback_uri(
            &started.flow_id,
            &started.invocation_id,
            USER,
            "status-state-secret",
            "&provider=github&account_label=work%20github&code=status-auth-code&scopes=repo",
        )))
        .await
        .expect("oneshot");
    assert_eq!(callback_response.status(), StatusCode::OK);

    // Origin-independent poll: the browser echoes the invocation id the start
    // response minted so the caller-scoped `get_flow` can match its own flow.
    let response = get_oauth_flow_status(
        &app,
        &started.flow_id,
        &format!("?invocation_id={}", started.invocation_id),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    let json: serde_json::Value = serde_json::from_str(&body).expect("status json");
    assert_eq!(json["status"], "completed");
    // Status enum only — no state/PKCE/code/token material may cross this read.
    assert!(!body.contains("status-state-secret"));
    assert!(!body.contains("status-pkce-secret"));
    assert!(!body.contains("status-auth-code"));
    assert!(!body.contains("oauth-access"));
    assert!(!body.contains("oauth-refresh"));

    // Malformed flow id → 4xx before any backend read.
    let malformed = get_oauth_flow_status(
        &app,
        "not-a-uuid",
        &format!("?invocation_id={}", started.invocation_id),
    )
    .await;
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

    // Unknown flow id → 404, indistinguishable from a cross-scope flow.
    let unknown = get_oauth_flow_status(
        &app,
        "11111111-1111-1111-1111-111111111111",
        &format!("?invocation_id={}", started.invocation_id),
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
}

// A flow owned by a DIFFERENT scope must surface as 404, never 403: the read
// cannot be used as a cross-user existence oracle. Full-scope equality in
// `get_flow` rejects the mismatched owner even when the attacker supplies the
// exact invocation id, because the trusted tenant/user come from the
// authenticated caller — not the browser.
#[tokio::test]
async fn product_auth_oauth_flow_status_hides_cross_scope_flow_as_not_found() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        shared.clone(),
        Arc::new(RecordingAuthDispatcher::default()),
    ));
    let app = build_app_with_product_auth_service(product_auth);

    // Seed a flow owned by a DIFFERENT user in the same tenant/agent/project.
    let other_invocation = InvocationId::new();
    let other_scope = AuthProductScope::new(
        ResourceScope {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            user_id: UserId::new("user-mallory").expect("user"),
            agent_id: Some(AgentId::new(AGENT).expect("agent")),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            mission_id: None,
            thread_id: None,
            invocation_id: other_invocation,
        },
        AuthSurface::Callback,
    );
    let flow = shared
        .create_flow(NewAuthFlow {
            id: Some(AuthFlowId::new()),
            scope: other_scope,
            kind: AuthFlowKind::IntegrationCredential,
            provider: AuthProviderId::new("github").expect("provider"),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .expect("authorization url"),
                expires_at: Utc::now() + ChronoDuration::minutes(5),
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at: Utc::now() + ChronoDuration::minutes(5),
        })
        .await
        .expect("seed cross-user flow");

    // USER (the only authenticated identity) polls mallory's flow, even with the
    // exact invocation id. Cross-scope must read as not-found, never forbidden.
    let response = get_oauth_flow_status(
        &app,
        &flow.id.to_string(),
        &format!("?invocation_id={other_invocation}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn product_auth_google_oauth_start_builds_provider_authorization_url() {
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        Arc::new(InMemoryAuthProductServices::new()),
        Arc::new(RecordingAuthDispatcher::default()),
    ));
    let app = build_app_with_product_auth_service_and_config(
        product_auth,
        Some(
            GoogleOAuthRouteConfig::new(
                "google-client.apps.googleusercontent.com",
                "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
            )
            .expect("google oauth route config")
            .with_hosted_domain_hint("example.com")
            .expect("hosted-domain hint"),
        ),
    );

    let response = post_google_oauth_start(
        &app,
        google_oauth_start_body(json!({
            "session_id": "web-session-google",
            "thread_id": "thread-auth-google"
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    assert!(!body.contains("google-pkce"));
    let json: serde_json::Value = serde_json::from_str(&body).expect("start json");
    assert_eq!(json["provider"], "google");
    assert_eq!(json["continuation"]["type"], "setup_only");
    let authorization_url = json["authorization_url"]
        .as_str()
        .expect("authorization url");
    let parsed = url::Url::parse(authorization_url).expect("google authorization url");
    assert_eq!(parsed.host_str(), Some("accounts.google.com"));
    let query = parsed.query_pairs().collect::<Vec<_>>();
    assert!(
        query.iter().any(|(name, value)| name == "client_id"
            && value == "google-client.apps.googleusercontent.com")
    );
    assert!(query.iter().any(|(name, value)| name == "redirect_uri"
        && value == "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback"));
    assert!(query.iter().any(|(name, value)| name == "scope"
        && value.contains(GOOGLE_GMAIL_READONLY_SCOPE)
        && value.contains(GOOGLE_CALENDAR_READONLY_SCOPE)));
    assert!(
        query
            .iter()
            .any(|(name, value)| name == "access_type" && value == "offline")
    );
    assert!(
        query
            .iter()
            .any(|(name, value)| name == "hd" && value == "example.com")
    );
}

#[tokio::test]
async fn extension_oauth_start_rejects_package_missing_from_installed_inventory() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        shared.clone(),
        Arc::new(RecordingAuthDispatcher::default()),
    ));
    let app = build_app_with_product_auth_service_and_config(
        product_auth,
        Some(google_oauth_route_config()),
    );

    let response = post_extension_oauth_start(
        &app,
        "google-calendar",
        json!({
            "provider": "google",
            "account_label": "work google",
            "invocation_id": InvocationId::new().to_string(),
            "scopes": [GOOGLE_CALENDAR_READONLY_SCOPE],
            "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"invalid_request\""));
    assert!(
        shared.flow_records_snapshot().is_empty(),
        "an absent extension must be rejected before an OAuth flow is created"
    );
}

#[tokio::test]
async fn extension_oauth_start_for_installed_package_attaches_update_binding() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        shared.clone(),
        Arc::new(RecordingAuthDispatcher::default()),
    ));
    let app = build_app_with_product_auth_service_config_and_extensions(
        product_auth,
        Some(google_oauth_route_config()),
        &["google-calendar"],
    );
    let invocation_id = InvocationId::new();
    let scope = AuthProductScope::new(
        ResourceScope {
            tenant_id: TenantId::new(TENANT).expect("tenant id"),
            user_id: UserId::new(USER).expect("user id"),
            agent_id: Some(AgentId::new(AGENT).expect("agent id")),
            project_id: Some(ProjectId::new(PROJECT).expect("project id")),
            mission_id: None,
            thread_id: None,
            invocation_id,
        },
        AuthSurface::Callback,
    );
    let account = shared
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: AuthProviderId::new("google").expect("provider id"),
            label: CredentialAccountLabel::new("google-calendar google").expect("account label"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("existing-google-access").expect("secret")),
            refresh_secret: Some(SecretHandle::new("existing-google-refresh").expect("secret")),
            scopes: vec![
                ProviderScope::new(GOOGLE_CALENDAR_READONLY_SCOPE.to_string()).expect("scope"),
            ],
        })
        .await
        .expect("seed credential account");

    let response = post_extension_oauth_start(
        &app,
        "google-calendar",
        json!({
            "provider": "google",
            "account_label": "work google",
            "invocation_id": invocation_id.to_string(),
            "scopes": [GOOGLE_CALENDAR_READONLY_SCOPE],
            "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    let json: serde_json::Value = serde_json::from_str(&body).expect("start json");
    let flow_id = AuthFlowId::from_uuid(
        Uuid::parse_str(json["flow_id"].as_str().expect("flow id")).expect("flow uuid"),
    );
    let flow = shared
        .get_flow(&scope, flow_id)
        .await
        .expect("flow lookup")
        .expect("created flow");
    assert_eq!(
        flow.update_binding
            .as_ref()
            .map(|binding| binding.account_id),
        Some(account.id)
    );
    assert_eq!(
        flow.continuation,
        AuthContinuationRef::LifecycleActivation {
            package_ref: ironclaw_auth::LifecyclePackageRef::new("google-calendar")
                .expect("lifecycle package ref"),
        }
    );
}

#[tokio::test]
async fn product_auth_google_oauth_callback_completes_setup_flow() {
    let (app, dispatcher) = build_app_with_google_oauth();
    let (start_json, state) = start_google_oauth_flow(&app).await;
    let scopes = format!("{GOOGLE_GMAIL_READONLY_SCOPE}%20{GOOGLE_CALENDAR_READONLY_SCOPE}");

    let callback_response = app
        .oneshot(callback_request(format!(
            "/api/reborn/product-auth/oauth/google/callback?state={state}&code=google-auth-code&scope={scopes}"
        )))
        .await
        .expect("oneshot");
    assert_eq!(callback_response.status(), StatusCode::OK);
    let callback_body = read_body_string(callback_response).await;
    assert!(!callback_body.contains("google-auth-code"));
    let callback_json: serde_json::Value =
        serde_json::from_str(&callback_body).expect("callback json");
    assert_eq!(callback_json["flow_id"], start_json["flow_id"]);
    assert_eq!(callback_json["status"], "completed");
    assert_eq!(dispatcher.events().len(), 1);
}

#[tokio::test]
async fn product_auth_google_oauth_callback_accepts_provider_extra_scopes_without_overclaiming() {
    let provider_client = Arc::new(RecordingProviderClient::default());
    let (app, dispatcher) = build_app_with_google_oauth_provider(provider_client.clone());
    let (start_json, state) = start_google_oauth_flow(&app).await;
    let scopes = format!(
        "openid%20email%20profile%20{GOOGLE_GMAIL_READONLY_SCOPE}%20{GOOGLE_CALENDAR_READONLY_SCOPE}"
    );

    let callback_response = app
        .oneshot(callback_request(format!(
            "/api/reborn/product-auth/oauth/google/callback?state={state}&code=google-auth-code&scope={scopes}"
        )))
        .await
        .expect("oneshot");

    assert_eq!(callback_response.status(), StatusCode::OK);
    let callback_body = read_body_string(callback_response).await;
    let callback_json: serde_json::Value =
        serde_json::from_str(&callback_body).expect("callback json");
    assert_eq!(callback_json["flow_id"], start_json["flow_id"]);
    assert_eq!(callback_json["status"], "completed");
    assert_eq!(dispatcher.events().len(), 1);
    assert_eq!(
        provider_client.exchanged_scopes(),
        vec![vec![
            GOOGLE_GMAIL_READONLY_SCOPE.to_string(),
            GOOGLE_CALENDAR_READONLY_SCOPE.to_string()
        ]]
    );
}

#[tokio::test]
async fn product_auth_google_oauth_browser_callback_notifies_chat_without_secrets() {
    let (app, dispatcher) = build_app_with_google_oauth();
    let (start_json, state) = start_google_oauth_flow(&app).await;
    let scopes = format!("{GOOGLE_GMAIL_READONLY_SCOPE}%20{GOOGLE_CALENDAR_READONLY_SCOPE}");

    let callback_response = app
        .oneshot(callback_request_accept(
            format!(
                "/api/reborn/product-auth/oauth/google/callback?state={state}&code=google-auth-code&scope={scopes}"
            ),
            HeaderValue::from_static("text/html,application/xhtml+xml"),
        ))
        .await
        .expect("oneshot");

    assert_eq!(callback_response.status(), StatusCode::OK);
    let content_type = callback_response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(content_type.starts_with("text/html"));

    let callback_body = read_body_string(callback_response).await;
    assert!(callback_body.contains("ironclaw:product-auth:oauth-complete"));
    assert!(callback_body.contains("ironclaw-product-auth"));
    assert!(callback_body.contains(start_json["flow_id"].as_str().expect("flow id")));
    assert!(callback_body.contains("\"continuation\""));
    assert!(!callback_body.contains(&state));
    assert!(!callback_body.contains("google-auth-code"));
    assert_eq!(dispatcher.events().len(), 1);
}

#[tokio::test]
async fn product_auth_google_oauth_callback_rejects_disallowed_scopes() {
    let (app, dispatcher) = build_app_with_google_oauth();
    let (_, state) = start_google_oauth_flow(&app).await;

    let response = app
        .clone()
        .oneshot(callback_request(format!(
            "/api/reborn/product-auth/oauth/google/callback?state={state}&code=google-auth-code&scope={DISALLOWED_GOOGLE_SCOPE}"
        )))
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"provider_denied\""));
    assert!(!body.contains(&state));
    assert!(!body.contains("google-auth-code"));
    assert!(!body.contains(DISALLOWED_GOOGLE_SCOPE));
    assert!(dispatcher.events().is_empty());

    let replay_response = app
        .oneshot(callback_request(format!(
            "/api/reborn/product-auth/oauth/google/callback?state={state}&code=google-auth-code&scope={GOOGLE_GMAIL_READONLY_SCOPE}"
        )))
        .await
        .expect("oneshot");
    assert_eq!(replay_response.status(), StatusCode::CONFLICT);
    assert!(
        read_body_string(replay_response)
            .await
            .contains("\"code\":\"flow_already_terminal\"")
    );
}

#[tokio::test]
async fn product_auth_google_oauth_callback_provider_denial_is_sanitized() {
    let (app, dispatcher) = build_app_with_google_oauth();
    let (_, state) = start_google_oauth_flow(&app).await;

    let response = app
        .oneshot(callback_request(format!(
            "/api/reborn/product-auth/oauth/google/callback?state={state}&error=access_denied"
        )))
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"provider_denied\""));
    assert!(!body.contains(&state));
    assert!(!body.contains("access_denied"));
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn product_auth_google_oauth_callback_unknown_state_is_sanitized() {
    let (app, dispatcher) = build_app_with_google_oauth();

    let response = app
        .oneshot(callback_request(
            "/api/reborn/product-auth/oauth/google/callback?state=unknown-google-state&error=access_denied"
                .to_string(),
        ))
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"malformed_callback\""));
    assert!(!body.contains("unknown-google-state"));
    assert!(!body.contains("access_denied"));
    assert!(dispatcher.events().is_empty());
}

#[tokio::test]
async fn product_auth_google_oauth_callback_rejects_empty_parsed_scopes() {
    let (app, dispatcher) = build_app_with_google_oauth();
    let (_, state) = start_google_oauth_flow(&app).await;

    let response = app
        .clone()
        .oneshot(callback_request(format!(
            "/api/reborn/product-auth/oauth/google/callback?state={state}&code=google-auth-code&scope="
        )))
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"provider_denied\""));
    assert!(!body.contains(&state));
    assert!(!body.contains("google-auth-code"));
    assert!(dispatcher.events().is_empty());

    let replay_response = app
        .oneshot(callback_request(format!(
            "/api/reborn/product-auth/oauth/google/callback?state={state}&code=google-auth-code&scope="
        )))
        .await
        .expect("oneshot");
    assert_eq!(replay_response.status(), StatusCode::CONFLICT);
    assert!(
        read_body_string(replay_response)
            .await
            .contains("\"code\":\"flow_already_terminal\"")
    );
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

// Caller-level coverage for the Slack personal OAuth serve wiring: the
// handler-level tests in product_auth_serve construct ProductAuthRouteState
// directly, which would stay green if webui_serve stopped carrying
// WebuiServeConfig::with_slack_personal_oauth into webui_v2_app. These tests
// drive the composed router so that composition seam cannot regress silently.
#[cfg(feature = "slack-v2-host-beta")]
mod slack_personal_oauth_serve {
    use super::*;
    use ironclaw_product_adapters::AdapterInstallationId;
    use ironclaw_reborn_composition::{
        OAuthRedirectUri, SlackPersonalOAuthBindingConfig, SlackPersonalSetupServiceSlot,
    };

    const SLACK_PERSONAL_CALLBACK_PATH: &str =
        "/api/reborn/product-auth/oauth/slack_personal/callback";
    const SLACK_START_PATH: &str = "/api/webchat/v2/extensions/slack/setup/oauth/start";

    async fn slack_personal_slot() -> SlackPersonalSetupServiceSlot {
        SlackPersonalSetupServiceSlot::filled_with_in_memory_setup_for_tests(
            OAuthRedirectUri::new(
                "http://127.0.0.1:3000/api/reborn/product-auth/oauth/slack_personal/callback",
            )
            .expect("slack redirect uri"),
            "slack-client-id",
            "slack-client-secret",
        )
        .await
    }

    async fn build_app_with_slack_personal_oauth() -> axum::Router {
        let dispatcher = Arc::new(RecordingAuthDispatcher::default());
        let product_auth = Arc::new(RebornProductAuthServices::from_shared(
            Arc::new(InMemoryAuthProductServices::new()),
            dispatcher,
        ));
        let bundle = RebornWebuiBundle {
            api: Arc::new(UnusedServices::with_installed_extensions(&["slack"])),
            product_auth: Some(product_auth),
            readiness: RebornReadiness::disabled(),
        };
        let binding = SlackPersonalOAuthBindingConfig::in_memory_for_tests(
            TenantId::new(TENANT).expect("tenant"),
            AdapterInstallationId::new("install-test").expect("installation"),
        )
        .expect("in-memory Slack OAuth binding config");
        let config = WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(OnlyValidToken),
            vec![HeaderValue::from_static("http://localhost:1234")],
        )
        .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
        .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
        .with_slack_personal_oauth(slack_personal_slot().await)
        .with_slack_personal_oauth_binding(binding);
        webui_v2_app(bundle, config).expect("webui v2 app")
    }

    fn slack_oauth_start_body() -> serde_json::Value {
        json!({
            "provider": "slack_personal",
            "account_label": "personal slack",
            "scopes": ["search:read"],
            "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
            "invocation_id": InvocationId::new().to_string(),
        })
    }

    #[tokio::test]
    async fn slack_personal_oauth_start_requires_bearer_auth() {
        let app = build_app_with_slack_personal_oauth().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(SLACK_START_PATH)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(slack_oauth_start_body().to_string()))
                    .expect("request"),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn slack_personal_oauth_start_serves_through_composed_router() {
        let app = build_app_with_slack_personal_oauth().await;

        let response = post_extension_oauth_start(&app, "slack", slack_oauth_start_body()).await;

        let status = response.status();
        let body = read_body_string(response).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "unexpected OAuth start body: {body}"
        );
        let json: serde_json::Value = serde_json::from_str(&body).expect("start json");
        let authorization_url = json["authorization_url"]
            .as_str()
            .expect("authorization url");
        let parsed = url::Url::parse(authorization_url).expect("slack authorization url");
        assert_eq!(parsed.host_str(), Some("slack.com"));
        let user_scope = parsed
            .query_pairs()
            .find_map(|(name, value)| (name == "user_scope").then(|| value.into_owned()))
            .expect("user_scope in slack authorize url");
        assert!(
            user_scope.contains("search:read"),
            "server-side scope set must reach the authorize url: {user_scope}"
        );
    }

    #[tokio::test]
    async fn slack_personal_oauth_start_fails_closed_without_slot() {
        // Product auth is mounted but the Slack slot was never carried into
        // the serve config — the exact state a dropped webui_serve wiring
        // block would produce.
        let (app, _) = build_app_with_product_auth_and_installed_extensions(&["slack"]);

        let response = post_extension_oauth_start(&app, "slack", slack_oauth_start_body()).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = read_body_string(response).await;
        assert!(body.contains("\"code\":\"backend_unavailable\""));
    }

    #[tokio::test]
    async fn slack_personal_oauth_callback_is_mounted_and_fails_closed_sanitized() {
        // The slot is wired but the binding config deliberately is not — the
        // callback must be mounted (not 404) and fail closed with a sanitized
        // error rather than proceeding without an identity binding path.
        let app = build_app_with_slack_personal_oauth().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "{SLACK_PERSONAL_CALLBACK_PATH}?state=malformed-state&code=denied"
                    ))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");

        assert_ne!(
            response.status(),
            StatusCode::NOT_FOUND,
            "the public slack_personal callback must be mounted by webui_v2_app"
        );
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = read_body_string(response).await;
        assert!(
            !body.contains("malformed-state"),
            "raw state must not be echoed: {body}"
        );
    }
}
