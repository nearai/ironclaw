//! Local Codex profile loader for onboarding.
//!
//! Reads credentials from:
//! - `$CODEX_HOME/auth.json` (or `~/.codex/auth.json`)
//! - `$CODEX_HOME/config.toml` (or `~/.codex/config.toml`)
//!
//! This path supports API-key mode only and requires `wire_api = "responses"`.

use std::collections::HashMap;
use std::path::PathBuf;

use secrecy::SecretString;
use serde::Deserialize;

pub(crate) const CODEX_LOCAL_BACKEND: &str = "codex_local";
pub(crate) const CODEX_LOCAL_API_KEY_ENV: &str = "CODEX_LOCAL_API_KEY";
pub(crate) const CODEX_LOCAL_BASE_URL_ENV: &str = "CODEX_LOCAL_BASE_URL";
pub(crate) const CODEX_LOCAL_MODEL_ENV: &str = "CODEX_LOCAL_MODEL";

const REQUIRED_WIRE_API: &str = "responses";

#[derive(Debug, Clone)]
pub(crate) struct CodexLocalProfile {
    pub codex_home: PathBuf,
    pub auth_path: PathBuf,
    pub config_path: PathBuf,
    pub api_key: SecretString,
    pub base_url: String,
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexConfigToml {
    model_provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    model_providers: HashMap<String, CodexModelProviderToml>,
}

#[derive(Debug, Deserialize)]
struct CodexModelProviderToml {
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    wire_api: Option<String>,
}

pub(crate) fn codex_home_dir() -> PathBuf {
    if let Ok(path) = std::env::var("CODEX_HOME") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    let home_dir = dirs::home_dir().unwrap_or_else(|| {
        tracing::warn!(
            "Could not determine home directory; using current directory for local Codex paths"
        );
        PathBuf::from(".")
    });
    home_dir.join(".codex")
}

pub(crate) fn auth_path() -> PathBuf {
    codex_home_dir().join("auth.json")
}

pub(crate) fn config_path() -> PathBuf {
    codex_home_dir().join("config.toml")
}

pub(crate) fn has_local_codex_files() -> bool {
    auth_path().exists() && config_path().exists()
}

pub(crate) fn load_local_profile() -> Result<CodexLocalProfile, String> {
    let codex_home = codex_home_dir();
    let auth_path = codex_home.join("auth.json");
    let config_path = codex_home.join("config.toml");

    if !auth_path.exists() {
        return Err(format!(
            "Codex auth file not found: {}",
            auth_path.display()
        ));
    }
    if !config_path.exists() {
        return Err(format!(
            "Codex config file not found: {}",
            config_path.display()
        ));
    }

    let creds = crate::llm::codex_auth::load_codex_credentials(&auth_path).ok_or_else(|| {
        format!(
            "Failed to load usable credentials from {}",
            auth_path.display()
        )
    })?;

    if creds.is_chatgpt_mode {
        return Err(
            "Detected ChatGPT token mode in auth.json. Use `openai_codex` (device OAuth) instead."
                .to_string(),
        );
    }

    let config_raw = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read {}: {}", config_path.display(), e))?;
    let parsed: CodexConfigToml = toml::from_str(&config_raw)
        .map_err(|e| format!("Invalid TOML in {}: {}", config_path.display(), e))?;

    let provider_id = parsed
        .model_provider
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "Missing `model_provider` in {}",
                config_path.as_path().display()
            )
        })?;

    let provider = parsed.model_providers.get(provider_id).ok_or_else(|| {
        format!(
            "Missing [model_providers.{provider_id}] section in {}",
            config_path.as_path().display()
        )
    })?;

    let base_url = provider
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "Missing `base_url` for provider '{provider_id}' in {}",
                config_path.as_path().display()
            )
        })?
        .to_string();

    let wire_api = provider
        .wire_api
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "Missing `wire_api` for provider '{provider_id}' in {} (must be '{REQUIRED_WIRE_API}')",
                config_path.as_path().display()
            )
        })?;

    if wire_api != REQUIRED_WIRE_API {
        return Err(format!(
            "Unsupported wire_api '{wire_api}' in {}. Required: '{REQUIRED_WIRE_API}'",
            config_path.as_path().display()
        ));
    }

    let model = parsed
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    Ok(CodexLocalProfile {
        codex_home,
        auth_path,
        config_path,
        api_key: creds.token,
        base_url,
        model,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::config::helpers::lock_env;

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: tests serialize env mutations with ENV_MUTEX.
            unsafe { std::env::set_var(key, value) };
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: tests serialize env mutations with ENV_MUTEX.
            unsafe {
                if let Some(ref val) = self.original {
                    std::env::set_var(self.key, val);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    fn write_api_key_auth(path: &Path) {
        std::fs::write(path, r#"{"OPENAI_API_KEY":"sk-test-local-codex"}"#).unwrap();
    }

    fn write_chatgpt_auth(path: &Path) {
        std::fs::write(
            path,
            r#"{
  "auth_mode":"chatgpt",
  "tokens":{
    "access_token":"test-access-token",
    "refresh_token":"test-refresh-token"
  }
}"#,
        )
        .unwrap();
    }

    fn write_config(path: &Path, wire_api: &str) {
        let content = format!(
            r#"
model_provider = "packycode"
model = "gpt-5.3-codex"

[model_providers.packycode]
base_url = "https://codex-api.packycode.com/v1"
wire_api = "{wire_api}"
"#
        );
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn load_local_profile_success() {
        let _lock = lock_env();
        let dir = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CODEX_HOME", dir.path().to_string_lossy().as_ref());

        write_api_key_auth(&dir.path().join("auth.json"));
        write_config(&dir.path().join("config.toml"), "responses");

        let profile = load_local_profile().expect("profile should load");
        assert_eq!(profile.base_url, "https://codex-api.packycode.com/v1");
        assert_eq!(profile.model.as_deref(), Some("gpt-5.3-codex"));
        assert!(profile.auth_path.ends_with("auth.json"));
        assert!(profile.config_path.ends_with("config.toml"));
    }

    #[test]
    fn has_local_codex_files_checks_both() {
        let _lock = lock_env();
        let dir = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CODEX_HOME", dir.path().to_string_lossy().as_ref());

        assert!(!has_local_codex_files());
        write_api_key_auth(&dir.path().join("auth.json"));
        assert!(!has_local_codex_files());
        write_config(&dir.path().join("config.toml"), "responses");
        assert!(has_local_codex_files());
    }

    #[test]
    fn load_local_profile_rejects_chatgpt_mode() {
        let _lock = lock_env();
        let dir = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CODEX_HOME", dir.path().to_string_lossy().as_ref());

        write_chatgpt_auth(&dir.path().join("auth.json"));
        write_config(&dir.path().join("config.toml"), "responses");

        let err = load_local_profile().unwrap_err();
        assert!(
            err.contains("openai_codex"),
            "error should guide user to openai_codex: {err}"
        );
    }

    #[test]
    fn load_local_profile_rejects_non_responses_wire_api() {
        let _lock = lock_env();
        let dir = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CODEX_HOME", dir.path().to_string_lossy().as_ref());

        write_api_key_auth(&dir.path().join("auth.json"));
        write_config(&dir.path().join("config.toml"), "chat_completions");

        let err = load_local_profile().unwrap_err();
        assert!(
            err.contains("wire_api"),
            "error should mention wire_api mismatch: {err}"
        );
    }

    #[test]
    fn load_local_profile_missing_config_errors() {
        let _lock = lock_env();
        let dir = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("CODEX_HOME", dir.path().to_string_lossy().as_ref());
        write_api_key_auth(&dir.path().join("auth.json"));

        let err = load_local_profile().unwrap_err();
        assert!(err.contains("config file"), "unexpected error: {err}");
    }
}
