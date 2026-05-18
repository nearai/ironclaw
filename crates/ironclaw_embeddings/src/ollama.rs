//! Ollama embedding provider (local Ollama HTTP API).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::provider::{EmbeddingError, EmbeddingProvider};

/// Ollama embedding provider using a local Ollama instance.
///
/// Ollama serves embedding models (e.g. `nomic-embed-text`, `mxbai-embed-large`)
/// via a REST API, typically at `http://localhost:11434`.
pub(crate) struct OllamaEmbeddings {
    client: reqwest::Client,
    base_url: String,
    model: String,
    dimension: usize,
}

impl OllamaEmbeddings {
    /// Create a new Ollama embedding provider.
    ///
    /// Defaults to `nomic-embed-text` (768 dimensions).
    pub(crate) fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            model: "nomic-embed-text".to_string(),
            dimension: 768,
        }
    }

    /// Use a specific model with a given dimension.
    pub(crate) fn with_model(mut self, model: impl Into<String>, dimension: usize) -> Self {
        self.model = model.into();
        self.dimension = dimension;
        self
    }
}

#[derive(Debug, Serialize)]
struct OllamaEmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Debug, Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbeddings {
    fn dimension(&self) -> usize {
        self.dimension
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn max_input_length(&self) -> usize {
        // Most Ollama embedding models support ~8192 tokens, budgeted
        // here as ~32_000 UTF-8 bytes (matches `str::len()` semantics —
        // see `EmbeddingProvider::max_input_length` doc).
        32_000
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        if text.len() > self.max_input_length() {
            return Err(EmbeddingError::TextTooLong {
                length: text.len(),
                max: self.max_input_length(),
            });
        }

        let embeddings = self.embed_batch(&[text.to_string()]).await?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::InvalidResponse("No embedding returned".to_string()))
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let request = OllamaEmbedRequest {
            model: &self.model,
            input: texts,
        };

        let url = format!("{}/api/embed", self.base_url);

        let response = self.client.post(&url).json(&request).send().await?;

        let status = response.status();

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(EmbeddingError::HttpError(format!(
                "Ollama returned HTTP {}: {}",
                status, error_text
            )));
        }

        let result: OllamaEmbedResponse = response.json().await.map_err(|e| {
            EmbeddingError::InvalidResponse(format!("Failed to parse Ollama response: {}", e))
        })?;

        // Validate that returned embeddings match the configured dimension.
        for (i, emb) in result.embeddings.iter().enumerate() {
            if emb.len() != self.dimension {
                return Err(EmbeddingError::InvalidResponse(format!(
                    "Ollama returned embedding of dimension {}, expected {} at index {}",
                    emb.len(),
                    self.dimension,
                    i
                )));
            }
        }

        Ok(result.embeddings)
    }
}
