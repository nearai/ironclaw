//! Session management for NEAR AI authentication.
//!
//! Handles session token persistence, expiration detection, and renewal via
//! OAuth flow. Tokens are stored in `~/.ironclaw/session.json` and refreshed
//! automatically when expired.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use reqwest::Client;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::error::LlmError;

/// Session data persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionData {
    pub session_token: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub auth_provider: Option<String>,
}

/// Configuration for session management.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Base URL for auth endpoints (e.g., https://private.near.ai).
    pub auth_base_url: String,
    /// Path to session file (e.g., ~/.ironclaw/session.json).
    pub session_path: PathBuf,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            auth_base_url: "https://private.near.ai".to_string(),
            session_path: default_session_path(),
        }
    }
}

/// Get the default session file path (~/.ironclaw/session.json).
pub fn default_session_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("session.json")
}

/// Manages NEAR AI session tokens with persistence and automatic renewal.
pub struct SessionManager {
    config: SessionConfig,
    client: Client,
    /// Current token in memory.
    token: RwLock<Option<SecretString>>,
    /// Prevents thundering herd during concurrent 401s.
    renewal_lock: Mutex<()>,
    /// Optional database store for persisting session to the settings table.
    store: RwLock<Option<Arc<dyn crate::db::Database>>>,
    /// User ID for DB settings (default: "default").
    user_id: RwLock<String>,
}

impl SessionManager {
    /// Create a new session manager and load any existing token from disk.
    pub fn new(config: SessionConfig) -> Self {
        let manager = Self {
            config,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| Client::new()),
            token: RwLock::new(None),
            renewal_lock: Mutex::new(()),
            store: RwLock::new(None),
            user_id: RwLock::new("default".to_string()),
        };

        // Try to load existing session synchronously during construction
        if let Ok(data) = std::fs::read_to_string(&manager.config.session_path)
            && let Ok(session) = serde_json::from_str::<SessionData>(&data)
        {
            // We can't await here, so we use try_write
            if let Ok(mut guard) = manager.token.try_write() {
                *guard = Some(SecretString::from(session.session_token));
                tracing::info!(
                    "Loaded session token from {}",
                    manager.config.session_path.display()
                );
            }
        }

        manager
    }

    /// Create a session manager and load token asynchronously.
    pub async fn new_async(config: SessionConfig) -> Self {
        let manager = Self {
            config,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| Client::new()),
            token: RwLock::new(None),
            renewal_lock: Mutex::new(()),
            store: RwLock::new(None),
            user_id: RwLock::new("default".to_string()),
        };

        if let Err(e) = manager.load_session().await {
            tracing::debug!("No existing session found: {}", e);
        }

        manager
    }

    /// Attach a database store for persisting session tokens.
    ///
    /// When a store is attached, session tokens are saved to the `settings`
    /// table (key: `nearai.session_token`) in addition to the disk file.
    /// On load, DB is preferred over disk.
    pub async fn attach_store(&self, store: Arc<dyn crate::db::Database>, user_id: &str) {
        *self.store.write().await = Some(store);
        *self.user_id.write().await = user_id.to_string();

        // Try to load from DB (may have been saved by a previous run)
        if let Err(e) = self.load_session_from_db().await {
            tracing::debug!("No session in DB: {}", e);
        }
    }

    /// Get the current session token, returning an error if not authenticated.
    pub async fn get_token(&self) -> Result<SecretString, LlmError> {
        let guard = self.token.read().await;
        guard.clone().ok_or_else(|| LlmError::AuthFailed {
            provider: "nearai".to_string(),
        })
    }

    /// Check if we have a valid token (doesn't verify with server).
    pub async fn has_token(&self) -> bool {
        self.token.read().await.is_some()
    }

    /// Ensure we have a valid session, triggering login flow if needed.
    ///
    /// If no token exists, triggers the OAuth login flow. If a token exists,
    /// validates it by making a test API call. If validation fails, triggers
    /// the login flow.
    pub async fn ensure_authenticated(&self) -> Result<(), LlmError> {
        if !self.has_token().await {
            // No token, need to authenticate
            return self.initiate_login().await;
        }

        // Token exists, validate it by calling /v1/users/me
        println!("Validating session...");
        match self.validate_token().await {
            Ok(()) => {
                println!("Session valid.");
                Ok(())
            }
            Err(e) => {
                println!("Session expired or invalid: {}", e);
                self.initiate_login().await
            }
        }
    }

    /// Validate the current token by calling the /v1/users/me endpoint.
    async fn validate_token(&self) -> Result<(), LlmError> {
        use secrecy::ExposeSecret;

        let token = self.get_token().await?;
        let url = format!("{}/v1/users/me", self.config.auth_base_url);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token.expose_secret()))
            .send()
            .await
            .map_err(|e| LlmError::SessionRenewalFailed {
                provider: "nearai".to_string(),
                reason: format!("Validation request failed: {}", e),
            })?;

        if response.status().is_success() {
            return Ok(());
        }

        if response.status().as_u16() == 401 {
            return Err(LlmError::SessionExpired {
                provider: "nearai".to_string(),
            });
        }

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Err(LlmError::SessionRenewalFailed {
            provider: "nearai".to_string(),
            reason: format!("Validation failed: HTTP {}: {}", status, body),
        })
    }

    /// Handle an authentication failure (401 response).
    ///
    /// Triggers the OAuth login flow to get a new session token.
    pub async fn handle_auth_failure(&self) -> Result<(), LlmError> {
        // Acquire renewal lock to prevent thundering herd
        let _guard = self.renewal_lock.lock().await;

        tracing::info!("Session expired or invalid, re-authenticating...");
        self.initiate_login().await
    }

    /// Start the login flow.
    ///
    /// Shows the auth method menu FIRST (before binding any listener), so
    /// that the API-key path can skip network binding entirely. This is
    /// important for remote/headless servers where `127.0.0.1` is
    /// unreachable from the user's browser.
    ///
    /// For OAuth paths (GitHub, Google):
    /// 1. Bind the callback listener
    /// 2. Print the auth URL and attempt to open browser
    /// 3. Wait for OAuth callback with session token
    /// 4. Save and return the token
    ///
    /// For NEAR AI Cloud API key:
    /// 1. Prompt user for API key from cloud.near.ai
    /// 2. Set NEARAI_API_KEY env var and save to bootstrap .env
    /// 3. No session token saved (different auth model)
    async fn initiate_login(&self) -> Result<(), LlmError> {
        use crate::cli::oauth_defaults;

        let cb_url = oauth_defaults::callback_url();

        // Show auth provider menu BEFORE binding the listener
        println!();
        println!("╔════════════════════════════════════════════════════════════════╗");
        println!("║                    NEAR AI Authentication                      ║");
        println!("╠════════════════════════════════════════════════════════════════╣");
        println!("║  Choose an authentication method:                              ║");
        println!("║                                                                ║");
        println!("║    [1] GitHub            (requires localhost browser access)    ║");
        println!("║    [2] Google            (requires localhost browser access)    ║");
        println!("║    [3] NEAR Wallet (coming soon)                               ║");
        println!("║    [4] NEAR AI Cloud API key                                   ║");
        println!("║                                                                ║");
        println!("╚════════════════════════════════════════════════════════════════╝");
        println!();
        print!("Enter choice [1-4]: ");

        // Flush stdout to ensure prompt is displayed
        use std::io::Write;
        std::io::stdout().flush().ok();

        // Read user choice
        let mut choice = String::new();
        std::io::stdin()
            .read_line(&mut choice)
            .map_err(|e| LlmError::SessionRenewalFailed {
                provider: "nearai".to_string(),
                reason: format!("Failed to read input: {}", e),
            })?;

        match choice.trim() {
            "4" => return self.api_key_login().await,
            "3" => {
                println!();
                println!("NEAR Wallet authentication is not yet implemented.");
                println!("Please use GitHub or Google for now.");
                return Err(LlmError::SessionRenewalFailed {
                    provider: "nearai".to_string(),
                    reason: "NEAR Wallet auth not yet implemented".to_string(),
                });
            }
            "1" | "" | "2" => {} // handled below after listener bind
            other => {
                return Err(LlmError::SessionRenewalFailed {
                    provider: "nearai".to_string(),
                    reason: format!("Invalid choice: {}", other),
                });
            }
        }

        // OAuth paths: bind the callback listener now
        let listener = oauth_defaults::bind_callback_listener()
            .await
            .map_err(|e| LlmError::SessionRenewalFailed {
                provider: "nearai".to_string(),
                reason: e.to_string(),
            })?;

        let (auth_provider, auth_url) = match choice.trim() {
            "2" => {
                let url = format!(
                    "{}/v1/auth/google?frontend_callback={}",
                    self.config.auth_base_url,
                    urlencoding::encode(&cb_url)
                );
                ("google", url)
            }
            _ => {
                // "1" or "" (default)
                let url = format!(
                    "{}/v1/auth/github?frontend_callback={}",
                    self.config.auth_base_url,
                    urlencoding::encode(&cb_url)
                );
                ("github", url)
            }
        };

        println!();
        println!("Opening {} authentication...", auth_provider);
        println!();
        println!("  {}", auth_url);
        println!();

        // Try to open browser automatically
        if let Err(e) = open::that(&auth_url) {
            tracing::debug!("Could not open browser automatically: {}", e);
            println!("(Could not open browser automatically, please copy the URL above)");
        } else {
            println!("(Opening browser...)");
        }
        println!();
        println!("Waiting for authentication...");

        // The NEAR AI API redirects to: {frontend_callback}/auth/callback?token=X&...
        let session_token =
            oauth_defaults::wait_for_callback(listener, "/auth/callback", "token", "NEAR AI")
                .await
                .map_err(|e| LlmError::SessionRenewalFailed {
                    provider: "nearai".to_string(),
                    reason: e.to_string(),
                })?;

        let auth_provider = Some(auth_provider.to_string());

        // Save the token
        self.save_session(&session_token, auth_provider.as_deref())
            .await?;

        // Update in-memory token
        {
            let mut guard = self.token.write().await;
            *guard = Some(SecretString::from(session_token));
        }

        println!();
        println!("✓ Authentication successful!");
        println!();

        Ok(())
    }

    /// NEAR AI Cloud API key entry flow.
    ///
    /// Prompts the user to enter a NEAR AI Cloud API key from
    /// cloud.near.ai. The key is set as `NEARAI_API_KEY` env var so
    /// `LlmConfig::resolve()` auto-selects ChatCompletions mode, and
    /// saved to `~/.ironclaw/.env` for persistence across restarts.
    /// No session token is saved and no `/v1/users/me` validation is
    /// performed (different auth model).
    async fn api_key_login(&self) -> Result<(), LlmError> {
        println!();
        println!("NEAR AI Cloud API key");
        println!("─────────────────────");
        println!();
        println!("  1. Open https://cloud.near.ai in your browser");
        println!("  2. Sign in and navigate to API Keys");
        println!("  3. Create or copy an existing API key");
        println!();

        let key_secret =
            crate::setup::secret_input("API key").map_err(|e| LlmError::SessionRenewalFailed {
                provider: "nearai".to_string(),
                reason: format!("Failed to read input: {}", e),
            })?;

        use secrecy::ExposeSecret;
        let key = key_secret.expose_secret().to_string();
        if key.is_empty() {
            return Err(LlmError::SessionRenewalFailed {
                provider: "nearai".to_string(),
                reason: "API key cannot be empty".to_string(),
            });
        }

        // Set env var so Config picks it up immediately
        // (LlmConfig::resolve() auto-selects ChatCompletions mode when
        // NEARAI_API_KEY is present).
        //
        // SAFETY: called during single-threaded interactive login flow.
        #[allow(unused_unsafe)]
        unsafe {
            std::env::set_var("NEARAI_API_KEY", &key);
        }

        // Persist to ~/.ironclaw/.env so the key survives restarts
        // (bootstrap layer — available before DB is connected).
        // Uses upsert to avoid clobbering existing bootstrap vars.
        if let Err(e) = crate::bootstrap::upsert_bootstrap_var("NEARAI_API_KEY", &key) {
            tracing::warn!("Failed to save API key to bootstrap .env: {}", e);
        }

        println!();
        println!("✓ NEAR AI Cloud API key saved.");
        println!();

        Ok(())
    }

    /// Save session data to disk and (if available) to the database.
    async fn save_session(&self, token: &str, auth_provider: Option<&str>) -> Result<(), LlmError> {
        let session = SessionData {
            session_token: token.to_string(),
            created_at: Utc::now(),
            auth_provider: auth_provider.map(String::from),
        };

        // Save to disk (always, as bootstrap fallback)
        if let Some(parent) = self.config.session_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                LlmError::Io(std::io::Error::new(
                    e.kind(),
                    format!("Failed to create session directory: {}", e),
                ))
            })?;
        }

        let json =
            serde_json::to_string_pretty(&session).map_err(|e| LlmError::SessionRenewalFailed {
                provider: "nearai".to_string(),
                reason: format!("Failed to serialize session: {}", e),
            })?;

        tokio::fs::write(&self.config.session_path, json)
            .await
            .map_err(|e| {
                LlmError::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "Failed to write session file {}: {}",
                        self.config.session_path.display(),
                        e
                    ),
                ))
            })?;

        // Restrictive permissions: session file contains a secret token
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            tokio::fs::set_permissions(&self.config.session_path, perms)
                .await
                .map_err(|e| {
                    LlmError::Io(std::io::Error::new(
                        e.kind(),
                        format!(
                            "Failed to set permissions on {}: {}",
                            self.config.session_path.display(),
                            e
                        ),
                    ))
                })?;
        }

        tracing::debug!("Session saved to {}", self.config.session_path.display());

        // Also save to DB if a store is attached
        if let Some(ref store) = *self.store.read().await {
            let user_id = self.user_id.read().await.clone();
            let session_json = serde_json::to_value(&session)
                .unwrap_or(serde_json::Value::String(token.to_string()));
            if let Err(e) = store
                .set_setting(&user_id, "nearai.session_token", &session_json)
                .await
            {
                tracing::warn!("Failed to save session to DB: {}", e);
            } else {
                tracing::debug!("Session also saved to DB settings");
            }
        }

        Ok(())
    }

    /// Try to load session from the database.
    async fn load_session_from_db(&self) -> Result<(), LlmError> {
        let store_guard = self.store.read().await;
        let store = store_guard
            .as_ref()
            .ok_or_else(|| LlmError::SessionRenewalFailed {
                provider: "nearai".to_string(),
                reason: "No DB store attached".to_string(),
            })?;

        let user_id = self.user_id.read().await.clone();
        let value = if let Some(value) = store
            .get_setting(&user_id, "nearai.session_token")
            .await
            .map_err(|e| LlmError::SessionRenewalFailed {
            provider: "nearai".to_string(),
            reason: format!("DB query failed: {}", e),
        })? {
            value
        } else {
            tracing::warn!(
                "nearai.session_token missing; falling back to legacy nearai.session for backwards compatibility"
            );
            store
                .get_setting(&user_id, "nearai.session")
                .await
                .map_err(|e| LlmError::SessionRenewalFailed {
                    provider: "nearai".to_string(),
                    reason: format!("DB query failed: {}", e),
                })?
                .ok_or(LlmError::SessionRenewalFailed {
                    provider: "nearai".to_string(),
                    reason: "No session in DB".to_string(),
                })?
        };

        let session: SessionData =
            serde_json::from_value(value).map_err(|e| LlmError::SessionRenewalFailed {
                provider: "nearai".to_string(),
                reason: format!("Failed to parse DB session: {}", e),
            })?;

        let mut guard = self.token.write().await;
        *guard = Some(SecretString::from(session.session_token));
        tracing::info!("Loaded session from DB settings");

        Ok(())
    }

    /// Load session data from disk.
    async fn load_session(&self) -> Result<(), LlmError> {
        let data = tokio::fs::read_to_string(&self.config.session_path)
            .await
            .map_err(|e| {
                LlmError::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "Failed to read session file {}: {}",
                        self.config.session_path.display(),
                        e
                    ),
                ))
            })?;

        let session: SessionData =
            serde_json::from_str(&data).map_err(|e| LlmError::SessionRenewalFailed {
                provider: "nearai".to_string(),
                reason: format!("Failed to parse session file: {}", e),
            })?;

        {
            let mut guard = self.token.write().await;
            *guard = Some(SecretString::from(session.session_token));
        }

        tracing::info!(
            "Loaded session from {} (created: {})",
            self.config.session_path.display(),
            session.created_at
        );

        Ok(())
    }

    /// Set token directly (useful for testing or migration from env var).
    pub async fn set_token(&self, token: SecretString) {
        let mut guard = self.token.write().await;
        *guard = Some(token);
    }
}

/// Create a session manager from a config, loading env var if present.
///
/// When `NEARAI_SESSION_TOKEN` is set, it takes precedence over file-based
/// tokens. This supports hosting providers that inject the token via env var.
pub async fn create_session_manager(config: SessionConfig) -> Arc<SessionManager> {
    let manager = SessionManager::new_async(config).await;

    // NEARAI_SESSION_TOKEN env var always takes precedence over file-based
    // tokens. Hosting providers set this env var and expect it to be used
    // directly — no file persistence needed.
    if let Ok(token) = std::env::var("NEARAI_SESSION_TOKEN") {
        if !token.is_empty() {
            tracing::info!("Using session token from NEARAI_SESSION_TOKEN env var");
            manager.set_token(SecretString::from(token)).await;
        }
    }

    Arc::new(manager)
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_session_save_load() {
        let dir = tempdir().unwrap();
        let session_path = dir.path().join("session.json");

        let config = SessionConfig {
            auth_base_url: "https://example.com".to_string(),
            session_path: session_path.clone(),
        };

        let manager = SessionManager::new_async(config.clone()).await;

        // No token initially
        assert!(!manager.has_token().await);

        // Save a token
        manager
            .save_session("test_token_123", Some("near"))
            .await
            .unwrap();
        manager
            .set_token(SecretString::from("test_token_123"))
            .await;

        // Verify it's set
        assert!(manager.has_token().await);
        let token = manager.get_token().await.unwrap();
        assert_eq!(token.expose_secret(), "test_token_123");

        // Create new manager and verify it loads the token
        let manager2 = SessionManager::new_async(config).await;
        assert!(manager2.has_token().await);
        let token2 = manager2.get_token().await.unwrap();
        assert_eq!(token2.expose_secret(), "test_token_123");

        // Verify file contents
        let data: SessionData =
            serde_json::from_str(&std::fs::read_to_string(&session_path).unwrap()).unwrap();
        assert_eq!(data.session_token, "test_token_123");
        assert_eq!(data.auth_provider, Some("near".to_string()));
    }

    #[tokio::test]
    async fn test_get_token_without_auth_fails() {
        let dir = tempdir().unwrap();
        let config = SessionConfig {
            auth_base_url: "https://example.com".to_string(),
            session_path: dir.path().join("nonexistent.json"),
        };

        let manager = SessionManager::new_async(config).await;
        let result = manager.get_token().await;
        assert!(result.is_err());
        assert!(matches!(result, Err(LlmError::AuthFailed { .. })));
    }

    #[test]
    fn test_default_session_path() {
        let path = default_session_path();
        assert!(path.ends_with("session.json"));
        assert!(path.to_string_lossy().contains(".ironclaw"));
    }
}
