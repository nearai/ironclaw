pub mod oauth;
pub mod providers;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::db::{SettingsStore, UserStore};
use crate::secrets::{CreateSecretParams, DecryptedSecret, SecretError, SecretsStore};
use crate::tools::wasm::OAuthRefreshConfig;

const AUTH_DESCRIPTORS_SETTING_KEY: &str = "auth.descriptors_v1";

fn auth_descriptor_cache()
-> &'static tokio::sync::Mutex<HashMap<String, HashMap<String, AuthDescriptor>>> {
    static CACHE: std::sync::OnceLock<
        tokio::sync::Mutex<HashMap<String, HashMap<String, AuthDescriptor>>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
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
    pub extension_name: String,
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
    debug_assert_ne!(
        user_id, "default",
        "auth descriptors should remain user-scoped; global reads must stay explicit"
    );

    let cache = auth_descriptor_cache();
    if let Some(descriptors) = cache.lock().await.get(user_id).cloned() {
        return Ok(descriptors);
    }

    let descriptors = match store
        .get_setting(user_id, AUTH_DESCRIPTORS_SETTING_KEY)
        .await?
    {
        Some(value) => serde_json::from_value(value)
            .map_err(|error| crate::error::DatabaseError::Query(error.to_string())),
        None => Ok(HashMap::new()),
    }?;

    cache
        .lock()
        .await
        .insert(user_id.to_string(), descriptors.clone());
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

pub async fn upsert_auth_descriptor(
    store: Option<&dyn SettingsStore>,
    user_id: &str,
    descriptor: AuthDescriptor,
) {
    let Some(store) = store else {
        return;
    };

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

    auth_descriptor_cache()
        .lock()
        .await
        .insert(user_id.to_string(), descriptors);
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
        Ok(Some(user)) => user.role == "admin",
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
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in as i64);
        access_params = access_params.with_expiry(expires_at);
    }

    if let Err(e) = store.create(user_id, access_params).await {
        tracing::warn!(error = %e, "Failed to store refreshed access token");
        return false;
    }

    if let Some(refresh_token) = token_response.refresh_token {
        let mut refresh_params = CreateSecretParams::new(refresh_name, refresh_token);
        if let Some(ref provider) = config.provider {
            refresh_params = refresh_params.with_provider(provider);
        }
        if let Err(e) = store.create(user_id, refresh_params).await {
            tracing::warn!(error = %e, "Failed to store rotated refresh token");
            return false;
        }
    }

    true
}

fn reject_private_ip(url: &str) -> Result<(), &'static str> {
    crate::tools::wasm::reject_private_ip(url).map_err(|_| "host resolves to private/internal IP")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultFallback {
    Denied,
    AdminOnly,
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
    if let Err(reason) = reject_private_ip(&config.token_url) {
        tracing::warn!(
            token_url = %config.token_url,
            reason = %reason,
            "OAuth token_url points to a private/internal IP, refusing token refresh"
        );
        return false;
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
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
    const MAX_TOKEN_BODY_BYTES: usize = 64 * 1024;

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
