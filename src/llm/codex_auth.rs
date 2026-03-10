//! Read Codex CLI credentials for LLM authentication.
//!
//! When `LLM_USE_CODEX_AUTH=true`, IronClaw reads the Codex CLI's
//! `auth.json` file (default: `~/.codex/auth.json`) and extracts
//! credentials. This lets IronClaw piggyback on a Codex login without
//! implementing its own OAuth flow.
//!
//! Codex supports two auth modes:
//! - **API key** (`auth_mode: "apiKey"`) → uses `OPENAI_API_KEY` field
//!   against `api.openai.com/v1`.
//! - **ChatGPT** (`auth_mode: "chatgpt"`) → uses `tokens.access_token`
//!   (OAuth JWT) against `chatgpt.com/backend-api/codex`.
//!
//! When in ChatGPT mode, the provider supports automatic token refresh
//! on 401 responses using the `refresh_token` from `auth.json`.

use std::path::{Path, PathBuf};

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

/// ChatGPT backend API endpoint used by Codex in ChatGPT auth mode.
const CHATGPT_BACKEND_URL: &str = "https://chatgpt.com/backend-api/codex";

/// Standard OpenAI API endpoint used by Codex in API key mode.
const OPENAI_API_URL: &str = "https://api.openai.com/v1";

/// OAuth token refresh endpoint (same as Codex CLI).
const REFRESH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// OAuth client ID used for token refresh (same as Codex CLI).
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Credentials extracted from Codex's `auth.json`.
#[derive(Debug, Clone)]
pub struct CodexCredentials {
    /// The bearer token (API key or ChatGPT access_token).
    pub token: SecretString,
    /// Whether this is a ChatGPT OAuth token (vs. an OpenAI API key).
    pub is_chatgpt_mode: bool,
    /// OAuth refresh token (only present in ChatGPT mode).
    pub refresh_token: Option<SecretString>,
    /// Path to the auth.json file (for persisting refreshed tokens).
    pub auth_path: Option<PathBuf>,
}

impl CodexCredentials {
    /// Returns the correct base URL for the auth mode.
    ///
    /// - ChatGPT mode → `https://chatgpt.com/backend-api/codex`
    /// - API key mode → `https://api.openai.com/v1`
    pub fn base_url(&self) -> &'static str {
        if self.is_chatgpt_mode {
            CHATGPT_BACKEND_URL
        } else {
            OPENAI_API_URL
        }
    }
}

/// Partial representation of Codex's `$CODEX_HOME/auth.json`.
#[derive(Debug, Deserialize)]
struct CodexAuthJson {
    auth_mode: Option<String>,
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    tokens: Option<CodexTokens>,
}

#[derive(Debug, Deserialize)]
struct CodexTokens {
    access_token: SecretString,
    refresh_token: Option<SecretString>,
}

/// Request body for OAuth token refresh.
#[derive(Debug, Serialize)]
struct RefreshRequest<'a> {
    client_id: &'a str,
    grant_type: &'a str,
    refresh_token: &'a str,
}

/// Response from the OAuth token refresh endpoint.
#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: SecretString,
    refresh_token: Option<SecretString>,
}

/// Default path used by Codex CLI: `~/.codex/auth.json`.
pub fn default_codex_auth_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("auth.json")
}

/// Load credentials from a Codex `auth.json` file.
///
/// Returns `None` if the file is missing, unreadable, or contains
/// no usable credentials.
pub fn load_codex_credentials(path: &Path) -> Option<CodexCredentials> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Could not read Codex auth file {}: {}", path.display(), e);
            return None;
        }
    };

    let auth: CodexAuthJson = match serde_json::from_str(&content) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("Failed to parse Codex auth file {}: {}", path.display(), e);
            return None;
        }
    };

    let is_chatgpt = auth
        .auth_mode
        .as_deref()
        .map(|m| m == "chatgpt" || m == "chatgptAuthTokens")
        .unwrap_or(false);

    // API key mode: use OPENAI_API_KEY field.
    if !is_chatgpt {
        if let Some(key) = auth.openai_api_key.filter(|k| !k.is_empty()) {
            tracing::info!("Loaded API key from Codex auth.json (API key mode)");
            return Some(CodexCredentials {
                token: SecretString::from(key),
                is_chatgpt_mode: false,
                refresh_token: None,
                auth_path: None,
            });
        }
        // If auth_mode was explicitly `apiKey`, do not fall back to checking for a token.
        if auth.auth_mode.is_some() {
            return None;
        }
    }

    // ChatGPT mode: use access_token as bearer token.
    if let Some(tokens) = auth.tokens
        && !tokens.access_token.expose_secret().is_empty()
    {
        tracing::info!(
            "Loaded access token from Codex auth.json (ChatGPT mode, base_url={})",
            CHATGPT_BACKEND_URL
        );
        return Some(CodexCredentials {
            token: tokens.access_token,
            is_chatgpt_mode: true,
            refresh_token: tokens.refresh_token,
            auth_path: Some(path.to_path_buf()),
        });
    }

    tracing::debug!(
        "Codex auth.json at {} contains no usable credentials",
        path.display()
    );
    None
}

/// Attempt to refresh an expired access token using the refresh token.
///
/// On success, returns the new `access_token` and persists the refreshed
/// tokens back to `auth.json`. This follows the same OAuth protocol as
/// Codex CLI (`POST https://auth.openai.com/oauth/token`).
///
/// Returns `None` if the refresh token is missing, the request fails,
/// or the response is malformed.
pub async fn refresh_access_token(
    client: &reqwest::Client,
    refresh_token: &SecretString,
    auth_path: Option<&Path>,
) -> Option<SecretString> {
    let req = RefreshRequest {
        client_id: CLIENT_ID,
        grant_type: "refresh_token",
        refresh_token: refresh_token.expose_secret(),
    };

    tracing::info!("Attempting to refresh Codex OAuth access token");

    let resp = match client
        .post(REFRESH_TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&req)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Token refresh request failed: {e}");
            return None;
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!("Token refresh failed: HTTP {status}: {body}");
        if status.as_u16() == 401 {
            tracing::warn!(
                "Refresh token may be expired or revoked. \
                 Please re-authenticate with: codex --login"
            );
        }
        return None;
    }

    let refresh_resp: RefreshResponse = match resp.json().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Failed to parse token refresh response: {e}");
            return None;
        }
    };

    let new_access_token = refresh_resp.access_token.clone();

    // Persist refreshed tokens back to auth.json
    if let Some(path) = auth_path {
        if let Err(e) = persist_refreshed_tokens(
            path,
            refresh_resp.access_token.expose_secret(),
            refresh_resp
                .refresh_token
                .as_ref()
                .map(ExposeSecret::expose_secret),
        ) {
            tracing::warn!(
                "Failed to persist refreshed tokens to {}: {e}",
                path.display()
            );
        } else {
            tracing::info!("Refreshed tokens persisted to {}", path.display());
        }
    }

    Some(new_access_token)
}

/// Update `auth.json` with refreshed tokens, preserving other fields.
fn persist_refreshed_tokens(
    path: &Path,
    new_access_token: &str,
    new_refresh_token: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let mut json: serde_json::Value = serde_json::from_str(&content)?;

    if let Some(tokens) = json.get_mut("tokens") {
        tokens["access_token"] = serde_json::Value::String(new_access_token.to_string());
        if let Some(rt) = new_refresh_token {
            tokens["refresh_token"] = serde_json::Value::String(rt.to_string());
        }
    }

    let updated = serde_json::to_string_pretty(&json)?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, updated)?;
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(Box::new(e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn loads_api_key_mode() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"{{"auth_mode":"apiKey","OPENAI_API_KEY":"sk-test-123"}}"#
        )
        .unwrap();
        let creds = load_codex_credentials(f.path()).expect("should load");
        assert_eq!(creds.token.expose_secret(), "sk-test-123");
        assert!(!creds.is_chatgpt_mode);
        assert_eq!(creds.base_url(), OPENAI_API_URL);
    }

    #[test]
    fn loads_chatgpt_mode() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":{{}},"access_token":"eyJ-test","refresh_token":"rt-x"}}}}"#
        )
        .unwrap();
        let creds = load_codex_credentials(f.path()).expect("should load");
        assert_eq!(creds.token.expose_secret(), "eyJ-test");
        assert!(creds.is_chatgpt_mode);
        assert_eq!(
            creds
                .refresh_token
                .as_ref()
                .expect("refresh token should be present")
                .expose_secret(),
            "rt-x"
        );
        assert_eq!(creds.base_url(), CHATGPT_BACKEND_URL);
    }

    #[test]
    fn api_key_mode_ignores_tokens() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"{{"auth_mode":"apiKey","OPENAI_API_KEY":"sk-priority","tokens":{{"id_token":{{}},"access_token":"eyJ-fallback","refresh_token":"rt-x"}}}}"#
        )
        .unwrap();
        let creds = load_codex_credentials(f.path()).expect("should load");
        assert_eq!(creds.token.expose_secret(), "sk-priority");
        assert!(!creds.is_chatgpt_mode);
    }

    #[test]
    fn returns_none_for_missing_file() {
        assert!(load_codex_credentials(Path::new("/tmp/nonexistent_codex_auth.json")).is_none());
    }

    #[test]
    fn returns_none_for_empty_json() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "{{}}").unwrap();
        assert!(load_codex_credentials(f.path()).is_none());
    }

    #[test]
    fn returns_none_for_empty_key() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"auth_mode":"apiKey","OPENAI_API_KEY":""}}"#).unwrap();
        assert!(load_codex_credentials(f.path()).is_none());
    }

    #[test]
    fn api_key_mode_missing_key_does_not_fallback_to_chatgpt() {
        // Bug: if auth_mode is "apiKey" but key is missing, the old code would
        // fall through to check for a ChatGPT token, returning is_chatgpt_mode: true.
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"{{"auth_mode":"apiKey","OPENAI_API_KEY":"","tokens":{{"id_token":{{}},"access_token":"eyJ-bad","refresh_token":"rt-x"}}}}"#
        )
        .unwrap();
        assert!(load_codex_credentials(f.path()).is_none());
    }
}
