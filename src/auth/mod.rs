pub mod oauth;
pub mod providers;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Weak};
use std::time::Duration;

use ironclaw_common::ExtensionName;
use serde::{Deserialize, Serialize};

use crate::db::{SettingsStore, UserStore};
use crate::secrets::{CreateSecretParams, DecryptedSecret, SecretError, SecretsStore};
use crate::tools::wasm::OAuthRefreshConfig;
use crate::tools::wasm::{ssrf_safe_client_builder_for_target, validate_and_resolve_http_target};

const AUTH_DESCRIPTORS_SETTING_KEY: &str = "auth.descriptors_v1";

/// TTL for cached auth-descriptor maps. Bounded so that:
/// - a deleted/suspended user's descriptors fall out of cache within the
///   window even if no explicit invalidation hook fires;
/// - the cache cannot grow unboundedly across long-lived processes — every
///   `load_auth_descriptors` call also evicts entries past TTL on its way
///   through.
const AUTH_DESCRIPTOR_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Clone)]
struct CachedDescriptors {
    descriptors: HashMap<String, AuthDescriptor>,
    inserted_at: std::time::Instant,
}

fn auth_descriptor_cache() -> &'static tokio::sync::Mutex<HashMap<String, CachedDescriptors>> {
    static CACHE: std::sync::OnceLock<tokio::sync::Mutex<HashMap<String, CachedDescriptors>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

/// Drop the cached auth descriptors for `user_id`. Call this when a user is
/// deleted/suspended/has their descriptors mutated by an out-of-band path so
/// the in-process cache doesn't hand back stale entries until TTL expires.
pub async fn invalidate_auth_descriptor_cache(user_id: &str) {
    auth_descriptor_cache().lock().await.remove(user_id);
}

/// Drop ALL cached auth descriptors. Used in tests and on settings-store
/// reconfiguration.
#[cfg(test)]
pub async fn clear_auth_descriptor_cache() {
    auth_descriptor_cache().lock().await.clear();
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthDescriptorKind {
    SkillCredential,
    WasmTool,
    WasmChannel,
    McpServer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OAuthFlowDescriptor {
    pub authorization_url: String,
    pub token_url: String,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_id_env: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub client_secret_env: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub use_pkce: bool,
    #[serde(default)]
    pub extra_params: HashMap<String, String>,
    #[serde(default = "default_access_token_field")]
    pub access_token_field: String,
    #[serde(default)]
    pub validation_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthDescriptor {
    pub kind: AuthDescriptorKind,
    pub secret_name: String,
    pub integration_name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub setup_url: Option<String>,
    #[serde(default)]
    pub oauth: Option<OAuthFlowDescriptor>,
}

pub struct PendingOAuthLaunch {
    pub auth_url: String,
    pub expected_state: String,
    pub flow: crate::auth::oauth::PendingOAuthFlow,
}

pub struct PendingOAuthLaunchParams {
    pub extension_name: ExtensionName,
    pub display_name: String,
    pub authorization_url: String,
    pub token_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    pub access_token_field: String,
    pub secret_name: String,
    pub provider: Option<String>,
    pub validation_endpoint: Option<crate::tools::wasm::ValidationEndpointSchema>,
    pub scopes: Vec<String>,
    pub use_pkce: bool,
    pub extra_params: HashMap<String, String>,
    pub user_id: String,
    pub secrets: Arc<dyn SecretsStore + Send + Sync>,
    pub sse_manager: Option<Arc<crate::channels::web::sse::SseManager>>,
    pub gateway_token: Option<String>,
    pub token_exchange_extra_params: HashMap<String, String>,
    pub client_id_secret_name: Option<String>,
    pub client_secret_secret_name: Option<String>,
    pub client_secret_expires_at: Option<u64>,
    pub auto_activate_extension: bool,
}

fn default_access_token_field() -> String {
    "access_token".to_string()
}

pub fn build_pending_oauth_launch(params: PendingOAuthLaunchParams) -> PendingOAuthLaunch {
    let oauth_result = oauth::build_oauth_url(
        &params.authorization_url,
        &params.client_id,
        &params.redirect_uri,
        &params.scopes,
        params.use_pkce,
        &params.extra_params,
    );

    let flow = crate::auth::oauth::PendingOAuthFlow {
        extension_name: params.extension_name,
        display_name: params.display_name,
        token_url: params.token_url,
        client_id: params.client_id,
        client_secret: params.client_secret,
        redirect_uri: params.redirect_uri,
        code_verifier: oauth_result.code_verifier.clone(),
        access_token_field: params.access_token_field,
        secret_name: params.secret_name,
        provider: params.provider,
        validation_endpoint: params.validation_endpoint,
        scopes: params.scopes,
        user_id: params.user_id,
        secrets: params.secrets,
        sse_manager: params.sse_manager,
        gateway_token: params.gateway_token,
        token_exchange_extra_params: params.token_exchange_extra_params,
        client_id_secret_name: params.client_id_secret_name,
        client_secret_secret_name: params.client_secret_secret_name,
        client_secret_expires_at: params.client_secret_expires_at,
        created_at: std::time::Instant::now(),
        auto_activate_extension: params.auto_activate_extension,
    };

    PendingOAuthLaunch {
        auth_url: oauth_result.url,
        expected_state: oauth_result.state,
        flow,
    }
}

async fn load_auth_descriptors(
    store: &dyn SettingsStore,
    user_id: &str,
) -> Result<HashMap<String, AuthDescriptor>, crate::error::DatabaseError> {
    // NOTE: `user_id == "default"` is a *legitimate* value in single-tenant
    // deployments where `Config::owner_id` defaults to "default". The
    // multi-tenant safety concern this module guards against is *implicit*
    // global fallback reads, not single-user owners. The
    // `DefaultFallback::AdminOnly` policy in `resolve_secret_for_runtime` is
    // what enforces the actual cross-tenant boundary.
    let cache = auth_descriptor_cache();
    let now = std::time::Instant::now();
    {
        let mut guard = cache.lock().await;
        // Evict expired entries opportunistically so the cache cannot grow
        // unboundedly across long-lived processes.
        guard.retain(|_, entry| now.duration_since(entry.inserted_at) < AUTH_DESCRIPTOR_CACHE_TTL);
        if let Some(entry) = guard.get(user_id) {
            return Ok(entry.descriptors.clone());
        }
    }

    let descriptors = match store
        .get_setting(user_id, AUTH_DESCRIPTORS_SETTING_KEY)
        .await?
    {
        Some(value) => serde_json::from_value(value)
            .map_err(|error| crate::error::DatabaseError::Query(error.to_string())),
        None => Ok(HashMap::new()),
    }?;

    cache.lock().await.insert(
        user_id.to_string(),
        CachedDescriptors {
            descriptors: descriptors.clone(),
            inserted_at: std::time::Instant::now(),
        },
    );
    Ok(descriptors)
}

pub async fn auth_descriptor_for_secret(
    store: Option<&dyn SettingsStore>,
    user_id: &str,
    secret_name: &str,
) -> Option<AuthDescriptor> {
    let store = store?;
    match load_auth_descriptors(store, user_id).await {
        Ok(descriptors) => descriptors.get(&secret_name.to_lowercase()).cloned(),
        Err(error) => {
            tracing::warn!(
                user_id = %user_id,
                secret_name = %secret_name,
                error = %error,
                "Failed to load auth descriptors"
            );
            None
        }
    }
}

/// Per-user serialization lock for `upsert_auth_descriptor`. Without this,
/// two concurrent upserts for the same `user_id` can lose updates: each loads
/// the same base descriptor map from the DB, mutates only its own entry, and
/// the second writer's `set_setting` overwrites the first writer's descriptor.
async fn upsert_lock(user_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        tokio::sync::Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    let registry = LOCKS.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()));
    let mut locks = registry.lock().await;
    if let Some(lock) = locks.get(user_id).and_then(Weak::upgrade) {
        return lock;
    }
    locks.retain(|_, lock| lock.strong_count() > 0);
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(user_id.to_string(), Arc::downgrade(&lock));
    lock
}

pub async fn upsert_auth_descriptor(
    store: Option<&dyn SettingsStore>,
    user_id: &str,
    descriptor: AuthDescriptor,
) {
    let Some(store) = store else {
        return;
    };

    // Hold a per-user lock for the entire load → mutate → persist → cache
    // update cycle so concurrent upserts cannot drop each other's changes.
    let lock = upsert_lock(user_id).await;
    let _guard = lock.lock().await;

    let mut descriptors = match load_auth_descriptors(store, user_id).await {
        Ok(descriptors) => descriptors,
        Err(error) => {
            tracing::warn!(
                user_id = %user_id,
                secret_name = %descriptor.secret_name,
                error = %error,
                "Failed to load auth descriptors for update"
            );
            return;
        }
    };
    descriptors.insert(descriptor.secret_name.to_lowercase(), descriptor.clone());

    let value = match serde_json::to_value(&descriptors) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                user_id = %user_id,
                secret_name = %descriptor.secret_name,
                error = %error,
                "Failed to serialize auth descriptors"
            );
            return;
        }
    };

    if let Err(error) = store
        .set_setting(user_id, AUTH_DESCRIPTORS_SETTING_KEY, &value)
        .await
    {
        tracing::warn!(
            user_id = %user_id,
            secret_name = %descriptor.secret_name,
            error = %error,
            "Failed to persist auth descriptor"
        );
        return;
    }

    auth_descriptor_cache().lock().await.insert(
        user_id.to_string(),
        CachedDescriptors {
            descriptors,
            inserted_at: std::time::Instant::now(),
        },
    );
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RefreshLockKey {
    secret_name: String,
    user_id: String,
}

fn refresh_lock_key(secret_name: &str, user_id: &str) -> RefreshLockKey {
    RefreshLockKey {
        secret_name: secret_name.to_string(),
        user_id: user_id.to_string(),
    }
}

async fn refresh_lock(secret_name: &str, user_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        tokio::sync::Mutex<HashMap<RefreshLockKey, Weak<tokio::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();

    let registry = LOCKS.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()));
    let mut locks = registry.lock().await;

    let key = refresh_lock_key(secret_name, user_id);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }

    locks.retain(|_, lock| lock.strong_count() > 0);
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialResolutionError {
    Missing,
    RefreshFailed,
    Secret(String),
}

pub async fn resolve_access_token_string_with_refresh<F, Fut>(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    secret_name: &str,
    log_name: &str,
    refresh: F,
) -> Result<Option<String>, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    match store.get_decrypted(user_id, secret_name).await {
        Ok(token) => Ok(Some(token.expose().to_string())),
        Err(SecretError::NotFound(_)) => Ok(None),
        Err(SecretError::Expired) => {
            tracing::debug!(target = "auth", subject = %log_name, "Access token expired, attempting refresh");
            match refresh().await {
                Ok(token) => {
                    tracing::debug!(target = "auth", subject = %log_name, "Access token refreshed successfully");
                    Ok(Some(token))
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error.to_string()),
    }
}

impl CredentialResolutionError {
    pub fn requires_authentication(&self) -> bool {
        matches!(self, Self::Missing | Self::RefreshFailed)
    }
}

pub async fn can_use_default_credential_fallback(
    role_lookup: Option<&dyn UserStore>,
    user_id: &str,
) -> bool {
    let Some(role_lookup) = role_lookup else {
        return false;
    };
    if user_id == "default" {
        return false;
    }

    match role_lookup.get_user(user_id).await {
        Ok(Some(user)) => user.is_admin(),
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(
                user_id = %user_id,
                error = %error,
                "Failed to resolve user role for default credential fallback"
            );
            false
        }
    }
}

async fn load_oauth_refresh_secret(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    refresh_name: &str,
) -> Option<DecryptedSecret> {
    match store.get_decrypted(user_id, refresh_name).await {
        Ok(secret) => Some(secret),
        Err(error) => {
            tracing::debug!(
                secret_name = %refresh_name,
                error = %error,
                "No refresh token available, skipping token refresh"
            );
            None
        }
    }
}

async fn persist_refreshed_oauth_tokens(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    config: &OAuthRefreshConfig,
    refresh_name: &str,
    token_response: oauth::OAuthTokenResponse,
) -> bool {
    let mut access_params =
        CreateSecretParams::new(&config.secret_name, &token_response.access_token);
    if let Some(ref provider) = config.provider {
        access_params = access_params.with_provider(provider);
    }
    if let Some(expires_in) = token_response.expires_in {
        // Saturating cast: an `expires_in` value above `i64::MAX` would
        // silently wrap to a negative duration and immediately invalidate
        // the freshly-stored token. Real OAuth providers cap this at
        // minutes/days, but defend against the corner case anyway. We
        // also use `try_seconds` (which is fallible on overflow inside
        // `chrono`'s internal millisecond representation) and saturate to
        // `Duration::max_value()` so a hostile / buggy provider returning
        // `u64::MAX` cannot panic the process — earlier `chrono` versions
        // panicked on `Duration::seconds(i64::MAX)`.
        let expires_secs = i64::try_from(expires_in).unwrap_or(i64::MAX);
        let expires_delta =
            chrono::Duration::try_seconds(expires_secs).unwrap_or(chrono::TimeDelta::MAX);
        let expires_at = chrono::Utc::now() + expires_delta;
        access_params = access_params.with_expiry(expires_at);
    }

    if let Err(e) = store.create(user_id, access_params).await {
        tracing::warn!(error = %e, "Failed to store refreshed access token");
        return false;
    }

    if let Some(refresh_token) = token_response.refresh_token {
        // Some OAuth providers occasionally echo an empty `refresh_token`
        // field instead of omitting it. Storing an empty string under the
        // refresh-token secret name would silently break the next refresh
        // (the token endpoint rejects an empty value with a generic 400)
        // and look like a credentials problem to the user. Skip the write
        // and warn instead — the existing refresh token (if any) stays in
        // place and the next refresh attempt re-uses it.
        if refresh_token.is_empty() {
            tracing::warn!(
                user_id = %user_id,
                refresh_name = %refresh_name,
                "OAuth token endpoint returned an empty refresh_token; not overwriting stored value"
            );
        } else {
            let mut refresh_params = CreateSecretParams::new(refresh_name, refresh_token);
            if let Some(ref provider) = config.provider {
                refresh_params = refresh_params.with_provider(provider);
            }
            if let Err(e) = store.create(user_id, refresh_params).await {
                tracing::warn!(error = %e, "Failed to store rotated refresh token");
                return false;
            }
        }
    }

    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultFallback {
    Denied,
    AdminOnly,
}

/// Validate an OAuth refresh-proxy URL against SSRF.
///
/// Wraps [`validate_and_resolve_http_target`] but optionally allows loopback
/// targets when `IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK=1` is set in the
/// environment. The escape hatch exists so unit tests that stand up a mock
/// proxy on `127.0.0.1` can still exercise the refresh path.
///
/// **The env-var check is gated to `cfg(any(test, debug_assertions))`** so a
/// production binary ignores the variable entirely. Setting it on a release
/// deployment is a no-op — the SSRF guard refuses loopback unconditionally
/// regardless of what the variable says.
async fn validate_oauth_proxy_url(proxy_url: &str) -> Result<(), String> {
    let allow_loopback = if cfg!(any(test, debug_assertions)) {
        std::env::var("IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE"))
            .unwrap_or(false)
    } else {
        false
    };

    match validate_and_resolve_http_target(proxy_url).await {
        Ok(_) => Ok(()),
        Err(error) => {
            if allow_loopback
                && let Ok(parsed) = url::Url::parse(proxy_url)
                && let Some(host) = parsed.host_str()
                && let Ok(ip) = host.parse::<std::net::IpAddr>()
                && ip.is_loopback()
            {
                tracing::debug!(
                    proxy_url = %proxy_url,
                    "Loopback OAuth proxy permitted via IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK"
                );
                return Ok(());
            }
            Err(error)
        }
    }
}

pub async fn refresh_oauth_access_token(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    config: &OAuthRefreshConfig,
) -> bool {
    let lock = refresh_lock(&config.secret_name, user_id).await;
    let _guard = lock.lock().await;

    let refresh_name = format!("{}_refresh_token", config.secret_name);

    if let Some(proxy_url) = config.exchange_proxy_url.as_deref() {
        let Some(oauth_proxy_auth_token) = config.oauth_proxy_auth_token() else {
            tracing::warn!(
                "OAuth refresh proxy is configured, but no OAuth proxy auth token is available"
            );
            return false;
        };

        // SSRF guard for the proxy path. The direct refresh path validates
        // `token_url` below; the proxy path was previously trusting whatever
        // the operator put in `IRONCLAW_OAUTH_EXCHANGE_URL`. Without this,
        // a misconfigured/compromised proxy URL could be pointed at internal
        // infrastructure and the refresh request (carrying the user's
        // refresh token) would happily POST there.
        //
        // Loopback (`127.0.0.0/8`, `::1`) is conditionally exempt: in normal
        // production this is still blocked, but tests that spin up a local
        // mock proxy can set `IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK=1` to
        // exercise the refresh path end-to-end. Loopback exemption is gated
        // explicitly so an operator does not silently widen the SSRF surface
        // by setting `IRONCLAW_OAUTH_EXCHANGE_URL=http://localhost/...`.
        if let Err(error) = validate_oauth_proxy_url(proxy_url).await {
            tracing::warn!(
                proxy_url = %proxy_url,
                error = %error,
                "OAuth refresh proxy URL failed SSRF validation; refusing token refresh"
            );
            return false;
        }

        let refresh_secret = match load_oauth_refresh_secret(store, user_id, &refresh_name).await {
            Some(secret) => secret,
            None => return false,
        };
        let token_response = match oauth::refresh_token_via_proxy(oauth::ProxyRefreshTokenRequest {
            proxy_url,
            gateway_token: oauth_proxy_auth_token,
            token_url: &config.token_url,
            client_id: &config.client_id,
            client_secret: config.client_secret.as_deref(),
            refresh_token: refresh_secret.expose(),
            resource: None,
            provider: config.provider.as_deref(),
        })
        .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(error = %error, "OAuth token refresh via proxy failed");
                return false;
            }
        };

        return persist_refreshed_oauth_tokens(
            store,
            user_id,
            config,
            &refresh_name,
            token_response,
        )
        .await;
    }

    if !config.token_url.starts_with("https://") {
        tracing::warn!(
            token_url = %config.token_url,
            "OAuth token_url must use HTTPS, refusing token refresh"
        );
        return false;
    }
    let resolved_target = match validate_and_resolve_http_target(&config.token_url).await {
        Ok(target) => target,
        Err(reason) => {
            tracing::warn!(
                token_url = %config.token_url,
                reason = %reason,
                "OAuth token_url points to a private/internal IP, refusing token refresh"
            );
            return false;
        }
    };

    let client = match ssrf_safe_client_builder_for_target(&resolved_target)
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to build HTTP client for token refresh");
            return false;
        }
    };

    let refresh_secret = match load_oauth_refresh_secret(store, user_id, &refresh_name).await {
        Some(secret) => secret,
        None => return false,
    };
    let mut params = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_secret.expose().to_string()),
        ("client_id", config.client_id.clone()),
    ];
    if let Some(ref secret) = config.client_secret {
        params.push(("client_secret", secret.clone()));
    }
    for (key, value) in &config.extra_refresh_params {
        params.push((key.as_str(), value.clone()));
    }

    let response = match client.post(&config.token_url).form(&params).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "OAuth token refresh request failed");
            return false;
        }
    };

    // Cap the response body at 64 KiB. Legitimate OAuth token responses are
    // a few hundred bytes; a misbehaving or hostile token endpoint must not
    // be able to OOM the process by streaming an unbounded body.
    //
    // Pre-check `Content-Length`: when the server sends an honest header that
    // exceeds the cap, reject *before* `bytes()` allocates the buffer. The
    // post-read length check below remains as defense for chunked / unknown
    // content-length / lying headers, but for the common honest case the
    // big body is never materialized.
    const MAX_TOKEN_BODY_BYTES: usize = 64 * 1024;
    if let Some(content_length) = response.content_length()
        && content_length > MAX_TOKEN_BODY_BYTES as u64
    {
        tracing::warn!(
            token_url = %config.token_url,
            content_length,
            limit = MAX_TOKEN_BODY_BYTES,
            "OAuth token refresh Content-Length exceeds size limit; refusing to read body"
        );
        return false;
    }

    if !response.status().is_success() {
        let status = response.status();
        let body_bytes = response
            .bytes()
            .await
            .map(|b| b.slice(..b.len().min(MAX_TOKEN_BODY_BYTES)))
            .unwrap_or_default();
        let body = String::from_utf8_lossy(&body_bytes);
        tracing::warn!(
            status = %status,
            body = %body,
            "OAuth token refresh returned non-success status"
        );
        return false;
    }

    let body_bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to read token refresh response body");
            return false;
        }
    };
    if body_bytes.len() > MAX_TOKEN_BODY_BYTES {
        tracing::warn!(
            len = body_bytes.len(),
            limit = MAX_TOKEN_BODY_BYTES,
            "OAuth token refresh response exceeds size limit"
        );
        return false;
    }
    let token_data: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to parse token refresh response");
            return false;
        }
    };
    let token_response = match token_data.get("access_token").and_then(|v| v.as_str()) {
        Some(access_token) => oauth::OAuthTokenResponse {
            access_token: access_token.to_string(),
            refresh_token: token_data
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            expires_in: token_data.get("expires_in").and_then(|v| v.as_u64()),
            token_type: token_data
                .get("token_type")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            scope: token_data
                .get("scope")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        },
        None => {
            tracing::warn!("Token refresh response missing access_token field");
            return false;
        }
    };

    persist_refreshed_oauth_tokens(store, user_id, config, &refresh_name, token_response).await
}

async fn maybe_refresh_before_read(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    secret_name: &str,
    oauth_refresh: Option<&OAuthRefreshConfig>,
) -> bool {
    let Some(config) = oauth_refresh.filter(|config| config.secret_name == secret_name) else {
        return false;
    };

    let needs_refresh = match store.get(user_id, secret_name).await {
        Ok(secret) => match secret.expires_at {
            Some(expires_at) => {
                let buffer = chrono::Duration::minutes(5);
                expires_at - buffer < chrono::Utc::now()
            }
            None => false,
        },
        Err(SecretError::Expired) => true,
        Err(SecretError::NotFound(_)) => {
            let refresh_name = format!("{}_refresh_token", secret_name);
            matches!(store.exists(user_id, &refresh_name).await, Ok(true))
        }
        Err(_) => false,
    };

    if !needs_refresh {
        return false;
    }

    tracing::debug!(
        secret_name = %secret_name,
        "Access token expired or near expiry, attempting refresh"
    );
    refresh_oauth_access_token(store, user_id, config).await
}

async fn load_secret_for_scope(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    secret_name: &str,
    oauth_refresh: Option<&OAuthRefreshConfig>,
) -> Result<DecryptedSecret, CredentialResolutionError> {
    let refresh_attempted =
        maybe_refresh_before_read(store, user_id, secret_name, oauth_refresh).await;
    match store.get_decrypted(user_id, secret_name).await {
        Ok(secret) => Ok(secret),
        Err(SecretError::NotFound(_) | SecretError::Expired) => {
            if refresh_attempted {
                Err(CredentialResolutionError::RefreshFailed)
            } else {
                Err(CredentialResolutionError::Missing)
            }
        }
        Err(error) => Err(CredentialResolutionError::Secret(error.to_string())),
    }
}

pub async fn resolve_secret_for_runtime(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    secret_name: &str,
    role_lookup: Option<&dyn UserStore>,
    oauth_refresh: Option<&OAuthRefreshConfig>,
    default_fallback: DefaultFallback,
) -> Result<DecryptedSecret, CredentialResolutionError> {
    match load_secret_for_scope(store, user_id, secret_name, oauth_refresh).await {
        Ok(secret) => return Ok(secret),
        Err(error)
            if error.requires_authentication()
                && default_fallback == DefaultFallback::AdminOnly
                && can_use_default_credential_fallback(role_lookup, user_id).await =>
        {
            tracing::debug!(
                secret_name = %secret_name,
                user_id = %user_id,
                "Credential unavailable in user scope, trying admin-only default scope"
            );
        }
        Err(error) => return Err(error),
    }

    load_secret_for_scope(store, "default", secret_name, oauth_refresh).await
}
