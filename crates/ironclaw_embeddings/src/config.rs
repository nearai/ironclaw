//! Embeddings provider configuration (resolved from binary-side `Settings`).
//!
//! The resolver that reads `Settings` lives in the binary
//! (`src/config/embeddings.rs::resolve_embeddings_config`); this crate only
//! owns the resolved data shape and helpers that depend on nothing but the
//! shape itself.

use secrecy::{ExposeSecret, SecretString};

/// Default maximum number of cached embeddings.
pub const DEFAULT_EMBEDDING_CACHE_SIZE: usize = 10_000;

/// Embeddings provider configuration.
#[derive(Debug, Clone)]
pub struct EmbeddingsConfig {
    /// Whether embeddings are enabled.
    pub enabled: bool,
    /// Provider to use: "openai", "nearai", "ollama", or "bedrock"
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

impl EmbeddingsConfig {
    /// Get the OpenAI API key if configured.
    pub fn openai_api_key(&self) -> Option<&str> {
        self.openai_api_key.as_ref().map(|s| s.expose_secret())
    }
}

/// Infer the embedding dimension from a well-known model name.
///
/// Falls back to 1536 (OpenAI text-embedding-3-small default) for unknown models.
pub fn default_dimension_for_model(model: &str) -> usize {
    match model {
        "text-embedding-3-small" => 1536,
        "text-embedding-3-large" => 3072,
        "text-embedding-ada-002" => 1536,
        "amazon.titan-embed-text-v2:0" => 1024,
        "nomic-embed-text" => 768,
        "mxbai-embed-large" => 1024,
        "all-minilm" => 384,
        _ => 1536,
    }
}
