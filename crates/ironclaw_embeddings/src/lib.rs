//! Embedding-provider trait + implementations (OpenAI, NearAI, Ollama, AWS Bedrock)
//! and an LRU-caching decorator.
//!
//! Extracted from `src/workspace/embeddings.rs` and `src/workspace/embedding_cache.rs`.
//! The resolver that reads the binary-side `Settings` lives in
//! `src/config/embeddings.rs::resolve_embeddings_config`; everything else
//! (the trait, error, providers, config shape, cache, and factory) lives here.

pub mod bedrock;
pub mod cache;
pub mod config;
pub mod factory;
pub mod mock;
pub mod nearai;
pub mod ollama;
pub mod openai;
pub mod provider;

pub use bedrock::BedrockEmbeddingSetup;
#[cfg(feature = "bedrock")]
pub use bedrock::BedrockEmbeddings;
pub use cache::{CachedEmbeddingProvider, EmbeddingCacheConfig};
pub use config::{DEFAULT_EMBEDDING_CACHE_SIZE, EmbeddingsConfig, default_dimension_for_model};
pub use factory::create_provider;
pub use mock::MockEmbeddings;
pub use nearai::NearAiEmbeddings;
pub use ollama::OllamaEmbeddings;
pub use openai::OpenAiEmbeddings;
pub use provider::{EmbeddingError, EmbeddingProvider};
