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
use crate::host::{
    NoopKeyPersistor, NoopSessionRenewer, SharedSessionDb, SharedSessionKeyPersistor,
    SharedSessionRenewer, SharedSessionSecrets,
};

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
            // Real path is set by LlmConfig::resolve() via config/llm.rs.
            // This default is only used in tests.
            session_path: PathBuf::from("session.json"),
        }
    }
}

/// Manages NEAR AI session tokens with persistence and automatic renewal.
///
/// The DB / encrypted-secrets / interactive-renewal / env-persist hooks are
/// abstracted behind traits in [`crate::host`] so this crate doesn't need to
/// depend on the embedding application. Headless deployments can leave them
/// unset; the CLI build wires in real impls.
pub struct SessionManager {
    config: SessionConfig,
    client: Client,
    /// Current token in memory.
    token: RwLock<Option<SecretString>>,
    /// Prevents thundering herd during concurrent 401s.
    renewal_lock: Mutex<()>,
    /// Optional database store for persisting session to the settings table.
    store: RwLock<Option<SharedSessionDb>>,
    /// User ID for DB settings (default: "default").
    user_id: RwLock<String>,
    /// Optional encrypted secrets store — preferred over plaintext settings when present.
    secrets: RwLock<Option<SharedSessionSecrets>>,
    /// Interactive renewal hook. Defaults to a no-op that returns `SessionRenewalFailed`.
    renewer: RwLock<SharedSessionRenewer>,
    /// Persistor for one-shot API key entry (runtime env + .env file). Defaults to no-op.
    key_persistor: RwLock<SharedSessionKeyPersistor>,
}

impl SessionManager {
    fn empty(config: SessionConfig) -> Self {
        Self {
            config,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| Client::new()),
            token: RwLock::new(None),
            renewal_lock: Mutex::new(()),
            store: RwLock::new(None),
            // Placeholder; overwritten by attach_store() with the real owner_id at startup.
            user_id: RwLock::new("<unset>".to_string()),
            secrets: RwLock::new(None),
            renewer: RwLock::new(Arc::new(NoopSessionRenewer) as SharedSessionRenewer),
            key_persistor: RwLock::new(Arc::new(NoopKeyPersistor) as SharedSessionKeyPersistor),
        }
    }

    /// Create a new session manager and load any existing token from disk.
    pub fn new(config: SessionConfig) -> Self {
        let manager = Self::empty(config);

        // Try to load existing session synchronously during construction
        if let Ok(data) = std::fs::read_to_string(&manager.config.session_path)
            && let Ok(session) = serde_json::from_str::<SessionData>(&data)
            && let Ok(mut guard) = manager.token.try_write()
        {
            *guard = Some(SecretString::from(session.session_token));
            tracing::info!(
                "Loaded session token from {}",
                manager.config.session_path.display()
            );
        }

        manager
    }

    /// Create a session manager and load token asynchronously.
    pub async fn new_async(config: SessionConfig) -> Self {
        let manager = Self::empty(config);

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
    pub async fn attach_store(&self, store: SharedSessionDb, user_id: &str) {
        *self.store.write().await = Some(store);
        *self.user_id.write().await = user_id.to_string();

        // Try to load from DB (may have been saved by a previous run)
        if let Err(e) = self.load_session_from_db().await {
            tracing::debug!("No session in DB: {}", e);
        }
    }

    /// Attach an encrypted secrets store for secure session token persistence.
    ///
    /// When attached, `save_session` writes to the secrets store in addition
    /// to the disk file, and `load_session_from_db` prefers the secrets store
    /// over the plaintext settings table.
    pub async fn attach_secrets(&self, secrets: SharedSessionSecrets) {
        *self.secrets.write().await = Some(secrets);

        // Try to load from encrypted secrets (preferred over settings table)
        if let Err(e) = self.load_session_from_secrets().await {
            tracing::debug!("No session in secrets store: {}", e);
        }
    }

    /// Attach an interactive renewer used when a session expires.
    ///
    /// Headless deployments can skip this; the default `NoopSessionRenewer`
    /// returns `SessionRenewalFailed` and the caller is expected to set
    /// `NEARAI_SESSION_TOKEN` or `NEARAI_API_KEY` ahead of time.
    pub async fn attach_renewer(&self, renewer: SharedSessionRenewer) {
        *self.renewer.write().await = renewer;
    }

    /// Attach a persistor used by the API-key entry path inside a renewer.
    pub async fn attach_key_persistor(&self, persistor: SharedSessionKeyPersistor) {
        *self.key_persistor.write().await = persistor;
    }

    /// Read-only access to the configured auth base URL (used by renewer impls).
    pub fn auth_base_url(&self) -> &str {
        &self.config.auth_base_url
    }

    /// Returns the persistor most recently set via `attach_key_persistor`.
    pub async fn key_persistor(&self) -> SharedSessionKeyPersistor {
        Arc::clone(&*self.key_persistor.read().await)
    }

    /// Public hook used by `SessionRenewer` impls to write a freshly received
    /// session token back through the same disk + DB + secrets pipeline as
    /// the internal flow.
    pub async fn save_session_for_renewer(
        &self,
        token: &str,
        auth_provider: Option<&str>,
    ) -> Result<(), LlmError> {
        self.save_session(token, auth_provider).await?;
        let mut guard = self.token.write().await;
        *guard = Some(SecretString::from(token.to_string()));
        Ok(())
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

    /// Ensure we have a valid session, triggering the renewer if needed.
    ///
    /// If no token exists, asks the registered `SessionRenewer` for one. If a
    /// token exists, validates it by hitting `/v1/users/me`. If validation
    /// fails, asks the renewer for a fresh token.
    pub async fn ensure_authenticated(&self) -> Result<(), LlmError> {
        if !self.has_token().await {
            return self.run_renewer().await;
        }

        tracing::debug!("Validating session...");
        match self.validate_token().await {
            Ok(()) => {
                tracing::debug!("Session valid");
                Ok(())
            }
            Err(e) => {
                tracing::info!("Session expired or invalid: {}", e);
                self.run_renewer().await
            }
        }
    }

    async fn run_renewer(&self) -> Result<(), LlmError> {
        let renewer = Arc::clone(&*self.renewer.read().await);
        renewer.renew(self).await
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
        let preview = ironclaw_common::truncate_for_preview(&body, 200);
        Err(LlmError::SessionRenewalFailed {
            provider: "nearai".to_string(),
            reason: format!("Validation failed: HTTP {status}: {preview}"),
        })
    }

    /// Handle an authentication failure (401 response).
    ///
    /// Acquires the renewal lock to prevent a thundering herd, then asks the
    /// registered `SessionRenewer` for a fresh token.
    pub async fn handle_auth_failure(&self) -> Result<(), LlmError> {
        let _guard = self.renewal_lock.lock().await;

        tracing::info!("Session expired or invalid, re-authenticating...");
        self.run_renewer().await
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

        // Persist to encrypted secrets store (preferred) if attached
        if let Some(ref secrets) = *self.secrets.read().await {
            let user_id = self.user_id.read().await.clone();
            let session_json_str =
                serde_json::to_string(&session).unwrap_or_else(|_| token.to_string());
            if let Err(e) = secrets
                .create(
                    &user_id,
                    "nearai_session_token",
                    session_json_str,
                    Some("nearai"),
                )
                .await
            {
                tracing::warn!("Failed to save session to encrypted secrets: {}", e);
            } else {
                tracing::debug!("Session saved to encrypted secrets store");
            }
        // Save to DB settings table only as fallback when no secrets store is attached
        } else if let Some(ref store) = *self.store.read().await {
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
            // Try the legacy key. Only warn if it actually exists (real
            // backwards-compat migration). When neither key is present
            // (fresh install), just return the "No session in DB" error.
            let legacy = store
                .get_setting(&user_id, "nearai.session")
                .await
                .map_err(|e| LlmError::SessionRenewalFailed {
                    provider: "nearai".to_string(),
                    reason: format!("DB query failed: {}", e),
                })?;
            match legacy {
                Some(value) => {
                    tracing::warn!(
                        "nearai.session_token missing; falling back to legacy nearai.session for backwards compatibility"
                    );
                    value
                }
                None => {
                    return Err(LlmError::SessionRenewalFailed {
                        provider: "nearai".to_string(),
                        reason: "No session in DB".to_string(),
                    });
                }
            }
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

    /// Try to load session from the encrypted secrets store.
    ///
    /// The session is stored as a JSON-serialized `SessionData` string under
    /// the secret name `nearai_session_token`. This is preferred over the
    /// plaintext settings table when a secrets store is available.
    async fn load_session_from_secrets(&self) -> Result<(), LlmError> {
        let secrets_guard = self.secrets.read().await;
        let secrets = secrets_guard
            .as_ref()
            .ok_or_else(|| LlmError::SessionRenewalFailed {
                provider: "nearai".to_string(),
                reason: "No secrets store attached".to_string(),
            })?;

        let user_id = self.user_id.read().await.clone();
        let decrypted = secrets
            .get_decrypted(&user_id, "nearai_session_token")
            .await
            .map_err(|e| LlmError::SessionRenewalFailed {
                provider: "nearai".to_string(),
                reason: format!("Secrets lookup failed: {}", e),
            })?;

        use secrecy::ExposeSecret as _;
        let session: SessionData =
            serde_json::from_str(decrypted.expose_secret()).map_err(|e| {
                LlmError::SessionRenewalFailed {
                    provider: "nearai".to_string(),
                    reason: format!("Failed to parse session from secrets: {}", e),
                }
            })?;

        let mut guard = self.token.write().await;
        *guard = Some(SecretString::from(session.session_token));
        tracing::info!("Loaded session from encrypted secrets store");

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
    if let Ok(token) = std::env::var("NEARAI_SESSION_TOKEN")
        && !token.is_empty()
    {
        tracing::info!("Using session token from NEARAI_SESSION_TOKEN env var");
        manager.set_token(SecretString::from(token)).await;
    }

    Arc::new(manager)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TEST_SESSION_NEARAI_ABC, TEST_SESSION_NEARAI_XYZ, TEST_SESSION_TOKEN};
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
            .save_session(TEST_SESSION_TOKEN, Some("near"))
            .await
            .unwrap();
        manager
            .set_token(SecretString::from(TEST_SESSION_TOKEN))
            .await;

        // Verify it's set
        assert!(manager.has_token().await);
        let token = manager.get_token().await.unwrap();
        assert_eq!(token.expose_secret(), TEST_SESSION_TOKEN);

        // Create new manager and verify it loads the token
        let manager2 = SessionManager::new_async(config).await;
        assert!(manager2.has_token().await);
        let token2 = manager2.get_token().await.unwrap();
        assert_eq!(token2.expose_secret(), TEST_SESSION_TOKEN);

        // Verify file contents
        let data: SessionData =
            serde_json::from_str(&std::fs::read_to_string(&session_path).unwrap()).unwrap();
        assert_eq!(data.session_token, TEST_SESSION_TOKEN);
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
    fn test_session_data_serde_roundtrip_with_auth_provider() {
        let original = SessionData {
            session_token: TEST_SESSION_NEARAI_ABC.to_string(),
            created_at: Utc::now(),
            auth_provider: Some("github".to_string()),
        };
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: SessionData = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.session_token, original.session_token);
        assert_eq!(deserialized.auth_provider, Some("github".to_string()));
        assert_eq!(deserialized.created_at, original.created_at);
    }

    #[test]
    fn test_session_data_serde_roundtrip_without_auth_provider() {
        let original = SessionData {
            session_token: TEST_SESSION_NEARAI_XYZ.to_string(),
            created_at: Utc::now(),
            auth_provider: None,
        };
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: SessionData = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.session_token, original.session_token);
        assert_eq!(deserialized.auth_provider, None);
    }

    #[test]
    fn test_session_data_missing_auth_provider_defaults_to_none() {
        let json = r#"{"session_token":"tok_legacy","created_at":"2025-01-01T00:00:00Z"}"#;
        let data: SessionData = serde_json::from_str(json).unwrap();
        assert_eq!(data.session_token, "tok_legacy");
        assert_eq!(data.auth_provider, None);
    }

    #[test]
    fn test_session_config_default() {
        let config = SessionConfig::default();
        assert_eq!(config.auth_base_url, "https://private.near.ai");
        assert!(config.session_path.ends_with("session.json"));
    }

    #[tokio::test]
    async fn test_new_with_nonexistent_session_file() {
        let dir = tempdir().unwrap();
        let config = SessionConfig {
            auth_base_url: "https://example.com".to_string(),
            session_path: dir.path().join("does_not_exist.json"),
        };
        let manager = SessionManager::new(config);
        assert!(!manager.has_token().await);
    }

    #[tokio::test]
    async fn test_set_token_get_token_roundtrip() {
        let dir = tempdir().unwrap();
        let config = SessionConfig {
            auth_base_url: "https://example.com".to_string(),
            session_path: dir.path().join("session.json"),
        };
        let manager = SessionManager::new(config);
        manager
            .set_token(SecretString::from("my_secret_token"))
            .await;
        let token = manager.get_token().await.unwrap();
        assert_eq!(token.expose_secret(), "my_secret_token");
    }

    #[tokio::test]
    async fn test_has_token_false_then_true() {
        let dir = tempdir().unwrap();
        let config = SessionConfig {
            auth_base_url: "https://example.com".to_string(),
            session_path: dir.path().join("session.json"),
        };
        let manager = SessionManager::new(config);
        assert!(!manager.has_token().await);
        manager.set_token(SecretString::from("tok_something")).await;
        assert!(manager.has_token().await);
    }

    #[tokio::test]
    async fn test_save_session_then_load_in_new_manager() {
        let dir = tempdir().unwrap();
        let session_path = dir.path().join("session.json");
        let config = SessionConfig {
            auth_base_url: "https://example.com".to_string(),
            session_path: session_path.clone(),
        };

        let manager = SessionManager::new_async(config.clone()).await;
        manager
            .save_session("persist_me", Some("google"))
            .await
            .unwrap();

        // Load in a fresh manager
        let manager2 = SessionManager::new_async(config).await;
        assert!(manager2.has_token().await);
        let token = manager2.get_token().await.unwrap();
        assert_eq!(token.expose_secret(), "persist_me");

        // Verify auth_provider was persisted
        let raw: SessionData =
            serde_json::from_str(&std::fs::read_to_string(&session_path).unwrap()).unwrap();
        assert_eq!(raw.auth_provider, Some("google".to_string()));
    }

    #[tokio::test]
    async fn test_save_session_with_no_auth_provider() {
        let dir = tempdir().unwrap();
        let session_path = dir.path().join("session.json");
        let config = SessionConfig {
            auth_base_url: "https://example.com".to_string(),
            session_path: session_path.clone(),
        };

        let manager = SessionManager::new_async(config).await;
        manager.save_session("anon_tok", None).await.unwrap();

        let raw: SessionData =
            serde_json::from_str(&std::fs::read_to_string(&session_path).unwrap()).unwrap();
        assert_eq!(raw.session_token, "anon_tok");
        assert_eq!(raw.auth_provider, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_session_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let session_path = dir.path().join("session.json");
        let config = SessionConfig {
            auth_base_url: "https://example.com".to_string(),
            session_path: session_path.clone(),
        };

        let manager = SessionManager::new_async(config).await;
        manager
            .save_session("secret_tok", Some("github"))
            .await
            .unwrap();

        let metadata = std::fs::metadata(&session_path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "Session file should have 0600 permissions");
    }
}
