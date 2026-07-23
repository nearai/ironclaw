//! Reborn-native product-auth route composition.
//!
//! This module owns only HTTP parsing, scope derivation from host-owned
//! composition, one-way hashing of callback material, and sanitized response
//! rendering. It deliberately delegates durable flow state, provider exchange,
//! credential mutation, and continuation dispatch to [`RebornProductAuthServices`].
// arch-exempt: large_file, product-auth serve router and DTO/route composition surface; decomposition into per-route submodules tracked by the Slack-OAuth audit, plan #5604

mod accounts;
mod lifecycle;
mod manual_token;
mod oauth;
#[cfg(test)]
mod oauth_start_tests;

use std::{
    hash::Hash,
    num::{NonZeroU32, NonZeroU64, NonZeroUsize},
    sync::{Arc, Mutex},
    time::Duration,
};

use lru::LruCache;

use axum::{
    Json, Router,
    extract::{Extension, Path, RawQuery, State},
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthFlowStatus, AuthGateRef, AuthInteractionId,
    AuthProductError, AuthProductScope, AuthProviderId, AuthSessionId, AuthSurface,
    AuthorizationCodeHash, CredentialAccountChoiceRequest, CredentialAccountId,
    CredentialAccountLabel, CredentialAccountListPage, CredentialAccountListRequest,
    CredentialAccountProjection, CredentialAccountSelectionRequest, CredentialAccountStatus,
    CredentialAccountUpdateBinding, CredentialRecoveryProjection, CredentialRecoveryRequest,
    CredentialRefreshReport, CredentialRefreshRequest, OAuthAuthorizationCode,
    OAuthAuthorizationUrl, OAuthCallbackState, OAuthCallbackStateKind,
    OAuthProviderCallbackRequest, OpaqueStateHash, PkceVerifierHash, PkceVerifierSecret,
    ProviderScope, SecretCleanupAction, SecretCleanupReport, SecretCleanupRequest, Timestamp,
    TurnRunRef, binding_scope_owns_account,
};
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor, ListenerClass,
    RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_host_api::{
    AgentId, BoundProductSurface, ExtensionId, InvocationId, ProductSurface, ProductSurfaceCaller,
    ProductSurfaceError, ProductSurfaceQueryRequest, ProjectId, ResourceScope, TenantId, ThreadId,
    UserId,
};
use ironclaw_product::{
    EXTENSION_SETUP_VIEW, EXTENSIONS_VIEW, LifecyclePackageKind, RebornExtensionCredentialSetup,
    RebornExtensionListResponse, RebornSetupExtensionResponse,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use url::Url;
use uuid::Uuid;

use crate::extension_host::channel_identity::ProviderIdentityHookFactory;
use crate::product_auth::api::auth::RebornOAuthStartFlowRequest;
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
/// One public callback per vendor, `{provider}` resolved as recipe data —
/// the path shape vendor-registered redirect URLs already point at
/// (checklist AUTH-13).
pub(crate) const VENDOR_OAUTH_CALLBACK_PATH: &str =
    "/api/reborn/product-auth/oauth/{provider}/callback";
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
const VENDOR_OAUTH_CALLBACK_ROUTE_ID: &str = "product_auth.oauth.vendor.callback";
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
const PRODUCT_AUTH_FLOW_MAX_TTL_SECONDS: i64 = 10 * 60;
const PRODUCT_AUTH_BACKEND_TIMEOUT: Duration = Duration::from_secs(30);
const OAUTH_CALLBACK_QUERY_MAX_BYTES: usize = 16 * 1024;
const OAUTH_CALLBACK_FIELD_MAX_BYTES: usize = 512;
const OAUTH_CALLBACK_SCOPES_MAX_BYTES: usize = 4 * 1024;
const RAW_OAUTH_VALUE_MAX_BYTES: usize = 4 * 1024;

#[derive(Clone)]
pub struct ProductAuthRouteState {
    product_auth: Arc<RebornProductAuthServices>,
    /// Installed-inventory guard for extension OAuth starts: a flow may be
    /// minted only for an extension the caller actually has installed
    /// (fail-closed — an unwired lookup rejects rather than skips).
    installed_extension_lookup: Option<Arc<InstalledExtensionLookup>>,
    tenant_id: TenantId,
    default_agent_id: Option<AgentId>,
    default_project_id: Option<ProjectId>,
    /// The vendor-blind post-exchange provider-identity hook — registered
    /// by composition wiring as data (the generic channel identity
    /// binding), never a vendor match arm in a handler. The factory
    /// receives the callback's vendor id and resolves what (if anything)
    /// to bind itself.
    provider_identity_hook: Option<Arc<ProviderIdentityHookFactory>>,
    // First-slice WebUI OAuth stores the raw PKCE verifier process-locally
    // because `AuthFlowRecord` deliberately serializes hashes only. Production
    // HA must replace this with a host-owned encrypted verifier store before
    // routing callbacks across replicas or restarts.
    pkce_verifiers: ExpiringLruCache<AuthFlowId, StoredPkceVerifier>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstalledExtensionOAuthRequirement {
    provider: String,
    account_label: String,
    scopes: Vec<String>,
}

/// Closed installed-extension lookup selected by composition. This remains an
/// enum rather than a mock-driven trait: production has one authoritative
/// lifecycle projection, while the scripted variant exists only in unit tests.
#[derive(Clone)]
enum InstalledExtensionLookup {
    ProductSurface(Arc<dyn ProductSurface>),
    #[cfg(test)]
    Scripted {
        extension_id: ExtensionId,
        requirement_name: String,
        requirement: InstalledExtensionOAuthRequirement,
    },
    #[cfg(test)]
    InstalledThenRemoved {
        extension_id: ExtensionId,
        requirement_name: String,
        requirement: InstalledExtensionOAuthRequirement,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    },
}

impl InstalledExtensionLookup {
    async fn is_installed(
        &self,
        caller: &ProductSurfaceCaller,
        extension_id: &ExtensionId,
    ) -> Result<bool, ProductSurfaceError> {
        match self {
            Self::ProductSurface(api) => {
                let surface = BoundProductSurface::new(Arc::clone(api), caller.clone());
                let page = surface
                    .query(ProductSurfaceQueryRequest {
                        view_id: EXTENSIONS_VIEW.id.to_string(),
                        input: json!({}),
                        cursor: None,
                        limit: None,
                    })
                    .await?;
                let payload = page
                    .items
                    .into_iter()
                    .next()
                    .ok_or_else(ProductSurfaceError::internal)?;
                let inventory: RebornExtensionListResponse =
                    serde_json::from_value(payload).map_err(ProductSurfaceError::internal_from)?;
                Ok(inventory.extensions.iter().any(|extension| {
                    extension.package_ref.kind == LifecyclePackageKind::Extension
                        && extension.package_ref.id.as_str() == extension_id.as_str()
                }))
            }
            #[cfg(test)]
            Self::Scripted {
                extension_id: installed_extension_id,
                ..
            } => Ok(extension_id == installed_extension_id),
            #[cfg(test)]
            Self::InstalledThenRemoved {
                extension_id: installed_extension_id,
                calls,
                ..
            } => Ok(extension_id == installed_extension_id
                && calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0),
        }
    }

    async fn oauth_requirement(
        &self,
        caller: &ProductSurfaceCaller,
        extension_id: &ExtensionId,
        requirement_name: &str,
    ) -> Result<Option<InstalledExtensionOAuthRequirement>, ProductSurfaceError> {
        match self {
            Self::ProductSurface(api) => {
                let surface = BoundProductSurface::new(Arc::clone(api), caller.clone());
                let page = surface
                    .query(ProductSurfaceQueryRequest {
                        view_id: EXTENSION_SETUP_VIEW.id.to_string(),
                        input: json!({ "package_id": extension_id.as_str() }),
                        cursor: None,
                        limit: None,
                    })
                    .await?;
                let payload = page
                    .items
                    .into_iter()
                    .next()
                    .ok_or_else(ProductSurfaceError::internal)?;
                let setup: RebornSetupExtensionResponse =
                    serde_json::from_value(payload).map_err(ProductSurfaceError::internal_from)?;
                Ok(setup.secrets.into_iter().find_map(|secret| {
                    if secret.name != requirement_name {
                        return None;
                    }
                    let RebornExtensionCredentialSetup::OAuth {
                        account_label,
                        scopes,
                        ..
                    } = secret.setup
                    else {
                        return None;
                    };
                    Some(InstalledExtensionOAuthRequirement {
                        provider: secret.provider,
                        account_label,
                        scopes,
                    })
                }))
            }
            #[cfg(test)]
            Self::Scripted {
                extension_id: installed_extension_id,
                requirement_name: installed_requirement_name,
                requirement,
            } => Ok((extension_id == installed_extension_id
                && requirement_name == installed_requirement_name)
                .then(|| requirement.clone())),
            #[cfg(test)]
            Self::InstalledThenRemoved {
                extension_id: installed_extension_id,
                requirement_name: installed_requirement_name,
                requirement,
                ..
            } => Ok((extension_id == installed_extension_id
                && requirement_name == installed_requirement_name)
                .then(|| requirement.clone())),
        }
    }
}

impl ProductAuthRouteState {
    pub fn new(
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
            provider_identity_hook: None,
            pkce_verifiers: ExpiringLruCache::new(
                OAUTH_PKCE_VERIFIER_CACHE_CAPACITY,
                StoredPkceVerifier::expires_at,
            ),
        }
    }

    /// Wire the product surface as the installed-extension inventory source for
    /// the extension OAuth start guard.
    pub fn with_product_surface(mut self, product_surface: Arc<dyn ProductSurface>) -> Self {
        self.installed_extension_lookup = Some(Arc::new(InstalledExtensionLookup::ProductSurface(
            product_surface,
        )));
        self
    }

    #[cfg(test)]
    fn with_test_installed_extension_lookup(mut self) -> Self {
        self.installed_extension_lookup = Some(Arc::new(InstalledExtensionLookup::Scripted {
            extension_id: ExtensionId::new("vendorco-tools").expect("test extension id"), // safety: cfg(test)-only static fixture.
            requirement_name: "vendorco_oauth".to_string(),
            requirement: InstalledExtensionOAuthRequirement {
                provider: "vendorco".to_string(),
                account_label: "vendorco-tools vendorco".to_string(),
                scopes: vec!["items:read".to_string()],
            },
        }));
        self
    }

    /// Fail-closed installed-inventory guard for extension OAuth starts: no
    /// wired lookup rejects as unavailable (never skips the check), a lookup
    /// failure rejects as unavailable, and a missing installation rejects
    /// with the terminal not-installed conflict.
    pub(super) async fn require_installed_extension(
        &self,
        caller: &ProductSurfaceCaller,
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

    /// Resolve one OAuth requirement from the installed extension's lifecycle
    /// projection. The browser supplies only the manifest requirement key;
    /// provider, label, and scopes remain server-owned data.
    async fn resolve_extension_oauth_requirement(
        &self,
        caller: &ProductSurfaceCaller,
        requester_extension: &ExtensionId,
        requirement_name: &str,
    ) -> Result<InstalledExtensionOAuthRequirement, ProductAuthRouteFailure> {
        let Some(lookup) = self.installed_extension_lookup.as_ref() else {
            return Err(ProductAuthRouteFailure::backend_unavailable());
        };
        tokio::time::timeout(
            PRODUCT_AUTH_BACKEND_TIMEOUT,
            lookup.oauth_requirement(caller, requester_extension, requirement_name),
        )
        .await
        .map_err(|_| ProductAuthRouteFailure::backend_timeout())?
        .map_err(|error| {
            tracing::warn!(
                %error,
                extension_id = %requester_extension,
                "installed extension OAuth requirement lookup failed before OAuth start"
            );
            ProductAuthRouteFailure::backend_unavailable()
        })?
        .ok_or_else(ProductAuthRouteFailure::invalid_request)
    }

    /// Register the post-exchange provider-identity hook. The handler
    /// hands it the callback's vendor id — data lookup, no vendor branch.
    pub fn with_provider_identity_hook(
        mut self,
        factory: Arc<ProviderIdentityHookFactory>,
    ) -> Self {
        self.provider_identity_hook = Some(factory);
        self
    }

    pub(super) fn provider_identity_hook(
        &self,
        vendor: &str,
        callback_scope: &AuthProductScope,
    ) -> Option<crate::product_auth::api::auth::OAuthProviderIdentityCheck> {
        self.provider_identity_hook
            .as_ref()
            .and_then(|factory| factory(vendor, callback_scope))
    }

    fn auth_engine(&self) -> Result<Arc<ironclaw_auth::AuthEngine>, ProductAuthRouteFailure> {
        self.product_auth
            .auth_engine()
            .ok_or_else(ProductAuthRouteFailure::backend_unavailable)
    }

    fn store_pkce_verifier(
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

    fn remove_pkce_verifier(&self, flow_id: AuthFlowId) {
        self.pkce_verifiers.remove(&flow_id);
    }

    /// Terminal-outcome cleanup: drop the same-process cached verifier AND
    /// the durable per-flow copy (best-effort). Early defensive cache
    /// removals (unknown flow, cross-vendor path) must NOT use this — the
    /// legitimate callback may still need the durable copy.
    async fn forget_pkce_verifier_everywhere(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        flow_id: AuthFlowId,
    ) {
        self.remove_pkce_verifier(flow_id);
        self.product_auth
            .discard_setup_pkce_verifier(scope, flow_id)
            .await;
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
            .field(
                "provider_identity_hook",
                &self.provider_identity_hook.is_some(),
            );
        builder
            .field("pkce_verifiers", &"ExpiringLruCache<...>")
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

pub struct ProductAuthRouteMount {
    pub protected: Router,
    pub public: Router,
    pub descriptors: Vec<IngressRouteDescriptor>,
}

// Product-auth HTTP is a host-owned auth/secret-ingress boundary. Its
// mutations enter `RebornProductAuthServices` directly; they are not in-turn
// tool calls and must not surface raw secrets through the model-visible
// tool-dispatch path. Contract: `docs/reborn/contracts/auth-product.md`.
// dispatch-exempt: host-owned auth/secret ingress, not in-turn tool dispatch
pub fn product_auth_route_mount(state: ProductAuthRouteState) -> ProductAuthRouteMount {
    let public = Router::new()
        .route(OAUTH_CALLBACK_PATH, get(oauth::oauth_callback_handler))
        .route(
            VENDOR_OAUTH_CALLBACK_PATH,
            get(oauth::vendor_oauth_callback_handler),
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
                EXTENSION_OAUTH_START_PATH,
                post(oauth::extension_oauth_start_handler),
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
        (EXTENSION_OAUTH_START_ROUTE_ID, EXTENSION_OAUTH_START_PATH),
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
        OAUTH_FLOW_RECONCILE_ROUTE_ID,
        NetworkMethod::Post,
        OAUTH_FLOW_RECONCILE_PATH,
        flow_reconcile_policy(),
    ));
    descriptors.push(descriptor(
        OAUTH_CALLBACK_ROUTE_ID,
        NetworkMethod::Get,
        OAUTH_CALLBACK_PATH,
        callback_policy(),
    ));
    descriptors.push(descriptor(
        VENDOR_OAUTH_CALLBACK_ROUTE_ID,
        NetworkMethod::Get,
        VENDOR_OAUTH_CALLBACK_PATH,
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

pub(super) fn flow_reconcile_policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::BearerToken],
        },
        scope_source: ironclaw_host_api::IngressScopeSource::AuthenticatedCaller,
        // The command carries no browser-selected lifecycle inputs. Its only
        // authority is the caller-scoped durable flow id plus invocation id.
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
    .expect("product-auth OAuth flow-reconcile policy must validate") // safety: authenticated LocalGateway command with no body and a bounded per-caller poll cadence.
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
#[serde(deny_unknown_fields)]
pub(super) struct ExtensionOAuthStartRequest {
    requirement: String,
    expires_at: Timestamp,
    invocation_id: Option<String>,
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
pub(super) struct VendorOAuthCallbackQuery {
    state: Option<RawCallbackValue>,
    code: Option<RawSecretValue>,
    error: Option<String>,
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

    /// The requested extension is not in the caller's installed inventory —
    /// terminal for this start attempt (409, not retryable).
    fn extension_not_installed() -> Self {
        Self::new(StatusCode::CONFLICT, AuthErrorCode::InvalidRequest)
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
        AuthErrorCode::LifecycleActivationFailed => StatusCode::CONFLICT,
        AuthErrorCode::BackendUnavailable | AuthErrorCode::MalformedConfig => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        AuthErrorCode::TokenExchangeFailed | AuthErrorCode::RefreshFailed => {
            StatusCode::BAD_GATEWAY
        }
        AuthErrorCode::CredentialMissing
        | AuthErrorCode::AccountSelectionRequired
        | AuthErrorCode::ProviderIdentityAlreadyConnected => StatusCode::CONFLICT,
    };
    ProductAuthRouteFailure {
        status,
        body: error,
    }
}

pub(super) fn scope_from_authenticated_caller(
    caller: &ProductSurfaceCaller,
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
    caller: &ProductSurfaceCaller,
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
    caller: &ProductSurfaceCaller,
    fields: &ScopeFields,
) -> Result<AuthProductScope, ProductAuthRouteFailure> {
    if fields.invocation_id.is_none() {
        return Err(ProductAuthRouteFailure::invalid_request());
    }
    scope_from_authenticated_caller_parts(caller, fields)
}

pub(super) async fn scoped_update_binding_for_requester(
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
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, header};
    use ironclaw_auth::{
        AuthChallenge, AuthFlowKind, AuthFlowManager, AuthInteractionService, AuthProviderClient,
        CredentialAccountLookupRequest, CredentialAccountService, CredentialAccountStatus,
        CredentialSetupService, NewAuthFlow, OAuthCallbackInput, OAuthProviderExchange,
        ProviderCallbackOutcome, SecretCleanupService,
    };
    use ironclaw_host_api::{
        NetworkMethod, RuntimeCredentialAuthRequirement, RuntimeHttpEgress,
        RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, SecretHandle, VendorId,
    };
    use ironclaw_product::AuthChallengeProvider;
    use ironclaw_secrets::{FilesystemSecretStore, SecretMaterial, SecretStore};
    use ironclaw_turns::{TurnRunId, TurnScope};
    use std::sync::Mutex;
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

    #[test]
    fn flow_reconcile_route_descriptor_locks_authenticated_no_body_policy() {
        let descriptors = product_auth_route_descriptors();
        let reconcile = descriptors
            .iter()
            .find(|descriptor| descriptor.route_id().as_str() == OAUTH_FLOW_RECONCILE_ROUTE_ID)
            .expect("the flow-reconcile descriptor must be registered");

        assert_eq!(reconcile.method(), NetworkMethod::Post);
        let policy = reconcile.policy();
        assert!(matches!(policy.body_limit(), BodyLimitPolicy::NoBody));
        assert!(matches!(
            policy.auth(),
            IngressAuthPolicy::Required { schemes }
                if schemes.contains(&IngressAuthScheme::BearerToken)
        ));
        assert_eq!(
            policy.scope_source(),
            ironclaw_host_api::IngressScopeSource::AuthenticatedCaller
        );
        assert!(matches!(
            policy.rate_limit(),
            RateLimitPolicy::Limited {
                scope: RateLimitScope::PerCaller,
                ..
            }
        ));
        assert_eq!(policy.cors(), CorsPolicy::SameOriginOnly);
    }

    async fn completed_reconcile_flow(
        services: &ironclaw_auth::InMemoryAuthProductServices,
        scope: &AuthProductScope,
        tag: &str,
    ) -> ironclaw_auth::AuthFlowRecord {
        let provider = AuthProviderId::new(format!("route-reconcile-{tag}")).expect("provider");
        let state_hash = opaque_state_hash(&format!("state-{tag}")).expect("state hash");
        let verifier_hash = pkce_verifier_hash(&format!("verifier-{tag}")).expect("PKCE hash");
        let flow = services
            .create_flow(NewAuthFlow {
                id: None,
                scope: scope.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: provider.clone(),
                challenge: AuthChallenge::SetupRequired {
                    provider: provider.clone(),
                    message: "route reconciliation test".to_string(),
                },
                continuation: AuthContinuationRef::SetupOnly,
                update_binding: None,
                opaque_state_hash: Some(state_hash.clone()),
                pkce_verifier_hash: Some(verifier_hash.clone()),
                expires_at: Utc::now() + ChronoDuration::minutes(5),
            })
            .await
            .expect("create flow");
        services
            .complete_oauth_callback(
                scope,
                OAuthCallbackInput {
                    flow_id: flow.id,
                    opaque_state_hash: state_hash,
                    outcome: ProviderCallbackOutcome::Authorized {
                        exchange: Box::new(OAuthProviderExchange {
                            provider,
                            account_label: CredentialAccountLabel::new(format!("account-{tag}"))
                                .expect("account label"),
                            authorization_code_hash: AuthorizationCodeHash::new(sha256_hex(
                                &format!("code-{tag}"),
                            ))
                            .expect("authorization code hash"),
                            pkce_verifier_hash: verifier_hash,
                            access_secret: SecretHandle::new(format!("access-{tag}"))
                                .expect("access handle"),
                            refresh_secret: None,
                            scopes: Vec::new(),
                            account_id: None,
                            provider_identity: None,
                        }),
                    },
                },
            )
            .await
            .expect("complete flow")
    }

    fn reconcile_route_state(
        shared: &Arc<ironclaw_auth::InMemoryAuthProductServices>,
        dispatcher: Arc<RecordingDispatcher>,
    ) -> ProductAuthRouteState {
        let flow_manager: Arc<dyn AuthFlowManager> = shared.clone();
        let interaction_service: Arc<dyn AuthInteractionService> = shared.clone();
        let credential_setup_service: Arc<dyn CredentialSetupService> = shared.clone();
        let credential_account_service: Arc<dyn CredentialAccountService> = shared.clone();
        let provider_client: Arc<dyn AuthProviderClient> = shared.clone();
        let cleanup_service: Arc<dyn SecretCleanupService> = shared.clone();
        ProductAuthRouteState::new(
            Arc::new(RebornProductAuthServices::new(
                flow_manager,
                interaction_service,
                credential_setup_service,
                credential_account_service,
                provider_client,
                cleanup_service,
                dispatcher,
            )),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
    }

    fn reconcile_uri(flow: &ironclaw_auth::AuthFlowRecord) -> String {
        format!(
            "/api/reborn/product-auth/oauth/flow/{}/reconcile?invocation_id={}",
            flow.id, flow.scope.resource.invocation_id
        )
    }

    #[tokio::test]
    async fn flow_reconcile_route_dispatches_only_one_unfenced_completed_continuation() {
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let state = reconcile_route_state(&shared, dispatcher.clone());
        let mut resource = test_resource_scope();
        resource.invocation_id = InvocationId::new();
        let scope = AuthProductScope::new(resource, AuthSurface::Callback);
        let flow = completed_reconcile_flow(shared.as_ref(), &scope, "once").await;
        let app = product_auth_route_mount(state)
            .protected
            .layer(axum::Extension(test_caller()));

        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri(reconcile_uri(&flow))
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("route response");
            assert_eq!(response.status(), StatusCode::OK);
        }

        assert_eq!(dispatcher.events().len(), 1);
        let stored = shared
            .get_flow(&scope, flow.id)
            .await
            .expect("flow lookup")
            .expect("flow");
        assert!(stored.continuation_emitted_at.is_some());
    }

    #[tokio::test]
    async fn flow_reconcile_route_does_not_dispatch_fenced_terminal_or_foreign_flows() {
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let state = reconcile_route_state(&shared, dispatcher.clone());
        let mut resource = test_resource_scope();
        resource.invocation_id = InvocationId::new();
        let scope = AuthProductScope::new(resource, AuthSurface::Callback);

        let fenced = completed_reconcile_flow(shared.as_ref(), &scope, "fenced").await;
        shared
            .mark_continuation_dispatched(&scope, fenced.id, Utc::now())
            .await
            .expect("fence flow");
        let terminal = shared
            .create_flow(NewAuthFlow {
                id: None,
                scope: scope.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: AuthProviderId::new("route-reconcile-terminal").expect("provider"),
                challenge: AuthChallenge::SetupRequired {
                    provider: AuthProviderId::new("route-reconcile-terminal").expect("provider"),
                    message: "terminal route reconciliation test".to_string(),
                },
                continuation: AuthContinuationRef::SetupOnly,
                update_binding: None,
                opaque_state_hash: None,
                pkce_verifier_hash: None,
                expires_at: Utc::now() + ChronoDuration::minutes(5),
            })
            .await
            .expect("create terminal flow");
        let terminal = shared
            .cancel_flow(&scope, terminal.id)
            .await
            .expect("cancel flow");

        let owner_app = product_auth_route_mount(state.clone())
            .protected
            .layer(axum::Extension(test_caller()));
        for flow in [&fenced, &terminal] {
            let response = owner_app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri(reconcile_uri(flow))
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("route response");
            assert_eq!(response.status(), StatusCode::OK);
        }

        let foreign = completed_reconcile_flow(shared.as_ref(), &scope, "foreign").await;
        let foreign_caller = ProductSurfaceCaller::new(
            TenantId::new("tenant-alpha").expect("tenant"),
            UserId::new("user-beta").expect("user"),
            None,
            None,
        );
        let response = product_auth_route_mount(state)
            .protected
            .layer(axum::Extension(foreign_caller))
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(reconcile_uri(&foreign))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("route response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(dispatcher.events().is_empty());
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
        async fn dispatch_canceled_auth_continuation(
            &self,
            _event: ironclaw_auth::AuthContinuationEvent,
        ) -> Result<(), ironclaw_auth::AuthProductError> {
            Ok(())
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
        async fn dispatch_canceled_auth_continuation(
            &self,
            _event: ironclaw_auth::AuthContinuationEvent,
        ) -> Result<(), ironclaw_auth::AuthProductError> {
            Ok(())
        }
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

    fn test_caller() -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            TenantId::new("tenant-alpha").expect("tenant"),
            UserId::new("user-alpha").expect("user"),
            None,
            None,
        )
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

    /// Shared engine-backed test rig: a synthetic vendor recipe (statically
    /// credentialed or DCR) + scripted vendor egress behind the real engine.
    fn test_vendor_recipe(
        with_client_credentials: bool,
        resource: Option<&str>,
    ) -> ironclaw_auth::ResolvedVendorAuthRecipe {
        let mut recipe = serde_json::json!({
            "method": "oauth2_code",
            "display_name": "Vendor account",
            "authorization_endpoint": "https://auth.vendorco.example/authorize",
            "token_endpoint": "https://auth.vendorco.example/token",
            "scopes": ["items:read"],
            "token_response": {
                "access_token": "/access_token",
                "scope": { "path": "/scope", "missing": "fallback_to_requested" }
            },
        });
        if with_client_credentials {
            recipe["client_credentials"] =
                serde_json::json!({ "client_id_handle": "vendorco_oauth_client_id" });
        }
        ironclaw_auth::ResolvedVendorAuthRecipe {
            vendor: "vendorco".to_string(),
            recipe: serde_json::from_value(recipe).expect("test recipe parses"),
            token_exchange_resource: resource.map(str::to_string),
        }
    }

    #[derive(Debug)]
    struct StaticTestCredentials;

    #[async_trait]
    impl ironclaw_auth::EngineClientCredentialsSource for StaticTestCredentials {
        async fn resolve(
            &self,
            _vendor: &str,
            _credentials: &ironclaw_host_api::RecipeClientCredentials,
        ) -> Result<ironclaw_auth::EngineOAuthClientMaterial, AuthProductError> {
            Ok(ironclaw_auth::EngineOAuthClientMaterial {
                client_id: ironclaw_auth::OAuthClientId::new("vendorco-client-id")?,
                client_secret: None,
            })
        }
    }

    fn test_engine(
        recipe: ironclaw_auth::ResolvedVendorAuthRecipe,
        egress: Arc<dyn RuntimeHttpEgress>,
        secret_store: Arc<dyn SecretStore>,
    ) -> Arc<ironclaw_auth::AuthEngine> {
        Arc::new(ironclaw_auth::AuthEngine::new(
            ironclaw_auth::AuthEngineDeps {
                recipes: Arc::new(ironclaw_auth::StaticAuthRecipeResolver::new(vec![recipe])),
                client_credentials: Arc::new(StaticTestCredentials),
                egress,
                secret_store,
                callback_base: ironclaw_auth::EngineCallbackBase::new(
                    "http://127.0.0.1:3000/api/reborn/product-auth/oauth",
                )
                .expect("callback base"),
                dcr_client_name: "Ironclaw".to_string(),
            },
        ))
    }

    /// A recipe without `client_credentials` declares dynamic client
    /// registration: the start route triggers discovery + registration once
    /// and the authorize URL uses the DISCOVERED endpoint with the static
    /// vendor callback as its redirect (AUTH-13).
    #[tokio::test]
    async fn extension_oauth_start_runs_dcr_for_recipes_without_client_credentials() {
        let engine = test_engine(
            test_vendor_recipe(false, Some("https://mcp.vendorco.example/mcp")),
            Arc::new(RouteDcrSetupEgress),
            Arc::new(FilesystemSecretStore::ephemeral()),
        );
        let product_auth = RebornProductAuthServices::local_dev_in_memory(Arc::new(NoopDispatcher))
            .with_auth_engine(engine);
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
                    .uri("/api/webchat/v2/extensions/vendorco-tools/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "requirement": "vendorco_oauth",
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
        assert_eq!(json["provider"], "vendorco");
        assert_eq!(json["continuation"]["type"], "lifecycle_activation");
        assert_eq!(json["continuation"]["package_ref"], "vendorco-tools");
        let authorization_url = json["authorization_url"]
            .as_str()
            .expect("authorization url");
        let parsed = url::Url::parse(authorization_url).expect("authorization URL");
        assert_eq!(
            parsed.host_str(),
            Some("oauth.vendorco.example"),
            "authorize URL uses the DISCOVERED endpoint, not the manifest placeholder"
        );
        let redirect_uri = parsed
            .query_pairs()
            .find_map(|(name, value)| (name == "redirect_uri").then(|| value.into_owned()))
            .expect("redirect uri");
        assert_eq!(
            redirect_uri, "http://127.0.0.1:3000/api/reborn/product-auth/oauth/vendorco/callback",
            "the registered redirect is the static vendor callback path"
        );
    }

    /// The full serve-tier round trip on the generic routes: start → vendor
    /// callback → grant persisted — including that a start whose
    /// account-binding lookup is unavailable still proceeds without a binding.
    #[tokio::test]
    async fn vendor_oauth_callback_completes_a_started_flow() {
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let flow_manager: Arc<dyn AuthFlowManager> = shared.clone();
        let interaction_service: Arc<dyn AuthInteractionService> = shared.clone();
        let credential_setup_service: Arc<dyn CredentialSetupService> = shared.clone();
        let credential_account_service: Arc<dyn CredentialAccountService> = shared.clone();
        let provider_client: Arc<dyn AuthProviderClient> = shared.clone();
        let cleanup_service: Arc<dyn SecretCleanupService> = shared.clone();
        let engine = test_engine(
            test_vendor_recipe(true, None),
            Arc::new(PanickingDcrEgress),
            Arc::new(FilesystemSecretStore::ephemeral()),
        );
        // `RebornProductAuthServices::new` wires no account record source: the
        // update-binding lookup is unavailable and the start must proceed
        // without a binding rather than failing.
        let product_auth = Arc::new(
            RebornProductAuthServices::new(
                flow_manager,
                interaction_service,
                credential_setup_service,
                credential_account_service,
                provider_client,
                cleanup_service,
                Arc::new(NoopDispatcher),
            )
            .with_auth_engine(engine),
        );
        let state = ProductAuthRouteState::new(
            product_auth,
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_test_installed_extension_lookup();
        let app = product_auth_route_mount(state.clone())
            .protected
            .layer(axum::Extension(test_caller()));
        let flow_invocation_id = InvocationId::new();

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/vendorco-tools/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "requirement": "vendorco_oauth",
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
        let uri = format!(
            "/api/reborn/product-auth/oauth/vendorco/callback?state={encoded_state}&code=vendor-auth-code&scope=items:read"
        )
        .parse::<Uri>()
        .expect("callback uri");

        let response = oauth::vendor_oauth_callback_handler(
            State(state),
            Path("vendorco".to_string()),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            HeaderMap::new(),
        )
        .await
        .expect("vendor callback should complete");

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
                requester_extension: Some(ExtensionId::new("vendorco-tools").expect("extension")),
            })
            .await
            .expect("account lookup")
            .expect("account");

        assert_eq!(account.status, CredentialAccountStatus::Configured);
        assert_eq!(account.provider.as_str(), "vendorco");
    }

    /// Restart/replica regression for the durable setup-PKCE port: the
    /// process-local verifier cache dies with the route state, so a callback
    /// arriving after a restart (or on another replica) can only complete
    /// through the per-flow verifier `start_setup_oauth_flow` wrote to the
    /// injected secret store before creating the flow.
    #[tokio::test]
    async fn vendor_oauth_callback_completes_after_route_state_restart() {
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let flow_manager: Arc<dyn AuthFlowManager> = shared.clone();
        let interaction_service: Arc<dyn AuthInteractionService> = shared.clone();
        let credential_setup_service: Arc<dyn CredentialSetupService> = shared.clone();
        let credential_account_service: Arc<dyn CredentialAccountService> = shared.clone();
        let provider_client: Arc<dyn AuthProviderClient> = shared.clone();
        let cleanup_service: Arc<dyn SecretCleanupService> = shared.clone();
        let engine = test_engine(
            test_vendor_recipe(true, None),
            Arc::new(PanickingDcrEgress),
            Arc::new(FilesystemSecretStore::ephemeral()),
        );
        let product_auth = Arc::new(
            RebornProductAuthServices::new(
                flow_manager,
                interaction_service,
                credential_setup_service,
                credential_account_service,
                provider_client,
                cleanup_service,
                Arc::new(NoopDispatcher),
            )
            .with_auth_engine(engine),
        );
        let started_state = ProductAuthRouteState::new(
            Arc::clone(&product_auth),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_test_installed_extension_lookup();
        let app = product_auth_route_mount(started_state)
            .protected
            .layer(axum::Extension(test_caller()));
        let flow_invocation_id = InvocationId::new();

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/vendorco-tools/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "requirement": "vendorco_oauth",
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
        let uri = format!(
            "/api/reborn/product-auth/oauth/vendorco/callback?state={encoded_state}&code=vendor-auth-code&scope=items:read"
        )
        .parse::<Uri>()
        .expect("callback uri");

        // Simulated restart: a FRESH route state over the same product-auth
        // services. Its process-local PKCE cache is empty by construction.
        let restarted_state = ProductAuthRouteState::new(
            Arc::clone(&product_auth),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        )
        .with_test_installed_extension_lookup();

        let response = oauth::vendor_oauth_callback_handler(
            State(restarted_state),
            Path("vendorco".to_string()),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            HeaderMap::new(),
        )
        .await
        .expect("callback must complete from the durable setup verifier");
        assert_eq!(response.status(), StatusCode::OK);

        let mut flow_resource = test_resource_scope();
        flow_resource.invocation_id = flow_invocation_id;
        let flow_scope = AuthProductScope::new(flow_resource, AuthSurface::Callback);
        let completed_flow = shared
            .get_flow(&flow_scope, flow_id)
            .await
            .expect("completed flow lookup")
            .expect("completed flow");
        assert_eq!(completed_flow.status, AuthFlowStatus::Completed);
        assert!(
            completed_flow.credential_account_id.is_some(),
            "callback should persist an account id"
        );
    }

    #[tokio::test]
    async fn extension_oauth_start_fails_closed_without_a_composed_engine() {
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
                    .uri("/api/webchat/v2/extensions/vendorco-tools/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "requirement": "vendorco_oauth",
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
        assert_eq!(json["code"], "backend_unavailable");
    }

    /// The installed-inventory guard is fail-closed: a state without a wired
    /// lookup rejects the start rather than skipping the check.
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

    /// An uninstall racing the start (installed at the pre-check, gone at
    /// the post-check) must abort the just-started flow: the caller gets the
    /// terminal not-installed conflict and the minted flow is canceled so a
    /// late callback cannot complete it.
    #[tokio::test]
    async fn extension_oauth_start_aborts_the_started_flow_when_uninstall_races() {
        let engine = test_engine(
            test_vendor_recipe(true, None),
            Arc::new(PanickingDcrEgress),
            Arc::new(FilesystemSecretStore::ephemeral()),
        );
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let product_auth =
            RebornProductAuthServices::from_shared(shared.clone(), Arc::new(NoopDispatcher))
                .with_auth_engine(engine);
        let mut state = ProductAuthRouteState::new(
            Arc::new(product_auth),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
        state.installed_extension_lookup =
            Some(Arc::new(InstalledExtensionLookup::InstalledThenRemoved {
                extension_id: ExtensionId::new("vendorco-tools").expect("extension"),
                requirement_name: "vendorco_oauth".to_string(),
                requirement: InstalledExtensionOAuthRequirement {
                    provider: "vendorco".to_string(),
                    account_label: "vendorco-tools vendorco".to_string(),
                    scopes: vec!["items:read".to_string()],
                },
                calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }));
        let app = product_auth_route_mount(state)
            .protected
            .layer(axum::Extension(test_caller()));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/webchat/v2/extensions/vendorco-tools/setup/oauth/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "requirement": "vendorco_oauth",
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

    /// The callback retrieves the PKCE verifier from the durable gate store
    /// when the route-local cache misses (cross-process callback).
    #[tokio::test]
    async fn oauth_callback_retrieves_pkce_from_gate_store_when_route_cache_misses() {
        let secret_store = Arc::new(FilesystemSecretStore::ephemeral());
        let engine = test_engine(
            test_vendor_recipe(true, None),
            Arc::new(PanickingDcrEgress),
            Arc::new(FilesystemSecretStore::ephemeral()),
        );
        let driver = Arc::new(
            crate::product_auth::oauth::oauth_gate::OAuthGateFlowDriver::new(
                engine.clone(),
                secret_store.clone() as Arc<dyn SecretStore>,
            ),
        );
        let product_auth = RebornProductAuthServices::local_dev_in_memory(Arc::new(NoopDispatcher))
            .with_auth_engine(engine)
            .with_oauth_gate_driver(driver);
        let state = ProductAuthRouteState::new(
            Arc::new(product_auth),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
        let flow_id = AuthFlowId::new();
        let scope = AuthProductScope::new(test_resource_scope(), AuthSurface::Callback);
        let provider = AuthProviderId::new("vendorco").expect("provider");
        let verifier = "gate-pkce-verifier";

        secret_store
            .put(
                scope.resource.clone(),
                SecretHandle::new(format!("oauth-gate-flow-pkce-{flow_id}"))
                    .expect("gate pkce handle"),
                SecretMaterial::from(verifier.to_string()),
                None,
            )
            .await
            .expect("stored gate PKCE verifier");

        let query = OAuthCallbackQuery {
            user_id: Some(scope.resource.user_id.to_string()),
            invocation_id: Some(scope.resource.invocation_id.to_string()),
            state: Some(RawCallbackValue::new("opaque-state".to_string()).expect("state")),
            provider: Some("vendorco".to_string()),
            account_label: Some("vendorco".to_string()),
            code: Some(RawSecretValue::new("oauth-code".to_string()).expect("code")),
            error: None,
            agent_id: None,
            project_id: None,
            thread_id: None,
            session_id: None,
            scopes: None,
        };

        let outcome =
            oauth::callback_outcome_from_query(&state, flow_id, &scope, Some(&provider), &query)
                .await
                .expect("callback outcome");

        let RebornOAuthCallbackOutcome::Authorized { provider_request } = outcome else {
            panic!("expected authorized callback outcome");
        };
        assert_eq!(provider_request.provider, provider);
        assert_eq!(provider_request.pkce_verifier.expose_secret(), verifier);
    }

    /// The blocked-turn gate round trip on the generic paths: gate challenge
    /// (recipe-driven driver) → vendor callback → continuation dispatched.
    #[tokio::test]
    async fn vendor_oauth_callback_resumes_blocked_turn_gate() {
        let shared = Arc::new(ironclaw_auth::InMemoryAuthProductServices::new());
        let secret_store: Arc<dyn SecretStore> = Arc::new(FilesystemSecretStore::ephemeral());
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let engine = test_engine(
            test_vendor_recipe(true, None),
            Arc::new(ScriptedTokenExchangeEgress),
            Arc::new(FilesystemSecretStore::ephemeral()),
        );
        let driver = Arc::new(
            crate::product_auth::oauth::oauth_gate::OAuthGateFlowDriver::new(
                engine.clone(),
                Arc::clone(&secret_store),
            ),
        );
        let product_auth = Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), dispatcher.clone())
                .with_flow_record_source(shared)
                .with_provider_client(engine.clone() as Arc<dyn AuthProviderClient>)
                .with_auth_engine(engine)
                .with_oauth_gate_driver(driver),
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
        let gate_ref = "gate:vendor-auth";
        let requirements = vec![RuntimeCredentialAuthRequirement {
            provider: VendorId::new("vendorco").expect("provider"),
            setup: Default::default(),
            requester_extension: ExtensionId::new("vendorco-tools").expect("extension"),
            provider_scopes: vec!["items:read".to_string()],
        }];

        let challenge = product_auth
            .challenge_for_gate(&turn_scope, &owner_user_id, run_id, gate_ref, &requirements)
            .await
            .expect("challenge lookup")
            .expect("vendor oauth challenge");
        let authorization_url = challenge.authorization_url.expect("authorization url");
        let parsed_authorization =
            Url::parse(authorization_url.as_str()).expect("authorization URL");
        let state_value = parsed_authorization
            .query_pairs()
            .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
            .expect("oauth state");
        let encoded_state =
            url::form_urlencoded::byte_serialize(state_value.as_bytes()).collect::<String>();
        let uri = format!(
            "/api/reborn/product-auth/oauth/vendorco/callback?state={encoded_state}&code=vendor-auth-code"
        )
        .parse::<Uri>()
        .expect("callback uri");

        let response = oauth::vendor_oauth_callback_handler(
            State(state),
            Path("vendorco".to_string()),
            RawQuery(uri.query().map(str::to_string)),
            uri,
            HeaderMap::new(),
        )
        .await
        .expect("vendor callback");

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
            panic!("this test must not perform vendor HTTP egress")
        }
    }

    /// Scripted vendor egress: DCR discovery/registration for the synthetic
    /// vendor's resource server.
    #[derive(Debug)]
    struct RouteDcrSetupEgress;

    #[async_trait]
    impl RuntimeHttpEgress for RouteDcrSetupEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, ironclaw_host_api::RuntimeHttpEgressError> {
            let body = match request.url.as_str() {
                "https://mcp.vendorco.example/mcp/.well-known/oauth-protected-resource" => {
                    br#"{"authorization_servers":["https://oauth.vendorco.example"]}"#.to_vec()
                }
                "https://oauth.vendorco.example/.well-known/oauth-authorization-server" => {
                    br#"{"authorization_endpoint":"https://oauth.vendorco.example/authorize","token_endpoint":"https://oauth.vendorco.example/token","registration_endpoint":"https://oauth.vendorco.example/register"}"#.to_vec()
                }
                "https://oauth.vendorco.example/register" => {
                    br#"{"client_id":"dcr-client"}"#.to_vec()
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

    /// Scripted vendor token endpoint for exchange-completing tests.
    #[derive(Debug)]
    struct ScriptedTokenExchangeEgress;

    #[async_trait]
    impl RuntimeHttpEgress for ScriptedTokenExchangeEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, ironclaw_host_api::RuntimeHttpEgressError> {
            assert_eq!(request.url, "https://auth.vendorco.example/token");
            let body = br#"{"access_token":"vendor-access-token","scope":"items:read"}"#.to_vec();
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
}
