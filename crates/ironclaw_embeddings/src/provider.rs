//! [`EmbeddingProvider`] trait and shared [`EmbeddingError`] type.

use async_trait::async_trait;

/// Error type for embedding operations.
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("HTTP request failed: {0}")]
    HttpError(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Rate limited, retry after {retry_after:?}")]
    RateLimited {
        retry_after: Option<std::time::Duration>,
    },

    #[error("Authentication failed")]
    AuthFailed,

    #[error("Text too long: {length} > {max}")]
    TextTooLong { length: usize, max: usize },

    #[error("Invalid provider URL '{url}': {reason}")]
    InvalidUrl { url: String, reason: String },
}

impl From<reqwest::Error> for EmbeddingError {
    fn from(e: reqwest::Error) -> Self {
        EmbeddingError::HttpError(e.to_string())
    }
}

/// The provider family backing an [`EmbeddingProvider`].
///
/// A closed set rather than a free string, so credential-hint dispatch is
/// exhaustive: adding a provider forces every `match` to handle it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProviderKind {
    OpenAi,
    NearAi,
    Ollama,
    Bedrock,
}

impl EmbeddingProviderKind {
    /// Operator-facing hint appended to an embedding `AuthFailed` warning,
    /// tailored to where this provider keeps its credential. `AuthFailed`
    /// surfaces from OpenAI (401), NEAR AI (401 or session-token fetch failure),
    /// and Bedrock (AccessDenied), so a single OpenAI-flavored hint misleads the
    /// others (#3755).
    pub fn auth_failed_hint(self) -> &'static str {
        match self {
            EmbeddingProviderKind::NearAi => {
                ". Run `ironclaw onboard` to refresh your NEAR AI session, or set \
                 EMBEDDING_PROVIDER=ollama for local embeddings"
            }
            EmbeddingProviderKind::Bedrock => {
                ". Check your AWS credentials (AWS_PROFILE or AWS_ACCESS_KEY_ID + \
                 AWS_SECRET_ACCESS_KEY), or set EMBEDDING_PROVIDER=ollama for local \
                 embeddings"
            }
            // Ollama is local and not auth-bearing, so it shares the generic
            // OpenAI hint rather than getting a bespoke one.
            EmbeddingProviderKind::OpenAi | EmbeddingProviderKind::Ollama => {
                ". Check OPENAI_API_KEY or set EMBEDDING_PROVIDER=ollama for local \
                 embeddings"
            }
        }
    }
}

/// Trait for embedding providers.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Get the embedding dimension.
    fn dimension(&self) -> usize;

    /// Get the model name.
    fn model_name(&self) -> &str;

    /// The provider family backing this provider, used to tailor operator-facing
    /// hints (e.g. which credential to check on an auth failure). The four
    /// production providers override this; the default is only reached by test
    /// doubles.
    fn provider_kind(&self) -> EmbeddingProviderKind {
        EmbeddingProviderKind::OpenAi
    }

    /// Maximum input length in **bytes** (matches `str::len()` semantics).
    ///
    /// Provider implementations enforce this against `text.len()`, which
    /// counts UTF-8 bytes, not Unicode characters. Implementations document
    /// the byte budget for their underlying model (typically derived from a
    /// token limit; e.g. 8191 tokens ≈ 32_000 bytes for the OpenAI
    /// embedding family).
    fn max_input_length(&self) -> usize;

    /// Generate an embedding for a single text.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;

    /// Generate embeddings for multiple texts (batched).
    ///
    /// Default implementation calls embed() for each text.
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let mut embeddings = Vec::with_capacity(texts.len());
        for text in texts {
            embeddings.push(self.embed(text).await?);
        }
        Ok(embeddings)
    }
}

/// Enforce `max` (bytes) for every item in a batch.
///
/// The `embed_batch` overrides (OpenAI, NEAR AI, Ollama) issue a single
/// batched request, so — unlike the per-item `embed` path — they must validate
/// each input themselves before hitting the provider. Shared here so the three
/// overrides stay in lockstep (#3752).
pub(crate) fn ensure_batch_within_limit(
    texts: &[String],
    max: usize,
) -> Result<(), EmbeddingError> {
    for text in texts {
        if text.len() > max {
            return Err(EmbeddingError::TextTooLong {
                length: text.len(),
                max,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_batch_is_ok() {
        assert!(ensure_batch_within_limit(&[], 10).is_ok());
    }

    #[test]
    fn item_exactly_at_limit_is_ok() {
        assert!(ensure_batch_within_limit(&["aaaaa".to_string()], 5).is_ok());
    }

    #[test]
    fn second_item_over_limit_fails() {
        let texts = vec!["ok".to_string(), "way too long".to_string()];
        let err = ensure_batch_within_limit(&texts, 5).expect_err("second item exceeds the limit");
        assert!(matches!(
            err,
            EmbeddingError::TextTooLong { length: 12, max: 5 }
        ));
    }

    #[test]
    fn auth_failed_hint_is_provider_specific() {
        assert!(
            EmbeddingProviderKind::NearAi
                .auth_failed_hint()
                .contains("onboard")
        );
        assert!(
            EmbeddingProviderKind::Bedrock
                .auth_failed_hint()
                .contains("AWS_PROFILE")
        );
        assert!(
            EmbeddingProviderKind::OpenAi
                .auth_failed_hint()
                .contains("OPENAI_API_KEY")
        );
        assert!(
            EmbeddingProviderKind::Ollama
                .auth_failed_hint()
                .contains("OPENAI_API_KEY")
        );
    }
}
