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

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// ChatGPT backend API endpoint used by Codex in ChatGPT auth mode.
const CHATGPT_BACKEND_URL: &str = "https://chatgpt.com/backend-api/codex";

/// Standard OpenAI API endpoint used by Codex in API key mode.
const OPENAI_API_URL: &str = "https://api.openai.com/v1";

/// Credentials extracted from Codex's `auth.json`.
#[derive(Debug, Clone)]
pub struct CodexCredentials {
    /// The bearer token (API key or ChatGPT access_token).
    pub token: String,
    /// Whether this is a ChatGPT OAuth token (vs. an OpenAI API key).
    pub is_chatgpt_mode: bool,
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
    access_token: String,
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
            tracing::warn!(
                "Failed to parse Codex auth file {}: {}",
                path.display(),
                e
            );
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
                token: key,
                is_chatgpt_mode: false,
            });
        }
        // If auth_mode was explicitly `apiKey`, do not fall back to checking for a token.
        if auth.auth_mode.is_some() {
            return None;
        }
    }

    // ChatGPT mode: use access_token as bearer token.
    if let Some(tokens) = auth.tokens {
        if !tokens.access_token.is_empty() {
            tracing::info!(
                "Loaded access token from Codex auth.json (ChatGPT mode, base_url={})",
                CHATGPT_BACKEND_URL
            );
            return Some(CodexCredentials {
                token: tokens.access_token,
                is_chatgpt_mode: true,
            });
        }
    }

    tracing::debug!(
        "Codex auth.json at {} contains no usable credentials",
        path.display()
    );
    None
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
        assert_eq!(creds.token, "sk-test-123");
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
        assert_eq!(creds.token, "eyJ-test");
        assert!(creds.is_chatgpt_mode);
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
        assert_eq!(creds.token, "sk-priority");
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
