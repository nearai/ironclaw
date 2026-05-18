//! Embedding-provider trait + caching decorator.
//!
//! Concrete provider implementations (OpenAI, NEAR AI, Ollama, AWS Bedrock)
//! are crate-internal — construct one through [`create_provider`] using
//! [`EmbeddingsConfig`] + [`ProviderDeps`]. Callers should only ever hold
//! `Arc<dyn EmbeddingProvider>`.
//!
//! The resolver that reads the binary-side `Settings` lives in
//! `src/config/embeddings.rs::resolve_embeddings_config`; everything else
//! (trait, error, config shape, cache, factory, providers) lives here.

mod bedrock;
mod cache;
mod config;
mod factory;
#[cfg(any(test, feature = "testing"))]
mod mock;
mod nearai;
mod ollama;
mod openai;
mod provider;

pub use bedrock::BedrockEmbeddingSetup;
pub use cache::{CachedEmbeddingProvider, EmbeddingCacheConfig};
pub use config::{DEFAULT_EMBEDDING_CACHE_SIZE, EmbeddingsConfig, default_dimension_for_model};
pub use factory::{ProviderDeps, create_provider};
#[cfg(any(test, feature = "testing"))]
pub use mock::MockEmbeddings;
pub use provider::{EmbeddingError, EmbeddingProvider};
