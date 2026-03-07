//! Read Codex CLI credentials for LLM authentication.
//!
//! When `LLM_USE_CODEX_AUTH=true`, IronClaw reads the Codex CLI's
//! `auth.json` file (default: `~/.codex/auth.json`) and extracts an
//! API key or OAuth access token. This lets IronClaw piggyback on a
//! Codex login without implementing its own OAuth flow.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Partial representation of Codex's `$CODEX_HOME/auth.json`.
///
/// We only deserialize the fields we need, keeping this decoupled
/// from the Codex crate itself.
#[derive(Debug, Deserialize)]
struct CodexAuthJson {
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

/// Load an API key from a Codex `auth.json` file.
///
/// Resolution order:
/// 1. `OPENAI_API_KEY` field (set when user logged in with an API key)
/// 2. `tokens.access_token` (set when user logged in via ChatGPT OAuth)
///
/// Returns `None` if the file is missing, unreadable, or contains
/// neither a key nor tokens.
pub fn load_codex_api_key(path: &Path) -> Option<String> {
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

    // Prefer explicit API key over OAuth token.
    if let Some(key) = auth.openai_api_key.filter(|k| !k.is_empty()) {
        tracing::info!("Loaded API key from Codex auth.json (API key mode)");
        return Some(key);
    }

    if let Some(tokens) = auth.tokens {
        if !tokens.access_token.is_empty() {
            tracing::info!("Loaded access token from Codex auth.json (ChatGPT mode)");
            return Some(tokens.access_token);
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
            r#"{{"OPENAI_API_KEY":"sk-test-123","auth_mode":"apiKey"}}"#
        )
        .unwrap();
        let key = load_codex_api_key(f.path()).expect("should load key");
        assert_eq!(key, "sk-test-123");
    }

    #[test]
    fn loads_chatgpt_access_token() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":{{}},"access_token":"eyJ-test","refresh_token":"rt-x"}}}}"#
        )
        .unwrap();
        let key = load_codex_api_key(f.path()).expect("should load token");
        assert_eq!(key, "eyJ-test");
    }

    #[test]
    fn prefers_api_key_over_token() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"{{"OPENAI_API_KEY":"sk-priority","tokens":{{"id_token":{{}},"access_token":"eyJ-fallback","refresh_token":"rt-x"}}}}"#
        )
        .unwrap();
        let key = load_codex_api_key(f.path()).expect("should load key");
        assert_eq!(key, "sk-priority");
    }

    #[test]
    fn returns_none_for_missing_file() {
        let key = load_codex_api_key(Path::new("/tmp/nonexistent_auth.json"));
        assert!(key.is_none());
    }

    #[test]
    fn returns_none_for_empty_json() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "{{}}").unwrap();
        let key = load_codex_api_key(f.path());
        assert!(key.is_none());
    }

    #[test]
    fn returns_none_for_empty_key() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"OPENAI_API_KEY":""}}"#).unwrap();
        let key = load_codex_api_key(f.path());
        assert!(key.is_none());
    }
}
