//! Async factory that builds the configured [`EmbeddingProvider`].

use std::sync::Arc;

use ironclaw_llm::SessionManager;

use crate::bedrock::BedrockEmbeddingSetup;
use crate::config::EmbeddingsConfig;
use crate::nearai::NearAiEmbeddings;
use crate::ollama::OllamaEmbeddings;
use crate::openai::OpenAiEmbeddings;
use crate::provider::EmbeddingProvider;
use crate::url_check::check_base_url;

/// Runtime wiring the factory needs that doesn't fit in [`EmbeddingsConfig`].
///
/// `EmbeddingsConfig` is pure data (Debug/Clone, populated from `Settings`).
/// These are shared runtime objects supplied by the host and consulted only
/// by the matching provider — `session` for `nearai`, `bedrock_setup` for
/// `bedrock`. Construct once at startup and pass into [`create_provider`].
#[derive(Clone)]
pub struct ProviderDeps {
    pub session: Arc<SessionManager>,
    pub bedrock_setup: Option<BedrockEmbeddingSetup>,
}

/// Build the configured embedding provider.
///
/// Returns `None` if embeddings are disabled or required credentials are
/// missing.
pub async fn create_provider(
    config: &EmbeddingsConfig,
    deps: ProviderDeps,
) -> Option<Arc<dyn EmbeddingProvider>> {
    if !config.enabled {
        tracing::debug!("Embeddings disabled (set EMBEDDING_ENABLED=true to enable)");
        return None;
    }

    match config.provider.as_str() {
        "nearai" => {
            if let Err(e) = check_base_url(&config.nearai_base_url, "nearai_base_url") {
                tracing::warn!("Refusing to build NEAR AI embeddings: {e}");
                return None;
            }
            tracing::debug!(
                "Embeddings enabled via NEAR AI (model: {}, dim: {})",
                config.model,
                config.dimension,
            );
            Some(Arc::new(
                NearAiEmbeddings::new(&config.nearai_base_url, deps.session)
                    .with_model(&config.model, config.dimension),
            ) as Arc<dyn EmbeddingProvider>)
        }
        "bedrock" => {
            #[cfg(feature = "bedrock")]
            {
                let Some(bedrock) = deps.bedrock_setup.as_ref() else {
                    tracing::warn!(
                        "Embeddings configured for Bedrock but no Bedrock setup is available"
                    );
                    return None;
                };
                tracing::debug!(
                    "Embeddings enabled via Bedrock (model: {}, region: {}, dim: {})",
                    config.model,
                    bedrock.region,
                    config.dimension,
                );
                match crate::bedrock::BedrockEmbeddings::new(
                    bedrock,
                    &config.model,
                    config.dimension,
                )
                .await
                {
                    Ok(provider) => Some(Arc::new(provider) as Arc<dyn EmbeddingProvider>),
                    Err(e) => {
                        tracing::warn!("Failed to initialize Bedrock embeddings provider: {e}");
                        None
                    }
                }
            }
            #[cfg(not(feature = "bedrock"))]
            {
                let _ = deps.bedrock_setup;
                tracing::warn!(
                    "Embeddings configured for Bedrock but the `bedrock` feature is disabled"
                );
                None
            }
        }
        "ollama" => {
            if let Err(e) = check_base_url(&config.ollama_base_url, "ollama_base_url") {
                tracing::warn!("Refusing to build Ollama embeddings: {e}");
                return None;
            }
            tracing::debug!(
                "Embeddings enabled via Ollama (model: {}, url: {}, dim: {})",
                config.model,
                config.ollama_base_url,
                config.dimension,
            );
            Some(Arc::new(
                OllamaEmbeddings::new(&config.ollama_base_url)
                    .with_model(&config.model, config.dimension),
            ) as Arc<dyn EmbeddingProvider>)
        }
        _ => {
            if let Some(api_key) = config.openai_api_key() {
                let mut provider =
                    OpenAiEmbeddings::with_model(api_key, &config.model, config.dimension);
                if let Some(ref base_url) = config.openai_base_url {
                    if let Err(e) = check_base_url(base_url, "openai_base_url") {
                        tracing::warn!("Refusing to build OpenAI embeddings: {e}");
                        return None;
                    }
                    tracing::debug!(
                        "Embeddings enabled via OpenAI (model: {}, base_url: {}, dim: {})",
                        config.model,
                        base_url,
                        config.dimension,
                    );
                    provider = provider.with_base_url(base_url);
                } else {
                    tracing::debug!(
                        "Embeddings enabled via OpenAI (model: {}, dim: {})",
                        config.model,
                        config.dimension,
                    );
                }
                Some(Arc::new(provider) as Arc<dyn EmbeddingProvider>)
            } else {
                tracing::warn!("Embeddings configured but OPENAI_API_KEY not set");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Regression coverage for the public factory + config surface: anyone
    //! that constructs `EmbeddingsConfig` directly and calls
    //! `create_provider` must hit the baseline URL check before any HTTP
    //! work happens. See PR #3739 review (P1).
    use super::*;
    use crate::config::EmbeddingsConfig;
    use ironclaw_llm::{SessionConfig, SessionManager};
    use secrecy::SecretString;

    fn stub_deps() -> ProviderDeps {
        ProviderDeps {
            session: Arc::new(SessionManager::new(SessionConfig::default())),
            bedrock_setup: None,
        }
    }

    fn config_with_provider(provider: &str) -> EmbeddingsConfig {
        EmbeddingsConfig {
            enabled: true,
            provider: provider.to_string(),
            ..EmbeddingsConfig::default()
        }
    }

    #[tokio::test]
    async fn rejects_blocked_ollama_base_url() {
        let cfg = EmbeddingsConfig {
            ollama_base_url: "https://169.254.169.254".to_string(),
            ..config_with_provider("ollama")
        };
        let provider = create_provider(&cfg, stub_deps()).await;
        assert!(
            provider.is_none(),
            "Ollama provider must not be built with cloud-metadata IP"
        );
    }

    #[tokio::test]
    async fn rejects_blocked_nearai_base_url() {
        let cfg = EmbeddingsConfig {
            nearai_base_url: "https://169.254.169.254".to_string(),
            ..config_with_provider("nearai")
        };
        let provider = create_provider(&cfg, stub_deps()).await;
        assert!(
            provider.is_none(),
            "NEAR AI provider must not be built with cloud-metadata IP"
        );
    }

    #[tokio::test]
    async fn rejects_blocked_openai_base_url() {
        let cfg = EmbeddingsConfig {
            openai_api_key: Some(SecretString::from("sk-stub".to_string())),
            openai_base_url: Some("https://169.254.169.254".to_string()),
            ..config_with_provider("openai")
        };
        let provider = create_provider(&cfg, stub_deps()).await;
        assert!(
            provider.is_none(),
            "OpenAI-compatible provider must not be built with cloud-metadata IP"
        );
    }

    #[tokio::test]
    async fn rejects_non_http_scheme() {
        let cfg = EmbeddingsConfig {
            ollama_base_url: "file:///etc/passwd".to_string(),
            ..config_with_provider("ollama")
        };
        assert!(create_provider(&cfg, stub_deps()).await.is_none());
    }

    #[tokio::test]
    async fn accepts_localhost_ollama() {
        let cfg = EmbeddingsConfig {
            ollama_base_url: "http://localhost:11434".to_string(),
            ..config_with_provider("ollama")
        };
        let provider = create_provider(&cfg, stub_deps()).await;
        assert!(
            provider.is_some(),
            "loopback Ollama is a legitimate operator endpoint"
        );
    }
}
