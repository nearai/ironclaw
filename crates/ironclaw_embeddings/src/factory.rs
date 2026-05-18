//! Async factory that builds the configured [`EmbeddingProvider`].

use std::sync::Arc;

use ironclaw_llm::SessionManager;

use crate::bedrock::BedrockEmbeddingSetup;
use crate::config::EmbeddingsConfig;
use crate::nearai::NearAiEmbeddings;
use crate::ollama::OllamaEmbeddings;
use crate::openai::OpenAiEmbeddings;
use crate::provider::EmbeddingProvider;

/// Build the configured embedding provider.
///
/// Returns `None` if embeddings are disabled or required credentials are
/// missing. `nearai_base_url` and `session` are needed only for the NEAR AI
/// provider but must be passed unconditionally; `bedrock_setup` is consulted
/// only for the `bedrock` provider.
pub async fn create_provider(
    config: &EmbeddingsConfig,
    nearai_base_url: &str,
    session: Arc<SessionManager>,
    bedrock_setup: Option<&BedrockEmbeddingSetup>,
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
                NearAiEmbeddings::new(nearai_base_url, session)
                    .with_model(&config.model, config.dimension),
            ) as Arc<dyn EmbeddingProvider>)
        }
        "bedrock" => {
            #[cfg(feature = "bedrock")]
            {
                let Some(bedrock) = bedrock_setup else {
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
                let _ = bedrock_setup;
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
