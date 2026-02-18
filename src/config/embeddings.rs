use secrecy::{ExposeSecret, SecretString};

use crate::config::helpers::optional_env;
use crate::error::ConfigError;
use crate::settings::Settings;

/// Embeddings provider configuration.
#[derive(Debug, Clone)]
pub struct EmbeddingsConfig {
    /// Whether embeddings are enabled.
    pub enabled: bool,
    /// Provider to use: "openai" or "nearai"
    pub provider: String,
    /// OpenAI API key (for OpenAI provider).
    pub openai_api_key: Option<SecretString>,
    /// Model to use for embeddings.
    pub model: String,
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "openai".to_string(),
            openai_api_key: None,
            model: "text-embedding-3-small".to_string(),
        }
    }
}

impl EmbeddingsConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let openai_api_key = optional_env("OPENAI_API_KEY")?.map(SecretString::from);

        let provider = optional_env("EMBEDDING_PROVIDER")?
            .unwrap_or_else(|| settings.embeddings.provider.clone());

        let model =
            optional_env("EMBEDDING_MODEL")?.unwrap_or_else(|| settings.embeddings.model.clone());

        let enabled = optional_env("EMBEDDING_ENABLED")?
            .map(|s| s.parse())
            .transpose()
            .map_err(|e| ConfigError::InvalidValue {
                key: "EMBEDDING_ENABLED".to_string(),
                message: format!("must be 'true' or 'false': {e}"),
            })?
            .unwrap_or_else(|| settings.embeddings.enabled || openai_api_key.is_some());

        Ok(Self {
            enabled,
            provider,
            openai_api_key,
            model,
        })
    }

    /// Get the OpenAI API key if configured.
    pub fn openai_api_key(&self) -> Option<&str> {
        self.openai_api_key.as_ref().map(|s| s.expose_secret())
    }
}
