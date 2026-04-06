use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Weak};
use std::time::Duration;

use crate::cli::oauth_defaults;
use crate::db::Database;
use crate::secrets::{CreateSecretParams, DecryptedSecret, SecretError, SecretsStore};
use crate::tools::wasm::OAuthRefreshConfig;

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
    locks.retain(|_, lock| lock.strong_count() > 0);

    let key = refresh_lock_key(secret_name, user_id);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }

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
            tracing::info!(target = "auth", subject = %log_name, "Access token expired, attempting refresh");
            match refresh().await {
                Ok(token) => {
                    tracing::info!(target = "auth", subject = %log_name, "Access token refreshed successfully");
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

pub async fn can_use_default_credential_fallback(db: Option<&dyn Database>, user_id: &str) -> bool {
    let Some(db) = db else {
        return false;
    };
    if user_id == "default" {
        return false;
    }

    match db.get_user(user_id).await {
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
    token_response: oauth_defaults::OAuthTokenResponse,
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
    let parsed = match reqwest::Url::parse(url) {
        Ok(url) => url,
        Err(_) => return Err("invalid URL"),
    };

    let Some(host) = parsed.host_str() else {
        return Err("missing host");
    };

    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let dangerous = match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_private()
                    || v4.is_loopback()
                    || v4.is_link_local()
                    || v4.is_broadcast()
                    || v4.is_unspecified()
                    || (v4.octets()[0] == 169 && v4.octets()[1] == 254)
                    || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
            }
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_unique_local()
                    || v6.is_unicast_link_local()
                    || v6.is_multicast()
                    || v6
                        .to_ipv4_mapped()
                        .is_some_and(|v4| match std::net::IpAddr::V4(v4) {
                            std::net::IpAddr::V4(v4) => {
                                v4.is_private()
                                    || v4.is_loopback()
                                    || v4.is_link_local()
                                    || v4.is_broadcast()
                                    || v4.is_unspecified()
                                    || (v4.octets()[0] == 169 && v4.octets()[1] == 254)
                                    || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
                            }
                            std::net::IpAddr::V6(_) => false,
                        })
            }
        };

        if dangerous {
            return Err("host resolves to private/internal IP");
        }
    }

    Ok(())
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
        let token_response = match oauth_defaults::refresh_token_via_proxy(
            oauth_defaults::ProxyRefreshTokenRequest {
                proxy_url,
                gateway_token: oauth_proxy_auth_token,
                token_url: &config.token_url,
                client_id: &config.client_id,
                client_secret: config.client_secret.as_deref(),
                refresh_token: refresh_secret.expose(),
                resource: None,
                provider: config.provider.as_deref(),
            },
        )
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

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::warn!(
            status = %status,
            body = %body,
            "OAuth token refresh returned non-success status"
        );
        return false;
    }

    let token_data: serde_json::Value = match response.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to parse token refresh response");
            return false;
        }
    };
    let token_response = match token_data.get("access_token").and_then(|v| v.as_str()) {
        Some(access_token) => oauth_defaults::OAuthTokenResponse {
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

    tracing::info!(
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
    db: Option<&dyn Database>,
    oauth_refresh: Option<&OAuthRefreshConfig>,
    allow_admin_default_fallback: bool,
) -> Result<DecryptedSecret, CredentialResolutionError> {
    match load_secret_for_scope(store, user_id, secret_name, oauth_refresh).await {
        Ok(secret) => return Ok(secret),
        Err(error)
            if error.requires_authentication()
                && allow_admin_default_fallback
                && can_use_default_credential_fallback(db, user_id).await =>
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
