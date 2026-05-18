//! NEAR AI embedding provider.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::provider::{EmbeddingError, EmbeddingProvider};

/// NEAR AI embedding provider using the NEAR AI API.
///
/// Uses the same session-based auth as the LLM provider.
pub struct NearAiEmbeddings {
    client: reqwest::Client,
    base_url: String,
    session: std::sync::Arc<ironclaw_llm::SessionManager>,
    model: String,
    dimension: usize,
}

impl NearAiEmbeddings {
    /// Create a new NEAR AI embedding provider.
    ///
    /// Uses the same session manager as the LLM provider for auth.
    pub fn new(
        base_url: impl Into<String>,
        session: std::sync::Arc<ironclaw_llm::SessionManager>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            session,
            model: "text-embedding-3-small".to_string(),
            dimension: 1536,
        }
    }

    /// Use a specific model.
    pub fn with_model(mut self, model: impl Into<String>, dimension: usize) -> Self {
        self.model = model.into();
        self.dimension = dimension;
        self
    }
}

#[derive(Debug, Serialize)]
struct NearAiEmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Debug, Deserialize)]
struct NearAiEmbeddingResponse {
    data: Vec<NearAiEmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct NearAiEmbeddingData {
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbeddingProvider for NearAiEmbeddings {
    fn dimension(&self) -> usize {
        self.dimension
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn max_input_length(&self) -> usize {
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
        use secrecy::ExposeSecret;

        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let request = NearAiEmbeddingRequest {
            model: &self.model,
            input: texts,
        };

        let token = self
            .session
            .get_token()
            .await
            .map_err(|_| EmbeddingError::AuthFailed)?;

        let url = format!("{}/v1/embeddings", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token.expose_secret()))
            .json(&request)
            .send()
            .await?;

        let status = response.status();

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(EmbeddingError::AuthFailed);
        }

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = Some(ironclaw_llm::retry::parse_retry_after(
                response.headers().get("retry-after"),
            ));
            return Err(EmbeddingError::RateLimited { retry_after });
        }

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(EmbeddingError::HttpError(format!(
                "Status {}: {}",
                status, error_text
            )));
        }

        let result: NearAiEmbeddingResponse = response.json().await.map_err(|e| {
            EmbeddingError::InvalidResponse(format!("Failed to parse response: {}", e))
        })?;

        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }
}
