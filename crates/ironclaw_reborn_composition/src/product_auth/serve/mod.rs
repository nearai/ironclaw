//! Reborn-native product-auth route composition.
//!
//! This module owns only HTTP parsing, scope derivation from host-owned
//! composition, one-way hashing of callback material, and sanitized response
//! rendering. It deliberately delegates durable flow state, provider exchange,
//! credential mutation, and continuation dispatch to [`RebornProductAuthServices`].

// arch-exempt: large_file, product-auth route composition surface, plan #5905

mod accounts;
mod lifecycle;
mod manual_token;
mod oauth;
#[cfg(test)]
mod oauth_start_tests;

#[cfg(feature = "slack-v2-host-beta")]
pub(crate) use oauth::{
    CallbackScopeResolution, OAuthCallbackDescriptor, OAuthCallbackTerminalHookFuture,
    oauth_provider_callback_handler,
};

use std::{
    hash::Hash,
    num::{NonZeroU32, NonZeroU64, NonZeroUsize},
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Extension, Path, RawQuery, State},
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthFlowStatus, AuthGateRef,
    AuthInteractionId, AuthProductError, AuthProductScope, AuthProviderId, AuthSessionId,
    AuthSurface, AuthorizationCodeHash, CredentialAccountChoiceRequest, CredentialAccountId,
    CredentialAccountLabel, CredentialAccountListPage, CredentialAccountListRequest,
    CredentialAccountProjection, CredentialAccountSelectionRequest, CredentialAccountStatus,
    CredentialAccountUpdateBinding, CredentialRecoveryProjection, CredentialRecoveryRequest,
    CredentialRefreshReport, CredentialRefreshRequest, GOOGLE_PROVIDER_ID, GoogleOAuthRouteConfig,
    OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthCallbackState, OAuthCallbackStateKind,
    OAuthProviderCallbackRequest, OpaqueStateHash, PkceVerifierHash, PkceVerifierSecret,
    ProviderScope, SLACK_PERSONAL_PROVIDER_ID, SecretCleanupAction, SecretCleanupReport,
    SecretCleanupRequest, Timestamp, TurnRunRef, binding_scope_owns_account,
    build_google_authorization_url, parse_google_callback_scopes, parse_google_requested_scopes,
    pkce_s256_challenge,
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
use ironclaw_product_workflow::{
    LifecyclePackageKind, RebornServicesApi, RebornServicesError, WebUiAuthenticatedCaller,
};
use lru::LruCache;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use url::Url;
use uuid::Uuid;

use crate::product_auth::api::auth::{
    RebornDcrOAuthStartFlowRequest, RebornOAuthCallbackAttemptError,
    RebornOAuthCallbackFailureStage, RebornOAuthStartFlowRequest,
};
#[cfg(feature = "slack-v2-host-beta")]
use crate::slack::slack_host_beta::SlackPersonalConnectionScopeResolver;
#[cfg(feature = "slack-v2-host-beta")]
use crate::slack::slack_personal_binding::{
    RebornUserIdentityBindingDeleteStore, SlackPersonalUserBinder, SlackUserBindingLifecycleStore,
};
use crate::{
    RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest, RebornManualTokenSubmitResponse,
    RebornOAuthCallbackError, RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest,
    RebornOAuthCallbackResponse, RebornProductAuthServices,
};

pub(crate) const OAUTH_START_PATH: &str = "/api/reborn/product-auth/oauth/start";
pub(crate) const OAUTH_CALLBACK_PATH: &str = "/api/reborn/product-auth/oauth/callback/{flow_id}";
pub(crate) const OAUTH_FLOW_STATUS_PATH: &str =
    "/api/reborn/product-auth/oauth/flow/{flow_id}/status";
pub(crate) const OAUTH_FLOW_RECONCILE_PATH: &str =
    "/api/reborn/product-auth/oauth/flow/{flow_id}/reconcile";
pub(crate) const GOOGLE_OAUTH_START_PATH: &str = "/api/reborn/product-auth/oauth/google/start";
pub(crate) const GOOGLE_OAUTH_CALLBACK_PATH: &str =
    "/api/reborn/product-auth/oauth/google/callback";
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) const SLACK_PERSONAL_OAUTH_CALLBACK_PATH: &str =
    "/api/reborn/product-auth/oauth/slack_personal/callback";
pub(crate) const EXTENSION_OAUTH_START_PATH: &str =
    "/api/webchat/v2/extensions/{package_id}/setup/oauth/start";
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
const OAUTH_FLOW_STATUS_ROUTE_ID: &str = "product_auth.oauth.flow_status";
const OAUTH_FLOW_RECONCILE_ROUTE_ID: &str = "product_auth.oauth.flow_reconcile";
const GOOGLE_OAUTH_START_ROUTE_ID: &str = "product_auth.oauth.google.start";
const GOOGLE_OAUTH_CALLBACK_ROUTE_ID: &str = "product_auth.oauth.google.callback";
#[cfg(feature = "slack-v2-host-beta")]
const SLACK_PERSONAL_OAUTH_CALLBACK_ROUTE_ID: &str = "product_auth.oauth.slack_personal.callback";
const EXTENSION_OAUTH_START_ROUTE_ID: &str = "webui_v2.extensions.oauth.start";
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
// The reconnect watcher polls flow status on a ~2s cadence (30 req/min) while a
// user completes OAuth in a popup, so the read cap is deliberately higher than
// the 20/min mutation cap. Still per-caller and bounded so a client cannot spin
// the read hot.
const OAUTH_FLOW_STATUS_MAX_REQUESTS: NonZeroU32 = match NonZeroU32::new(120) {
    Some(value) => value,
    // SAFETY: 120 is a non-zero literal rate limit.
    None => unreachable!(),
};
const OAUTH_RATE_WINDOW_SECONDS: NonZeroU32 = match NonZeroU32::new(60) {
    Some(value) => value,
    // SAFETY: 60 is a non-zero literal rate-limit window.
    None => unreachable!(),
};
pub(crate) const PRODUCT_AUTH_FLOW_MAX_TTL_SECONDS: i64 = 10 * 60;
const PRODUCT_AUTH_BACKEND_TIMEOUT: Duration = Duration::from_secs(30);
const OAUTH_CALLBACK_QUERY_MAX_BYTES: usize = 16 * 1024;
const OAUTH_CALLBACK_FIELD_MAX_BYTES: usize = 512;
const OAUTH_CALLBACK_SCOPES_MAX_BYTES: usize = 4 * 1024;
const RAW_OAUTH_VALUE_MAX_BYTES: usize = 4 * 1024;

#[derive(Clone)]
pub(crate) struct ProductAuthRouteState {
    product_auth: Arc<RebornProductAuthServices>,
    installed_extension_lookup: Option<Arc<dyn InstalledExtensionLookup>>,
    tenant_id: TenantId,
    default_agent_id: Option<AgentId>,
    default_project_id: Option<ProjectId>,
    google_oauth: Option<GoogleOAuthRouteConfig>,
    #[cfg(feature = "slack-v2-host-beta")]
    slack_personal_oauth: Option<crate::slack::slack_setup::SlackPersonalSetupServiceSlot>,
    #[cfg(feature = "slack-v2-host-beta")]
    slack_personal_oauth_binding: Option<SlackPersonalOAuthBindingConfig>,
    // First-slice WebUI OAuth stores the raw PKCE verifier process-locally
    // because `AuthFlowRecord` deliberately serializes hashes only. Production
    // HA must replace this with a host-owned encrypted verifier store before
    // routing callbacks across replicas or restarts.
    pkce_verifiers: ExpiringLruCache<AuthFlowId, StoredPkceVerifier>,
}

#[async_trait::async_trait]
trait InstalledExtensionLookup: Send + Sync {
    async fn is_installed(
        &self,
        caller: &WebUiAuthenticatedCaller,
        extension_id: &ExtensionId,
    ) -> Result<bool, RebornServicesError>;
}

struct RebornServicesInstalledExtensionLookup {
    api: Arc<dyn RebornServicesApi>,
}

#[async_trait::async_trait]
impl InstalledExtensionLookup for RebornServicesInstalledExtensionLookup {
    async fn is_installed(
        &self,
        caller: &WebUiAuthenticatedCaller,
        extension_id: &ExtensionId,
    ) -> Result<bool, RebornServicesError> {
        let inventory = self.api.list_extensions(caller.clone()).await?;
        Ok(inventory.extensions.iter().any(|extension| {
            extension.package_ref.kind == LifecyclePackageKind::Extension
                && extension.package_ref.id.as_str() == extension_id.as_str()
        }))
    }
}

#[cfg(test)]
struct TestInstalledExtensionLookup;

#[cfg(test)]
#[async_trait::async_trait]
impl InstalledExtensionLookup for TestInstalledExtensionLookup {
    async fn is_installed(
        &self,
        _caller: &WebUiAuthenticatedCaller,
        _extension_id: &ExtensionId,
    ) -> Result<bool, RebornServicesError> {
        Ok(true)
    }
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
            installed_extension_lookup: None,
            tenant_id,
            default_agent_id,
            default_project_id,
            google_oauth: None,
            #[cfg(feature = "slack-v2-host-beta")]
            slack_personal_oauth: None,
            #[cfg(feature = "slack-v2-host-beta")]
            slack_personal_oauth_binding: None,
            pkce_verifiers: ExpiringLruCache::new(
                OAUTH_PKCE_VERIFIER_CACHE_CAPACITY,
                StoredPkceVerifier::expires_at,
            ),
        }
    }

    pub(crate) fn with_webui_api(mut self, webui_api: Arc<dyn RebornServicesApi>) -> Self {
        self.installed_extension_lookup = Some(Arc::new(RebornServicesInstalledExtensionLookup {
            api: webui_api,
        }));
        self
    }

    #[cfg(test)]
    fn with_test_installed_extension_lookup(mut self) -> Self {
        self.installed_extension_lookup = Some(Arc::new(TestInstalledExtensionLookup));
        self
    }

    async fn require_installed_extension(
        &self,
        caller: &WebUiAuthenticatedCaller,
        requester_extension: &ExtensionId,
    ) -> Result<(), ProductAuthRouteFailure> {
        let Some(lookup) = self.installed_extension_lookup.as_ref() else {
            return Err(ProductAuthRouteFailure::backend_unavailable());
        };
        let is_installed = tokio::time::timeout(
            PRODUCT_AUTH_BACKEND_TIMEOUT,
            lookup.is_installed(caller, requester_extension),
        )
        .await
        .map_err(|_| ProductAuthRouteFailure::backend_timeout())?
        .map_err(|error| {
            tracing::warn!(
                %error,
                extension_id = %requester_extension,
                "installed extension lookup failed before OAuth start"
            );
            ProductAuthRouteFailure::backend_unavailable()
        })?;
        if !is_installed {
            return Err(ProductAuthRouteFailure::extension_not_installed());
        }
        Ok(())
    }

    pub(crate) fn with_google_oauth(mut self, config: GoogleOAuthRouteConfig) -> Self {
        self.google_oauth = Some(config);
        self
    }

    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) fn product_auth_services(&self) -> &RebornProductAuthServices {
        &self.product_auth
    }

    fn google_oauth_config(&self) -> Result<&GoogleOAuthRouteConfig, ProductAuthRouteFailure> {
        self.google_oauth
            .as_ref()
            .ok_or_else(ProductAuthRouteFailure::backend_unavailable)
    }

    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) fn with_slack_personal_oauth(
        mut self,
        slot: crate::slack::slack_setup::SlackPersonalSetupServiceSlot,
    ) -> Self {
        self.slack_personal_oauth = Some(slot);
        self
    }

    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) async fn slack_personal_oauth_credentials(
        &self,
    ) -> Result<
        (
            ironclaw_auth::OAuthClientId,
            ironclaw_auth::OAuthRedirectUri,
        ),
        ProductAuthRouteFailure,
    > {
        let slot = self.slack_personal_oauth.as_ref().ok_or_else(|| {
            tracing::warn!(
                "Slack personal OAuth slot not configured (IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI not set)"
            );
            ProductAuthRouteFailure::backend_unavailable()
        })?;
        let service = slot.get().ok_or_else(|| {
            tracing::warn!("Slack personal OAuth slot not yet filled (startup race)");
            ProductAuthRouteFailure::backend_unavailable()
        })?;
        let (client_id, _secret) = service.oauth_credentials().await.map_err(|e| {
            tracing::warn!(error = %e, "Slack personal OAuth credentials not configured");
            ProductAuthRouteFailure::malformed_config()
        })?;
        let redirect_uri = slot.redirect_uri().clone();
        Ok((client_id, redirect_uri))
    }

    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) fn with_slack_personal_oauth_binding(
        mut self,
        config: SlackPersonalOAuthBindingConfig,
    ) -> Self {
        self.slack_personal_oauth_binding = Some(config);
        self
    }

    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) fn slack_personal_oauth_binding_config(
        &self,
    ) -> Option<&SlackPersonalOAuthBindingConfig> {
        self.slack_personal_oauth_binding.as_ref()
    }

    pub(crate) fn store_pkce_verifier(
        &self,
        flow_id: AuthFlowId,
        verifier: SecretString,
        expires_at: Timestamp,
    ) -> Result<(), ProductAuthRouteFailure> {
        self.pkce_verifiers.store(
            flow_id,
            StoredPkceVerifier {
                verifier,
                expires_at,
            },
        )
    }

    fn pkce_verifier_for_callback(
        &self,
        flow_id: AuthFlowId,
    ) -> Result<SecretString, ProductAuthRouteFailure> {
        self.pkce_verifiers
            .get(&flow_id)
            .map(|stored| stored.verifier.clone())
            .ok_or_else(ProductAuthRouteFailure::unknown_or_expired_flow)
    }

    pub(crate) fn remove_pkce_verifier(&self, flow_id: AuthFlowId) {
        self.pkce_verifiers.remove(&flow_id);
    }
}

impl std::fmt::Debug for ProductAuthRouteState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut builder = formatter.debug_struct("ProductAuthRouteState");
        builder
            .field("product_auth", &"Arc<RebornProductAuthServices>")
            .field(
                "installed_extension_lookup",
                &self.installed_extension_lookup.is_some(),
            )
            .field("tenant_id", &self.tenant_id)
            .field("default_agent_id", &self.default_agent_id)
            .field("default_project_id", &self.default_project_id)
            .field("google_oauth", &self.google_oauth.is_some());
        #[cfg(feature = "slack-v2-host-beta")]
        builder.field("slack_personal_oauth", &self.slack_personal_oauth.is_some());
        #[cfg(feature = "slack-v2-host-beta")]
        builder.field(
            "slack_personal_oauth_binding",
            &self.slack_personal_oauth_binding.is_some(),
        );
        builder
            .field("pkce_verifiers", &"ExpiringLruCache<...>")
            .finish()
    }
}

#[cfg(feature = "slack-v2-host-beta")]
#[derive(Clone)]
pub struct SlackPersonalOAuthBindingConfig {
    pub(crate) binding_service: Arc<dyn SlackPersonalUserBinder>,
    pub(crate) connection_scope_resolver: Arc<dyn SlackPersonalConnectionScopeResolver>,
    /// Undoes an identity binding written by the callback identity hook when
    /// `complete_oauth_callback` fails afterwards; the binding is the
    /// user-visible "connected" signal, so it must not survive a completion
    /// failure that already deleted the token material.
    pub(crate) binding_rollback_store: Arc<dyn RebornUserIdentityBindingDeleteStore>,
    pub(crate) lifecycle_store: Arc<dyn SlackUserBindingLifecycleStore>,
}

#[cfg(feature = "slack-v2-host-beta")]
impl SlackPersonalOAuthBindingConfig {
    pub(crate) fn new(
        binding_service: Arc<dyn SlackPersonalUserBinder>,
        connection_scope_resolver: Arc<dyn SlackPersonalConnectionScopeResolver>,
        binding_rollback_store: Arc<dyn RebornUserIdentityBindingDeleteStore>,
        lifecycle_store: Arc<dyn SlackUserBindingLifecycleStore>,
    ) -> Self {
        Self {
            binding_service,
            connection_scope_resolver,
            binding_rollback_store,
            lifecycle_store,
        }
    }
}

#[cfg(feature = "slack-v2-host-beta")]
impl std::fmt::Debug for SlackPersonalOAuthBindingConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackPersonalOAuthBindingConfig")
            .field("binding_service", &self.binding_service)
            .field(
                "connection_scope_resolver",
                &"Arc<dyn SlackPersonalConnectionScopeResolver>",
            )
            .field(
                "binding_rollback_store",
                &"Arc<dyn RebornUserIdentityBindingDeleteStore>",
            )
            .field(
                "lifecycle_store",
                &"Arc<dyn SlackUserBindingLifecycleStore>",
            )
            .finish()
    }
}

#[derive(Clone)]
struct ExpiringLruCache<K, V> {
    entries: Arc<Mutex<LruCache<K, V>>>,
    expires_at: fn(&V) -> Timestamp,
}

impl<K, V> ExpiringLruCache<K, V>
where
    K: Clone + Eq + Hash,
{
    fn new(capacity: NonZeroUsize, expires_at: fn(&V) -> Timestamp) -> Self {
        Self {
            entries: Arc::new(Mutex::new(LruCache::new(capacity))),
            expires_at,
        }
    }

    fn store(&self, key: K, value: V) -> Result<(), ProductAuthRouteFailure> {
        let mut entries = self.lock();
        self.remove_expired(&mut entries);
        if entries.len() >= entries.cap().get() && !entries.contains(&key) {
            return Err(ProductAuthRouteFailure::backend_unavailable());
        }
        entries.put(key, value);
        Ok(())
    }

    fn get(&self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        let mut entries = self.lock();
        self.remove_expired(&mut entries);
        entries.get(key).cloned()
    }

    fn remove(&self, key: &K) {
        self.lock().pop(key);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LruCache<K, V>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn remove_expired(&self, entries: &mut LruCache<K, V>) {
        let now = Utc::now();
        let expired = entries
            .iter()
            .filter_map(|(key, value)| ((self.expires_at)(value) <= now).then_some(key.clone()))
            .collect::<Vec<_>>();
        for key in expired {
            entries.pop(&key);
        }
    }
}

#[derive(Clone)]
pub(super) struct StoredPkceVerifier {
    verifier: SecretString,
    expires_at: Timestamp,
}

impl StoredPkceVerifier {
    fn expires_at(&self) -> Timestamp {
        self.expires_at
    }
}

pub(crate) struct ProductAuthRouteMount {
    pub(crate) protected: Router,
    pub(crate) public: Router,
    pub(crate) descriptors: Vec<IngressRouteDescriptor>,
}

async fn extension_oauth_start_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(package_id): Path<String>,
    Json(request): Json<ExtensionOAuthStartRequest>,
) -> Result<Json<ProductOAuthStartResponse>, ProductAuthRouteFailure> {
    let requester_extension =
        ExtensionId::new(package_id).map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    state
        .require_installed_extension(&caller, &requester_extension)
        .await?;
    let response = if request.provider == SLACK_PERSONAL_PROVIDER_ID {
        #[cfg(feature = "slack-v2-host-beta")]
        let response = crate::slack::slack_personal_oauth::start_extension_oauth_flow(
            state.clone(),
            caller.clone(),
            request,
            requester_extension.clone(),
        )
        .await?;
        #[cfg(not(feature = "slack-v2-host-beta"))]
        return Err(ProductAuthRouteFailure::backend_unavailable());
        #[cfg(feature = "slack-v2-host-beta")]
        response
    } else {
        oauth::start_extension_oauth_flow(
            state.clone(),
            caller.clone(),
            request,
            requester_extension.clone(),
        )
        .await?
    };
    if let Err(error) = state
        .require_installed_extension(&caller, &requester_extension)
        .await
    {
        oauth::abort_started_extension_oauth_flow(&state, &response.0).await?;
        return Err(error);
    }
    Ok(response)
}

// Product-auth HTTP is a host-owned auth/secret-ingress boundary. Its
// mutations enter `RebornProductAuthServices` directly; they are not in-turn
// tool calls and must not surface raw secrets through the model-visible
// tool-dispatch path. Contract: `docs/reborn/contracts/auth-product.md`.
// dispatch-exempt: host-owned auth/secret ingress, not in-turn tool dispatch
pub(crate) fn product_auth_route_mount(state: ProductAuthRouteState) -> ProductAuthRouteMount {
    let public = Router::new()
        .route(OAUTH_CALLBACK_PATH, get(oauth::oauth_callback_handler))
        .route(
            GOOGLE_OAUTH_CALLBACK_PATH,
            get(oauth::google_oauth_callback_handler),
        );
    #[cfg(feature = "slack-v2-host-beta")]
    let public = public.route(
        SLACK_PERSONAL_OAUTH_CALLBACK_PATH,
        get(crate::slack::slack_personal_oauth::slack_personal_oauth_callback_handler),
    );

    ProductAuthRouteMount {
        protected: Router::new()
            .route(OAUTH_START_PATH, post(oauth::oauth_start_handler))
            .route(
                OAUTH_FLOW_STATUS_PATH,
                get(oauth::oauth_flow_status_handler),
            )
            .route(
                OAUTH_FLOW_RECONCILE_PATH,
                post(oauth::oauth_flow_reconcile_handler),
            )
            .route(
                GOOGLE_OAUTH_START_PATH,
                post(oauth::google_oauth_start_handler),
            )
            .route(
                EXTENSION_OAUTH_START_PATH,
                post(extension_oauth_start_handler),
            )
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
        public: public.with_state(state),
        descriptors: product_auth_route_descriptors(),
    }
}

pub(crate) fn product_auth_route_descriptors() -> Vec<IngressRouteDescriptor> {
    // All protected mutations share the same LocalGateway + Bearer + per-caller
    // policy. Listing them as a table keeps the policy choice next to the path
    // and stops descriptor blocks from drifting per-route.
    const PROTECTED_MUTATIONS: &[(&str, &str)] = &[
        (OAUTH_START_ROUTE_ID, OAUTH_START_PATH),
        (GOOGLE_OAUTH_START_ROUTE_ID, GOOGLE_OAUTH_START_PATH),
        (EXTENSION_OAUTH_START_ROUTE_ID, EXTENSION_OAUTH_START_PATH),
        (OAUTH_FLOW_RECONCILE_ROUTE_ID, OAUTH_FLOW_RECONCILE_PATH),
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
        OAUTH_FLOW_STATUS_ROUTE_ID,
        NetworkMethod::Get,
        OAUTH_FLOW_STATUS_PATH,
        flow_status_policy(),
    ));
    descriptors.push(descriptor(
        OAUTH_CALLBACK_ROUTE_ID,
        NetworkMethod::Get,
        OAUTH_CALLBACK_PATH,
        callback_policy(),
    ));
    descriptors.push(descriptor(
        GOOGLE_OAUTH_CALLBACK_ROUTE_ID,
        NetworkMethod::Get,
        GOOGLE_OAUTH_CALLBACK_PATH,
        callback_policy(),
    ));
    #[cfg(feature = "slack-v2-host-beta")]
    descriptors.push(descriptor(
        SLACK_PERSONAL_OAUTH_CALLBACK_ROUTE_ID,
        NetworkMethod::Get,
        SLACK_PERSONAL_OAUTH_CALLBACK_PATH,
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

pub(super) fn flow_status_policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::BearerToken],
        },
        scope_source: ironclaw_host_api::IngressScopeSource::AuthenticatedCaller,
        // Read-only status probe: no request body is read, so reject any.
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerCaller,
            max_requests: OAUTH_FLOW_STATUS_MAX_REQUESTS,
            window_seconds: OAUTH_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("product-auth OAuth flow-status policy must validate") // safety: same authenticated LocalGateway shape as the OAuth start mutation, but NoBody + read-only per-caller poll cadence.
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
pub(super) struct GoogleOAuthStartRequest {
    account_label: String,
    scopes: Vec<String>,
    expires_at: Timestamp,
    session_id: Option<String>,
    thread_id: Option<String>,
    invocation_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ExtensionOAuthStartRequest {
    pub(crate) provider: String,
    pub(crate) account_label: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) expires_at: Timestamp,
    pub(crate) invocation_id: Option<String>,
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
pub(crate) struct ProductOAuthStartResponse {
    pub(crate) flow_id: AuthFlowId,
    pub(crate) status: AuthFlowStatus,
    pub(crate) provider: AuthProviderId,
    pub(crate) authorization_url: OAuthAuthorizationUrl,
    pub(crate) expires_at: Timestamp,
    pub(crate) continuation: AuthContinuationRef,
    pub(crate) callback_scope: OAuthCallbackScopeHint,
}

/// Sanitized durable flow-status projection returned by the origin-independent
/// OAuth flow-status poll. Carries the lifecycle status enum ONLY — never
/// tokens, PKCE verifiers, authorization codes, or opaque state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OAuthFlowStatusResponse {
    pub(crate) status: AuthFlowStatus,
}

/// Query fields for the flow-status poll. The browser echoes back the
/// `invocation_id` the start response minted (`callback_scope.invocation_id`)
/// so the caller-scoped handler can re-derive the exact `AuthProductScope`
/// `get_flow` matched on when the flow was created; the trusted
/// tenant/user/agent/project still come from the authenticated caller, so a
/// forged `invocation_id` cannot reach another owner's flow.
#[derive(Deserialize)]
pub(super) struct OAuthFlowStatusQuery {
    invocation_id: Option<String>,
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
pub(crate) struct ScopeFields {
    pub(crate) session_id: Option<String>,
    pub(crate) thread_id: Option<String>,
    pub(crate) invocation_id: Option<String>,
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
    user_id: Option<String>,
    invocation_id: Option<String>,
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

#[derive(Deserialize)]
pub(super) struct GoogleOAuthCallbackQuery {
    state: Option<RawCallbackValue>,
    code: Option<RawSecretValue>,
    error: Option<String>,
    #[serde(alias = "scope")]
    scopes: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProductAuthRouteFailure {
    status: StatusCode,
    body: RebornOAuthCallbackError,
    callback_failure_stage: RebornOAuthCallbackFailureStage,
}

impl ProductAuthRouteFailure {
    pub(crate) fn new(status: StatusCode, code: AuthErrorCode) -> Self {
        Self {
            status,
            body: RebornOAuthCallbackError {
                code,
                retryable: matches!(code, AuthErrorCode::BackendUnavailable),
            },
            callback_failure_stage: RebornOAuthCallbackFailureStage::Terminal,
        }
    }

    pub(crate) fn invalid_request() -> Self {
        Self::new(StatusCode::BAD_REQUEST, AuthErrorCode::InvalidRequest)
    }

    pub(crate) fn extension_not_installed() -> Self {
        Self::new(StatusCode::CONFLICT, AuthErrorCode::InvalidRequest)
    }

    pub(crate) fn malformed_callback() -> Self {
        Self::new(StatusCode::BAD_REQUEST, AuthErrorCode::MalformedCallback)
    }

    pub(crate) fn unknown_or_expired_flow() -> Self {
        Self::new(StatusCode::NOT_FOUND, AuthErrorCode::UnknownOrExpiredFlow)
    }

    pub(crate) fn backend_unavailable() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            AuthErrorCode::BackendUnavailable,
        )
    }

    pub(crate) fn malformed_config() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            AuthErrorCode::MalformedConfig,
        )
    }

    pub(crate) fn backend_timeout() -> Self {
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

impl From<RebornOAuthCallbackAttemptError> for ProductAuthRouteFailure {
    fn from(error: RebornOAuthCallbackAttemptError) -> Self {
        let callback_failure_stage = error.stage();
        let mut failure = route_failure_from_callback_error(error.error());
        failure.callback_failure_stage = callback_failure_stage;
        failure
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
        AuthErrorCode::BackendUnavailable | AuthErrorCode::MalformedConfig => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        AuthErrorCode::TokenExchangeFailed | AuthErrorCode::RefreshFailed => {
            StatusCode::BAD_GATEWAY
        }
        AuthErrorCode::CredentialMissing
        | AuthErrorCode::AccountSelectionRequired
        | AuthErrorCode::ProviderIdentityAlreadyConnected
        | AuthErrorCode::ConnectionConflict => StatusCode::CONFLICT,
    };
    ProductAuthRouteFailure {
        status,
        body: error,
        callback_failure_stage: RebornOAuthCallbackFailureStage::Terminal,
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
pub(crate) fn scope_from_authenticated_caller_parts_requiring_invocation(
    caller: &WebUiAuthenticatedCaller,
    fields: &ScopeFields,
) -> Result<AuthProductScope, ProductAuthRouteFailure> {
    if fields.invocation_id.is_none() {
        return Err(ProductAuthRouteFailure::invalid_request());
    }
    scope_from_authenticated_caller_parts(caller, fields)
}

pub(crate) async fn scoped_update_binding_for_requester(
    state: &ProductAuthRouteState,
    scope: AuthProductScope,
    provider: AuthProviderId,
    requester_extension: Option<&ExtensionId>,
) -> Result<Option<CredentialAccountUpdateBinding>, ProductAuthRouteFailure> {
    let Some(requester_extension) = requester_extension else {
        return Ok(None);
    };
    // Bind a reconnect at durable owner granularity. `thread_id`/`mission_id`
    // (and the per-flow `invocation_id` the old `scope_matches` full-equality
    // compared) are transient invocation provenance, not identity — matching on
    // them meant the 2nd OAuth flow could never find the existing account and
    // forked a duplicate `UserReusable` account on every flow. `session_id` IS
    // path-segmenting (paths.rs), so it stays matched; the update path reads at
    // the flow's stored scope and would orphan across a different session.
    let owner_scope = scope.to_credential_owner();
    // Scope-agnostic on purpose: a reconnect that grants a NEW provider scope
    // must still bind to (and update) the existing account that lacks it.
    let account = state
        .product_auth
        .runtime_credential_account_selection_service()
        .select_configured_account_for_binding(
            CredentialAccountSelectionRequest::new(owner_scope.clone(), provider.clone())
                .for_extension(requester_extension.clone()),
            owner_scope.clone(),
        )
        .await;
    match account {
        Ok(account) if binding_scope_owns_account(&scope, &account) => Ok(Some(
            CredentialAccountUpdateBinding::from_projection(&account.projection()),
        )),
        Ok(_) => Ok(None),
        Err(AuthProductError::CredentialMissing) => Ok(None),
        Err(AuthProductError::CrossScopeDenied) => Ok(None),
        // Ambiguous owner state (e.g. mixed reusable + extension-owned accounts):
        // the selector cannot pick a single account to rebind. Start a fresh flow
        // rather than hard-failing the reconnect — failing the start route would
        // leave the owner unable to (re)connect at all, which is worse than the
        // rare extra account. Log it so the ambiguous reconnect is observable and
        // not silently conflated with the "no existing account" arms above.
        Err(AuthProductError::AccountSelectionRequired) => {
            tracing::warn!(
                target: "ironclaw_reborn_composition::product_auth::oauth",
                provider = %provider.as_str(),
                requester_extension = %requester_extension.as_str(),
                "owner has multiple eligible accounts; starting extension OAuth without an update binding"
            );
            Ok(None)
        }
        Err(AuthProductError::BackendUnavailable) => {
            tracing::warn!(
                target: "ironclaw_reborn_composition::product_auth::oauth",
                provider = %provider.as_str(),
                requester_extension = %requester_extension.as_str(),
                "credential account status unavailable during extension OAuth start; starting setup without update binding"
            );
            Ok(None)
        }
        Err(error) => Err(ProductAuthRouteFailure::from(error)),
    }
}

pub(super) fn scope_from_callback_query(
    state: &ProductAuthRouteState,
    query: &OAuthCallbackQuery,
) -> Result<AuthProductScope, ProductAuthRouteFailure> {
    let user_id = UserId::new(
        query
            .user_id
            .clone()
            .ok_or_else(ProductAuthRouteFailure::malformed_callback)?,
    )
    .map_err(|_| ProductAuthRouteFailure::malformed_callback())?;
    let invocation_id = InvocationId::parse(
        query
            .invocation_id
            .as_deref()
            .ok_or_else(ProductAuthRouteFailure::malformed_callback)?,
    )
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
    validate_optional_callback_field(
        query.user_id.as_deref(),
        OAUTH_CALLBACK_FIELD_MAX_BYTES,
        false,
    )?;
    validate_optional_callback_field(
        query.invocation_id.as_deref(),
        OAUTH_CALLBACK_FIELD_MAX_BYTES,
        false,
    )?;
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

pub(crate) fn scope_hint(scope: &AuthProductScope) -> OAuthCallbackScopeHint {
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

pub(crate) fn opaque_state_hash(value: &str) -> Result<OpaqueStateHash, ProductAuthRouteFailure> {
    OpaqueStateHash::new(sha256_hex(value)).map_err(ProductAuthRouteFailure::from)
}

pub(crate) fn pkce_verifier_hash(value: &str) -> Result<PkceVerifierHash, ProductAuthRouteFailure> {
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
pub(crate) async fn run_with_backend_timeout<T, E, F>(
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
    use crate::AuthChallengeProvider;
    use crate::RebornAuthContinuationDispatcher;
    use crate::product_auth::oauth::notion_oauth::notion_provider_spec;
    use crate::product_auth::oauth::oauth_dcr::{
        OAuthDcrProvider, OAuthDcrProviderConfig, OAuthDcrProviderRegistry,
    };
    use crate::product_auth::oauth::oauth_dcr_protocol::flow_secret_handle;
    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, header};
    use ironclaw_auth::{
        AuthFlowManager, AuthInteractionService, AuthProviderClient,
        CredentialAccountLookupRequest, CredentialAccountService, CredentialAccountStatus,
        CredentialOwnership, CredentialSetupService, NewCredentialAccount, SecretCleanupService,
    };
    use ironclaw_capabilities::{CapabilityObligationHandler, CapabilityObligationRequest};
    use ironclaw_host_api::{
        NetworkMethod, RuntimeCredentialAccountProviderId, RuntimeCredentialAuthRequirement,
        RuntimeHttpEgress, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, SecretHandle,
    };
    use ironclaw_secrets::{InMemorySecretStore, SecretMaterial, SecretStore};
    use ironclaw_turns::{TurnRunId, TurnScope};
    use tower::ServiceExt;

    // Contract: the origin-independent reconnect flow-status route is a
    // read-only, authenticated, per-caller poll. Locking its policy here stops
    // a future edit from silently loosening it (e.g. dropping bearer auth,
    // accepting a body, or widening the rate-limit scope to per-IP/public).
    #[test]
    fn flow_status_route_descriptor_locks_read_only_bearer_policy() {
        let descriptors = product_auth_route_descriptors();
        let flow_status = descriptors
            .iter()
            .find(|descriptor| descriptor.route_id().as_str() == OAUTH_FLOW_STATUS_ROUTE_ID)
            .expect("the flow-status descriptor must be registered");

        assert_eq!(flow_status.method(), NetworkMethod::Get);

        let policy = flow_status.policy();
        // Read-only status probe: it never reads a request body.
        assert!(
            matches!(policy.body_limit(), BodyLimitPolicy::NoBody),
            "flow-status must reject any request body"
        );
        // Bearer auth, scoped to the authenticated caller — so a browser cannot
        // forge tenant/user and read another caller's flow.
        assert!(
            matches!(
                policy.auth(),
                IngressAuthPolicy::Required { schemes }
                    if schemes.contains(&IngressAuthScheme::BearerToken)
            ),
            "flow-status must require bearer auth"
        );
        assert_eq!(
            policy.scope_source(),
            ironclaw_host_api::IngressScopeSource::AuthenticatedCaller
        );
        // Per-caller rate limit for the poll cadence; never per-IP/public.
        assert!(
            matches!(
                policy.rate_limit(),
                RateLimitPolicy::Limited {
                    scope: RateLimitScope::PerCaller,
                    ..
                }
            ),
            "flow-status must be per-caller rate limited"
        );
        assert_eq!(policy.cors(), CorsPolicy::SameOriginOnly);
    }

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

    struct SequencedInstalledExtensionLookup {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl InstalledExtensionLookup for SequencedInstalledExtensionLookup {
        async fn is_installed(
            &self,
            _caller: &WebUiAuthenticatedCaller,
            _extension_id: &ExtensionId,
        ) -> Result<bool, RebornServicesError> {
            Ok(self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0)
        }
    }

    #[derive(Default)]
    struct RecordingDispatcher {
        events: Mutex<Vec<ironclaw_auth::AuthContinuationEvent>>,
    }

    impl RecordingDispatcher {
        fn events(&self) -> Vec<ironclaw_auth::AuthContinuationEvent> {
            self.events
                .lock()
                .expect("recording dispatcher lock")
                .clone()
        }
    }

    #[async_trait]
    impl RebornAuthContinuationDispatcher for RecordingDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            event: ironclaw_auth::AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            self.events
                .lock()
                .expect("recording dispatcher lock")
                .push(event);
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

    fn test_resource_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: UserId::new("user-alpha").expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn test_caller() -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            TenantId::new("tenant-alpha").expect("tenant"),
            UserId::new("user-alpha").expect("user"),
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

    #[tokio::test]
    async fn extension_oauth_start_handler_starts_dcr_setup_flow_for_notion() {
        let secret_store = Arc::new(InMemorySecretStore::new());
        let dcr_provider = Arc::new(
            OAuthDcrProvider::new(
                OAuthDcrProviderConfig {
                    spec: notion_provider_spec(),
                    callback_origin: "http://127.0.0.1:3000".to_string(),
                    client_name: "Ironclaw".to_string(),
                    account_label: CredentialAccountLabel::new("notion").expect("label"),
                    scopes: Vec::new(),
                },
                Arc::new(RouteDcrSetupEgress),
                secret_store,
                Arc::new(NoopObligationHandler),
            )
            .expect("DCR provider"),
        );
        let product_auth = RebornProductAuthServices::local_dev_in_memory(Arc::new(NoopDispatcher))
            .with_dcr_oauth_registry(Arc::new(OAuthDcrProviderRegistry::new(vec![dcr_provider])));
        let state = ProductAuthRouteState::new(
            Arc::new(product_auth),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_test_installed_extension_lookup();
        let app = product_auth_route_mount(state)
            .protected
            .layer(axum::Extension(test_caller()));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/notion/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "provider": "notion",
                            "account_label": "work notion",
                            "scopes": [],
                            "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
                            "invocation_id": InvocationId::new().to_string(),
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("route response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("start json");
        assert_eq!(json["provider"], "notion");
        assert_eq!(json["continuation"]["type"], "lifecycle_activation");
        assert_eq!(json["continuation"]["package_ref"], "notion");
        let authorization_url = json["authorization_url"]
            .as_str()
            .expect("authorization url");
        let parsed = url::Url::parse(authorization_url).expect("authorization URL");
        assert_eq!(parsed.host_str(), Some("oauth.notion.com"));
        let redirect_uri = parsed
            .query_pairs()
            .find_map(|(name, value)| (name == "redirect_uri").then(|| value.into_owned()))
            .expect("redirect uri");
        let redirect = url::Url::parse(&redirect_uri).expect("callback redirect URL");
        assert_eq!(
            redirect
                .query_pairs()
                .find_map(|(name, value)| (name == "account_label").then(|| value.into_owned())),
            Some("work notion".to_string())
        );
    }

    #[tokio::test]
    async fn extension_oauth_start_aborts_flow_when_extension_disappears_after_creation() {
        let secret_store = Arc::new(InMemorySecretStore::new());
        let dcr_provider = Arc::new(
            OAuthDcrProvider::new(
                OAuthDcrProviderConfig {
                    spec: notion_provider_spec(),
                    callback_origin: "http://127.0.0.1:3000".to_string(),
                    client_name: "Ironclaw".to_string(),
                    account_label: CredentialAccountLabel::new("notion").expect("label"),
                    scopes: Vec::new(),
                },
                Arc::new(RouteDcrSetupEgress),
                secret_store,
                Arc::new(NoopObligationHandler),
            )
            .expect("DCR provider"),
        );
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), Arc::new(NoopDispatcher))
                .with_dcr_oauth_registry(Arc::new(OAuthDcrProviderRegistry::new(vec![
                    dcr_provider,
                ]))),
        );
        let mut state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
        state.installed_extension_lookup = Some(Arc::new(SequencedInstalledExtensionLookup {
            calls: std::sync::atomic::AtomicUsize::new(0),
        }));
        let app = product_auth_route_mount(state)
            .protected
            .layer(axum::Extension(test_caller()));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/notion/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "provider": "notion",
                            "account_label": "work notion",
                            "scopes": [],
                            "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
                            "invocation_id": InvocationId::new().to_string(),
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("route response");

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let flows = shared.flow_records_snapshot();
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].status, AuthFlowStatus::Canceled);
    }

    #[tokio::test]
    async fn installed_extension_lookup_is_required_even_in_test_builds() {
        let state = ProductAuthRouteState::new(
            Arc::new(RebornProductAuthServices::local_dev_in_memory(Arc::new(
                NoopDispatcher,
            ))),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );

        let error = state
            .require_installed_extension(
                &test_caller(),
                &ExtensionId::new("notion").expect("extension"),
            )
            .await
            .expect_err("missing production lookup must fail closed");

        assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.body.code, AuthErrorCode::BackendUnavailable);
    }

    #[tokio::test]
    async fn extension_google_oauth_start_binds_existing_configured_account_with_matching_scope() {
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let product_auth = Arc::new(RebornProductAuthServices::from_shared(
            shared.clone(),
            Arc::new(NoopDispatcher),
        ));
        let state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_test_installed_extension_lookup()
        .with_google_oauth(
            GoogleOAuthRouteConfig::new(
                "google-client.apps.googleusercontent.com",
                "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
            )
            .expect("google oauth route config"),
        );
        let app = product_auth_route_mount(state)
            .protected
            .layer(axum::Extension(test_caller()));
        let flow_invocation_id = InvocationId::new();
        let mut existing_resource = test_resource_scope();
        existing_resource.invocation_id = flow_invocation_id;
        let existing_scope = AuthProductScope::new(existing_resource, AuthSurface::Callback);
        let account = shared
            .create_account(NewCredentialAccount {
                scope: existing_scope.clone(),
                provider: AuthProviderId::new(GOOGLE_PROVIDER_ID).expect("provider"),
                label: CredentialAccountLabel::new("google-drive google").expect("label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("google-drive-access").expect("secret")),
                refresh_secret: Some(SecretHandle::new("google-drive-refresh").expect("secret")),
                scopes: vec![
                    ProviderScope::new("https://www.googleapis.com/auth/drive")
                        .expect("provider scope"),
                ],
            })
            .await
            .expect("seed configured google account");

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/google-drive/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "provider": GOOGLE_PROVIDER_ID,
                            "account_label": "google-drive google",
                            "scopes": ["https://www.googleapis.com/auth/drive"],
                            "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
                            "invocation_id": flow_invocation_id.to_string(),
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("route response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("start json");
        let flow_id = AuthFlowId::from_uuid(
            Uuid::parse_str(json["flow_id"].as_str().expect("flow id")).expect("flow uuid"),
        );
        let flow = shared
            .get_flow(&existing_scope, flow_id)
            .await
            .expect("flow lookup")
            .expect("flow");
        let update_binding = flow
            .update_binding
            .expect("matching account should be bound for OAuth reconnect");
        assert_eq!(update_binding.account_id, account.id);
        assert_eq!(update_binding.ownership, CredentialOwnership::UserReusable);
    }

    #[tokio::test]
    async fn extension_google_oauth_start_continues_when_update_binding_lookup_is_unavailable() {
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let flow_manager: Arc<dyn AuthFlowManager> = shared.clone();
        let interaction_service: Arc<dyn AuthInteractionService> = shared.clone();
        let credential_setup_service: Arc<dyn CredentialSetupService> = shared.clone();
        let credential_account_service: Arc<dyn CredentialAccountService> = shared.clone();
        let provider_client: Arc<dyn AuthProviderClient> = shared.clone();
        let cleanup_service: Arc<dyn SecretCleanupService> = shared.clone();
        let product_auth = Arc::new(RebornProductAuthServices::new(
            flow_manager,
            interaction_service,
            credential_setup_service,
            credential_account_service,
            provider_client,
            cleanup_service,
            Arc::new(NoopDispatcher),
        ));
        let state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_test_installed_extension_lookup()
        .with_google_oauth(
            GoogleOAuthRouteConfig::new(
                "google-client.apps.googleusercontent.com",
                "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
            )
            .expect("google oauth route config"),
        );
        let app = product_auth_route_mount(state.clone())
            .protected
            .layer(axum::Extension(test_caller()));
        let flow_invocation_id = InvocationId::new();

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/google-drive/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "provider": GOOGLE_PROVIDER_ID,
                            "account_label": "google-drive google",
                            "scopes": ["https://www.googleapis.com/auth/drive"],
                            "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
                            "invocation_id": flow_invocation_id.to_string(),
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("route response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("start json");
        let flow_id = AuthFlowId::from_uuid(
            Uuid::parse_str(json["flow_id"].as_str().expect("flow id")).expect("flow uuid"),
        );
        let mut flow_resource = test_resource_scope();
        flow_resource.invocation_id = flow_invocation_id;
        let flow_scope = AuthProductScope::new(flow_resource, AuthSurface::Callback);
        let flow = shared
            .get_flow(&flow_scope, flow_id)
            .await
            .expect("flow lookup")
            .expect("flow");
        assert!(flow.update_binding.is_none());

        let authorization_url = json["authorization_url"]
            .as_str()
            .expect("authorization url");
        let state_value = Url::parse(authorization_url)
            .expect("authorization url")
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("oauth state");
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let encoded_scope = url::form_urlencoded::byte_serialize(
            "https://www.googleapis.com/auth/drive".as_bytes(),
        )
        .collect::<String>();
        let uri = format!(
            "{GOOGLE_OAUTH_CALLBACK_PATH}?state={encoded_state}&code=google-auth-code&scope={encoded_scope}"
        )
        .parse::<Uri>()
        .expect("callback uri");

        let response = oauth::google_oauth_callback_handler(
            State(state),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            HeaderMap::new(),
        )
        .await
        .expect("extension google callback should complete");

        assert_eq!(response.status(), StatusCode::OK);
        let completed_flow = shared
            .get_flow(&flow_scope, flow_id)
            .await
            .expect("completed flow lookup")
            .expect("completed flow");
        let account_id = completed_flow
            .credential_account_id
            .expect("callback should persist account id");
        let account = shared
            .get_account(CredentialAccountLookupRequest {
                scope: flow_scope,
                account_id,
                requester_extension: Some(ExtensionId::new("google-drive").expect("extension")),
            })
            .await
            .expect("account lookup")
            .expect("account");

        assert_eq!(account.status, CredentialAccountStatus::Configured);
        assert_eq!(account.provider.as_str(), GOOGLE_PROVIDER_ID);
    }

    #[tokio::test]
    async fn extension_oauth_start_handler_returns_config_error_when_dcr_registry_is_missing() {
        let state = ProductAuthRouteState::new(
            Arc::new(RebornProductAuthServices::local_dev_in_memory(Arc::new(
                NoopDispatcher,
            ))),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_test_installed_extension_lookup();
        let app = product_auth_route_mount(state)
            .protected
            .layer(axum::Extension(test_caller()));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/notion/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "provider": "notion",
                            "account_label": "work notion",
                            "scopes": [],
                            "expires_at": (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
                            "invocation_id": InvocationId::new().to_string(),
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("route response");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("error json");
        assert_eq!(json["code"], "malformed_config");
    }

    #[tokio::test]
    async fn dcr_oauth_callback_retrieves_pkce_from_registry_when_route_cache_misses() {
        let secret_store = Arc::new(InMemorySecretStore::new());
        let secret_store_for_provider: Arc<dyn SecretStore> = secret_store.clone();
        let dcr_provider = Arc::new(
            OAuthDcrProvider::new(
                OAuthDcrProviderConfig {
                    spec: notion_provider_spec(),
                    callback_origin: "http://127.0.0.1:3000".to_string(),
                    client_name: "Ironclaw".to_string(),
                    account_label: CredentialAccountLabel::new("notion").expect("label"),
                    scopes: Vec::new(),
                },
                Arc::new(PanickingDcrEgress),
                secret_store_for_provider,
                Arc::new(NoopObligationHandler),
            )
            .expect("DCR provider"),
        );
        let product_auth = RebornProductAuthServices::local_dev_in_memory(Arc::new(NoopDispatcher))
            .with_dcr_oauth_registry(Arc::new(OAuthDcrProviderRegistry::new(vec![dcr_provider])));
        let state = ProductAuthRouteState::new(
            Arc::new(product_auth),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
        let flow_id = AuthFlowId::new();
        let scope = AuthProductScope::new(test_resource_scope(), AuthSurface::Callback);
        let provider = AuthProviderId::new("notion").expect("provider");
        let verifier = "dcr-pkce-verifier";

        secret_store
            .put(
                scope.resource.clone(),
                flow_secret_handle(&notion_provider_spec(), flow_id, "pkce")
                    .expect("flow secret handle"),
                SecretMaterial::from(verifier.to_string()),
                None,
            )
            .await
            .expect("stored DCR PKCE verifier");

        let query = OAuthCallbackQuery {
            user_id: Some(scope.resource.user_id.to_string()),
            invocation_id: Some(scope.resource.invocation_id.to_string()),
            state: Some(RawCallbackValue::new("opaque-state".to_string()).expect("state")),
            provider: Some("notion".to_string()),
            account_label: Some("notion".to_string()),
            code: Some(RawSecretValue::new("oauth-code".to_string()).expect("code")),
            error: None,
            agent_id: None,
            project_id: None,
            thread_id: None,
            session_id: None,
            scopes: None,
        };

        let outcome = oauth::callback_outcome_from_query(
            &state,
            flow_id,
            &scope,
            Some(&provider),
            None,
            &query,
        )
        .await
        .expect("callback outcome");

        let RebornOAuthCallbackOutcome::Authorized { provider_request } = outcome else {
            panic!("expected authorized callback outcome");
        };
        assert_eq!(provider_request.provider, provider);
        assert_eq!(provider_request.account_label.as_str(), "notion");
        assert_eq!(provider_request.pkce_verifier.expose_secret(), verifier);
    }

    #[tokio::test]
    async fn dcr_oauth_callback_resumes_blocked_turn_gate() {
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let secret_store = Arc::new(InMemorySecretStore::new());
        let secret_store_for_provider: Arc<dyn SecretStore> = secret_store;
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let dcr_provider = Arc::new(
            OAuthDcrProvider::new(
                OAuthDcrProviderConfig {
                    spec: notion_provider_spec(),
                    callback_origin: "http://127.0.0.1:3000".to_string(),
                    client_name: "Ironclaw".to_string(),
                    account_label: CredentialAccountLabel::new("notion").expect("label"),
                    scopes: Vec::new(),
                },
                Arc::new(RouteDcrSetupEgress),
                secret_store_for_provider,
                Arc::new(NoopObligationHandler),
            )
            .expect("DCR provider"),
        );
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), dispatcher.clone())
                .with_flow_record_source(shared)
                .with_dcr_oauth_registry(Arc::new(OAuthDcrProviderRegistry::new(vec![
                    dcr_provider,
                ]))),
        );
        let state = ProductAuthRouteState::new(
            product_auth.clone(),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
        let turn_scope = TurnScope::new(
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
            ThreadId::new("thread-alpha").expect("thread"),
        );
        let owner_user_id = UserId::new("user-alpha").expect("user");
        let run_id = TurnRunId::new();
        let gate_ref = "gate:notion-auth";
        let requirements = vec![RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("notion").expect("provider"),
            setup: Default::default(),
            requester_extension: ExtensionId::new("notion").expect("extension"),
            provider_scopes: Vec::new(),
        }];

        let challenge = product_auth
            .challenge_for_gate(&turn_scope, &owner_user_id, run_id, gate_ref, &requirements)
            .await
            .expect("challenge lookup")
            .expect("notion oauth challenge");
        let authorization_url = challenge.authorization_url.expect("authorization url");
        let parsed_authorization =
            Url::parse(authorization_url.as_str()).expect("authorization URL");
        let state_value = parsed_authorization
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("oauth state");
        let redirect_uri = parsed_authorization
            .query_pairs()
            .find_map(|(name, value)| (name == "redirect_uri").then(|| value.into_owned()))
            .expect("redirect uri");
        let mut callback_url = Url::parse(&redirect_uri).expect("callback redirect URL");
        {
            let mut query = callback_url.query_pairs_mut();
            query.append_pair("state", &state_value);
            query.append_pair("code", "notion-auth-code");
        }
        let uri = format!(
            "{}?{}",
            callback_url.path(),
            callback_url.query().expect("callback query")
        )
        .parse::<Uri>()
        .expect("callback uri");
        let flow_id = callback_url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .expect("flow id")
            .to_string();

        let response = oauth::oauth_callback_handler(
            State(state),
            Path(flow_id),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            HeaderMap::new(),
        )
        .await
        .expect("notion callback");

        assert_eq!(response.status(), StatusCode::OK);
        let events = dispatcher.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].continuation,
            AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(run_id.to_string()).expect("run ref"),
                gate_ref: AuthGateRef::new(gate_ref).expect("gate ref"),
            }
        );
    }

    #[derive(Debug)]
    struct PanickingDcrEgress;

    #[async_trait]
    impl RuntimeHttpEgress for PanickingDcrEgress {
        async fn execute(
            &self,
            _request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, ironclaw_host_api::RuntimeHttpEgressError> {
            panic!("callback PKCE fallback test must not perform DCR HTTP egress")
        }
    }

    #[derive(Debug)]
    struct RouteDcrSetupEgress;

    #[async_trait]
    impl RuntimeHttpEgress for RouteDcrSetupEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, ironclaw_host_api::RuntimeHttpEgressError> {
            let body = match request.url.as_str() {
                "https://mcp.notion.com/mcp/.well-known/oauth-protected-resource" => {
                    br#"{"authorization_servers":["https://oauth.notion.com"]}"#.to_vec()
                }
                "https://oauth.notion.com/.well-known/oauth-authorization-server" => {
                    br#"{"authorization_endpoint":"https://oauth.notion.com/authorize","token_endpoint":"https://oauth.notion.com/token","registration_endpoint":"https://oauth.notion.com/register"}"#.to_vec()
                }
                "https://oauth.notion.com/register" => br#"{"client_id":"dcr-client","registration_client_uri":"https://oauth.notion.com/register/dcr-client","registration_access_token":"registration-token"}"#.to_vec(),
                "https://oauth.notion.com/register/dcr-client"
                    if request.method == NetworkMethod::Delete =>
                {
                    br#"{}"#.to_vec()
                }
                other => panic!("unexpected DCR route egress URL: {other}"),
            };
            Ok(RuntimeHttpEgressResponse {
                status: 200,
                headers: Vec::new(),
                request_bytes: request.body.len() as u64,
                response_bytes: body.len() as u64,
                body,
                saved_body: None,
                redaction_applied: false,
            })
        }
    }

    #[derive(Debug)]
    struct NoopObligationHandler;

    #[async_trait]
    impl CapabilityObligationHandler for NoopObligationHandler {
        async fn satisfy(
            &self,
            _request: CapabilityObligationRequest<'_>,
        ) -> Result<(), ironclaw_capabilities::CapabilityObligationError> {
            Ok(())
        }
    }
}
