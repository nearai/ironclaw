//! OpenClaw configuration parsing.
//!
//! Parses OpenClaw's JSON5 config file and OAuth credentials, mapping
//! known keys to IronClaw settings and extracting API credentials.

use std::collections::HashMap;
use std::path::Path;

use crate::import::ImportError;

/// A credential extracted from the OpenClaw config or OAuth file.
#[derive(Debug, Clone)]
pub struct CredentialEntry {
    /// Secret name (e.g., `openai_api_key`).
    pub name: String,
    /// Plaintext value.
    pub value: String,
    /// Optional provider hint (e.g., `openai`, `anthropic`).
    pub provider: Option<String>,
}

/// Parsed OpenClaw configuration.
#[derive(Debug)]
pub struct OpenClawConfig {
    /// The raw parsed JSON value.
    pub raw: serde_json::Value,
    /// Settings mapped to IronClaw dotted keys.
    pub mapped_settings: HashMap<String, serde_json::Value>,
    /// Extracted credentials (API keys, OAuth tokens).
    pub credentials: Vec<CredentialEntry>,
}

impl OpenClawConfig {
    /// Parse the OpenClaw config file and optional OAuth credentials.
    pub fn parse(config_path: &Path, oauth_path: Option<&Path>) -> Result<Self, ImportError> {
        let mut mapped_settings = HashMap::new();
        let mut credentials = Vec::new();

        // Parse config (JSON5 allows comments and trailing commas)
        let raw = if config_path.is_file() {
            let content = std::fs::read_to_string(config_path)?;
            let val: serde_json::Value = json5::from_str(&content).map_err(|e| {
                ImportError::ConfigParse(format!("{}: {}", config_path.display(), e))
            })?;
            Self::map_settings(&val, &mut mapped_settings);
            Self::extract_credentials(&val, &mut credentials);
            val
        } else {
            serde_json::Value::Object(serde_json::Map::new())
        };

        // Parse OAuth tokens
        if let Some(oauth_path) = oauth_path
            && oauth_path.is_file()
            && let Err(e) = Self::parse_oauth(oauth_path, &mut credentials)
        {
            tracing::warn!("Failed to parse OAuth file: {}", e);
        }

        Ok(Self {
            raw,
            mapped_settings,
            credentials,
        })
    }

    /// Map known OpenClaw config keys to IronClaw settings.
    fn map_settings(val: &serde_json::Value, settings: &mut HashMap<String, serde_json::Value>) {
        // agents.defaults.provider -> llm_backend
        if let Some(provider) = val
            .pointer("/agents/defaults/provider")
            .and_then(|v| v.as_str())
        {
            let backend = match provider.to_lowercase().as_str() {
                "openai" | "gpt" => "openai",
                "anthropic" | "claude" => "anthropic",
                "ollama" => "ollama",
                "nearai" | "near" => "nearai",
                _ => provider,
            };
            settings.insert(
                "llm_backend".to_string(),
                serde_json::Value::String(backend.to_string()),
            );
        }

        // agents.defaults.model -> selected_model
        if let Some(model) = val
            .pointer("/agents/defaults/model")
            .and_then(|v| v.as_str())
        {
            settings.insert(
                "selected_model".to_string(),
                serde_json::Value::String(model.to_string()),
            );
        }

        // agents.defaults.memorySearch.provider -> embeddings.provider
        if let Some(emb_provider) = val
            .pointer("/agents/defaults/memorySearch/provider")
            .and_then(|v| v.as_str())
        {
            settings.insert(
                "embeddings.provider".to_string(),
                serde_json::Value::String(emb_provider.to_string()),
            );
        }

        // agents.defaults.memorySearch.model -> embeddings.model
        if let Some(emb_model) = val
            .pointer("/agents/defaults/memorySearch/model")
            .and_then(|v| v.as_str())
        {
            settings.insert(
                "embeddings.model".to_string(),
                serde_json::Value::String(emb_model.to_string()),
            );
        }
    }

    /// Extract API keys from auth.profiles where mode == "api_key".
    fn extract_credentials(val: &serde_json::Value, credentials: &mut Vec<CredentialEntry>) {
        let Some(profiles) = val.pointer("/auth/profiles").and_then(|v| v.as_object()) else {
            return;
        };

        for (profile_id, profile) in profiles {
            let mode = profile.get("mode").and_then(|v| v.as_str()).unwrap_or("");
            let provider = profile
                .get("provider")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            if mode == "api_key" {
                // Look for the key in common field names
                let key_value = profile
                    .get("apiKey")
                    .or_else(|| profile.get("api_key"))
                    .or_else(|| profile.get("key"))
                    .and_then(|v| v.as_str());

                if let Some(key) = key_value {
                    let name = if let Some(ref p) = provider {
                        format!("{}_api_key", p.to_lowercase())
                    } else {
                        format!("{}_api_key", profile_id.to_lowercase())
                    };

                    credentials.push(CredentialEntry {
                        name,
                        value: key.to_string(),
                        provider,
                    });
                }
            }
        }
    }

    /// Parse OAuth tokens from the credentials/oauth.json file.
    fn parse_oauth(
        oauth_path: &Path,
        credentials: &mut Vec<CredentialEntry>,
    ) -> Result<(), ImportError> {
        let content = std::fs::read_to_string(oauth_path)?;
        let val: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| ImportError::ConfigParse(format!("oauth.json: {}", e)))?;

        // OAuth file is typically { "provider_name": { "access_token": "...", ... } }
        let Some(obj) = val.as_object() else {
            return Ok(());
        };

        for (provider, tokens) in obj {
            if let Some(token) = tokens
                .get("access_token")
                .or_else(|| tokens.get("token"))
                .and_then(|v| v.as_str())
            {
                credentials.push(CredentialEntry {
                    name: format!("{}_oauth_token", provider.to_lowercase()),
                    value: token.to_string(),
                    provider: Some(provider.clone()),
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json5_config_with_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("openclaw.json");

        // JSON5 with comments and trailing commas
        std::fs::write(
            &config_path,
            r#"{
                // Main config
                "auth": {
                    "profiles": {
                        "main": {
                            "provider": "openai",
                            "mode": "api_key",
                            "apiKey": "sk-test-12345",
                        },
                    },
                },
                "agents": {
                    "defaults": {
                        "provider": "openai",
                        "model": "gpt-4o",
                        "memorySearch": {
                            "provider": "openai",
                            "model": "text-embedding-3-small",
                        },
                    },
                },
            }"#,
        )
        .unwrap();

        let config = OpenClawConfig::parse(&config_path, None).unwrap();

        // Check mapped settings
        assert_eq!(
            config.mapped_settings.get("llm_backend"),
            Some(&serde_json::Value::String("openai".to_string()))
        );
        assert_eq!(
            config.mapped_settings.get("selected_model"),
            Some(&serde_json::Value::String("gpt-4o".to_string()))
        );
        assert_eq!(
            config.mapped_settings.get("embeddings.provider"),
            Some(&serde_json::Value::String("openai".to_string()))
        );
        assert_eq!(
            config.mapped_settings.get("embeddings.model"),
            Some(&serde_json::Value::String(
                "text-embedding-3-small".to_string()
            ))
        );

        // Check credentials
        assert_eq!(config.credentials.len(), 1);
        assert_eq!(config.credentials[0].name, "openai_api_key");
        assert_eq!(config.credentials[0].value, "sk-test-12345");
    }

    #[test]
    fn parse_oauth_file() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("openclaw.json");
        let oauth_path = tmp.path().join("oauth.json");

        std::fs::write(&config_path, "{}").unwrap();
        std::fs::write(
            &oauth_path,
            r#"{
                "github": {"access_token": "ghp_test123"},
                "google": {"access_token": "ya29.test456"}
            }"#,
        )
        .unwrap();

        let config = OpenClawConfig::parse(&config_path, Some(&oauth_path)).unwrap();

        assert_eq!(config.credentials.len(), 2);
        let names: Vec<&str> = config.credentials.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"github_oauth_token"));
        assert!(names.contains(&"google_oauth_token"));
    }

    #[test]
    fn parse_missing_config_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("nonexistent.json");

        let config = OpenClawConfig::parse(&config_path, None).unwrap();
        assert!(config.mapped_settings.is_empty());
        assert!(config.credentials.is_empty());
    }

    #[test]
    fn parse_various_content_formats() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("openclaw.json");

        // Empty object
        std::fs::write(&config_path, "{}").unwrap();
        let config = OpenClawConfig::parse(&config_path, None).unwrap();
        assert!(config.mapped_settings.is_empty());

        // Anthropic provider mapping
        std::fs::write(
            &config_path,
            r#"{"agents": {"defaults": {"provider": "claude"}}}"#,
        )
        .unwrap();
        let config = OpenClawConfig::parse(&config_path, None).unwrap();
        assert_eq!(
            config.mapped_settings.get("llm_backend"),
            Some(&serde_json::Value::String("anthropic".to_string()))
        );
    }
}
