use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};

use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::llm::SessionManager;
use crate::settings::Settings;
use crate::workspace::EmbeddingProvider;

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
        }
    }
}

/// Infer the embedding dimension from a well-known model name.
///
/// Falls back to 1536 (OpenAI text-embedding-3-small default) for unknown models.
fn default_dimension_for_model(model: &str) -> usize {
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
        let openai_api_key = optional_env("OPENAI_API_KEY")?.map(SecretString::from);

        let provider = optional_env("EMBEDDING_PROVIDER")?
            .unwrap_or_else(|| settings.embeddings.provider.clone());

        let model =
            optional_env("EMBEDDING_MODEL")?.unwrap_or_else(|| settings.embeddings.model.clone());

        let ollama_base_url = optional_env("OLLAMA_BASE_URL")?
            .or_else(|| settings.ollama_base_url.clone())
            .unwrap_or_else(|| "http://localhost:11434".to_string());

        let dimension =
            parse_optional_env("EMBEDDING_DIMENSION", default_dimension_for_model(&model))?;

        let enabled = parse_bool_env("EMBEDDING_ENABLED", settings.embeddings.enabled)?;

        let openai_base_url = optional_env("EMBEDDING_BASE_URL")?;
        if let Some(ref base_url) = openai_base_url {
            validate_embedding_base_url(base_url)?;
        }

        Ok(Self {
            enabled,
            provider,
            openai_api_key,
            model,
            ollama_base_url,
            dimension,
            openai_base_url,
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

fn validate_embedding_base_url(base_url: &str) -> Result<(), ConfigError> {
    let parsed = reqwest::Url::parse(base_url).map_err(|e| ConfigError::InvalidValue {
        key: "EMBEDDING_BASE_URL".to_string(),
        message: format!("must be a valid URL: {e}"),
    })?;

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ConfigError::InvalidValue {
            key: "EMBEDDING_BASE_URL".to_string(),
            message: "must not include URL credentials".to_string(),
        });
    }

    let scheme = parsed.scheme();
    if !matches!(scheme, "http" | "https") {
        return Err(ConfigError::InvalidValue {
            key: "EMBEDDING_BASE_URL".to_string(),
            message: "must use http:// or https://".to_string(),
        });
    }

    let host = parsed.host_str().ok_or_else(|| ConfigError::InvalidValue {
        key: "EMBEDDING_BASE_URL".to_string(),
        message: "must include a host".to_string(),
    })?;

    if is_forbidden_embedding_host(host) {
        return Err(ConfigError::InvalidValue {
            key: "EMBEDDING_BASE_URL".to_string(),
            message: format!("host '{host}' is not allowed"),
        });
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_forbidden_embedding_ip(ip) {
            return Err(ConfigError::InvalidValue {
                key: "EMBEDDING_BASE_URL".to_string(),
                message: format!("IP '{ip}' is not allowed"),
            });
        }
    } else if scheme == "http" && !host.eq_ignore_ascii_case("localhost") {
        return Err(ConfigError::InvalidValue {
            key: "EMBEDDING_BASE_URL".to_string(),
            message: "http:// is only allowed for localhost/loopback embedding servers".to_string(),
        });
    }

    Ok(())
}

fn is_forbidden_embedding_host(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    lower == "host.docker.internal"
        || lower == "metadata.google.internal"
        || lower == "metadata.aws.internal"
}

fn is_forbidden_embedding_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                return false;
            }
            is_forbidden_embedding_ipv4(v4)
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return false;
            }
            if let Some(mapped) = ipv6_mapped_ipv4(v6) {
                if mapped.is_loopback() {
                    return false;
                }
                return is_forbidden_embedding_ipv4(mapped);
            }
            v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_multicast()
                || is_documentation_ipv6(v6)
        }
    }
}

fn is_forbidden_embedding_ipv4(v4: Ipv4Addr) -> bool {
    if v4.is_private()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_unspecified()
        || v4.is_multicast()
    {
        return true;
    }

    let octets = v4.octets();
    // Carrier-grade NAT range (100.64.0.0/10).
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return true;
    }
    // Benchmark testing range (198.18.0.0/15).
    octets[0] == 198 && matches!(octets[1], 18 | 19)
}

fn ipv6_mapped_ipv4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = v6.segments();
    if segments[0] == 0
        && segments[1] == 0
        && segments[2] == 0
        && segments[3] == 0
        && segments[4] == 0
        && segments[5] == 0xffff
    {
        Some(Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        ))
    } else {
        None
    }
}

fn is_documentation_ipv6(v6: Ipv6Addr) -> bool {
    let segments = v6.segments();
    segments[0] == 0x2001 && segments[1] == 0x0db8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::ENV_MUTEX;
    use crate::settings::{EmbeddingsSettings, Settings};
    use crate::testing::credentials::*;

    /// Clear all embedding-related env vars.
    fn clear_embedding_env() {
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
            std::env::remove_var("EMBEDDING_PROVIDER");
            std::env::remove_var("EMBEDDING_MODEL");
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("EMBEDDING_BASE_URL");
        }
    }

    #[test]
    fn embeddings_disabled_not_overridden_by_openai_key() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");

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
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
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
    fn embeddings_env_override_takes_precedence() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");

        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("EMBEDDING_ENABLED", "true");
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
            config.enabled,
            "EMBEDDING_ENABLED=true env var should override settings"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
        }
    }

    #[test]
    fn embedding_base_url_parsed_from_env() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_embedding_env();

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("EMBEDDING_BASE_URL", "https://custom.example.com");
        }

        let settings = Settings::default();
        let config = EmbeddingsConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(
            config.openai_base_url.as_deref(),
            Some("https://custom.example.com"),
            "EMBEDDING_BASE_URL env var should be parsed into openai_base_url"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_BASE_URL");
        }
    }

    #[test]
    fn embedding_base_url_defaults_to_none() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_embedding_env();

        let settings = Settings::default();
        let config = EmbeddingsConfig::resolve(&settings).expect("resolve should succeed");
        assert!(
            config.openai_base_url.is_none(),
            "openai_base_url should be None when EMBEDDING_BASE_URL is not set"
        );
    }

    #[test]
    fn embedding_base_url_rejects_http_non_localhost() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("EMBEDDING_BASE_URL", "http://example.com/v1");
        }
        let settings = Settings::default();
        let err = EmbeddingsConfig::resolve(&settings).expect_err("resolve should fail");
        assert!(err.to_string().contains("EMBEDDING_BASE_URL"));
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_BASE_URL");
        }
    }

    #[test]
    fn embedding_base_url_allows_http_localhost() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("EMBEDDING_BASE_URL", "http://localhost:11434/v1");
        }
        let settings = Settings::default();
        let config = EmbeddingsConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(
            config.openai_base_url.as_deref(),
            Some("http://localhost:11434/v1")
        );
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_BASE_URL");
        }
    }

    #[test]
    fn embedding_base_url_rejects_private_ip() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("EMBEDDING_BASE_URL", "https://10.0.0.1/v1");
        }
        let settings = Settings::default();
        let err = EmbeddingsConfig::resolve(&settings).expect_err("resolve should fail");
        assert!(err.to_string().contains("not allowed"));
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_BASE_URL");
        }
    }

    #[test]
    fn embedding_base_url_rejects_url_credentials() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("EMBEDDING_BASE_URL", "https://user:pass@example.com/v1");
        }
        let settings = Settings::default();
        let err = EmbeddingsConfig::resolve(&settings).expect_err("resolve should fail");
        assert!(err.to_string().contains("credentials"));
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_BASE_URL");
        }
    }

    #[test]
    fn embedding_base_url_rejects_metadata_host() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var(
                "EMBEDDING_BASE_URL",
                "https://metadata.google.internal/computeMetadata/v1",
            );
        }
        let settings = Settings::default();
        let err = EmbeddingsConfig::resolve(&settings).expect_err("resolve should fail");
        assert!(err.to_string().contains("not allowed"));
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_BASE_URL");
        }
    }
}
