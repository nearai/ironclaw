use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tokio::sync::RwLock;

// ─── Risk: hardcoded VS Code Copilot identity ───────────────────────────────
//
// The client ID and editor identity headers below are extracted from the
// VS Code Copilot Chat extension.  This is the *only* publicly documented
// way to access the Copilot completions API with a personal GitHub token.
//
// **Known risks:**
//   • GitHub may rotate or revoke this client ID at any time, which would
//     break authentication for all IronClaw users until the constant is
//     updated and a new release is shipped.
//   • Using another product's client ID may violate GitHub's Terms of
//     Service.  Maintainers should seek explicit guidance from GitHub
//     before shipping this to a wide audience.
//   • The editor version strings (`vscode/1.99.3`, `copilot-chat/0.26.7`)
//     will become stale and could eventually be rejected by the API.
//
// **Mitigation:** If GitHub publishes an official Copilot API client ID or
// an OAuth app registration flow for third-party tools, migrate to it
// immediately.
// ─────────────────────────────────────────────────────────────────────────────
pub const GITHUB_COPILOT_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
pub const GITHUB_COPILOT_SCOPE: &str = "read:user";
pub const GITHUB_COPILOT_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
pub const GITHUB_COPILOT_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
pub const GITHUB_COPILOT_MODELS_URL: &str = "https://api.githubcopilot.com/models";
pub const GITHUB_COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
pub const GITHUB_COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.26.7";
pub const GITHUB_COPILOT_EDITOR_VERSION: &str = "vscode/1.99.3";
pub const GITHUB_COPILOT_EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.26.7";
pub const GITHUB_COPILOT_INTEGRATION_ID: &str = "vscode-chat";

/// Buffer before token expiry to trigger a refresh (5 minutes).
const TOKEN_REFRESH_BUFFER_SECS: u64 = 300;

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    #[serde(default = "default_poll_interval_secs")]
    pub interval: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct AccessTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum GithubCopilotAuthError {
    #[error("failed to start device login: {0}")]
    DeviceCodeRequest(String),
    #[error("failed to poll device login: {0}")]
    TokenPolling(String),
    #[error("device login was denied")]
    AccessDenied,
    #[error("device login expired before authorization completed")]
    Expired,
    #[error("github copilot token validation failed: {0}")]
    Validation(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePollingStatus {
    Pending,
    SlowDown,
    Authorized(String),
}

pub fn default_headers() -> Vec<(String, String)> {
    vec![
        (
            "User-Agent".to_string(),
            GITHUB_COPILOT_USER_AGENT.to_string(),
        ),
        (
            "Editor-Version".to_string(),
            GITHUB_COPILOT_EDITOR_VERSION.to_string(),
        ),
        (
            "Editor-Plugin-Version".to_string(),
            GITHUB_COPILOT_EDITOR_PLUGIN_VERSION.to_string(),
        ),
        (
            "Copilot-Integration-Id".to_string(),
            GITHUB_COPILOT_INTEGRATION_ID.to_string(),
        ),
    ]
}

pub fn default_poll_interval_secs() -> u64 {
    5
}

pub async fn request_device_code(
    client: &reqwest::Client,
) -> Result<DeviceCodeResponse, GithubCopilotAuthError> {
    let response = client
        .post(GITHUB_COPILOT_DEVICE_CODE_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, GITHUB_COPILOT_USER_AGENT)
        .form(&[
            ("client_id", GITHUB_COPILOT_CLIENT_ID),
            ("scope", GITHUB_COPILOT_SCOPE),
        ])
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(
                error = %e,
                is_timeout = e.is_timeout(),
                is_connect = e.is_connect(),
                url = %GITHUB_COPILOT_DEVICE_CODE_URL,
                "Copilot: device code request failed"
            );
            GithubCopilotAuthError::DeviceCodeRequest(format_reqwest_error(&e))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::warn!(
            status = %status,
            body = %truncate_for_error(&body),
            "Copilot: device code endpoint returned error"
        );
        return Err(GithubCopilotAuthError::DeviceCodeRequest(format!(
            "HTTP {status}: {}",
            truncate_for_error(&body)
        )));
    }

    let device = response
        .json::<DeviceCodeResponse>()
        .await
        .map_err(|e| GithubCopilotAuthError::DeviceCodeRequest(e.to_string()))?;

    Ok(device)
}

pub async fn poll_for_access_token(
    client: &reqwest::Client,
    device_code: &str,
) -> Result<DevicePollingStatus, GithubCopilotAuthError> {
    let response = client
        .post(GITHUB_COPILOT_ACCESS_TOKEN_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, GITHUB_COPILOT_USER_AGENT)
        .form(&[
            ("client_id", GITHUB_COPILOT_CLIENT_ID),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(
                error = %e,
                is_timeout = e.is_timeout(),
                is_connect = e.is_connect(),
                url = %GITHUB_COPILOT_ACCESS_TOKEN_URL,
                "Copilot: poll request failed"
            );
            GithubCopilotAuthError::TokenPolling(format_reqwest_error(&e))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::warn!(
            status = %status,
            body = %truncate_for_error(&body),
            "Copilot: poll endpoint returned error"
        );
        return Err(GithubCopilotAuthError::TokenPolling(format!(
            "HTTP {status}: {}",
            truncate_for_error(&body)
        )));
    }

    let body = response
        .json::<AccessTokenResponse>()
        .await
        .map_err(|e| GithubCopilotAuthError::TokenPolling(e.to_string()))?;

    if let Some(token) = body.access_token {
        return Ok(DevicePollingStatus::Authorized(token));
    }

    match body.error.as_deref() {
        Some("authorization_pending") | None => Ok(DevicePollingStatus::Pending),
        Some("slow_down") => {
            tracing::debug!("Copilot: GitHub requested slow_down, increasing poll interval");
            Ok(DevicePollingStatus::SlowDown)
        }
        Some("access_denied") => {
            tracing::warn!("Copilot: device login was denied by user");
            Err(GithubCopilotAuthError::AccessDenied)
        }
        Some("expired_token") => {
            tracing::warn!("Copilot: device code expired before authorization");
            Err(GithubCopilotAuthError::Expired)
        }
        Some(other) => {
            let desc = body
                .error_description
                .filter(|description| !description.is_empty())
                .unwrap_or_else(|| other.to_string());
            tracing::warn!(error = %other, description = %desc, "Copilot: unexpected poll error");
            Err(GithubCopilotAuthError::TokenPolling(desc))
        }
    }
}

/// Maximum consecutive transient poll failures before giving up.
const MAX_POLL_FAILURES: u32 = 5;

pub async fn wait_for_device_login(
    client: &reqwest::Client,
    device: &DeviceCodeResponse,
) -> Result<String, GithubCopilotAuthError> {
    let expires_at = std::time::Instant::now()
        .checked_add(Duration::from_secs(device.expires_in))
        .ok_or(GithubCopilotAuthError::Expired)?;
    let mut poll_interval = device.interval.max(1);
    let mut consecutive_failures: u32 = 0;

    loop {
        if std::time::Instant::now() >= expires_at {
            tracing::warn!("Copilot: device login expired");
            return Err(GithubCopilotAuthError::Expired);
        }

        tokio::time::sleep(Duration::from_secs(poll_interval)).await;

        match poll_for_access_token(client, &device.device_code).await {
            Ok(DevicePollingStatus::Pending) => {
                consecutive_failures = 0;
            }
            Ok(DevicePollingStatus::SlowDown) => {
                consecutive_failures = 0;
                poll_interval = poll_interval.saturating_add(5);
            }
            Ok(DevicePollingStatus::Authorized(token)) => {
                return Ok(token);
            }
            // Definitive failures — propagate immediately
            Err(GithubCopilotAuthError::AccessDenied) => {
                return Err(GithubCopilotAuthError::AccessDenied);
            }
            Err(GithubCopilotAuthError::Expired) => {
                return Err(GithubCopilotAuthError::Expired);
            }
            // Transient failures — retry with backoff
            Err(e) => {
                consecutive_failures += 1;
                tracing::warn!(
                    error = %e,
                    attempt = consecutive_failures,
                    max = MAX_POLL_FAILURES,
                    "Copilot: transient poll failure, will retry"
                );
                if consecutive_failures >= MAX_POLL_FAILURES {
                    tracing::error!(
                        error = %e,
                        "Copilot: too many consecutive poll failures, giving up"
                    );
                    return Err(e);
                }
                // Back off on transient errors
                poll_interval = (poll_interval + 2).min(30);
            }
        }
    }
}

/// Validate a GitHub OAuth token by performing the Copilot token exchange.
///
/// This exchanges the raw OAuth token for a Copilot session token (proving the
/// token is valid and the user has Copilot access), then verifies the session
/// token works against the models endpoint.
pub async fn validate_token(
    client: &reqwest::Client,
    token: &str,
) -> Result<(), GithubCopilotAuthError> {
    // Step 1: Exchange the OAuth token for a Copilot session token.
    // This validates both that the OAuth token is valid and that the user
    // has an active Copilot subscription.
    let session = exchange_copilot_token(client, token).await?;
    // Step 2: Verify the session token works against the models endpoint.
    let mut request = client
        .get(GITHUB_COPILOT_MODELS_URL)
        .bearer_auth(&session.token)
        .timeout(Duration::from_secs(15));

    for (key, value) in default_headers() {
        request = request.header(&key, value);
    }

    let response = request.send().await.map_err(|e| {
        tracing::warn!(
            error = %e,
            is_timeout = e.is_timeout(),
            is_connect = e.is_connect(),
            "Copilot: models endpoint request failed"
        );
        GithubCopilotAuthError::Validation(format_reqwest_error(&e))
    })?;

    if response.status().is_success() {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    tracing::warn!(
        status = %status,
        body = %truncate_for_error(&body),
        "Copilot: models endpoint returned error during validation"
    );
    Err(GithubCopilotAuthError::Validation(format!(
        "HTTP {status}: {}",
        truncate_for_error(&body)
    )))
}

/// Response from the Copilot token exchange endpoint.
///
/// The `token` field is an HMAC-signed session token (not a JWT) used as
/// `Authorization: Bearer <token>` for requests to `api.githubcopilot.com`.
#[derive(Debug, Clone, Deserialize)]
pub struct CopilotTokenResponse {
    /// The Copilot session token (HMAC-signed, not a JWT).
    pub token: String,
    /// Unix timestamp (seconds) when this token expires.
    pub expires_at: u64,
}

/// Exchange a GitHub OAuth token for a Copilot API session token.
///
/// Calls `GET https://api.github.com/copilot_internal/v2/token` with the
/// GitHub OAuth token in `Authorization: token <oauth_token>` format.
/// Returns a short-lived session token for `api.githubcopilot.com`.
pub async fn exchange_copilot_token(
    client: &reqwest::Client,
    oauth_token: &str,
) -> Result<CopilotTokenResponse, GithubCopilotAuthError> {
    let mut request = client
        .get(GITHUB_COPILOT_TOKEN_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        // GitHub Copilot uses `token` auth scheme, not `Bearer`
        .header(
            reqwest::header::AUTHORIZATION,
            format!("token {oauth_token}"),
        )
        .timeout(Duration::from_secs(15));

    for (key, value) in default_headers() {
        request = request.header(&key, value);
    }

    let response = request.send().await.map_err(|e| {
        tracing::warn!(
            error = %e,
            is_timeout = e.is_timeout(),
            is_connect = e.is_connect(),
            "Copilot: token exchange HTTP request failed"
        );
        GithubCopilotAuthError::Validation(format_reqwest_error(&e))
    })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::warn!(
            status = %status,
            body = %truncate_for_error(&body),
            "Copilot: token exchange endpoint returned error"
        );
        return Err(GithubCopilotAuthError::Validation(format!(
            "Copilot token exchange failed: HTTP {status}: {}",
            truncate_for_error(&body)
        )));
    }

    let token_response = response.json::<CopilotTokenResponse>().await.map_err(|e| {
        tracing::warn!(error = %e, "Copilot: failed to parse token exchange response");
        GithubCopilotAuthError::Validation(e.to_string())
    })?;

    Ok(token_response)
}

/// Manages a cached Copilot API session token with automatic refresh.
///
/// The GitHub Copilot API requires a two-step authentication:
/// 1. A long-lived GitHub OAuth token (from device login or IDE sign-in)
/// 2. A short-lived Copilot session token (exchanged via `/copilot_internal/v2/token`)
///
/// This manager caches the session token and refreshes it automatically
/// before it expires (with a 5-minute buffer).
pub struct CopilotTokenManager {
    client: reqwest::Client,
    oauth_token: SecretString,
    cached: RwLock<Option<CachedCopilotToken>>,
}

#[derive(Clone)]
struct CachedCopilotToken {
    token: SecretString,
    expires_at: u64,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl CopilotTokenManager {
    /// Create a new token manager with the given GitHub OAuth token.
    pub fn new(client: reqwest::Client, oauth_token: String) -> Self {
        Self {
            client,
            oauth_token: SecretString::from(oauth_token),
            cached: RwLock::new(None),
        }
    }

    /// Get a valid Copilot session token, refreshing if needed.
    ///
    /// Returns the cached token if it has more than 5 minutes remaining,
    /// otherwise exchanges the OAuth token for a fresh session token.
    pub async fn get_token(&self) -> Result<SecretString, GithubCopilotAuthError> {
        // Fast path: check if cached token is still valid under read lock.
        {
            let guard = self.cached.read().await;
            if let Some(ref cached) = *guard {
                let now = unix_now();
                if cached.expires_at > now + TOKEN_REFRESH_BUFFER_SECS {
                    return Ok(cached.token.clone());
                }
                tracing::debug!(
                    expires_at = cached.expires_at,
                    now = now,
                    "Copilot: cached session token expired or expiring soon, refreshing"
                );
            }
        }

        // Slow path: acquire write lock and re-check (another caller may have
        // already refreshed while we waited for the lock).
        let mut guard = self.cached.write().await;
        if let Some(ref cached) = *guard {
            let now = unix_now();
            if cached.expires_at > now + TOKEN_REFRESH_BUFFER_SECS {
                return Ok(cached.token.clone());
            }
        }

        let response =
            exchange_copilot_token(&self.client, self.oauth_token.expose_secret()).await?;
        let token = SecretString::from(response.token);

        let expires_at = response.expires_at;
        *guard = Some(CachedCopilotToken {
            token: token.clone(),
            expires_at,
        });

        tracing::debug!(
            expires_at = expires_at,
            "Copilot session token refreshed"
        );

        Ok(token)
    }

    /// Invalidate the cached session token.
    ///
    /// Called when the API returns 401, so the next `get_token()` call
    /// will perform a fresh token exchange instead of reusing the stale token.
    pub async fn invalidate(&self) {
        let mut guard = self.cached.write().await;
        *guard = None;
        tracing::debug!("Copilot session token invalidated");
    }
}

fn truncate_for_error(body: &str) -> String {
    const LIMIT: usize = 200;
    if body.len() <= LIMIT {
        return body.to_string();
    }

    let mut end = LIMIT;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &body[..end])
}

/// Format a reqwest error with its full causal chain for debugging.
///
/// `reqwest::Error::to_string()` often just says "error sending request"
/// without the underlying cause (timeout, DNS, TLS, connection refused).
/// This walks the `source()` chain to surface the real problem.
fn format_reqwest_error(e: &reqwest::Error) -> String {
    use std::error::Error;
    let mut msg = e.to_string();
    let mut source = e.source();
    while let Some(cause) = source {
        msg.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_headers_include_required_identity_headers() {
        let headers = default_headers();
        assert!(headers.iter().any(|(key, value)| {
            key == "Copilot-Integration-Id" && value == GITHUB_COPILOT_INTEGRATION_ID
        }));
        assert!(
            headers
                .iter()
                .any(|(key, value)| key == "Editor-Version"
                    && value == GITHUB_COPILOT_EDITOR_VERSION)
        );
        assert!(
            headers
                .iter()
                .any(|(key, value)| key == "User-Agent" && value == GITHUB_COPILOT_USER_AGENT)
        );
    }

    #[test]
    fn truncate_for_error_preserves_utf8_boundaries() {
        let long = "日本語".repeat(100);
        let truncated = truncate_for_error(&long);
        assert!(truncated.ends_with("..."));
        assert!(truncated.is_char_boundary(truncated.len() - 3));
    }

    #[test]
    fn truncate_for_error_short_strings_unchanged() {
        let short = "hello";
        assert_eq!(truncate_for_error(short), "hello");
    }

    // --- poll_for_access_token response parsing ---

    fn parse_access_token_body(json: &str) -> AccessTokenResponse {
        serde_json::from_str(json).expect("valid JSON")
    }

    #[test]
    fn parse_authorization_pending_response() {
        let body: AccessTokenResponse =
            parse_access_token_body(r#"{"error": "authorization_pending"}"#);
        assert!(body.access_token.is_none());
        assert_eq!(body.error.as_deref(), Some("authorization_pending"));
    }

    #[test]
    fn parse_slow_down_response() {
        let body: AccessTokenResponse = parse_access_token_body(r#"{"error": "slow_down"}"#);
        assert_eq!(body.error.as_deref(), Some("slow_down"));
    }

    #[test]
    fn parse_access_denied_response() {
        let body: AccessTokenResponse = parse_access_token_body(r#"{"error": "access_denied"}"#);
        assert_eq!(body.error.as_deref(), Some("access_denied"));
    }

    #[test]
    fn parse_expired_token_response() {
        let body: AccessTokenResponse = parse_access_token_body(r#"{"error": "expired_token"}"#);
        assert_eq!(body.error.as_deref(), Some("expired_token"));
    }

    #[test]
    fn parse_successful_token_response() {
        let body: AccessTokenResponse =
            parse_access_token_body(r#"{"access_token": "ghu_abc123"}"#);
        assert_eq!(body.access_token.as_deref(), Some("ghu_abc123"));
        assert!(body.error.is_none());
    }

    #[test]
    fn parse_error_with_description() {
        let body: AccessTokenResponse = parse_access_token_body(
            r#"{"error": "bad_verification_code", "error_description": "The code has expired"}"#,
        );
        assert_eq!(body.error.as_deref(), Some("bad_verification_code"));
        assert_eq!(
            body.error_description.as_deref(),
            Some("The code has expired")
        );
    }

    #[test]
    fn parse_device_code_response_with_defaults() {
        let json = r#"{
            "device_code": "dc_123",
            "user_code": "ABCD-1234",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 900
        }"#;
        let resp: DeviceCodeResponse = serde_json::from_str(json).expect("valid JSON");
        assert_eq!(resp.device_code, "dc_123");
        assert_eq!(resp.user_code, "ABCD-1234");
        assert_eq!(resp.interval, 5); // default_poll_interval_secs
        assert_eq!(resp.expires_in, 900);
    }

    #[test]
    fn parse_device_code_response_with_custom_interval() {
        let json = r#"{
            "device_code": "dc_456",
            "user_code": "EFGH-5678",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 600,
            "interval": 10
        }"#;
        let resp: DeviceCodeResponse = serde_json::from_str(json).expect("valid JSON");
        assert_eq!(resp.interval, 10);
    }

    // --- CopilotTokenManager ---

    #[tokio::test]
    async fn token_manager_caches_token_and_returns_same_value() {
        // Pre-populate the cache with a token that expires far in the future.
        let client = reqwest::Client::new();
        let manager = CopilotTokenManager::new(client, "unused_oauth".to_string());

        let far_future = unix_now() + 3600;
        {
            let mut guard = manager.cached.write().await;
            *guard = Some(CachedCopilotToken {
                token: SecretString::from("cached_session_token".to_string()),
                expires_at: far_future,
            });
        }

        let token = manager.get_token().await.expect("should return cached");
        assert_eq!(token.expose_secret(), "cached_session_token");

        // A second call should return the same cached token.
        let token2 = manager.get_token().await.expect("should return cached");
        assert_eq!(token2.expose_secret(), "cached_session_token");
    }

    #[tokio::test]
    async fn token_manager_invalidation_clears_cache() {
        let client = reqwest::Client::new();
        let manager = CopilotTokenManager::new(client, "unused_oauth".to_string());

        let far_future = unix_now() + 3600;
        {
            let mut guard = manager.cached.write().await;
            *guard = Some(CachedCopilotToken {
                token: SecretString::from("old_token".to_string()),
                expires_at: far_future,
            });
        }

        manager.invalidate().await;

        let guard = manager.cached.read().await;
        assert!(guard.is_none(), "cache should be empty after invalidation");
    }

    #[tokio::test]
    async fn token_manager_expired_token_triggers_refresh_path() {
        let client = reqwest::Client::new();
        let manager = CopilotTokenManager::new(client, "unused_oauth".to_string());

        // Set a token that is already expired (expires_at in the past).
        {
            let mut guard = manager.cached.write().await;
            *guard = Some(CachedCopilotToken {
                token: SecretString::from("stale_token".to_string()),
                expires_at: 1, // way in the past
            });
        }

        // get_token will try the slow path (token exchange) which will fail
        // because we have no real server, but this proves the cached stale
        // token is NOT returned.
        let result = manager.get_token().await;
        assert!(
            result.is_err(),
            "expired cached token should trigger exchange, which fails without a server"
        );
    }

    #[tokio::test]
    async fn token_manager_within_buffer_triggers_refresh() {
        let client = reqwest::Client::new();
        let manager = CopilotTokenManager::new(client, "unused_oauth".to_string());

        // Set a token that expires within the refresh buffer window.
        let expires_soon = unix_now() + TOKEN_REFRESH_BUFFER_SECS - 10;
        {
            let mut guard = manager.cached.write().await;
            *guard = Some(CachedCopilotToken {
                token: SecretString::from("expiring_soon".to_string()),
                expires_at: expires_soon,
            });
        }

        let result = manager.get_token().await;
        assert!(
            result.is_err(),
            "token within buffer should trigger exchange"
        );
    }

    // --- CopilotTokenResponse parsing ---

    #[test]
    fn parse_copilot_token_response() {
        let json = r#"{"token": "tid=abc;exp=999;sku=123;sig=xyz", "expires_at": 1700000000}"#;
        let resp: CopilotTokenResponse = serde_json::from_str(json).expect("valid JSON");
        assert!(resp.token.starts_with("tid="));
        assert_eq!(resp.expires_at, 1700000000);
    }
}
