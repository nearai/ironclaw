//! OpenAI embedding provider (also used for any OpenAI-compatible endpoint).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::provider::{EmbeddingError, EmbeddingProvider};

/// Default base URL for the OpenAI API.
const OPENAI_API_BASE_URL: &str = "https://api.openai.com";

/// OpenAI embedding provider using text-embedding-ada-002 or text-embedding-3-small.
///
/// Supports any OpenAI-compatible embedding endpoint via [`with_base_url`](Self::with_base_url).
pub(crate) struct OpenAiEmbeddings {
    client: reqwest::Client,
    api_key: String,
    model: String,
    dimension: usize,
    base_url: String,
}

impl OpenAiEmbeddings {
    /// Create a new OpenAI embedding provider with the default model.
    ///
    /// Uses text-embedding-3-small which has 1536 dimensions.
    #[allow(dead_code)]
    pub(crate) fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            model: "text-embedding-3-small".to_string(),
            dimension: 1536,
            base_url: OPENAI_API_BASE_URL.to_string(),
        }
    }

    /// Use text-embedding-ada-002 model.
    #[allow(dead_code)]
    pub(crate) fn ada_002(api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            model: "text-embedding-ada-002".to_string(),
            dimension: 1536,
            base_url: OPENAI_API_BASE_URL.to_string(),
        }
    }

    /// Use text-embedding-3-large model.
    #[allow(dead_code)]
    pub(crate) fn large(api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            model: "text-embedding-3-large".to_string(),
            dimension: 3072,
            base_url: OPENAI_API_BASE_URL.to_string(),
        }
    }

    /// Use a custom model with specified dimension.
    pub(crate) fn with_model(
        api_key: impl Into<String>,
        model: impl Into<String>,
        dimension: usize,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            model: model.into(),
            dimension,
            base_url: OPENAI_API_BASE_URL.to_string(),
        }
    }

    /// Set a custom base URL for OpenAI-compatible embedding providers.
    ///
    /// The URL must use `http://` or `https://` scheme. If no scheme is present,
    /// `https://` is prepended automatically. Trailing slashes are stripped.
    pub(crate) fn with_base_url(mut self, base_url: &str) -> Self {
        let url = base_url.trim();

        // Auto-prepend https:// if no scheme is present.
        let mut url = if !url.starts_with("http://") && !url.starts_with("https://") {
            tracing::debug!(
                "No scheme in embedding base URL '{}', prepending https://",
                url
            );
            format!("https://{url}")
        } else {
            url.to_string()
        };

        while url.ends_with('/') {
            url.pop();
        }

        self.base_url = url;
        self
    }
}

#[derive(Debug, Serialize)]
struct OpenAiEmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<OpenAiEmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingData {
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddings {
    fn dimension(&self) -> usize {
        self.dimension
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn max_input_length(&self) -> usize {
        // text-embedding-3-small/large + ada-002: ~8191 tokens, budgeted
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

        let request = OpenAiEmbeddingRequest {
            model: &self.model,
            input: texts,
        };

        let url = format!("{}/v1/embeddings", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
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

        let result: OpenAiEmbeddingResponse = response.json().await.map_err(|e| {
            EmbeddingError::InvalidResponse(format!("Failed to parse response: {}", e))
        })?;

        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_embeddings_config() {
        let provider = OpenAiEmbeddings::new("test-key");
        assert_eq!(provider.dimension(), 1536);
        assert_eq!(provider.model_name(), "text-embedding-3-small");
        assert_eq!(provider.base_url, OPENAI_API_BASE_URL);

        let provider = OpenAiEmbeddings::large("test-key");
        assert_eq!(provider.dimension(), 3072);
        assert_eq!(provider.model_name(), "text-embedding-3-large");
        assert_eq!(provider.base_url, OPENAI_API_BASE_URL);
    }

    #[test]
    fn test_openai_with_base_url_valid() {
        let provider =
            OpenAiEmbeddings::new("test-key").with_base_url("https://custom.example.com");
        assert_eq!(provider.base_url, "https://custom.example.com");
    }

    #[test]
    fn test_openai_with_base_url_strips_trailing_slashes() {
        let provider =
            OpenAiEmbeddings::new("test-key").with_base_url("https://custom.example.com///");
        assert_eq!(provider.base_url, "https://custom.example.com");
    }

    #[test]
    fn test_openai_with_base_url_http_scheme() {
        let provider = OpenAiEmbeddings::new("test-key").with_base_url("http://localhost:8080");
        assert_eq!(provider.base_url, "http://localhost:8080");
    }

    #[test]
    fn test_openai_with_base_url_schemeless_prepends_https() {
        let provider = OpenAiEmbeddings::new("test-key").with_base_url("custom.example.com/v1");
        assert_eq!(provider.base_url, "https://custom.example.com/v1");
    }
}
