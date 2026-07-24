//! Caller-level tests for Reborn WebUI v2 product-auth OAuth routes.

// arch-exempt: large_file, caller-level product-auth route regression coverage, plan #5905

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationEvent, AuthContinuationRef, AuthFlowId, AuthFlowKind,
    AuthFlowManager, AuthFlowStatus, AuthInteractionId, AuthInteractionService, AuthProductError,
    AuthProductScope, AuthProviderClient, AuthProviderId, AuthSurface, CredentialAccountLabel,
    CredentialAccountService, CredentialAccountStatus, CredentialOwnership,
    CredentialSelectionInput, CredentialSetupService, GOOGLE_CALENDAR_READONLY_SCOPE,
    GOOGLE_GMAIL_READONLY_SCOPE, InMemoryAuthProductServices, ManualTokenSetupRequest, NewAuthFlow,
    NewCredentialAccount, OAuthAuthorizationUrl, OAuthProviderCallbackRequest,
    OAuthProviderExchange, OAuthProviderExchangeContext, OAuthProviderRefresh,
    OAuthProviderRefreshRequest, ProviderScope, SecretCleanupService, SecretSubmitRequest,
    SecretSubmitResult,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProductSurfaceCaller, ProductSurfaceError, ProjectId, ResourceScope,
    SecretHandle, TenantId, UserId,
};
use ironclaw_product::{
    EXTENSION_SETUP_VIEW, EXTENSIONS_VIEW, LifecyclePackageKind, LifecyclePackageRef,
    LifecyclePublicState, RebornExtensionCredentialSetup, RebornExtensionInfo,
    RebornExtensionListResponse, RebornExtensionSetupSecret, RebornSetupExtensionResponse,
    rejecting_product_surface_error,
};
use ironclaw_reborn_composition::{
    RebornAuthContinuationDispatcher, RebornProductAuthServices, RebornReadiness, RebornWebuiBundle,
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
    async fn dispatch_canceled_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

/// Returns a queued sequence of dispatch outcomes, one per call — models a
/// lifecycle continuation whose fan-out is first deferred (a retryable error,
/// e.g. setup still incomplete) and later succeeds once readiness settles.
struct SequencedAuthDispatcher {
    outcomes: Mutex<VecDeque<Result<(), AuthProductError>>>,
    events: Mutex<Vec<AuthContinuationEvent>>,
}

impl SequencedAuthDispatcher {
    fn new(outcomes: impl IntoIterator<Item = Result<(), AuthProductError>>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
            events: Mutex::new(Vec::new()),
        }
    }

    fn dispatch_count(&self) -> usize {
        self.events.lock().expect("auth events lock").len()
    }
}

#[async_trait]
impl RebornAuthContinuationDispatcher for SequencedAuthDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.events.lock().expect("auth events lock").push(event);
        self.outcomes
            .lock()
            .expect("outcomes lock")
            .pop_front()
            .expect("a queued dispatch outcome for each reconcile pass")
    }
    async fn dispatch_canceled_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
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
    extension_setups: HashMap<String, RebornSetupExtensionResponse>,
}

impl UnusedServices {
    fn with_installed_extensions(package_ids: &[&str]) -> Self {
        let extension_setups = package_ids
            .iter()
            .map(|package_id| {
                let package_ref =
                    LifecyclePackageRef::new(LifecyclePackageKind::Extension, *package_id)
                        .expect("installed extension package ref");
                let scopes = vec![
                    GOOGLE_GMAIL_READONLY_SCOPE.to_string(),
                    GOOGLE_CALENDAR_READONLY_SCOPE.to_string(),
                ];
                (
                    (*package_id).to_string(),
                    RebornSetupExtensionResponse {
                        package_ref,
                        phase: LifecyclePublicState::SetupNeeded,
                        blockers: Vec::new(),
                        payload: None,
                        secrets: vec![RebornExtensionSetupSecret {
                            name: "google_oauth".to_string(),
                            provider: "google".to_string(),
                            prompt: "google credential".to_string(),
                            optional: false,
                            provided: false,
                            setup: RebornExtensionCredentialSetup::OAuth {
                                account_label: format!("{package_id} google"),
                                scopes,
                                invocation_id: InvocationId::new().to_string(),
                            },
                            credential_ref: None,
                        }],
                        onboarding: None,
                    },
                )
            })
            .collect();
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
                    runtime: "wasm".to_string(),
                    description: "test installed extension".to_string(),
                    tools: Vec::new(),
                    installation_state: ironclaw_product::LifecyclePublicState::SetupNeeded,
                    activation_error: None,
                    version: None,
                    onboarding: None,
                    auth_accounts: Vec::new(),
                    surfaces: Vec::new(),
                    install_scope: None,
                })
                .collect(),
            extension_setups,
        }
    }
}

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
        request: ironclaw_host_api::ProductSurfaceQueryRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceQueryPage, ProductSurfaceError> {
        match request.view_id.as_str() {
            id if id == EXTENSIONS_VIEW.id => Ok(ironclaw_host_api::ProductSurfaceQueryPage {
                items: vec![
                    serde_json::to_value(RebornExtensionListResponse {
                        extensions: self.installed_extensions.clone(),
                    })
                    .expect("extension list payload"),
                ],
                next_cursor: None,
            }),
            id if id == EXTENSION_SETUP_VIEW.id => {
                let package_id = request
                    .input
                    .get("package_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(rejecting_product_surface_error)?;
                let setup = self
                    .extension_setups
                    .get(package_id)
                    .cloned()
                    .ok_or_else(rejecting_product_surface_error)?;
                Ok(ironclaw_host_api::ProductSurfaceQueryPage {
                    items: vec![serde_json::to_value(setup).expect("extension setup payload")],
                    next_cursor: None,
                })
            }
            _ => Err(rejecting_product_surface_error()),
        }
    }

    async fn stream_events(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ironclaw_host_api::ProductSurfaceStreamRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceStreamResponse, ProductSurfaceError> {
        Err(rejecting_product_surface_error())
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
            installed_package_ids,
        ),
        dispatcher,
    )
}

fn build_app_with_product_auth_service(
    product_auth: Arc<RebornProductAuthServices>,
) -> axum::Router {
    build_app_with_product_auth_service_and_config(product_auth)
}

fn build_app_with_product_auth_service_and_config(
    product_auth: Arc<RebornProductAuthServices>,
) -> axum::Router {
    build_app_with_product_auth_service_config_and_extensions(product_auth, &[])
}

fn build_app_with_product_auth_service_config_and_extensions(
    product_auth: Arc<RebornProductAuthServices>,
    installed_package_ids: &[&str],
) -> axum::Router {
    let bundle = RebornWebuiBundle {
        product_surface: Arc::new(UnusedServices::with_installed_extensions(
            installed_package_ids,
        )),
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

/// Deployment client material for the synthetic test recipes, keyed by
/// vendor — the engine resolves it exactly as production does.
#[derive(Debug)]
struct StaticVendorClientCredentials;

#[async_trait]
impl ironclaw_auth::EngineClientCredentialsSource for StaticVendorClientCredentials {
    async fn resolve(
        &self,
        vendor: &str,
        _credentials: &ironclaw_host_api::RecipeClientCredentials,
    ) -> Result<ironclaw_auth::EngineOAuthClientMaterial, AuthProductError> {
        let client_id = match vendor {
            "google" => "google-client.apps.googleusercontent.com",
            other => &format!("{other}-client-id"),
        };
        Ok(ironclaw_auth::EngineOAuthClientMaterial {
            client_id: ironclaw_auth::OAuthClientId::new(client_id)?,
            client_secret: None,
        })
    }
}

#[derive(Debug)]
struct PanicVendorEgress;

#[async_trait]
impl ironclaw_host_api::RuntimeHttpEgress for PanicVendorEgress {
    async fn execute(
        &self,
        request: ironclaw_host_api::RuntimeHttpEgressRequest,
    ) -> Result<
        ironclaw_host_api::RuntimeHttpEgressResponse,
        ironclaw_host_api::RuntimeHttpEgressError,
    > {
        panic!(
            "route tests must not perform vendor HTTP egress: {}",
            request.url
        );
    }
}

/// Engine over a synthetic Google-shaped recipe: the ceiling and extra
/// authorize params mirror the bundled manifest recipe, so route behavior
/// (ceiling rejection, host-built params) is exercised as production data
/// would drive it.
fn google_test_engine() -> Arc<ironclaw_auth::AuthEngine> {
    let recipe: ironclaw_host_api::VendorAuthRecipe = serde_json::from_value(json!({
        "method": "oauth2_code",
        "display_name": "Google account",
        "authorization_endpoint": "https://accounts.google.com/o/oauth2/v2/auth",
        "token_endpoint": "https://oauth2.googleapis.com/token",
        "scopes": [GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_CALENDAR_READONLY_SCOPE],
        "extra_authorize_params": {
            "access_type": "offline",
            "include_granted_scopes": "true",
            "prompt": "consent"
        },
        "client_credentials": { "client_id_handle": "google_oauth_client_id" },
        "token_response": {
            "access_token": "/access_token",
            "refresh_token": "/refresh_token",
            "expires_in": "/expires_in",
            "scope": { "path": "/scope", "missing": "fallback_to_requested" }
        },
    }))
    .expect("google test recipe parses");
    vendor_test_engine(ironclaw_auth::ResolvedVendorAuthRecipe {
        vendor: "google".to_string(),
        recipe,
        token_exchange_resource: None,
    })
}

fn vendor_test_engine(
    recipe: ironclaw_auth::ResolvedVendorAuthRecipe,
) -> Arc<ironclaw_auth::AuthEngine> {
    Arc::new(ironclaw_auth::AuthEngine::new(
        ironclaw_auth::AuthEngineDeps {
            recipes: Arc::new(ironclaw_auth::StaticAuthRecipeResolver::new(vec![recipe])),
            client_credentials: Arc::new(StaticVendorClientCredentials),
            egress: Arc::new(PanicVendorEgress),
            secret_store: Arc::new(ironclaw_secrets::SecretStore::ephemeral()),
            callback_base: ironclaw_auth::EngineCallbackBase::new(
                "http://127.0.0.1:3000/api/reborn/product-auth/oauth",
            )
            .expect("callback base"),
            dcr_client_name: "Ironclaw".to_string(),
        },
    ))
}

fn build_app_with_google_oauth() -> (axum::Router, Arc<RecordingAuthDispatcher>) {
    let dispatcher = Arc::new(RecordingAuthDispatcher::default());
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(
            Arc::new(InMemoryAuthProductServices::new()),
            dispatcher.clone(),
        )
        .with_auth_engine(google_test_engine()),
    );
    (
        build_app_with_product_auth_service_config_and_extensions(product_auth, &["google-tools"]),
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
    let product_auth = Arc::new(
        RebornProductAuthServices::new(
            flow_manager,
            interaction_service,
            credential_setup_service,
            credential_account_service,
            provider_client,
            cleanup_service,
            dispatcher.clone(),
        )
        .with_auth_engine(google_test_engine()),
    );
    (
        build_app_with_product_auth_service_config_and_extensions(product_auth, &["google-tools"]),
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

async fn post_oauth_flow_reconcile(
    app: &axum::Router,
    flow_id: &str,
    query: &str,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/api/reborn/product-auth/oauth/flow/{flow_id}/reconcile{query}"
                ))
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot")
}

fn callback_scope_for(user_id: &str, invocation_id: InvocationId) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            user_id: UserId::new(user_id).expect("user"),
            agent_id: Some(AgentId::new(AGENT).expect("agent")),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            mission_id: None,
            thread_id: None,
            invocation_id,
        },
        AuthSurface::Callback,
    )
}

async fn seed_completed_unfenced_selection_flow(
    shared: &InMemoryAuthProductServices,
    scope: AuthProductScope,
) -> AuthFlowId {
    let provider = AuthProviderId::new("github").expect("provider");
    let account = shared
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: provider.clone(),
            label: CredentialAccountLabel::new("reconcile github").expect("account label"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(
                SecretHandle::new("reconcile-account-secret").expect("secret handle"),
            ),
            refresh_secret: None,
            scopes: vec![ProviderScope::new("repo").expect("provider scope")],
        })
        .await
        .expect("seed configured credential account");
    let flow = shared
        .create_flow(NewAuthFlow {
            id: Some(AuthFlowId::new()),
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider.clone(),
            challenge: AuthChallenge::AccountSelectionRequired {
                provider,
                accounts: vec![account.projection()],
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at: Utc::now() + ChronoDuration::minutes(5),
        })
        .await
        .expect("seed account-selection flow");
    let completed = shared
        .complete_credential_selection(
            &scope,
            CredentialSelectionInput {
                flow_id: flow.id,
                credential_account_id: account.id,
            },
        )
        .await
        .expect("complete flow without dispatching its continuation");
    assert_eq!(completed.status, AuthFlowStatus::Completed);
    assert!(completed.continuation_emitted_at.is_none());
    flow.id
}

/// Seed a completed-but-unfenced flow whose continuation is a lifecycle
/// activation (the shape the `LifecycleAuthContinuationDispatcher` handles),
/// mirroring [`seed_completed_unfenced_selection_flow`].
async fn seed_completed_unfenced_lifecycle_flow(
    shared: &InMemoryAuthProductServices,
    scope: AuthProductScope,
    package_id: &str,
) -> AuthFlowId {
    let provider = AuthProviderId::new("google").expect("provider");
    let account = shared
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: provider.clone(),
            label: CredentialAccountLabel::new("lifecycle google").expect("account label"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(
                SecretHandle::new("lifecycle-account-secret").expect("secret handle"),
            ),
            refresh_secret: None,
            scopes: vec![ProviderScope::new("repo").expect("provider scope")],
        })
        .await
        .expect("seed configured credential account");
    let flow = shared
        .create_flow(NewAuthFlow {
            id: Some(AuthFlowId::new()),
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider.clone(),
            challenge: AuthChallenge::AccountSelectionRequired {
                provider,
                accounts: vec![account.projection()],
            },
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: ironclaw_auth::LifecyclePackageRef::new(package_id)
                    .expect("lifecycle package ref"),
            },
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at: Utc::now() + ChronoDuration::minutes(5),
        })
        .await
        .expect("seed lifecycle-activation flow");
    let completed = shared
        .complete_credential_selection(
            &scope,
            CredentialSelectionInput {
                flow_id: flow.id,
                credential_account_id: account.id,
            },
        )
        .await
        .expect("complete flow without dispatching its continuation");
    assert_eq!(completed.status, AuthFlowStatus::Completed);
    assert!(completed.continuation_emitted_at.is_none());
    flow.id
}

/// Item 3: a lifecycle continuation whose fan-out is deferred (the dispatcher
/// reports a retryable error — the shape `LifecycleAuthContinuationDispatcher`
/// returns when OAuth completed but setup is still incomplete) must leave the
/// completed flow UNFENCED, so a later reconcile finishes the fan-out. A plain
/// success would durably fence it and permanently strand the blocked runs.
#[tokio::test]
async fn product_auth_deferred_lifecycle_continuation_stays_redrivable_until_readiness() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let dispatcher = Arc::new(SequencedAuthDispatcher::new([
        Err(AuthProductError::BackendUnavailable), // first pass: setup incomplete
        Ok(()),                                    // later reconcile: readiness Active
    ]));
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        shared.clone(),
        dispatcher.clone(),
    ));
    let app = build_app_with_product_auth_service(product_auth);
    let invocation_id = InvocationId::new();
    let scope = callback_scope_for(USER, invocation_id);
    let flow_id =
        seed_completed_unfenced_lifecycle_flow(&shared, scope.clone(), "google-calendar").await;

    // First reconcile: the fan-out is deferred → the flow must stay UN-fenced.
    let first = post_oauth_flow_reconcile(
        &app,
        &flow_id.to_string(),
        &format!("?invocation_id={invocation_id}"),
    )
    .await;
    assert_ne!(
        first.status(),
        StatusCode::OK,
        "a deferred continuation surfaces as retryable, not a settled success"
    );
    assert_eq!(dispatcher.dispatch_count(), 1);
    assert!(
        shared
            .get_flow(&scope, flow_id)
            .await
            .expect("read reconciled flow")
            .expect("reconciled flow exists")
            .continuation_emitted_at
            .is_none(),
        "a deferred (retryable) continuation must not be durably fenced",
    );

    // Later reconcile: readiness now Active → the same continuation re-drives,
    // fans out, and only NOW is the flow fenced. Not permanently stuck.
    let second = post_oauth_flow_reconcile(
        &app,
        &flow_id.to_string(),
        &format!("?invocation_id={invocation_id}"),
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        dispatcher.dispatch_count(),
        2,
        "the deferred continuation re-drove on the readiness reconcile"
    );
    assert!(
        shared
            .get_flow(&scope, flow_id)
            .await
            .expect("read reconciled flow")
            .expect("reconciled flow exists")
            .continuation_emitted_at
            .is_some(),
        "the successful reconcile finally fences the continuation",
    );
}

fn google_oauth_start_body(extra_fields: serde_json::Value) -> serde_json::Value {
    let expires_at = (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339();
    let mut body = json!({
        "requirement": "google_oauth",
        "expires_at": expires_at,
        "invocation_id": InvocationId::new().to_string(),
    });
    merge_json_object(&mut body, extra_fields);
    body
}

async fn post_google_oauth_start(
    app: &axum::Router,
    body: serde_json::Value,
) -> axum::response::Response {
    post_extension_oauth_start(app, "google-tools", body).await
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
    let start_response = post_google_oauth_start(app, google_oauth_start_body(json!({}))).await;
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
                .uri("/api/webchat/v2/extensions/google-tools/setup/oauth/start")
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
    // Installed inventory present so the engine absence (not the inventory
    // guard) is what this test pins.
    let (app, _) = build_app_with_product_auth_and_installed_extensions(&["google-tools"]);

    let response = post_google_oauth_start(&app, google_oauth_start_body(json!({}))).await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"backend_unavailable\""));
}

#[tokio::test]
async fn product_auth_google_oauth_start_rejects_browser_owned_provider_scopes() {
    let (app, _) = build_app_with_google_oauth();

    // The extension route accepts only a manifest requirement key. A browser
    // cannot override that requirement with another provider/scope set, even
    // when the global provider recipe catalog knows the provider.
    let response = post_google_oauth_start(
        &app,
        google_oauth_start_body(json!({
            "provider": "google",
            "account_label": "attacker-selected account",
            "scopes": [DISALLOWED_GOOGLE_SCOPE],
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = read_body_string(response).await;
    assert!(!body.contains(DISALLOWED_GOOGLE_SCOPE));
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

#[tokio::test]
async fn product_auth_oauth_flow_reconcile_dispatches_completed_unfenced_flow_exactly_once() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let dispatcher = Arc::new(RecordingAuthDispatcher::default());
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        shared.clone(),
        dispatcher.clone(),
    ));
    let app = build_app_with_product_auth_service(product_auth);
    let invocation_id = InvocationId::new();
    let scope = callback_scope_for(USER, invocation_id);
    let flow_id = seed_completed_unfenced_selection_flow(&shared, scope.clone()).await;

    let response = post_oauth_flow_reconcile(
        &app,
        &flow_id.to_string(),
        &format!("?invocation_id={invocation_id}"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    let json: serde_json::Value = serde_json::from_str(&body).expect("reconcile json");
    assert_eq!(json, json!({ "status": "completed" }));
    let events = dispatcher.events();
    assert_eq!(events.len(), 1, "unfenced continuation dispatches once");
    assert_eq!(events[0].flow_id, flow_id);
    assert_eq!(events[0].scope, scope);
    assert_eq!(events[0].continuation, AuthContinuationRef::SetupOnly);
    assert!(
        shared
            .get_flow(&events[0].scope, flow_id)
            .await
            .expect("read reconciled flow")
            .expect("reconciled flow exists")
            .continuation_emitted_at
            .is_some(),
        "successful route reconciliation durably fences the continuation"
    );

    let replay = post_oauth_flow_reconcile(
        &app,
        &flow_id.to_string(),
        &format!("?invocation_id={invocation_id}"),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(
        dispatcher.events().len(),
        1,
        "the fenced flow must not dispatch on route replay"
    );
}

#[tokio::test]
async fn product_auth_oauth_flow_reconcile_fenced_flow_does_not_redispatch() {
    let (app, dispatcher) = build_app_with_product_auth();
    let started = start_oauth_flow(&app, "reconcile-state", "reconcile-pkce", json!({})).await;
    let callback_response = app
        .clone()
        .oneshot(callback_request(callback_uri(
            &started.flow_id,
            &started.invocation_id,
            USER,
            "reconcile-state",
            "&provider=github&account_label=work%20github&code=reconcile-code&scopes=repo",
        )))
        .await
        .expect("oneshot");
    assert_eq!(callback_response.status(), StatusCode::OK);
    assert_eq!(dispatcher.events().len(), 1);

    let response = post_oauth_flow_reconcile(
        &app,
        &started.flow_id,
        &format!("?invocation_id={}", started.invocation_id),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the frontend completion watcher must target a production-mounted route"
    );
    let body = read_body_string(response).await;
    let json: serde_json::Value = serde_json::from_str(&body).expect("reconcile json");
    assert_eq!(json["status"], "completed");
    assert_eq!(
        dispatcher.events().len(),
        1,
        "an acknowledged continuation must not dispatch twice"
    );
    assert!(!body.contains("reconcile-state"));
    assert!(!body.contains("reconcile-pkce"));
    assert!(!body.contains("reconcile-code"));
}

#[tokio::test]
async fn product_auth_oauth_flow_reconcile_terminal_flow_does_not_dispatch() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let dispatcher = Arc::new(RecordingAuthDispatcher::default());
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        shared.clone(),
        dispatcher.clone(),
    ));
    let app = build_app_with_product_auth_service(product_auth);
    let invocation_id = InvocationId::new();
    let scope = callback_scope_for(USER, invocation_id);
    let flow = shared
        .create_flow(NewAuthFlow {
            id: Some(AuthFlowId::new()),
            scope: scope.clone(),
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
        .expect("seed flow");
    let canceled = shared
        .cancel_flow(&scope, flow.id)
        .await
        .expect("terminalize flow");
    assert_eq!(canceled.status, AuthFlowStatus::Canceled);

    let response = post_oauth_flow_reconcile(
        &app,
        &flow.id.to_string(),
        &format!("?invocation_id={invocation_id}"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    let json: serde_json::Value = serde_json::from_str(&body).expect("reconcile json");
    assert_eq!(json, json!({ "status": "canceled" }));
    assert!(
        dispatcher.events().is_empty(),
        "terminal flows must never enter continuation dispatch"
    );
}

// A flow owned by a DIFFERENT scope must surface as 404, never 403: the read
// cannot be used as a cross-user existence oracle. Full-scope equality in
// `get_flow` rejects the mismatched owner even when the attacker supplies the
// exact invocation id, because the trusted tenant/user come from the
// authenticated caller — not the browser.
#[tokio::test]
async fn product_auth_oauth_flow_routes_hide_cross_scope_flow_without_dispatch() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let dispatcher = Arc::new(RecordingAuthDispatcher::default());
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        shared.clone(),
        dispatcher.clone(),
    ));
    let app = build_app_with_product_auth_service(product_auth);

    // Seed a completed, unfenced flow owned by a DIFFERENT user in the same
    // tenant/agent/project. If caller scoping regresses, reconcile would enter
    // the production continuation boundary and make the dispatcher observable.
    let other_invocation = InvocationId::new();
    let other_scope = callback_scope_for("user-mallory", other_invocation);
    let flow_id = seed_completed_unfenced_selection_flow(&shared, other_scope).await;

    // USER (the only authenticated identity) polls mallory's flow, even with the
    // exact invocation id. Cross-scope must read as not-found, never forbidden.
    let response = get_oauth_flow_status(
        &app,
        &flow_id.to_string(),
        &format!("?invocation_id={other_invocation}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(response.status(), StatusCode::FORBIDDEN);

    let reconcile = post_oauth_flow_reconcile(
        &app,
        &flow_id.to_string(),
        &format!("?invocation_id={other_invocation}"),
    )
    .await;
    assert_eq!(reconcile.status(), StatusCode::NOT_FOUND);
    assert_ne!(reconcile.status(), StatusCode::FORBIDDEN);
    assert!(
        dispatcher.events().is_empty(),
        "cross-scope reconcile must not dispatch another caller's continuation"
    );
}

#[tokio::test]
async fn product_auth_google_oauth_start_builds_provider_authorization_url() {
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(
            Arc::new(InMemoryAuthProductServices::new()),
            Arc::new(RecordingAuthDispatcher::default()),
        )
        .with_auth_engine(google_test_engine()),
    );
    let app =
        build_app_with_product_auth_service_config_and_extensions(product_auth, &["google-tools"]);

    let response = post_google_oauth_start(&app, google_oauth_start_body(json!({}))).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    assert!(!body.contains("google-pkce"));
    let json: serde_json::Value = serde_json::from_str(&body).expect("start json");
    assert_eq!(json["provider"], "google");
    assert_eq!(json["continuation"]["type"], "lifecycle_activation");
    assert_eq!(json["continuation"]["package_ref"], "google-tools");
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
    // Host-owned PKCE parameters are always present.
    assert!(query.iter().any(|(name, _)| name == "code_challenge"));
    assert!(
        query
            .iter()
            .any(|(name, value)| name == "code_challenge_method" && value == "S256")
    );
}

#[tokio::test]
async fn extension_oauth_start_rejects_package_missing_from_installed_inventory() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let product_auth = Arc::new(RebornProductAuthServices::from_shared(
        shared.clone(),
        Arc::new(RecordingAuthDispatcher::default()),
    ));
    // No auth engine on purpose: the installed-inventory guard must fire
    // before the engine is even resolved, so an absent extension rejects
    // identically on engine-less deployments.
    let app = build_app_with_product_auth_service_and_config(product_auth);

    let response = post_extension_oauth_start(
        &app,
        "google-calendar",
        json!({
            "requirement": "google_oauth",
            "invocation_id": InvocationId::new().to_string(),
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
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(
            shared.clone(),
            Arc::new(RecordingAuthDispatcher::default()),
        )
        .with_auth_engine(google_test_engine()),
    );
    let app = build_app_with_product_auth_service_config_and_extensions(
        product_auth,
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
                ProviderScope::new(GOOGLE_GMAIL_READONLY_SCOPE.to_string()).expect("scope"),
                ProviderScope::new(GOOGLE_CALENDAR_READONLY_SCOPE.to_string()).expect("scope"),
            ],
        })
        .await
        .expect("seed credential account");

    let response = post_extension_oauth_start(
        &app,
        "google-calendar",
        json!({
            "requirement": "google_oauth",
            "invocation_id": invocation_id.to_string(),
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
async fn product_auth_google_oauth_callback_ignores_incomplete_redirect_scope() {
    let provider_client = Arc::new(RecordingProviderClient::default());
    let (app, dispatcher) = build_app_with_google_oauth_provider(provider_client.clone());
    let (start_json, state) = start_google_oauth_flow(&app).await;

    let callback_response = app
        .oneshot(callback_request(format!(
            "/api/reborn/product-auth/oauth/google/callback?state={state}&code=google-auth-code&scope={GOOGLE_GMAIL_READONLY_SCOPE}"
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
        ]],
        "the provider exchange must receive the server-owned request scopes, \
         not the redirect's non-authoritative scope echo"
    );
}

#[tokio::test]
async fn product_auth_google_oauth_callback_missing_code_is_rejected_without_exchange() {
    let provider_client = Arc::new(RecordingProviderClient::default());
    let (app, dispatcher) = build_app_with_google_oauth_provider(provider_client.clone());
    let (start_json, state) = start_google_oauth_flow(&app).await;

    let response = app
        .clone()
        .oneshot(callback_request(format!(
            "/api/reborn/product-auth/oauth/google/callback?state={state}"
        )))
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = read_body_string(response).await;
    assert!(body.contains("\"code\":\"malformed_callback\""));
    assert!(!body.contains(&state));
    assert!(dispatcher.events().is_empty());
    assert!(
        provider_client.exchanged_scopes().is_empty(),
        "a callback without an authorization code must not reach token exchange"
    );

    let flow_id = start_json["flow_id"].as_str().expect("flow id");
    let invocation_id = start_json["callback_scope"]["invocation_id"]
        .as_str()
        .expect("invocation id");
    let status_response =
        get_oauth_flow_status(&app, flow_id, &format!("?invocation_id={invocation_id}")).await;
    assert_eq!(status_response.status(), StatusCode::OK);
    let status_body = read_body_string(status_response).await;
    let status_json: serde_json::Value = serde_json::from_str(&status_body).expect("status json");
    assert_eq!(status_json["status"], "awaiting_user");
    assert!(!status_body.contains(&state));
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
async fn product_auth_google_oauth_callback_does_not_trust_redirect_scopes_outside_recipe() {
    let provider_client = Arc::new(RecordingProviderClient::default());
    let (app, dispatcher) = build_app_with_google_oauth_provider(provider_client.clone());
    let (start_json, state) = start_google_oauth_flow(&app).await;

    let response = app
        .clone()
        .oneshot(callback_request(format!(
            "/api/reborn/product-auth/oauth/google/callback?state={state}&code=google-auth-code&scope={DISALLOWED_GOOGLE_SCOPE}"
        )))
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    let callback_json: serde_json::Value = serde_json::from_str(&body).expect("callback json");
    assert_eq!(callback_json["flow_id"], start_json["flow_id"]);
    assert_eq!(callback_json["status"], "completed");
    assert!(!body.contains(&state));
    assert!(!body.contains("google-auth-code"));
    assert!(!body.contains(DISALLOWED_GOOGLE_SCOPE));
    assert_eq!(dispatcher.events().len(), 1);
    assert_eq!(
        provider_client.exchanged_scopes(),
        vec![vec![
            GOOGLE_GMAIL_READONLY_SCOPE.to_string(),
            GOOGLE_CALENDAR_READONLY_SCOPE.to_string()
        ]],
        "a redirect scope outside the recipe must not become exchange authority"
    );

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
    let provider_client = Arc::new(RecordingProviderClient::default());
    let (app, dispatcher) = build_app_with_google_oauth_provider(provider_client.clone());
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
    assert!(
        provider_client.exchanged_scopes().is_empty(),
        "an explicit provider denial must not reach token exchange"
    );
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
async fn product_auth_google_oauth_callback_accepts_empty_redirect_scope() {
    let (app, dispatcher) = build_app_with_google_oauth();
    let (start_json, state) = start_google_oauth_flow(&app).await;

    let response = app
        .clone()
        .oneshot(callback_request(format!(
            "/api/reborn/product-auth/oauth/google/callback?state={state}&code=google-auth-code&scope="
        )))
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    let callback_json: serde_json::Value = serde_json::from_str(&body).expect("callback json");
    assert_eq!(callback_json["flow_id"], start_json["flow_id"]);
    assert_eq!(callback_json["status"], "completed");
    assert!(!body.contains(&state));
    assert!(!body.contains("google-auth-code"));
    assert_eq!(dispatcher.events().len(), 1);

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
