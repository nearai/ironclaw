//! Reborn-native product-auth route composition.
//!
//! This module owns only HTTP parsing, scope derivation from host-owned
//! composition, one-way hashing of callback material, and sanitized response
//! rendering. It deliberately delegates durable flow state, provider exchange,
//! credential mutation, and continuation dispatch to [`RebornProductAuthServices`].

mod accounts;
mod lifecycle;
mod manual_token;
mod oauth;

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
    AuthorizationCodeHash, CredentialAccountChoiceRequest, CredentialAccountId,
    CredentialAccountLabel, CredentialAccountListPage, CredentialAccountListRequest,
    CredentialAccountProjection, CredentialAccountStatus, CredentialRecoveryProjection,
    CredentialRecoveryRequest, CredentialRefreshReport, CredentialRefreshRequest,
    OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthProviderCallbackRequest, OpaqueStateHash,
    PkceVerifierHash, PkceVerifierSecret, ProviderScope, SecretCleanupAction, SecretCleanupReport,
    SecretCleanupRequest, Timestamp, TurnRunRef,
};
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor, ListenerClass,
    RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_host_api::{
    AgentId, ExtensionId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use lru::LruCache;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize};
use url::Url;
use uuid::Uuid;

use crate::auth::RebornOAuthStartFlowRequest;
use crate::{
    RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest, RebornManualTokenSubmitResponse,
    RebornOAuthCallbackError, RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest,
    RebornOAuthCallbackResponse, RebornProductAuthServices,
};

pub(crate) const OAUTH_START_PATH: &str = "/api/reborn/product-auth/oauth/start";
pub(crate) const OAUTH_CALLBACK_PATH: &str = "/api/reborn/product-auth/oauth/callback/{flow_id}";
pub(crate) const MANUAL_TOKEN_SUBMIT_PATH: &str = "/api/reborn/product-auth/manual-token/submit";
pub(crate) const MANUAL_TOKEN_SETUP_PATH: &str = "/api/reborn/product-auth/manual-token/setup";
pub(crate) const MANUAL_TOKEN_SECRET_SUBMIT_PATH: &str =
    "/api/reborn/product-auth/manual-token/secret-submit";
pub(crate) const ACCOUNTS_LIST_PATH: &str = "/api/reborn/product-auth/accounts/list";
pub(crate) const ACCOUNTS_SELECT_PATH: &str = "/api/reborn/product-auth/accounts/select";
pub(crate) const ACCOUNTS_RECOVERY_PATH: &str = "/api/reborn/product-auth/accounts/recovery";
pub(crate) const ACCOUNTS_REFRESH_PATH: &str = "/api/reborn/product-auth/accounts/refresh";
pub(crate) const LIFECYCLE_CLEANUP_PATH: &str = "/api/reborn/product-auth/lifecycle/cleanup";

const OAUTH_START_ROUTE_ID: &str = "product_auth.oauth.start";
const OAUTH_CALLBACK_ROUTE_ID: &str = "product_auth.oauth.callback";
const MANUAL_TOKEN_SUBMIT_ROUTE_ID: &str = "product_auth.manual_token.submit";
const MANUAL_TOKEN_SETUP_ROUTE_ID: &str = "product_auth.manual_token.setup";
const MANUAL_TOKEN_SECRET_SUBMIT_ROUTE_ID: &str = "product_auth.manual_token.secret_submit";
const ACCOUNTS_LIST_ROUTE_ID: &str = "product_auth.accounts.list";
const ACCOUNTS_SELECT_ROUTE_ID: &str = "product_auth.accounts.select";
const ACCOUNTS_RECOVERY_ROUTE_ID: &str = "product_auth.accounts.recovery";
const ACCOUNTS_REFRESH_ROUTE_ID: &str = "product_auth.accounts.refresh";
const LIFECYCLE_CLEANUP_ROUTE_ID: &str = "product_auth.lifecycle.cleanup";
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
// accounts/refresh triggers an external provider token-refresh call per request.
// Use a tighter rate limit so one caller cannot fan out 20 provider calls per minute.
const ACCOUNTS_REFRESH_MAX_REQUESTS: NonZeroU32 = match NonZeroU32::new(5) {
    Some(value) => value,
    // SAFETY: 5 is a non-zero literal rate limit.
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

pub(super) struct StoredPkceVerifier {
    verifier: SecretString,
    expires_at: Timestamp,
}

pub(super) fn remove_expired_pkce_verifiers(
    verifiers: &mut LruCache<AuthFlowId, StoredPkceVerifier>,
) {
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

// Product-auth HTTP is a host-owned auth/secret-ingress boundary. Its
// mutations enter `RebornProductAuthServices` directly; they are not in-turn
// tool calls and must not surface raw secrets through the model-visible
// tool-dispatch path. Contract: `docs/reborn/contracts/auth-product.md`.
// dispatch-exempt: host-owned auth/secret ingress, not in-turn tool dispatch
pub(crate) fn product_auth_route_mount(state: ProductAuthRouteState) -> ProductAuthRouteMount {
    ProductAuthRouteMount {
        protected: Router::new()
            .route(OAUTH_START_PATH, post(oauth::oauth_start_handler))
            .route(
                MANUAL_TOKEN_SUBMIT_PATH,
                post(manual_token::manual_token_submit_handler),
            )
            .route(
                MANUAL_TOKEN_SETUP_PATH,
                post(manual_token::manual_token_setup_handler),
            )
            .route(
                MANUAL_TOKEN_SECRET_SUBMIT_PATH,
                post(manual_token::manual_token_secret_submit_handler),
            )
            .route(ACCOUNTS_LIST_PATH, post(accounts::accounts_list_handler))
            .route(
                ACCOUNTS_SELECT_PATH,
                post(accounts::accounts_select_handler),
            )
            .route(
                ACCOUNTS_RECOVERY_PATH,
                post(accounts::accounts_recovery_handler),
            )
            .route(
                ACCOUNTS_REFRESH_PATH,
                post(accounts::accounts_refresh_handler),
            )
            .route(
                LIFECYCLE_CLEANUP_PATH,
                post(lifecycle::lifecycle_cleanup_handler),
            )
            .with_state(state.clone()),
        public: Router::new()
            .route(OAUTH_CALLBACK_PATH, get(oauth::oauth_callback_handler))
            .with_state(state),
        descriptors: product_auth_route_descriptors(),
    }
}

pub(crate) fn product_auth_route_descriptors() -> Vec<IngressRouteDescriptor> {
    // All protected mutations share the same LocalGateway + Bearer + per-caller
    // policy. Listing them as a table keeps the policy choice next to the path
    // and stops descriptor blocks from drifting per-route.
    const PROTECTED_MUTATIONS: &[(&str, &str)] = &[
        (OAUTH_START_ROUTE_ID, OAUTH_START_PATH),
        (MANUAL_TOKEN_SUBMIT_ROUTE_ID, MANUAL_TOKEN_SUBMIT_PATH),
        (MANUAL_TOKEN_SETUP_ROUTE_ID, MANUAL_TOKEN_SETUP_PATH),
        (
            MANUAL_TOKEN_SECRET_SUBMIT_ROUTE_ID,
            MANUAL_TOKEN_SECRET_SUBMIT_PATH,
        ),
        (ACCOUNTS_LIST_ROUTE_ID, ACCOUNTS_LIST_PATH),
        (ACCOUNTS_SELECT_ROUTE_ID, ACCOUNTS_SELECT_PATH),
        (ACCOUNTS_RECOVERY_ROUTE_ID, ACCOUNTS_RECOVERY_PATH),
        // accounts/refresh omitted here — uses tighter rate limit below.
        (LIFECYCLE_CLEANUP_ROUTE_ID, LIFECYCLE_CLEANUP_PATH),
    ];
    let mut descriptors: Vec<IngressRouteDescriptor> = PROTECTED_MUTATIONS
        .iter()
        .map(|(route_id, path)| {
            descriptor(
                route_id,
                NetworkMethod::Post,
                path,
                protected_mutation_policy(),
            )
        })
        .collect();
    // accounts/refresh triggers a provider-side token-refresh call per request;
    // give it a tighter per-caller rate limit than stateless mutation routes.
    descriptors.push(descriptor(
        ACCOUNTS_REFRESH_ROUTE_ID,
        NetworkMethod::Post,
        ACCOUNTS_REFRESH_PATH,
        accounts_refresh_policy(),
    ));
    descriptors.push(descriptor(
        OAUTH_CALLBACK_ROUTE_ID,
        NetworkMethod::Get,
        OAUTH_CALLBACK_PATH,
        callback_policy(),
    ));
    descriptors
}

pub(super) fn descriptor(
    route_id: &str,
    method: NetworkMethod,
    pattern: &str,
    policy: IngressPolicy,
) -> IngressRouteDescriptor {
    IngressRouteDescriptor::new(route_id.to_string(), method, pattern.to_string(), policy)
        .expect("product-auth route descriptor must validate at startup") // safety: ids/patterns are crate-local literals, and policies are constructed by sibling helpers that validate their parts.
}

pub(super) fn protected_mutation_policy() -> IngressPolicy {
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

pub(super) fn accounts_refresh_policy() -> IngressPolicy {
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
            max_requests: ACCOUNTS_REFRESH_MAX_REQUESTS,
            window_seconds: OAUTH_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("product-auth accounts refresh policy must validate") // safety: same shape as protected_mutation_policy but with tighter rate cap to guard against fan-out to provider refresh calls.
}

pub(super) fn callback_policy() -> IngressPolicy {
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
pub(super) struct OAuthStartRequest {
    provider: String,
    authorization_url: String,
    opaque_state: UnvalidatedRawCallbackValue,
    pkce_verifier: UnvalidatedRawSecretValue,
    expires_at: Timestamp,
    session_id: Option<String>,
    thread_id: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ManualTokenSubmitRequest {
    provider: String,
    account_label: String,
    token: UnvalidatedRawSecretValue,
    run_id: String,
    gate_ref: String,
    session_id: Option<String>,
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

/// Caller-supplied scope fields shared by every product-auth route body.
///
/// `invocation_id` is round-tripped from a prior start/setup response so the
/// host can re-derive the same `AuthProductScope` across follow-up calls
/// (mirroring the OAuth start/callback pattern). All three fields are
/// optional: routes default to a fresh scope when the browser has no prior
/// invocation to carry forward.
// Option<T> fields already default to None in serde without #[serde(default)].
#[derive(Default, Deserialize)]
pub(super) struct ScopeFields {
    session_id: Option<String>,
    thread_id: Option<String>,
    invocation_id: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ManualTokenSetupRequest {
    provider: String,
    account_label: String,
    run_id: Option<String>,
    gate_ref: Option<String>,
    #[serde(flatten)]
    scope: ScopeFields,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ManualTokenSetupResponse {
    pub(crate) interaction_id: AuthInteractionId,
    pub(crate) provider: AuthProviderId,
    pub(crate) label: CredentialAccountLabel,
    pub(crate) expires_at: Timestamp,
    /// Invocation scope used to mint this interaction. The browser carries it
    /// back on the secret-submit call so the host can re-derive the same
    /// `AuthProductScope` and let the interaction service match the pending
    /// scope without trusting browser-supplied scope identifiers.
    pub(crate) invocation_id: InvocationId,
}

#[derive(Deserialize)]
pub(super) struct ManualTokenSecretSubmitRequest {
    interaction_id: String,
    token: UnvalidatedRawSecretValue,
    #[serde(flatten)]
    scope: ScopeFields,
}

#[derive(Deserialize)]
pub(super) struct AccountsListRequest {
    provider: String,
    requester_extension: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
    #[serde(flatten)]
    scope: ScopeFields,
}

#[derive(Deserialize)]
pub(super) struct AccountsSelectRequest {
    provider: String,
    account_id: String,
    requester_extension: Option<String>,
    #[serde(flatten)]
    scope: ScopeFields,
}

#[derive(Deserialize)]
pub(super) struct AccountsRecoveryRequest {
    provider: String,
    requester_extension: Option<String>,
    #[serde(flatten)]
    scope: ScopeFields,
}

#[derive(Deserialize)]
pub(super) struct AccountsRefreshRequest {
    provider: String,
    account_id: String,
    requester_extension: Option<String>,
    #[serde(flatten)]
    scope: ScopeFields,
}

#[derive(Deserialize)]
pub(super) struct LifecycleCleanupRequest {
    extension_id: String,
    action: SecretCleanupAction,
    #[serde(flatten)]
    scope: ScopeFields,
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
pub(super) struct OAuthCallbackQuery {
    user_id: String,
    invocation_id: String,
    state: Option<RawCallbackValue>,
    provider: Option<String>,
    account_label: Option<String>,
    code: Option<RawSecretValue>,
    error: Option<String>,
    agent_id: Option<String>,
    project_id: Option<String>,
    thread_id: Option<String>,
    session_id: Option<String>,
    #[serde(alias = "scope")]
    scopes: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ProductAuthRouteFailure {
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

pub(super) fn route_failure_from_callback_error(
    error: RebornOAuthCallbackError,
) -> ProductAuthRouteFailure {
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

pub(super) fn scope_from_authenticated_caller(
    caller: &WebUiAuthenticatedCaller,
    request: &OAuthStartRequest,
) -> Result<AuthProductScope, ProductAuthRouteFailure> {
    scope_from_authenticated_caller_parts(
        caller,
        &ScopeFields {
            session_id: request.session_id.clone(),
            thread_id: request.thread_id.clone(),
            invocation_id: None,
        },
    )
}

/// Derive an `AuthProductScope` from the authenticated caller plus the
/// caller-supplied scope fields shared by every product-auth route body.
///
/// `invocation_id`, when supplied, must parse as an existing identifier
/// (round-tripped from a prior start/setup response). Otherwise we mint a
/// fresh one — mirroring the OAuth start/callback pattern from #4031 so the
/// host owns the canonical id and the browser carries it forward across
/// follow-up calls.
pub(super) fn scope_from_authenticated_caller_parts(
    caller: &WebUiAuthenticatedCaller,
    fields: &ScopeFields,
) -> Result<AuthProductScope, ProductAuthRouteFailure> {
    let thread_id = fields
        .thread_id
        .as_deref()
        .map(|value| {
            ThreadId::new(value.to_string()).map_err(|_| ProductAuthRouteFailure::invalid_request())
        })
        .transpose()?;
    let session_id = fields
        .session_id
        .as_deref()
        .map(|value| {
            AuthSessionId::new(value.to_string())
                .map_err(|_| ProductAuthRouteFailure::invalid_request())
        })
        .transpose()?;
    let invocation_id = match fields.invocation_id.as_deref() {
        Some(value) => {
            InvocationId::parse(value).map_err(|_| ProductAuthRouteFailure::invalid_request())?
        }
        None => InvocationId::new(),
    };

    let mut scope = AuthProductScope::new(
        ResourceScope {
            tenant_id: caller.tenant_id.clone(),
            user_id: caller.user_id.clone(),
            agent_id: caller.agent_id.clone(),
            project_id: caller.project_id.clone(),
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

/// Like [`scope_from_authenticated_caller_parts`] but returns `invalid_request`
/// when `invocation_id` is absent. Use for follow-up routes where the browser
/// MUST carry back the id minted by a prior setup/start response so the host
/// can re-derive the matching scope without minting a fresh, unmatched one.
pub(super) fn scope_from_authenticated_caller_parts_requiring_invocation(
    caller: &WebUiAuthenticatedCaller,
    fields: &ScopeFields,
) -> Result<AuthProductScope, ProductAuthRouteFailure> {
    if fields.invocation_id.is_none() {
        return Err(ProductAuthRouteFailure::invalid_request());
    }
    scope_from_authenticated_caller_parts(caller, fields)
}

pub(super) fn scope_from_callback_query(
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

pub(super) fn validate_callback_raw_query(
    raw_query: Option<&str>,
) -> Result<(), ProductAuthRouteFailure> {
    let Some(raw_query) = raw_query else {
        return Err(ProductAuthRouteFailure::malformed_callback());
    };
    if raw_query.len() > OAUTH_CALLBACK_QUERY_MAX_BYTES {
        return Err(ProductAuthRouteFailure::malformed_callback());
    }
    Ok(())
}

pub(super) fn validate_callback_query_fields(
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

pub(super) fn validate_optional_callback_field(
    value: Option<&str>,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<(), ProductAuthRouteFailure> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_callback_field(value, max_bytes, allow_empty)
}

pub(super) fn validate_callback_field(
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

pub(super) fn scope_hint(scope: &AuthProductScope) -> OAuthCallbackScopeHint {
    OAuthCallbackScopeHint {
        user_id: scope.resource.user_id.clone(),
        agent_id: scope.resource.agent_id.clone(),
        project_id: scope.resource.project_id.clone(),
        thread_id: scope.resource.thread_id.clone(),
        invocation_id: scope.resource.invocation_id,
        session_id: scope.session_id.clone(),
    }
}

pub(super) fn authorization_endpoint_url(raw: &str) -> Result<Url, ProductAuthRouteFailure> {
    let authorization_url =
        OAuthAuthorizationUrl::new(raw.to_string()).map_err(ProductAuthRouteFailure::from)?;
    let parsed = Url::parse(authorization_url.as_str())
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(ProductAuthRouteFailure::invalid_request());
    }
    Ok(parsed)
}

pub(super) fn compose_authorization_url(
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

pub(super) fn opaque_state_hash(value: &str) -> Result<OpaqueStateHash, ProductAuthRouteFailure> {
    OpaqueStateHash::new(sha256_hex(value)).map_err(ProductAuthRouteFailure::from)
}

pub(super) fn pkce_verifier_hash(value: &str) -> Result<PkceVerifierHash, ProductAuthRouteFailure> {
    PkceVerifierHash::new(sha256_hex(value)).map_err(ProductAuthRouteFailure::from)
}

pub(super) fn authorization_code_hash(
    value: &str,
) -> Result<AuthorizationCodeHash, ProductAuthRouteFailure> {
    AuthorizationCodeHash::new(sha256_hex(value)).map_err(ProductAuthRouteFailure::from)
}

pub(super) fn sha256_hex(value: &str) -> String {
    ironclaw_common::hashing::sha256_hex(value.as_bytes())
}

pub(super) fn parse_provider_scopes(
    raw: Option<&str>,
) -> Result<Vec<ProviderScope>, ProductAuthRouteFailure> {
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
pub(super) struct UnvalidatedRawCallbackValue(String);

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
pub(super) struct UnvalidatedRawSecretValue(SecretString);

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
pub(super) struct RawCallbackValue(String);

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
pub(super) struct RawSecretValue(SecretString);

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

pub(super) fn validate_raw_value_with_limit(
    value: &str,
    max_bytes: usize,
) -> Result<(), &'static str> {
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

// ── Shared parse and timeout helpers ────────────────────────────────────────

pub(super) fn parse_interaction_id(
    value: &str,
) -> Result<AuthInteractionId, ProductAuthRouteFailure> {
    let parsed = Uuid::parse_str(value).map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    Ok(AuthInteractionId::from_uuid(parsed))
}

pub(super) fn parse_credential_account_id(
    value: &str,
) -> Result<CredentialAccountId, ProductAuthRouteFailure> {
    let parsed = Uuid::parse_str(value).map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    Ok(CredentialAccountId::from_uuid(parsed))
}

pub(super) fn parse_extension_id(value: &str) -> Result<ExtensionId, ProductAuthRouteFailure> {
    ExtensionId::new(value.to_string()).map_err(|_| ProductAuthRouteFailure::invalid_request())
}

pub(super) fn parse_optional_extension(
    value: Option<&str>,
) -> Result<Option<ExtensionId>, ProductAuthRouteFailure> {
    value.map(parse_extension_id).transpose()
}

/// Await a product-auth backend call under the shared backend timeout and
/// project both the elapsed-timeout failure and any returned auth error onto
/// the route's sanitized failure shape.
///
/// Every protected product-auth route enters `RebornProductAuthServices` the
/// same way; centralising the timeout/error wiring stops each handler from
/// having to re-derive the same four lines and keeps the failure projection
/// identical across routes.
pub(super) async fn run_with_backend_timeout<T, E, F>(
    future: F,
) -> Result<T, ProductAuthRouteFailure>
where
    F: std::future::Future<Output = Result<T, E>>,
    ProductAuthRouteFailure: From<E>,
{
    match tokio::time::timeout(PRODUCT_AUTH_BACKEND_TIMEOUT, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error.into()),
        Err(_) => Err(ProductAuthRouteFailure::backend_timeout()),
    }
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
