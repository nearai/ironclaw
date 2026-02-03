//! Session management for NEAR AI authentication.
//!
//! Handles session token persistence, expiration detection, and renewal via
//! OAuth flow. Tokens are stored in `~/.near-agent/session.json` and refreshed
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
    /// Path to session file (e.g., ~/.near-agent/session.json).
    pub session_path: PathBuf,
    /// Port range for OAuth callback server.
    pub callback_port_range: (u16, u16),
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            auth_base_url: "https://private.near.ai".to_string(),
            session_path: default_session_path(),
            callback_port_range: (9876, 9886),
        }
    }
}

/// Get the default session file path (~/.near-agent/session.json).
pub fn default_session_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".near-agent")
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
        };

        // Try to load existing session synchronously during construction
        if let Ok(data) = std::fs::read_to_string(&manager.config.session_path) {
            if let Ok(session) = serde_json::from_str::<SessionData>(&data) {
                // We can't await here, so we use try_write
                if let Ok(mut guard) = manager.token.try_write() {
                    *guard = Some(SecretString::from(session.session_token));
                    tracing::info!(
                        "Loaded session token from {}",
                        manager.config.session_path.display()
                    );
                }
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
        };

        if let Err(e) = manager.load_session().await {
            tracing::debug!("No existing session found: {}", e);
        }

        manager
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
    /// it is assumed valid until a 401 response indicates otherwise.
    pub async fn ensure_authenticated(&self) -> Result<(), LlmError> {
        if self.has_token().await {
            tracing::debug!("Session token present, assuming valid");
            return Ok(());
        }

        // No token, need to authenticate
        self.initiate_login().await
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

    /// Start the OAuth login flow.
    ///
    /// 1. Find an available port for the callback server
    /// 2. Print the auth URL and attempt to open browser
    /// 3. Wait for OAuth callback with session token
    /// 4. Save and return the token
    async fn initiate_login(&self) -> Result<(), LlmError> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::TcpListener;

        // Find an available port
        let mut listener = None;
        let mut port = 0;

        for p in self.config.callback_port_range.0..=self.config.callback_port_range.1 {
            match TcpListener::bind(format!("127.0.0.1:{}", p)).await {
                Ok(l) => {
                    listener = Some(l);
                    port = p;
                    break;
                }
                Err(_) => continue,
            }
        }

        let listener = listener.ok_or_else(|| LlmError::SessionRenewalFailed {
            provider: "nearai".to_string(),
            reason: format!(
                "Could not find available port in range {}-{}",
                self.config.callback_port_range.0, self.config.callback_port_range.1
            ),
        })?;

        let callback_url = format!("http://127.0.0.1:{}", port);
        // Use GitHub OAuth (Google OAuth may not have the redirect URI configured)
        let auth_url = format!(
            "{}/v1/auth/github?frontend_callback={}",
            self.config.auth_base_url,
            urlencoding::encode(&callback_url)
        );

        // Print auth URL
        println!();
        println!("╔════════════════════════════════════════════════════════════════╗");
        println!("║                    NEAR AI Authentication                      ║");
        println!("╠════════════════════════════════════════════════════════════════╣");
        println!("║ Please open the following URL in your browser to authenticate: ║");
        println!("╚════════════════════════════════════════════════════════════════╝");
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

        // Wait for callback with timeout
        // The API redirects to: {frontend_callback}/auth/callback?token=X&session_id=X&expires_at=X&is_new_user=X
        let timeout = std::time::Duration::from_secs(300); // 5 minutes
        let (session_token, auth_provider) = tokio::time::timeout(timeout, async {
            loop {
                let (mut socket, _) = listener.accept().await.map_err(|e| {
                    LlmError::SessionRenewalFailed {
                        provider: "nearai".to_string(),
                        reason: format!("Failed to accept connection: {}", e),
                    }
                })?;

                let mut reader = BufReader::new(&mut socket);
                let mut request_line = String::new();
                reader.read_line(&mut request_line).await.map_err(|e| {
                    LlmError::SessionRenewalFailed {
                        provider: "nearai".to_string(),
                        reason: format!("Failed to read request: {}", e),
                    }
                })?;

                // Parse GET /auth/callback?token=xxx&session_id=xxx&expires_at=xxx&is_new_user=xxx HTTP/1.1
                if let Some(path) = request_line.split_whitespace().nth(1) {
                    if path.starts_with("/auth/callback") {
                        // Parse query parameters
                        if let Some(query) = path.split('?').nth(1) {
                            let mut token = None;

                            for param in query.split('&') {
                                let parts: Vec<&str> = param.splitn(2, '=').collect();
                                if parts.len() == 2 && parts[0] == "token" {
                                    token = Some(
                                        urlencoding::decode(parts[1])
                                            .unwrap_or_else(|_| parts[1].into())
                                            .into_owned(),
                                    );
                                }
                            }

                            if let Some(token) = token {
                                // Send success response
                                let response = concat!(
                                    "HTTP/1.1 200 OK\r\n",
                                    "Content-Type: text/html\r\n",
                                    "Connection: close\r\n",
                                    "\r\n",
                                    "<!DOCTYPE html><html><head><title>NEAR AI Auth</title></head>",
                                    "<body style=\"font-family: sans-serif; text-align: center; padding-top: 50px;\">",
                                    "<h1>✓ Authentication successful!</h1>",
                                    "<p>You can close this window and return to the terminal.</p>",
                                    "</body></html>"
                                );

                                let _ = socket.write_all(response.as_bytes()).await;
                                let _ = socket.shutdown().await;

                                // Provider is github since we used the github endpoint
                                return Ok::<_, LlmError>((token, Some("github".to_string())));
                            }
                        }
                    }
                }

                // Not the callback we're looking for, send 404
                let response = "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(response.as_bytes()).await;
            }
        })
        .await
        .map_err(|_| LlmError::SessionRenewalFailed {
            provider: "nearai".to_string(),
            reason: "Authentication timed out after 5 minutes".to_string(),
        })??;

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

    /// Save session data to disk.
    async fn save_session(&self, token: &str, auth_provider: Option<&str>) -> Result<(), LlmError> {
        let session = SessionData {
            session_token: token.to_string(),
            created_at: Utc::now(),
            auth_provider: auth_provider.map(String::from),
        };

        // Ensure parent directory exists
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

        tracing::debug!("Session saved to {}", self.config.session_path.display());
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

/// Create a session manager from a config, migrating from env var if present.
pub async fn create_session_manager(config: SessionConfig) -> Arc<SessionManager> {
    let manager = SessionManager::new_async(config).await;

    // Check for legacy env var and migrate if present and no file token
    if !manager.has_token().await {
        if let Ok(token) = std::env::var("NEARAI_SESSION_TOKEN") {
            if !token.is_empty() {
                tracing::info!("Migrating session token from NEARAI_SESSION_TOKEN env var to file");
                manager.set_token(SecretString::from(token.clone())).await;
                if let Err(e) = manager.save_session(&token, None).await {
                    tracing::warn!("Failed to save migrated session: {}", e);
                }
            }
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
            callback_port_range: (9900, 9910),
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
            callback_port_range: (9900, 9910),
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
        assert!(path.to_string_lossy().contains(".near-agent"));
    }
}
