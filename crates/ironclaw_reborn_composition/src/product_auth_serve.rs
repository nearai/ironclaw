//! Reborn-native product-auth route composition.
//!
//! This module owns only HTTP parsing, scope derivation from host-owned
//! composition, one-way hashing of callback material, and sanitized response
//! rendering. It deliberately delegates durable flow state, provider exchange,
//! credential mutation, and continuation dispatch to [`RebornProductAuthServices`].

use std::{
    num::{NonZeroU32, NonZeroU64, NonZeroUsize},
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Extension, Path, RawQuery, State},
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthFlowStatus, AuthGateRef, AuthInteractionId,
    AuthProductError, AuthProductScope, AuthProviderId, AuthSessionId, AuthSurface,
    AuthorizationCodeHash, CredentialAccountId, CredentialAccountLabel, CredentialAccountStatus,
    OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthProviderCallbackRequest, OpaqueStateHash,
    PkceVerifierHash, PkceVerifierSecret, ProviderScope, Timestamp, TurnRunRef,
};
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor, ListenerClass,
    RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use lru::LruCache;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize};
use url::Url;
use uuid::Uuid;

use crate::auth::RebornOAuthStartFlowRequest;
use crate::{
    RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest, RebornOAuthCallbackError,
    RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest, RebornOAuthCallbackResponse,
    RebornProductAuthServices,
};

pub(crate) const OAUTH_START_PATH: &str = "/api/reborn/product-auth/oauth/start";
pub(crate) const OAUTH_CALLBACK_PATH: &str = "/api/reborn/product-auth/oauth/callback/{flow_id}";
pub(crate) const MANUAL_TOKEN_SUBMIT_PATH: &str = "/api/reborn/product-auth/manual-token/submit";

const OAUTH_START_ROUTE_ID: &str = "product_auth.oauth.start";
const OAUTH_CALLBACK_ROUTE_ID: &str = "product_auth.oauth.callback";
const MANUAL_TOKEN_SUBMIT_ROUTE_ID: &str = "product_auth.manual_token.submit";
const OAUTH_PKCE_VERIFIER_CACHE_CAPACITY: NonZeroUsize = match NonZeroUsize::new(1024) {
    Some(value) => value,
    // SAFETY: 1024 is a non-zero literal cache cap.
    None => unreachable!(),
};
const PRODUCT_AUTH_MUTATION_BODY_LIMIT_BYTES: NonZeroU64 = match NonZeroU64::new(16 * 1024) {
    Some(value) => value,
    // SAFETY: 16 KiB is a non-zero literal body cap.
    None => unreachable!(),
};
const PRODUCT_AUTH_MUTATION_MAX_REQUESTS: NonZeroU32 = match NonZeroU32::new(20) {
    Some(value) => value,
    // SAFETY: 20 is a non-zero literal rate limit.
    None => unreachable!(),
};
const OAUTH_CALLBACK_MAX_REQUESTS: NonZeroU32 = match NonZeroU32::new(120) {
    Some(value) => value,
    // SAFETY: 120 is a non-zero literal rate limit.
    None => unreachable!(),
};
const OAUTH_RATE_WINDOW_SECONDS: NonZeroU32 = match NonZeroU32::new(60) {
    Some(value) => value,
    // SAFETY: 60 is a non-zero literal rate-limit window.
    None => unreachable!(),
};
const PRODUCT_AUTH_FLOW_MAX_TTL_SECONDS: i64 = 10 * 60;
const PRODUCT_AUTH_BACKEND_TIMEOUT: Duration = Duration::from_secs(30);
const OAUTH_CALLBACK_QUERY_MAX_BYTES: usize = 16 * 1024;
const OAUTH_CALLBACK_FIELD_MAX_BYTES: usize = 512;
const OAUTH_CALLBACK_SCOPES_MAX_BYTES: usize = 4 * 1024;
const RAW_OAUTH_VALUE_MAX_BYTES: usize = 4 * 1024;

#[derive(Clone)]
pub(crate) struct ProductAuthRouteState {
    product_auth: Arc<RebornProductAuthServices>,
    tenant_id: TenantId,
    default_agent_id: Option<AgentId>,
    default_project_id: Option<ProjectId>,
    // First-slice WebUI OAuth stores the raw PKCE verifier process-locally
    // because `AuthFlowRecord` deliberately serializes hashes only. Production
    // HA must replace this with a host-owned encrypted verifier store before
    // routing callbacks across replicas or restarts.
    pkce_verifiers: Arc<Mutex<LruCache<AuthFlowId, StoredPkceVerifier>>>,
}

impl ProductAuthRouteState {
    pub(crate) fn new(
        product_auth: Arc<RebornProductAuthServices>,
        tenant_id: TenantId,
        default_agent_id: Option<AgentId>,
        default_project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            product_auth,
            tenant_id,
            default_agent_id,
            default_project_id,
            pkce_verifiers: Arc::new(Mutex::new(LruCache::new(
                OAUTH_PKCE_VERIFIER_CACHE_CAPACITY,
            ))),
        }
    }

    fn store_pkce_verifier(
        &self,
        flow_id: AuthFlowId,
        verifier: SecretString,
        expires_at: Timestamp,
    ) -> Result<(), ProductAuthRouteFailure> {
        let mut verifiers = self.lock_pkce_verifiers();
        remove_expired_pkce_verifiers(&mut verifiers);
        if verifiers.len() >= verifiers.cap().get() && !verifiers.contains(&flow_id) {
            return Err(ProductAuthRouteFailure::backend_unavailable());
        }
        verifiers.put(
            flow_id,
            StoredPkceVerifier {
                verifier,
                expires_at,
            },
        );
        Ok(())
    }

    fn ensure_pkce_verifier_capacity(&self) -> Result<(), ProductAuthRouteFailure> {
        let mut verifiers = self.lock_pkce_verifiers();
        remove_expired_pkce_verifiers(&mut verifiers);
        if verifiers.len() >= verifiers.cap().get() {
            return Err(ProductAuthRouteFailure::backend_unavailable());
        }
        Ok(())
    }

    fn pkce_verifier_for_callback(
        &self,
        flow_id: AuthFlowId,
    ) -> Result<SecretString, ProductAuthRouteFailure> {
        let mut verifiers = self.lock_pkce_verifiers();
        remove_expired_pkce_verifiers(&mut verifiers);
        verifiers
            .get(&flow_id)
            .map(|stored| stored.verifier.clone())
            .ok_or_else(ProductAuthRouteFailure::unknown_or_expired_flow)
    }

    fn remove_pkce_verifier(&self, flow_id: AuthFlowId) {
        self.lock_pkce_verifiers().pop(&flow_id);
    }

    fn lock_pkce_verifiers(
        &self,
    ) -> std::sync::MutexGuard<'_, LruCache<AuthFlowId, StoredPkceVerifier>> {
        self.pkce_verifiers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl std::fmt::Debug for ProductAuthRouteState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProductAuthRouteState")
            .field("product_auth", &"Arc<RebornProductAuthServices>")
            .field("tenant_id", &self.tenant_id)
            .field("default_agent_id", &self.default_agent_id)
            .field("default_project_id", &self.default_project_id)
            .field("pkce_verifiers", &"Arc<Mutex<LruCache<...>>>")
            .finish()
    }
}

struct StoredPkceVerifier {
    verifier: SecretString,
    expires_at: Timestamp,
}

fn remove_expired_pkce_verifiers(verifiers: &mut LruCache<AuthFlowId, StoredPkceVerifier>) {
    let now = Utc::now();
    let expired = verifiers
        .iter()
        .filter_map(|(flow_id, stored)| (stored.expires_at <= now).then_some(*flow_id))
        .collect::<Vec<_>>();
    for flow_id in expired {
        verifiers.pop(&flow_id);
    }
}

pub(crate) struct ProductAuthRouteMount {
    pub(crate) protected: Router,
    pub(crate) public: Router,
    pub(crate) descriptors: Vec<IngressRouteDescriptor>,
}

pub(crate) fn product_auth_route_mount(state: ProductAuthRouteState) -> ProductAuthRouteMount {
    ProductAuthRouteMount {
        protected: Router::new()
            .route(OAUTH_START_PATH, post(oauth_start_handler))
            .route(MANUAL_TOKEN_SUBMIT_PATH, post(manual_token_submit_handler))
            .with_state(state.clone()),
        public: Router::new()
            .route(OAUTH_CALLBACK_PATH, get(oauth_callback_handler))
            .with_state(state),
        descriptors: product_auth_route_descriptors(),
    }
}

pub(crate) fn product_auth_route_descriptors() -> Vec<IngressRouteDescriptor> {
    vec![
        descriptor(
            OAUTH_START_ROUTE_ID,
            NetworkMethod::Post,
            OAUTH_START_PATH,
            protected_mutation_policy(),
        ),
        descriptor(
            OAUTH_CALLBACK_ROUTE_ID,
            NetworkMethod::Get,
            OAUTH_CALLBACK_PATH,
            callback_policy(),
        ),
        descriptor(
            MANUAL_TOKEN_SUBMIT_ROUTE_ID,
            NetworkMethod::Post,
            MANUAL_TOKEN_SUBMIT_PATH,
            protected_mutation_policy(),
        ),
    ]
}

fn descriptor(
    route_id: &str,
    method: NetworkMethod,
    pattern: &str,
    policy: IngressPolicy,
) -> IngressRouteDescriptor {
    IngressRouteDescriptor::new(route_id.to_string(), method, pattern.to_string(), policy)
        .expect("product-auth route descriptor must validate at startup") // safety: ids/patterns are crate-local literals, and policies are constructed by sibling helpers that validate their parts.
}

fn protected_mutation_policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::BearerToken],
        },
        scope_source: ironclaw_host_api::IngressScopeSource::AuthenticatedCaller,
        body_limit: BodyLimitPolicy::Limited {
            max_bytes: PRODUCT_AUTH_MUTATION_BODY_LIMIT_BYTES,
        },
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerCaller,
            max_requests: PRODUCT_AUTH_MUTATION_MAX_REQUESTS,
            window_seconds: OAUTH_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("product-auth OAuth start policy must validate") // safety: LocalGateway + bearer + AuthenticatedCaller is the same authenticated local product workflow shape used by WebUI mutations.
}

fn callback_policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::OAuthCallback,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::OAuthState],
        },
        scope_source: ironclaw_host_api::IngressScopeSource::HostResolved,
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerIp,
            max_requests: OAUTH_CALLBACK_MAX_REQUESTS,
            window_seconds: OAUTH_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::NotApplicable,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("product-auth OAuth callback policy must validate") // safety: OAuthCallback + OAuthState + HostResolved is the host callback shape; handler/service validation enforces state before product effects.
}

#[derive(Deserialize)]
struct OAuthStartRequest {
    provider: String,
    authorization_url: String,
    opaque_state: UnvalidatedRawCallbackValue,
    pkce_verifier: UnvalidatedRawSecretValue,
    expires_at: Timestamp,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
}

#[derive(Deserialize)]
struct ManualTokenSubmitRequest {
    provider: String,
    account_label: String,
    token: UnvalidatedRawSecretValue,
    run_id: String,
    gate_ref: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OAuthStartResponse {
    pub(crate) flow_id: AuthFlowId,
    pub(crate) status: AuthFlowStatus,
    pub(crate) provider: AuthProviderId,
    pub(crate) authorization_url: OAuthAuthorizationUrl,
    pub(crate) expires_at: Timestamp,
    pub(crate) continuation: AuthContinuationRef,
    pub(crate) callback_scope: OAuthCallbackScopeHint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ManualTokenSubmitResponse {
    pub(crate) credential_ref: CredentialAccountId,
    pub(crate) status: CredentialAccountStatus,
    pub(crate) continuation: AuthContinuationRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OAuthCallbackScopeHint {
    pub(crate) user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) project_id: Option<ProjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) thread_id: Option<ThreadId>,
    pub(crate) invocation_id: InvocationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) session_id: Option<AuthSessionId>,
}

#[derive(Deserialize)]
struct OAuthCallbackQuery {
    user_id: String,
    invocation_id: String,
    state: Option<RawCallbackValue>,
    provider: Option<String>,
    account_label: Option<String>,
    code: Option<RawSecretValue>,
    error: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default, alias = "scope")]
    scopes: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct ProductAuthRouteFailure {
    status: StatusCode,
    body: RebornOAuthCallbackError,
}

impl ProductAuthRouteFailure {
    fn new(status: StatusCode, code: AuthErrorCode) -> Self {
        Self {
            status,
            body: RebornOAuthCallbackError {
                code,
                retryable: matches!(code, AuthErrorCode::BackendUnavailable),
            },
        }
    }

    fn invalid_request() -> Self {
        Self::new(StatusCode::BAD_REQUEST, AuthErrorCode::InvalidRequest)
    }

    fn malformed_callback() -> Self {
        Self::new(StatusCode::BAD_REQUEST, AuthErrorCode::MalformedCallback)
    }

    fn unknown_or_expired_flow() -> Self {
        Self::new(StatusCode::NOT_FOUND, AuthErrorCode::UnknownOrExpiredFlow)
    }

    fn backend_unavailable() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            AuthErrorCode::BackendUnavailable,
        )
    }

    fn backend_timeout() -> Self {
        Self::new(
            StatusCode::GATEWAY_TIMEOUT,
            AuthErrorCode::BackendUnavailable,
        )
    }
}

impl IntoResponse for ProductAuthRouteFailure {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

impl From<AuthProductError> for ProductAuthRouteFailure {
    fn from(error: AuthProductError) -> Self {
        route_failure_from_callback_error(error.into())
    }
}

impl From<RebornOAuthCallbackError> for ProductAuthRouteFailure {
    fn from(error: RebornOAuthCallbackError) -> Self {
        route_failure_from_callback_error(error)
    }
}

fn route_failure_from_callback_error(error: RebornOAuthCallbackError) -> ProductAuthRouteFailure {
    let status = match error.code {
        AuthErrorCode::MalformedCallback | AuthErrorCode::InvalidRequest => StatusCode::BAD_REQUEST,
        AuthErrorCode::UnknownOrExpiredFlow => StatusCode::NOT_FOUND,
        AuthErrorCode::CrossScopeDenied => StatusCode::FORBIDDEN,
        AuthErrorCode::ProviderDenied | AuthErrorCode::Canceled => StatusCode::BAD_REQUEST,
        AuthErrorCode::FlowAlreadyTerminal => StatusCode::CONFLICT,
        AuthErrorCode::BackendUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        AuthErrorCode::TokenExchangeFailed | AuthErrorCode::RefreshFailed => {
            StatusCode::BAD_GATEWAY
        }
        AuthErrorCode::CredentialMissing | AuthErrorCode::AccountSelectionRequired => {
            StatusCode::CONFLICT
        }
    };
    ProductAuthRouteFailure {
        status,
        body: error,
    }
}

async fn oauth_start_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<OAuthStartRequest>,
) -> Result<Json<OAuthStartResponse>, ProductAuthRouteFailure> {
    let now = Utc::now();
    if request.expires_at <= now
        || request.expires_at > now + ChronoDuration::seconds(PRODUCT_AUTH_FLOW_MAX_TTL_SECONDS)
    {
        return Err(ProductAuthRouteFailure::invalid_request());
    }

    let scope = scope_from_authenticated_caller(&caller, &request)?;
    let provider = AuthProviderId::new(request.provider).map_err(|_| {
        ProductAuthRouteFailure::new(StatusCode::BAD_REQUEST, AuthErrorCode::InvalidRequest)
    })?;
    let authorization_endpoint = authorization_endpoint_url(&request.authorization_url)?;
    let opaque_state = request
        .opaque_state
        .into_validated()
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let pkce_verifier_value = request
        .pkce_verifier
        .into_validated()
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let opaque_state_hash = opaque_state_hash(opaque_state.as_str())?;
    let pkce_verifier_hash = pkce_verifier_hash(pkce_verifier_value.expose_secret())?;
    let pkce_verifier = pkce_verifier_value.clone_secret();
    state.ensure_pkce_verifier_capacity()?;

    let flow = state
        .product_auth
        .start_setup_oauth_flow(RebornOAuthStartFlowRequest {
            scope: scope.clone(),
            provider: provider.clone(),
            authorization_url: OAuthAuthorizationUrl::new(authorization_endpoint.to_string())
                .map_err(ProductAuthRouteFailure::from)?,
            opaque_state_hash,
            pkce_verifier_hash,
            expires_at: request.expires_at,
        })
        .await
        .map_err(ProductAuthRouteFailure::from)?;
    state.store_pkce_verifier(flow.id, pkce_verifier, flow.expires_at)?;
    let authorization_url = compose_authorization_url(authorization_endpoint, flow.id, &scope)?;

    Ok(Json(OAuthStartResponse {
        flow_id: flow.id,
        status: flow.status,
        provider,
        authorization_url,
        expires_at: flow.expires_at,
        continuation: flow.continuation,
        callback_scope: scope_hint(&scope),
    }))
}

async fn manual_token_submit_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<ManualTokenSubmitRequest>,
) -> Result<Json<ManualTokenSubmitResponse>, ProductAuthRouteFailure> {
    let scope = scope_from_authenticated_caller_parts(
        &caller,
        request.thread_id.as_ref(),
        request.session_id.as_ref(),
    )?;
    let provider = AuthProviderId::new(request.provider)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let label = CredentialAccountLabel::new(request.account_label)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let token = request
        .token
        .into_validated()
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    // This route only persists a scoped credential and returns its account id.
    // The browser must still call WebUI v2 resolve_gate, where the turn
    // coordinator verifies the caller, run id, and gate ref before resuming.
    let continuation = AuthContinuationRef::TurnGateResume {
        turn_run_ref: TurnRunRef::new(request.run_id)
            .map_err(|_| ProductAuthRouteFailure::invalid_request())?,
        gate_ref: AuthGateRef::new(request.gate_ref)
            .map_err(|_| ProductAuthRouteFailure::invalid_request())?,
    };
    let expires_at = Utc::now() + ChronoDuration::seconds(PRODUCT_AUTH_FLOW_MAX_TTL_SECONDS);

    let challenge = tokio::time::timeout(
        PRODUCT_AUTH_BACKEND_TIMEOUT,
        state
            .product_auth
            .request_manual_token_setup(RebornManualTokenSetupRequest::new(
                scope.clone(),
                provider,
                label,
                continuation,
                expires_at,
            )),
    )
    .await
    .map_err(|_| ProductAuthRouteFailure::backend_timeout())?
    .map_err(ProductAuthRouteFailure::from)?;
    let submitted = match tokio::time::timeout(
        PRODUCT_AUTH_BACKEND_TIMEOUT,
        state
            .product_auth
            .submit_manual_token(RebornManualTokenSubmitRequest::new(
                scope.clone(),
                challenge.interaction_id,
                token.into_secret(),
            )),
    )
    .await
    {
        Ok(Ok(submitted)) => submitted,
        Ok(Err(error)) => {
            abandon_manual_token_after_submit_failure(
                &state,
                &scope,
                challenge.interaction_id,
                error.code,
            )
            .await;
            return Err(ProductAuthRouteFailure::from(error));
        }
        Err(_) => {
            abandon_manual_token_after_submit_failure(
                &state,
                &scope,
                challenge.interaction_id,
                AuthErrorCode::BackendUnavailable,
            )
            .await;
            return Err(ProductAuthRouteFailure::backend_timeout());
        }
    };

    Ok(Json(ManualTokenSubmitResponse {
        credential_ref: submitted.account_id,
        status: submitted.status,
        continuation: submitted.continuation,
    }))
}

async fn abandon_manual_token_after_submit_failure(
    state: &ProductAuthRouteState,
    scope: &AuthProductScope,
    interaction_id: AuthInteractionId,
    submit_error_code: AuthErrorCode,
) {
    match tokio::time::timeout(
        PRODUCT_AUTH_BACKEND_TIMEOUT,
        state
            .product_auth
            .abandon_manual_token(scope, interaction_id),
    )
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(cleanup_error)) => {
            tracing::debug!(
                error_code = ?submit_error_code,
                cleanup_error_code = ?cleanup_error.code,
                "manual-token submit failed and interaction cleanup failed"
            );
        }
        Err(_) => {
            tracing::debug!(
                error_code = ?submit_error_code,
                cleanup_error_code = ?AuthErrorCode::BackendUnavailable,
                "manual-token submit failed and interaction cleanup timed out"
            );
        }
    }
}

async fn oauth_callback_handler(
    State(state): State<ProductAuthRouteState>,
    Path(flow_id): Path<String>,
    RawQuery(raw_query): RawQuery,
    uri: Uri,
) -> Result<Json<RebornOAuthCallbackResponse>, ProductAuthRouteFailure> {
    validate_callback_raw_query(raw_query.as_deref())?;
    let query = axum::extract::Query::<OAuthCallbackQuery>::try_from_uri(&uri)
        .map_err(|_| ProductAuthRouteFailure::malformed_callback())?
        .0;
    validate_callback_query_fields(&query)?;

    let flow_id = AuthFlowId::from_uuid(
        Uuid::parse_str(&flow_id).map_err(|_| ProductAuthRouteFailure::malformed_callback())?,
    );
    let scope = scope_from_callback_query(&state, &query)?;
    let state_hash = opaque_state_hash(
        query
            .state
            .as_ref()
            .ok_or_else(ProductAuthRouteFailure::malformed_callback)?
            .as_str(),
    )?;

    if is_authorized_callback_candidate(&query) {
        state
            .product_auth
            .ensure_oauth_callback_flow_known(&scope, flow_id)
            .await
            .map_err(ProductAuthRouteFailure::from)?;
    }
    let outcome = callback_outcome_from_query(&state, flow_id, &scope, &query)?;

    let response = match state
        .product_auth
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope,
            flow_id,
            opaque_state_hash: state_hash,
            outcome,
        })
        .await
    {
        Ok(response) => {
            state.remove_pkce_verifier(flow_id);
            response
        }
        Err(error) => {
            if should_forget_pkce_verifier(error.code) {
                state.remove_pkce_verifier(flow_id);
            }
            return Err(ProductAuthRouteFailure::from(error));
        }
    };

    Ok(Json(response))
}

fn callback_outcome_from_query(
    state: &ProductAuthRouteState,
    flow_id: AuthFlowId,
    _scope: &AuthProductScope,
    query: &OAuthCallbackQuery,
) -> Result<RebornOAuthCallbackOutcome, ProductAuthRouteFailure> {
    if query
        .error
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return Ok(RebornOAuthCallbackOutcome::ProviderDenied);
    }

    let provider = required_callback_value(query.provider.as_deref())?;
    let account_label = required_callback_value(query.account_label.as_deref())?;
    let code = query
        .code
        .as_ref()
        .ok_or_else(ProductAuthRouteFailure::malformed_callback)?;
    let pkce_verifier = state.pkce_verifier_for_callback(flow_id)?;
    let scopes = parse_provider_scopes(query.scopes.as_deref())?;
    if scopes.is_empty() {
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    let authorization_code_hash = authorization_code_hash(code.expose_secret())?;
    let pkce_verifier_hash = pkce_verifier_hash(pkce_verifier.expose_secret())?;

    Ok(RebornOAuthCallbackOutcome::Authorized {
        provider_request: OAuthProviderCallbackRequest {
            provider: AuthProviderId::new(provider.to_string())
                .map_err(|_| ProductAuthRouteFailure::malformed_callback())?,
            account_label: CredentialAccountLabel::new(account_label.to_string())
                .map_err(|_| ProductAuthRouteFailure::malformed_callback())?,
            authorization_code: OAuthAuthorizationCode::new(code.clone_secret())
                .map_err(ProductAuthRouteFailure::from)?,
            authorization_code_hash,
            pkce_verifier: PkceVerifierSecret::new(pkce_verifier)
                .map_err(ProductAuthRouteFailure::from)?,
            pkce_verifier_hash,
            scopes,
        },
    })
}

fn is_authorized_callback_candidate(query: &OAuthCallbackQuery) -> bool {
    query.error.as_deref().is_none_or(|value| value.is_empty())
        && query.provider.is_some()
        && query.account_label.is_some()
        && query.code.is_some()
}

fn required_callback_value(value: Option<&str>) -> Result<&str, ProductAuthRouteFailure> {
    value.ok_or_else(ProductAuthRouteFailure::malformed_callback)
}

fn should_forget_pkce_verifier(code: AuthErrorCode) -> bool {
    matches!(
        code,
        AuthErrorCode::ProviderDenied
            | AuthErrorCode::Canceled
            | AuthErrorCode::FlowAlreadyTerminal
            | AuthErrorCode::TokenExchangeFailed
            | AuthErrorCode::RefreshFailed
            | AuthErrorCode::CredentialMissing
            | AuthErrorCode::AccountSelectionRequired
    )
}

fn scope_from_authenticated_caller(
    caller: &WebUiAuthenticatedCaller,
    request: &OAuthStartRequest,
) -> Result<AuthProductScope, ProductAuthRouteFailure> {
    scope_from_authenticated_caller_parts(
        caller,
        request.thread_id.as_ref(),
        request.session_id.as_ref(),
    )
}

fn scope_from_authenticated_caller_parts(
    caller: &WebUiAuthenticatedCaller,
    thread_id: Option<&String>,
    session_id: Option<&String>,
) -> Result<AuthProductScope, ProductAuthRouteFailure> {
    let thread_id = thread_id
        .map(|value| {
            ThreadId::new(value.clone()).map_err(|_| ProductAuthRouteFailure::invalid_request())
        })
        .transpose()?;
    let session_id = session_id
        .map(|value| {
            AuthSessionId::new(value.clone())
                .map_err(|_| ProductAuthRouteFailure::invalid_request())
        })
        .transpose()?;

    let mut scope = AuthProductScope::new(
        ResourceScope {
            tenant_id: caller.tenant_id.clone(),
            user_id: caller.user_id.clone(),
            agent_id: caller.agent_id.clone(),
            project_id: caller.project_id.clone(),
            mission_id: None,
            thread_id,
            invocation_id: InvocationId::new(),
        },
        AuthSurface::Callback,
    );
    if let Some(session_id) = session_id {
        scope = scope.with_session_id(session_id);
    }
    Ok(scope)
}

fn scope_from_callback_query(
    state: &ProductAuthRouteState,
    query: &OAuthCallbackQuery,
) -> Result<AuthProductScope, ProductAuthRouteFailure> {
    let user_id = UserId::new(query.user_id.clone())
        .map_err(|_| ProductAuthRouteFailure::malformed_callback())?;
    let invocation_id = InvocationId::parse(&query.invocation_id)
        .map_err(|_| ProductAuthRouteFailure::malformed_callback())?;
    let agent_id = query
        .agent_id
        .as_ref()
        .map(|value| {
            AgentId::new(value.clone()).map_err(|_| ProductAuthRouteFailure::malformed_callback())
        })
        .transpose()?
        .or_else(|| state.default_agent_id.clone());
    let project_id = query
        .project_id
        .as_ref()
        .map(|value| {
            ProjectId::new(value.clone()).map_err(|_| ProductAuthRouteFailure::malformed_callback())
        })
        .transpose()?
        .or_else(|| state.default_project_id.clone());
    let thread_id = query
        .thread_id
        .as_ref()
        .map(|value| {
            ThreadId::new(value.clone()).map_err(|_| ProductAuthRouteFailure::malformed_callback())
        })
        .transpose()?;
    let session_id = query
        .session_id
        .as_ref()
        .map(|value| {
            AuthSessionId::new(value.clone())
                .map_err(|_| ProductAuthRouteFailure::malformed_callback())
        })
        .transpose()?;

    let mut scope = AuthProductScope::new(
        ResourceScope {
            tenant_id: state.tenant_id.clone(),
            user_id,
            agent_id,
            project_id,
            mission_id: None,
            thread_id,
            invocation_id,
        },
        AuthSurface::Callback,
    );
    if let Some(session_id) = session_id {
        scope = scope.with_session_id(session_id);
    }
    Ok(scope)
}

fn validate_callback_raw_query(raw_query: Option<&str>) -> Result<(), ProductAuthRouteFailure> {
    let Some(raw_query) = raw_query else {
        return Err(ProductAuthRouteFailure::malformed_callback());
    };
    if raw_query.len() > OAUTH_CALLBACK_QUERY_MAX_BYTES {
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    Ok(())
}

fn validate_callback_query_fields(
    query: &OAuthCallbackQuery,
) -> Result<(), ProductAuthRouteFailure> {
    validate_callback_field(&query.user_id, OAUTH_CALLBACK_FIELD_MAX_BYTES, false)?;
    validate_callback_field(&query.invocation_id, OAUTH_CALLBACK_FIELD_MAX_BYTES, false)?;
    validate_optional_callback_field(
        query.provider.as_deref(),
        OAUTH_CALLBACK_FIELD_MAX_BYTES,
        false,
    )?;
    validate_optional_callback_field(
        query.account_label.as_deref(),
        OAUTH_CALLBACK_FIELD_MAX_BYTES,
        false,
    )?;
    validate_optional_callback_field(
        query.error.as_deref(),
        OAUTH_CALLBACK_FIELD_MAX_BYTES,
        false,
    )?;
    validate_optional_callback_field(
        query.agent_id.as_deref(),
        OAUTH_CALLBACK_FIELD_MAX_BYTES,
        false,
    )?;
    validate_optional_callback_field(
        query.project_id.as_deref(),
        OAUTH_CALLBACK_FIELD_MAX_BYTES,
        false,
    )?;
    validate_optional_callback_field(
        query.thread_id.as_deref(),
        OAUTH_CALLBACK_FIELD_MAX_BYTES,
        false,
    )?;
    validate_optional_callback_field(
        query.session_id.as_deref(),
        OAUTH_CALLBACK_FIELD_MAX_BYTES,
        false,
    )?;
    validate_optional_callback_field(
        query.scopes.as_deref(),
        OAUTH_CALLBACK_SCOPES_MAX_BYTES,
        true,
    )?;
    Ok(())
}

fn validate_optional_callback_field(
    value: Option<&str>,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<(), ProductAuthRouteFailure> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_callback_field(value, max_bytes, allow_empty)
}

fn validate_callback_field(
    value: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<(), ProductAuthRouteFailure> {
    if value.is_empty() && allow_empty {
        return Ok(());
    }
    validate_raw_value_with_limit(value, max_bytes)
        .map_err(|_| ProductAuthRouteFailure::malformed_callback())
}

fn scope_hint(scope: &AuthProductScope) -> OAuthCallbackScopeHint {
    OAuthCallbackScopeHint {
        user_id: scope.resource.user_id.clone(),
        agent_id: scope.resource.agent_id.clone(),
        project_id: scope.resource.project_id.clone(),
        thread_id: scope.resource.thread_id.clone(),
        invocation_id: scope.resource.invocation_id,
        session_id: scope.session_id.clone(),
    }
}

fn authorization_endpoint_url(raw: &str) -> Result<Url, ProductAuthRouteFailure> {
    let authorization_url =
        OAuthAuthorizationUrl::new(raw.to_string()).map_err(ProductAuthRouteFailure::from)?;
    let parsed = Url::parse(authorization_url.as_str())
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(ProductAuthRouteFailure::invalid_request());
    }
    Ok(parsed)
}

fn compose_authorization_url(
    mut endpoint: Url,
    flow_id: AuthFlowId,
    scope: &AuthProductScope,
) -> Result<OAuthAuthorizationUrl, ProductAuthRouteFailure> {
    let flow_id = flow_id.to_string();
    let invocation_id = scope.resource.invocation_id.to_string();
    {
        let mut query = endpoint.query_pairs_mut();
        query.append_pair("reborn_flow_id", &flow_id);
        query.append_pair("reborn_user_id", scope.resource.user_id.as_str());
        query.append_pair("reborn_invocation_id", &invocation_id);
        if let Some(agent_id) = &scope.resource.agent_id {
            query.append_pair("reborn_agent_id", agent_id.as_str());
        }
        if let Some(project_id) = &scope.resource.project_id {
            query.append_pair("reborn_project_id", project_id.as_str());
        }
        if let Some(thread_id) = &scope.resource.thread_id {
            query.append_pair("reborn_thread_id", thread_id.as_str());
        }
        if let Some(session_id) = &scope.session_id {
            query.append_pair("reborn_session_id", session_id.as_str());
        }
    }
    OAuthAuthorizationUrl::new(endpoint.to_string()).map_err(ProductAuthRouteFailure::from)
}

fn opaque_state_hash(value: &str) -> Result<OpaqueStateHash, ProductAuthRouteFailure> {
    OpaqueStateHash::new(sha256_hex(value)).map_err(ProductAuthRouteFailure::from)
}

fn pkce_verifier_hash(value: &str) -> Result<PkceVerifierHash, ProductAuthRouteFailure> {
    PkceVerifierHash::new(sha256_hex(value)).map_err(ProductAuthRouteFailure::from)
}

fn authorization_code_hash(value: &str) -> Result<AuthorizationCodeHash, ProductAuthRouteFailure> {
    AuthorizationCodeHash::new(sha256_hex(value)).map_err(ProductAuthRouteFailure::from)
}

fn sha256_hex(value: &str) -> String {
    ironclaw_common::hashing::sha256_hex(value.as_bytes())
}

fn parse_provider_scopes(raw: Option<&str>) -> Result<Vec<ProviderScope>, ProductAuthRouteFailure> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    if raw.trim() != raw {
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    raw.split(',')
        .map(|scope| {
            if scope.is_empty() {
                return Err(ProductAuthRouteFailure::malformed_callback());
            }
            ProviderScope::new(scope.to_string())
                .map_err(|_| ProductAuthRouteFailure::malformed_callback())
        })
        .collect()
}

#[derive(Clone)]
struct UnvalidatedRawCallbackValue(String);

impl UnvalidatedRawCallbackValue {
    fn into_validated(self) -> Result<RawCallbackValue, &'static str> {
        RawCallbackValue::new(self.0)
    }
}

impl<'de> Deserialize<'de> for UnvalidatedRawCallbackValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self)
    }
}

#[derive(Clone)]
struct UnvalidatedRawSecretValue(SecretString);

impl UnvalidatedRawSecretValue {
    fn into_validated(self) -> Result<RawSecretValue, &'static str> {
        RawSecretValue::new(self.0.expose_secret().to_string())
    }
}

impl<'de> Deserialize<'de> for UnvalidatedRawSecretValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self(SecretString::from(value)))
    }
}

#[derive(Clone)]
struct RawCallbackValue(String);

impl RawCallbackValue {
    fn new(value: String) -> Result<Self, &'static str> {
        validate_raw_value_with_limit(&value, RAW_OAUTH_VALUE_MAX_BYTES)?;
        Ok(Self(value))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RawCallbackValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone)]
struct RawSecretValue(SecretString);

impl RawSecretValue {
    fn new(value: String) -> Result<Self, &'static str> {
        validate_raw_value_with_limit(&value, RAW_OAUTH_VALUE_MAX_BYTES)?;
        Ok(Self(SecretString::from(value)))
    }

    fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }

    fn into_secret(self) -> SecretString {
        self.0
    }

    fn clone_secret(&self) -> SecretString {
        SecretString::from(self.0.expose_secret().to_string())
    }
}

impl<'de> Deserialize<'de> for RawSecretValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

fn validate_raw_value_with_limit(value: &str, max_bytes: usize) -> Result<(), &'static str> {
    if value.is_empty() {
        return Err("value must not be empty");
    }
    if value.len() > max_bytes {
        return Err("value is too long");
    }
    if value.trim() != value {
        return Err("value must not contain leading or trailing whitespace");
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err("value must not contain NUL/control characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RebornAuthContinuationDispatcher;
    use async_trait::async_trait;

    struct NoopDispatcher;

    #[async_trait]
    impl RebornAuthContinuationDispatcher for NoopDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            _event: ironclaw_auth::AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            Ok(())
        }
    }

    fn test_state() -> ProductAuthRouteState {
        ProductAuthRouteState::new(
            Arc::new(RebornProductAuthServices::local_dev_in_memory(Arc::new(
                NoopDispatcher,
            ))),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
    }

    #[test]
    fn sha256_hex_produces_plain_lowercase_hex_without_prefix() {
        // Regression guard for the switch off `sha256_digest_token` (which
        // returned a "sha256:"-prefixed token): the route hashes must stay
        // plain lowercase hex so stored state/verifier/code hashes match.
        let hashed = sha256_hex("abc");
        assert_eq!(
            hashed,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(!hashed.starts_with("sha256:"));
    }

    #[test]
    fn pkce_cache_rejects_new_entries_when_full() {
        let state = test_state();
        let expires_at = Utc::now() + ChronoDuration::minutes(5);
        for index in 0..OAUTH_PKCE_VERIFIER_CACHE_CAPACITY.get() {
            state
                .store_pkce_verifier(
                    AuthFlowId::new(),
                    SecretString::from(format!("pkce-{index}")),
                    expires_at,
                )
                .expect("cache entry");
        }

        let error = state
            .store_pkce_verifier(
                AuthFlowId::new(),
                SecretString::from("pkce-overflow".to_string()),
                expires_at,
            )
            .expect_err("full cache must reject without LRU eviction");

        assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.body.code, AuthErrorCode::BackendUnavailable);
    }
}
