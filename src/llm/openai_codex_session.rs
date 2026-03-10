//! OAuth 2.0 session manager for OpenAI Codex (ChatGPT subscription).
//!
//! Supports two auth flows:
//! - **Device Code** (primary): Works on headless servers, no browser needed.
//! - **Browser PKCE** (fallback): Standard OAuth for local machines.
//!
//! Tokens are persisted to `~/.ironclaw/openai_codex_session.json` and
//! auto-refreshed before expiry.

use chrono::{DateTime, Utc};
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::config::OpenAiCodexConfig;
use crate::error::LlmError;

/// Persisted OAuth session data.
///
/// Note: `Debug` is manually implemented to redact tokens.
#[derive(Serialize, Deserialize)]
pub struct OpenAiCodexSession {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    pub(crate) expires_at: DateTime<Utc>,
    pub(crate) created_at: DateTime<Utc>,
}

impl std::fmt::Debug for OpenAiCodexSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCodexSession")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// Request body for the device code usercode endpoint.
#[derive(Debug, Serialize)]
struct UserCodeRequest {
    client_id: String,
}

/// Response from the device code usercode endpoint.
#[derive(Debug, Deserialize)]
struct UserCodeResponse {
    /// Unique ID for this device auth session.
    device_auth_id: String,
    /// Code the user enters in their browser.
    user_code: String,
    /// URL where the user enters the code (may not be present).
    #[serde(default = "default_verification_uri")]
    verification_uri: String,
    /// Polling interval in seconds (OpenAI sends this as a string).
    #[serde(
        default = "default_interval",
        deserialize_with = "deserialize_string_or_u64"
    )]
    interval: u64,
    /// Expiry timestamp (OpenAI sends `expires_at` as ISO-8601).
    #[serde(default)]
    expires_at: Option<String>,
    /// Seconds until the device code expires (standard field, may not be present).
    #[serde(default)]
    expires_in: Option<u64>,
}

fn default_verification_uri() -> String {
    "https://auth.openai.com/codex/device".to_string()
}

fn default_interval() -> u64 {
    5
}

/// Deserialize a value that may be either a string or a number as u64.
fn deserialize_string_or_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct StringOrU64;
    impl<'de> de::Visitor<'de> for StringOrU64 {
        type Value = u64;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string or integer")
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<u64, E> {
            Ok(v)
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<u64, E> {
            v.parse().map_err(de::Error::custom)
        }
    }
    deserializer.deserialize_any(StringOrU64)
}

impl UserCodeResponse {
    /// Get the expiry duration in seconds, from either `expires_in` or `expires_at`.
    fn expires_in_secs(&self) -> u64 {
        if let Some(secs) = self.expires_in {
            return secs;
        }
        if let Some(ref ts) = self.expires_at
            && let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts)
        {
            let remaining = dt.signed_duration_since(Utc::now()).num_seconds();
            return remaining.max(0) as u64;
        }
        900 // default 15 minutes
    }
}

/// Request body for polling the device auth token endpoint.
#[derive(Debug, Serialize)]
struct DeviceTokenPollRequest {
    device_auth_id: String,
    user_code: String,
}

/// Successful response from the device auth token endpoint.
/// Returns an authorization code + PKCE pair for the final token exchange.
#[derive(Debug, Deserialize)]
struct DeviceAuthCodeResponse {
    authorization_code: String,
    #[allow(dead_code)]
    code_challenge: String,
    code_verifier: String,
}

/// Response from the final OAuth token exchange.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    #[allow(dead_code)]
    token_type: String,
}

/// Manages OpenAI Codex OAuth sessions with persistence and auto-refresh.
pub struct OpenAiCodexSessionManager {
    config: OpenAiCodexConfig,
    client: Client,
    session: RwLock<Option<OpenAiCodexSession>>,
    renewal_lock: Mutex<()>,
}

impl OpenAiCodexSessionManager {
    /// Create a new session manager. Tries to load existing session from disk.
    ///
    /// # Errors
    ///
    /// Returns `LlmError` if the HTTP client cannot be constructed.
    pub fn new(config: OpenAiCodexConfig) -> Result<Self, LlmError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(concat!("ironclaw/", env!("CARGO_PKG_VERSION"))),
        );
        let client = Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "openai_codex".into(),
                reason: format!("HTTP client build failed: {e}"),
            })?;

        let mgr = Self {
            config,
            client,
            session: RwLock::new(None),
            renewal_lock: Mutex::new(()),
        };

        // Try synchronous load from disk during construction
        if let Ok(data) = std::fs::read_to_string(&mgr.config.session_path)
            && let Ok(session) = serde_json::from_str::<OpenAiCodexSession>(&data)
            && let Ok(mut guard) = mgr.session.try_write()
        {
            *guard = Some(session);
            tracing::info!(
                "Loaded OpenAI Codex session from {}",
                mgr.config.session_path.display()
            );
        }

        Ok(mgr)
    }

    /// Check if we have a session (may be expired).
    pub async fn has_session(&self) -> bool {
        self.session.read().await.is_some()
    }

    /// Check if the current access token needs refreshing.
    pub async fn needs_refresh(&self) -> bool {
        let guard = self.session.read().await;
        match guard.as_ref() {
            None => true,
            Some(s) => {
                let margin =
                    chrono::Duration::seconds(self.config.token_refresh_margin_secs as i64);
                Utc::now() + margin >= s.expires_at
            }
        }
    }

    /// Get the current access token, refreshing if needed.
    ///
    /// If the token is within the refresh margin, silently refreshes first.
    /// If no session exists, returns an AuthFailed error.
    pub async fn get_access_token(&self) -> Result<SecretString, LlmError> {
        if self.needs_refresh().await {
            let has_refresh = self
                .session
                .read()
                .await
                .as_ref()
                .map(|s| !s.refresh_token.is_empty())
                .unwrap_or(false);
            if has_refresh {
                self.refresh_tokens().await?;
            } else {
                return Err(LlmError::AuthFailed {
                    provider: "openai_codex".to_string(),
                });
            }
        }

        let guard = self.session.read().await;
        guard
            .as_ref()
            .map(|s| SecretString::from(s.access_token.clone()))
            .ok_or_else(|| LlmError::AuthFailed {
                provider: "openai_codex".to_string(),
            })
    }

    /// Ensure we have a valid session. Loads from disk, refreshes, or prompts login.
    pub async fn ensure_authenticated(&self) -> Result<(), LlmError> {
        // Try loading from disk if we don't have a session
        if !self.has_session().await {
            let _ = self.load_session().await;
        }

        if !self.has_session().await {
            // No session at all -- need to authenticate
            return self.device_code_login().await;
        }

        if self.needs_refresh().await {
            // Try refresh; if it fails, re-authenticate
            match self.refresh_tokens().await {
                Ok(()) => Ok(()),
                Err(e) => {
                    tracing::info!("Token refresh failed ({}), re-authenticating...", e);
                    self.device_code_login().await
                }
            }
        } else {
            Ok(())
        }
    }

    /// Run OpenAI's device code auth flow.
    ///
    /// Uses OpenAI's custom `/api/accounts/deviceauth/*` endpoints (not the standard
    /// Auth0 `/oauth/device/code` which is behind Cloudflare managed challenge).
    ///
    /// Flow:
    /// 1. POST `/api/accounts/deviceauth/usercode` → get device_auth_id + user_code
    /// 2. Poll POST `/api/accounts/deviceauth/token` → get authorization_code + PKCE
    /// 3. Exchange via POST `/oauth/token` → get access_token + refresh_token
    pub async fn device_code_login(&self) -> Result<(), LlmError> {
        let _guard = self.renewal_lock.lock().await;

        let auth_base = format!("{}/api/accounts", self.config.auth_endpoint);

        // Step 1: Request device code
        let usercode_url = format!("{}/deviceauth/usercode", auth_base);
        let resp = self
            .client
            .post(&usercode_url)
            .json(&UserCodeRequest {
                client_id: self.config.client_id.clone(),
            })
            .send()
            .await
            .map_err(|e| LlmError::SessionRenewalFailed {
                provider: "openai_codex".to_string(),
                reason: format!("Device code request failed: {}", e),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::SessionRenewalFailed {
                provider: "openai_codex".to_string(),
                reason: format!("Device code request failed: HTTP {} -- {}", status, body),
            });
        }

        let body_text = resp
            .text()
            .await
            .map_err(|e| LlmError::SessionRenewalFailed {
                provider: "openai_codex".to_string(),
                reason: format!("Failed to read device code response: {}", e),
            })?;
        tracing::debug!("Device code response received ({} bytes)", body_text.len());
        let device: UserCodeResponse =
            serde_json::from_str(&body_text).map_err(|e| LlmError::SessionRenewalFailed {
                provider: "openai_codex".to_string(),
                reason: format!(
                    "Failed to parse device code response: {} ({} bytes)",
                    e,
                    body_text.len()
                ),
            })?;

        // Step 2: Display code to user
        println!();
        println!("===========================================================");
        println!("               OpenAI Codex Authentication                  ");
        println!("===========================================================");
        println!();
        println!("  1. Open this URL in any browser:");
        println!("     {}", device.verification_uri);
        println!();
        println!("  2. Enter this code:");
        println!();
        println!("              [  {}  ]", device.user_code);
        println!();
        let expires_secs = device.expires_in_secs();
        println!(
            "  Waiting for authorization... (expires in {} min)",
            expires_secs / 60
        );
        println!("===========================================================");
        println!();

        // Step 3: Poll for authorization code
        let poll_url = format!("{}/deviceauth/token", auth_base);
        let mut interval = std::time::Duration::from_secs(device.interval.max(5));
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(expires_secs);

        let auth_code = loop {
            tokio::time::sleep(interval).await;

            if tokio::time::Instant::now() >= deadline {
                return Err(LlmError::SessionRenewalFailed {
                    provider: "openai_codex".to_string(),
                    reason: "Device code authorization timed out".to_string(),
                });
            }

            let resp = self
                .client
                .post(&poll_url)
                .json(&DeviceTokenPollRequest {
                    device_auth_id: device.device_auth_id.clone(),
                    user_code: device.user_code.clone(),
                })
                .send()
                .await
                .map_err(|e| LlmError::SessionRenewalFailed {
                    provider: "openai_codex".to_string(),
                    reason: format!("Token poll request failed: {}", e),
                })?;

            let status = resp.status();
            if status.is_success() {
                let code_resp: DeviceAuthCodeResponse =
                    resp.json()
                        .await
                        .map_err(|e| LlmError::SessionRenewalFailed {
                            provider: "openai_codex".to_string(),
                            reason: format!("Failed to parse auth code response: {}", e),
                        })?;
                break code_resp;
            }

            // 403 = authorization_pending, keep polling
            // 404 = device code not found / not enabled
            if status == reqwest::StatusCode::FORBIDDEN {
                continue;
            }

            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(LlmError::SessionRenewalFailed {
                    provider: "openai_codex".to_string(),
                    reason: "Device code login is not enabled. Please check your OpenAI account settings.".to_string(),
                });
            }

            // Slow down on 429
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                interval += std::time::Duration::from_secs(5);
                continue;
            }

            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::SessionRenewalFailed {
                provider: "openai_codex".to_string(),
                reason: format!("Device auth poll failed: HTTP {} -- {}", status, body),
            });
        };

        // Step 4: Exchange authorization code for tokens (form-encoded, per Auth0 spec)
        let token_url = format!("{}/oauth/token", self.config.auth_endpoint);
        let resp = self
            .client
            .post(&token_url)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", &auth_code.authorization_code),
                ("code_verifier", &auth_code.code_verifier),
                ("client_id", &self.config.client_id),
                (
                    "redirect_uri",
                    &format!("{}/deviceauth/callback", self.config.auth_endpoint),
                ),
            ])
            .send()
            .await
            .map_err(|e| LlmError::SessionRenewalFailed {
                provider: "openai_codex".to_string(),
                reason: format!("Token exchange failed: {}", e),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::SessionRenewalFailed {
                provider: "openai_codex".to_string(),
                reason: format!("Token exchange failed: HTTP {} -- {}", status, body),
            });
        }

        let token_resp: TokenResponse =
            resp.json()
                .await
                .map_err(|e| LlmError::SessionRenewalFailed {
                    provider: "openai_codex".to_string(),
                    reason: format!("Failed to parse token response: {}", e),
                })?;

        let session = OpenAiCodexSession {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token,
            expires_at: Utc::now() + chrono::Duration::seconds(token_resp.expires_in as i64),
            created_at: Utc::now(),
        };

        self.save_session(&session).await?;
        self.set_session(session).await;

        println!();
        println!("Authentication successful!");
        println!();
        Ok(())
    }

    /// Refresh the access token using the refresh token.
    pub async fn refresh_tokens(&self) -> Result<(), LlmError> {
        let _guard = self.renewal_lock.lock().await;

        // Double-check: another task may have refreshed while we waited on the lock
        if !self.needs_refresh().await {
            return Ok(());
        }

        let refresh_token = {
            let guard = self.session.read().await;
            guard
                .as_ref()
                .map(|s| s.refresh_token.clone())
                .ok_or_else(|| LlmError::AuthFailed {
                    provider: "openai_codex".to_string(),
                })?
        };

        let token_url = format!("{}/oauth/token", self.config.auth_endpoint);
        let resp = self
            .client
            .post(&token_url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
                ("client_id", self.config.client_id.as_str()),
            ])
            .send()
            .await
            .map_err(|e| LlmError::SessionRenewalFailed {
                provider: "openai_codex".to_string(),
                reason: format!("Token refresh request failed: {}", e),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::SessionRenewalFailed {
                provider: "openai_codex".to_string(),
                reason: format!("Token refresh failed: HTTP {} -- {}", status, body),
            });
        }

        let token_resp: TokenResponse =
            resp.json()
                .await
                .map_err(|e| LlmError::SessionRenewalFailed {
                    provider: "openai_codex".to_string(),
                    reason: format!("Failed to parse refresh response: {}", e),
                })?;

        let session = OpenAiCodexSession {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token,
            expires_at: Utc::now() + chrono::Duration::seconds(token_resp.expires_in as i64),
            created_at: Utc::now(),
        };

        self.save_session(&session).await?;
        self.set_session(session).await;

        tracing::debug!("OpenAI Codex token refreshed successfully");
        Ok(())
    }

    /// Save session data to disk with restrictive permissions.
    pub async fn save_session(&self, session: &OpenAiCodexSession) -> Result<(), LlmError> {
        if let Some(parent) = self.config.session_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                LlmError::Io(std::io::Error::new(
                    e.kind(),
                    format!("Failed to create session directory: {}", e),
                ))
            })?;
        }

        let json =
            serde_json::to_string_pretty(session).map_err(|e| LlmError::SessionRenewalFailed {
                provider: "openai_codex".to_string(),
                reason: format!("Failed to serialize session: {}", e),
            })?;

        tokio::fs::write(&self.config.session_path, &json)
            .await
            .map_err(|e| {
                LlmError::Io(std::io::Error::new(
                    e.kind(),
                    format!("Failed to write session file: {}", e),
                ))
            })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            tokio::fs::set_permissions(&self.config.session_path, perms)
                .await
                .map_err(|e| {
                    LlmError::Io(std::io::Error::new(
                        e.kind(),
                        format!("Failed to set permissions: {}", e),
                    ))
                })?;
        }

        Ok(())
    }

    /// Load session from disk.
    pub async fn load_session(&self) -> Result<(), LlmError> {
        let data = tokio::fs::read_to_string(&self.config.session_path)
            .await
            .map_err(|e| {
                LlmError::Io(std::io::Error::new(
                    e.kind(),
                    format!("Failed to read session file: {}", e),
                ))
            })?;

        let session: OpenAiCodexSession =
            serde_json::from_str(&data).map_err(|e| LlmError::SessionRenewalFailed {
                provider: "openai_codex".to_string(),
                reason: format!("Failed to parse session file: {}", e),
            })?;

        let mut guard = self.session.write().await;
        *guard = Some(session);
        tracing::info!(
            "Loaded OpenAI Codex session from {}",
            self.config.session_path.display()
        );
        Ok(())
    }

    /// Set session directly (for testing or after auth).
    pub async fn set_session(&self, session: OpenAiCodexSession) {
        let mut guard = self.session.write().await;
        *guard = Some(session);
    }

    /// Handle a 401 response by refreshing, or re-authenticating.
    pub async fn handle_auth_failure(&self) -> Result<(), LlmError> {
        match self.refresh_tokens().await {
            Ok(()) => Ok(()),
            Err(_) => self.device_code_login().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_config(session_path: std::path::PathBuf) -> OpenAiCodexConfig {
        OpenAiCodexConfig {
            model: "gpt-5.3-codex".to_string(),
            auth_endpoint: "https://auth.openai.com".to_string(),
            api_base_url: "https://chatgpt.com/backend-api/codex".to_string(),
            client_id: "test_client_id".to_string(),
            session_path,
            token_refresh_margin_secs: 300,
        }
    }

    #[tokio::test]
    async fn test_save_and_load_session() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.json");
        let config = test_config(path.clone());

        let mgr = OpenAiCodexSessionManager::new(config).unwrap();

        // No session initially
        assert!(!mgr.has_session().await);

        // Save a session
        let session = OpenAiCodexSession {
            access_token: "access_abc".to_string(),
            refresh_token: "refresh_xyz".to_string(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            created_at: chrono::Utc::now(),
        };
        mgr.save_session(&session).await.unwrap();
        mgr.set_session(session).await;

        assert!(mgr.has_session().await);

        // Load from disk in a new manager
        let config2 = test_config(path);
        let mgr2 = OpenAiCodexSessionManager::new(config2).unwrap();
        mgr2.load_session().await.unwrap();
        assert!(mgr2.has_session().await);
    }

    #[tokio::test]
    async fn test_needs_refresh_when_near_expiry() {
        let dir = tempdir().unwrap();
        let config = test_config(dir.path().join("session.json"));
        let mgr = OpenAiCodexSessionManager::new(config).unwrap();

        // Token expiring in 2 minutes (margin is 300s = 5 min)
        let session = OpenAiCodexSession {
            access_token: "access_abc".to_string(),
            refresh_token: "refresh_xyz".to_string(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(2),
            created_at: chrono::Utc::now(),
        };
        mgr.set_session(session).await;

        assert!(mgr.needs_refresh().await);
    }

    #[test]
    fn device_code_parse_error_redacts_body() {
        // Regression: the parse error used to include raw body_text which could
        // contain sensitive auth data. Now it only shows byte count.
        let body_text = r#"{"secret_token":"sk-12345","error":"unexpected"}"#;
        let err: Result<UserCodeResponse, _> = serde_json::from_str(body_text);
        assert!(err.is_err());
        let e = err.unwrap_err();
        let error_msg = format!(
            "Failed to parse device code response: {} ({} bytes)",
            e,
            body_text.len()
        );
        assert!(
            !error_msg.contains("sk-12345"),
            "error message must not contain raw body: {error_msg}"
        );
        assert!(
            error_msg.contains("bytes"),
            "error message should show byte count"
        );
    }

    #[tokio::test]
    async fn test_no_refresh_when_fresh() {
        let dir = tempdir().unwrap();
        let config = test_config(dir.path().join("session.json"));
        let mgr = OpenAiCodexSessionManager::new(config).unwrap();

        // Token expiring in 30 minutes (margin is 300s = 5 min)
        let session = OpenAiCodexSession {
            access_token: "access_abc".to_string(),
            refresh_token: "refresh_xyz".to_string(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
            created_at: chrono::Utc::now(),
        };
        mgr.set_session(session).await;

        assert!(!mgr.needs_refresh().await);
    }
}
