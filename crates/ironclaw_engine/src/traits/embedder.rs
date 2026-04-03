//! Embedder trait for semantic similarity in skill selection.
//!
//! A lightweight abstraction over embedding providers. The host crate
//! implements this by wrapping its `EmbeddingProvider`. The engine uses
//! it for semantic skill matching — embedding the user goal and skill
//! descriptions to compute cosine similarity scores.

use async_trait::async_trait;

/// Abstraction over embedding providers for the engine crate.
///
/// Deliberately minimal: just embed a single text string. Batching,
/// caching, and provider selection are host concerns.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Generate an embedding vector for the given text.
    ///
    /// Returns an empty vec if embeddings are not configured or the call fails.
    async fn embed(&self, text: &str) -> Vec<f32>;
}
