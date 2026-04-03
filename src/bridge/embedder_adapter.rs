//! Embedder bridge adapter — wraps `EmbeddingProvider` as `ironclaw_engine::Embedder`.

use std::sync::Arc;

use ironclaw_engine::Embedder;

use crate::workspace::EmbeddingProvider;

/// Wraps an existing `EmbeddingProvider` to implement the engine's `Embedder` trait.
pub struct EmbedderBridgeAdapter {
    provider: Arc<dyn EmbeddingProvider>,
}

impl EmbedderBridgeAdapter {
    pub fn new(provider: Arc<dyn EmbeddingProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait::async_trait]
impl Embedder for EmbedderBridgeAdapter {
    async fn embed(&self, text: &str) -> Vec<f32> {
        self.provider.embed(text).await.unwrap_or_default()
    }
}
