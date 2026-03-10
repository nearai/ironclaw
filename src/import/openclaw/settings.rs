//! OpenClaw configuration to IronClaw settings mapping.

use secrecy::SecretString;
use std::collections::HashMap;

use super::reader::OpenClawConfig;

/// Map OpenClaw configuration to IronClaw settings (dotted-key format).
pub fn map_openclaw_config_to_settings(
    config: &OpenClawConfig,
) -> HashMap<String, serde_json::Value> {
    let mut settings = HashMap::new();

    // Map LLM configuration
    if let Some(ref llm) = config.llm {
        if let Some(ref provider) = llm.provider {
            settings.insert(
                "llm.backend".to_string(),
                serde_json::Value::String(provider.clone()),
            );
        }

        if let Some(ref model) = llm.model {
            settings.insert(
                "llm.selected_model".to_string(),
                serde_json::Value::String(model.clone()),
            );
        }

        if let Some(ref base_url) = llm.base_url {
            settings.insert(
                "llm.base_url".to_string(),
                serde_json::Value::String(base_url.clone()),
            );
        }
    }

    // Map embeddings configuration
    if let Some(ref emb) = config.embeddings {
        if let Some(ref model) = emb.model {
            settings.insert(
                "embeddings.model".to_string(),
                serde_json::Value::String(model.clone()),
            );
        }

        if let Some(ref provider) = emb.provider {
            settings.insert(
                "embeddings.provider".to_string(),
                serde_json::Value::String(provider.clone()),
            );
        }
    }

    // Map any other top-level settings
    for (key, value) in &config.other_settings {
        // Safely pass through JSON-serializable values
        settings.insert(key.clone(), value.clone());
    }

    settings
}

/// Extract credentials from OpenClaw configuration.
///
/// Returns a list of (secret_name, secret_value) pairs that should be stored.
/// Secret values are never logged or printed.
pub fn extract_credentials(config: &OpenClawConfig) -> Vec<(String, SecretString)> {
    let mut credentials = Vec::new();

    // Extract LLM API key if present
    if let Some(ref llm) = config.llm
        && let Some(ref api_key) = llm.api_key
    {
        credentials.push((
            "llm_api_key".to_string(),
            SecretString::new(api_key.clone().into_boxed_str()),
        ));
    }

    // Extract embeddings API key if present
    if let Some(ref emb) = config.embeddings
        && let Some(ref api_key) = emb.api_key
    {
        credentials.push((
            "embeddings_api_key".to_string(),
            SecretString::new(api_key.clone().into_boxed_str()),
        ));
    }

    credentials
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_llm_config() {
        let mut config = OpenClawConfig {
            llm: None,
            embeddings: None,
            other_settings: HashMap::new(),
        };

        config.llm = Some(super::super::reader::OpenClawLlmConfig {
            provider: Some("openai".to_string()),
            model: Some("gpt-4".to_string()),
            api_key: Some("secret".to_string()),
            base_url: None,
        });

        let settings = map_openclaw_config_to_settings(&config);

        assert_eq!(
            settings.get("llm.backend"),
            Some(&serde_json::Value::String("openai".to_string()))
        );
        assert_eq!(
            settings.get("llm.selected_model"),
            Some(&serde_json::Value::String("gpt-4".to_string()))
        );
    }

    #[test]
    fn test_extract_credentials_never_logs() {
        let mut config = OpenClawConfig {
            llm: None,
            embeddings: None,
            other_settings: HashMap::new(),
        };

        config.llm = Some(super::super::reader::OpenClawLlmConfig {
            provider: Some("anthropic".to_string()),
            model: Some("claude-3".to_string()),
            api_key: Some("secret-key-value".to_string()),
            base_url: None,
        });

        let creds = extract_credentials(&config);
        assert_eq!(creds.len(), 1);
        assert_eq!(creds[0].0, "llm_api_key");
        // Verify the value is wrapped in SecretString (never exposed in Debug output)
        assert!(!format!("{:?}", creds[0].1).contains("secret-key-value"));
    }
}
