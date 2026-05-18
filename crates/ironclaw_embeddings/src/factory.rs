//! Async factory that builds the configured [`EmbeddingProvider`].

use std::sync::Arc;

use ironclaw_llm::SessionManager;

use crate::bedrock::BedrockEmbeddingSetup;
use crate::config::EmbeddingsConfig;
use crate::nearai::NearAiEmbeddings;
use crate::ollama::OllamaEmbeddings;
use crate::openai::OpenAiEmbeddings;
use crate::provider::EmbeddingProvider;

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
