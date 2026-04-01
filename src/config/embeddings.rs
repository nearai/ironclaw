use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};

use crate::config::helpers::{
    allow_local_network, db_first_bool, db_first_or_default, optional_env, parse_optional_env,
    validate_base_url,
};
use crate::error::ConfigError;
use crate::llm::SessionManager;
use crate::settings::Settings;
use crate::workspace::EmbeddingProvider;

/// Default maximum number of cached embeddings.
pub const DEFAULT_EMBEDDING_CACHE_SIZE: usize = 10_000;

/// Embeddings provider configuration.
#[derive(Debug, Clone)]
pub struct EmbeddingsConfig {
    /// Whether embeddings are enabled.
    pub enabled: bool,
    /// Provider to use: "openai", "nearai", or "ollama"
    pub provider: String,
    /// OpenAI API key (for OpenAI provider).
    pub openai_api_key: Option<SecretString>,
    /// Model to use for embeddings.
    pub model: String,
    /// Ollama base URL (for Ollama provider). Defaults to http://localhost:11434.
    pub ollama_base_url: String,
    /// Embedding vector dimension. Inferred from the model name when not set explicitly.
    pub dimension: usize,
    /// Custom base URL for OpenAI-compatible embedding providers.
    /// When set, overrides the default `https://api.openai.com`.
    pub openai_base_url: Option<String>,
    /// Maximum entries in the embedding LRU cache (default 10,000).
    ///
    /// Approximate raw embedding payload: `cache_size × dimension × 4 bytes`.
    /// 10,000 × 1536 floats ≈ 58 MB (payload only; actual memory is higher
    /// due to HashMap buckets, per-entry Vec/timestamp overhead).
    pub cache_size: usize,
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        let model = "text-embedding-3-small".to_string();
        let dimension = default_dimension_for_model(&model);
        Self {
            enabled: false,
            provider: "openai".to_string(),
            openai_api_key: None,
            model,
            ollama_base_url: "http://localhost:11434".to_string(),
            dimension,
            openai_base_url: None,
            cache_size: DEFAULT_EMBEDDING_CACHE_SIZE,
        }
    }
}

/// Infer the embedding dimension from a well-known model name.
///
/// Falls back to 1536 (OpenAI text-embedding-3-small default) for unknown models.
pub(crate) fn default_dimension_for_model(model: &str) -> usize {
    match model {
        "text-embedding-3-small" => 1536,
        "text-embedding-3-large" => 3072,
        "text-embedding-ada-002" => 1536,
        "nomic-embed-text" => 768,
        "mxbai-embed-large" => 1024,
        "all-minilm" => 384,
        _ => 1536,
    }
}

impl EmbeddingsConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = crate::settings::EmbeddingsSettings::default();

        let openai_api_key = optional_env("OPENAI_API_KEY")?.map(SecretString::from);

        let provider = db_first_or_default(
            &settings.embeddings.provider,
            &defaults.provider,
            "EMBEDDING_PROVIDER",
        )?;

        let model = db_first_or_default(
            &settings.embeddings.model,
            &defaults.model,
            "EMBEDDING_MODEL",
        )?;

        // ollama_base_url lives on the top-level Settings, not the embeddings
        // sub-struct. Use a manual DB > env > default chain.
        let default_ollama_url = "http://localhost:11434".to_string();
        let ollama_base_url = match settings
            .ollama_base_url
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
        {
            Some(url) => url,
            None => optional_env("OLLAMA_BASE_URL")?.unwrap_or(default_ollama_url),
        };

        // Dimension depends on the resolved model, not on a DB setting — env-only.
        let dimension =
            parse_optional_env("EMBEDDING_DIMENSION", default_dimension_for_model(&model))?;

        let enabled = db_first_bool(
            settings.embeddings.enabled,
            defaults.enabled,
            "EMBEDDING_ENABLED",
        )?;

        let openai_base_url = optional_env("EMBEDDING_BASE_URL")?;

        // Validate base URLs to prevent SSRF attacks (#1103).
        let local_net = allow_local_network()?;
        validate_base_url(&ollama_base_url, "OLLAMA_BASE_URL", local_net, true)?;
        if let Some(ref url) = openai_base_url {
            validate_base_url(url, "EMBEDDING_BASE_URL", local_net, true)?;
        }

        let cache_size = parse_optional_env("EMBEDDING_CACHE_SIZE", DEFAULT_EMBEDDING_CACHE_SIZE)?;

        if cache_size == 0 {
            return Err(ConfigError::InvalidValue {
                key: "EMBEDDING_CACHE_SIZE".to_string(),
                message: "must be at least 1".to_string(),
            });
        }

        Ok(Self {
            enabled,
            provider,
            openai_api_key,
            model,
            ollama_base_url,
            dimension,
            openai_base_url,
            cache_size,
        })
    }

    /// Get the OpenAI API key if configured.
    pub fn openai_api_key(&self) -> Option<&str> {
        self.openai_api_key.as_ref().map(|s| s.expose_secret())
    }

    /// Create the appropriate embedding provider based on configuration.
    ///
    /// Returns `None` if embeddings are disabled or the required credentials
    /// are missing. The `nearai_base_url` and `session` are needed only for
    /// the NEAR AI provider but must be passed unconditionally.
    pub fn create_provider(
        &self,
        nearai_base_url: &str,
        session: Arc<SessionManager>,
    ) -> Option<Arc<dyn EmbeddingProvider>> {
        if !self.enabled {
            tracing::debug!("Embeddings disabled (set EMBEDDING_ENABLED=true to enable)");
            return None;
        }

        match self.provider.as_str() {
            "nearai" => {
                tracing::debug!(
                    "Embeddings enabled via NEAR AI (model: {}, dim: {})",
                    self.model,
                    self.dimension,
                );
                Some(Arc::new(
                    crate::workspace::NearAiEmbeddings::new(nearai_base_url, session)
                        .with_model(&self.model, self.dimension),
                ))
            }
            "ollama" => {
                tracing::debug!(
                    "Embeddings enabled via Ollama (model: {}, url: {}, dim: {})",
                    self.model,
                    self.ollama_base_url,
                    self.dimension,
                );
                Some(Arc::new(
                    crate::workspace::OllamaEmbeddings::new(&self.ollama_base_url)
                        .with_model(&self.model, self.dimension),
                ))
            }
            _ => {
                if let Some(api_key) = self.openai_api_key() {
                    let mut provider = crate::workspace::OpenAiEmbeddings::with_model(
                        api_key,
                        &self.model,
                        self.dimension,
                    );
                    if let Some(ref base_url) = self.openai_base_url {
                        tracing::debug!(
                            "Embeddings enabled via OpenAI (model: {}, base_url: {}, dim: {})",
                            self.model,
                            base_url,
                            self.dimension,
                        );
                        provider = provider.with_base_url(base_url);
                    } else {
                        tracing::debug!(
                            "Embeddings enabled via OpenAI (model: {}, dim: {})",
                            self.model,
                            self.dimension,
                        );
                    }
                    Some(Arc::new(provider))
                } else {
                    tracing::warn!("Embeddings configured but OPENAI_API_KEY not set");
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;
    use crate::settings::{EmbeddingsSettings, Settings};
    use crate::testing::credentials::*;

    /// Clear all embedding-related env vars.
    fn clear_embedding_env() {
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
            std::env::remove_var("EMBEDDING_PROVIDER");
            std::env::remove_var("EMBEDDING_MODEL");
            std::env::remove_var("EMBEDDING_DIMENSION");
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("EMBEDDING_BASE_URL");
            std::env::remove_var("EMBEDDING_CACHE_SIZE");
            std::env::remove_var("OLLAMA_BASE_URL");
        }
    }

    #[test]
    fn embeddings_disabled_not_overridden_by_openai_key() {
        let _guard = lock_env();
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("OPENAI_API_KEY", TEST_OPENAI_API_KEY_ISSUE_129);
        }

        let settings = Settings {
            embeddings: EmbeddingsSettings {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let config = EmbeddingsConfig::resolve(&settings).expect("resolve should succeed");
        assert!(
            !config.enabled,
            "embeddings should remain disabled when settings.embeddings.enabled=false, \
             even when OPENAI_API_KEY is set (issue #129)"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
    }

    #[test]
    fn embeddings_enabled_from_settings() {
        let _guard = lock_env();
        clear_embedding_env();

        let settings = Settings {
            embeddings: EmbeddingsSettings {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let config = EmbeddingsConfig::resolve(&settings).expect("resolve should succeed");
        assert!(
            config.enabled,
            "embeddings should be enabled when settings say so"
        );
    }

    #[test]
    fn db_settings_override_env() {
        let _guard = lock_env();
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("EMBEDDING_ENABLED", "false");
            std::env::set_var("EMBEDDING_PROVIDER", "ollama");
            std::env::set_var("EMBEDDING_MODEL", "all-minilm");
        }

        let settings = Settings {
            embeddings: EmbeddingsSettings {
                enabled: true,
                provider: "openai".to_string(),
                model: "text-embedding-3-large".to_string(),
            },
            ..Default::default()
        };

        let config = EmbeddingsConfig::resolve(&settings).expect("resolve should succeed");
        assert!(
            config.enabled,
            "DB enabled=true should win over env EMBEDDING_ENABLED=false"
        );
        assert_eq!(config.provider, "openai", "DB provider should win over env");
        assert_eq!(
            config.model, "text-embedding-3-large",
            "DB model should win over env"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
            std::env::remove_var("EMBEDDING_PROVIDER");
            std::env::remove_var("EMBEDDING_MODEL");
        }
    }

    #[test]
    fn env_used_when_no_db_setting() {
        let _guard = lock_env();
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("EMBEDDING_ENABLED", "true");
            std::env::set_var("EMBEDDING_PROVIDER", "ollama");
            std::env::set_var("EMBEDDING_MODEL", "nomic-embed-text");
        }

        // Settings left at defaults — no explicit DB/TOML override
        let settings = Settings::default();

        let config = EmbeddingsConfig::resolve(&settings).expect("resolve should succeed");
        assert!(
            config.enabled,
            "env EMBEDDING_ENABLED should be used when settings at default"
        );
        assert_eq!(
            config.provider, "ollama",
            "env EMBEDDING_PROVIDER should be used when settings at default"
        );
        assert_eq!(
            config.model, "nomic-embed-text",
            "env EMBEDDING_MODEL should be used when settings at default"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
            std::env::remove_var("EMBEDDING_PROVIDER");
            std::env::remove_var("EMBEDDING_MODEL");
        }
    }

    #[test]
    fn embedding_base_url_parsed_from_env() {
        let _guard = lock_env();
        clear_embedding_env();

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("EMBEDDING_BASE_URL", "https://8.8.8.8");
        }

        let settings = Settings::default();
        let config = EmbeddingsConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(config.openai_base_url.as_deref(), Some("https://8.8.8.8"));
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_BASE_URL");
        }
    }

    #[test]
    fn embedding_base_url_defaults_to_none() {
        let _guard = lock_env();
        clear_embedding_env();

        let settings = Settings::default();
        let config = EmbeddingsConfig::resolve(&settings).expect("resolve should succeed");
        assert!(
            config.openai_base_url.is_none(),
            "openai_base_url should be None when EMBEDDING_BASE_URL is not set"
        );
    }

    #[test]
    fn cache_size_zero_rejected() {
        let _guard = lock_env();
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("EMBEDDING_CACHE_SIZE", "0");
        }

        let settings = Settings::default();
        let result = EmbeddingsConfig::resolve(&settings);
        assert!(result.is_err(), "cache_size=0 should be rejected");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("at least 1"), "should mention minimum: {err}");
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_CACHE_SIZE");
        }
    }
}
