//! Text embeddings: the [`EmbeddingProvider`] port and its implementations.
//!
//! This is a **different trait** from [`crate::LlmProvider`], like
//! [`crate::transcription::TranscriptionProvider`]: it shares only the crate's
//! hardened HTTP transport and base-URL SSRF guard, never the chat request
//! vocabulary or the decorator chain.
//!
//! The port is product-neutral: callers hand it a batch of strings and get one
//! vector per string back, in input order. [`create_embedding_provider`] picks
//! the client by provider id and fails closed; the composition root decides
//! when to call it and who receives the result, and consumers only ever hold
//! `Arc<dyn EmbeddingProvider>`.

mod factory;
mod openai_compatible;

pub use self::factory::{
    DEFAULT_EMBEDDING_API_KEY_ENV, EMBEDDING_API_KEY_ENV_ENV, EMBEDDING_BASE_URL_ENV,
    EMBEDDING_DIMENSION_ENV, EMBEDDING_MAX_BATCH_SIZE_ENV, EMBEDDING_MODEL_ENV,
    EMBEDDING_PROVIDER_ENV, EMBEDDING_REQUEST_TIMEOUT_SECS_ENV, EmbeddingProviderSettings,
    OPENAI_COMPATIBLE_EMBEDDING_PROVIDER_ID, OPENAI_EMBEDDING_PROVIDER_ID,
    create_embedding_provider,
};

pub use self::openai_compatible::{
    DEFAULT_EMBEDDING_MAX_BATCH_SIZE, DEFAULT_EMBEDDING_MAX_INPUT_BYTES, OPENAI_DEFAULT_BASE_URL,
    OpenAiCompatibleEmbeddingConfig, OpenAiCompatibleEmbeddings,
};

use std::time::Duration;

use async_trait::async_trait;

/// Errors from an [`EmbeddingProvider`].
///
/// Every variant is a failure of the one call that produced it; none of them
/// leaves the provider unusable except [`EmbeddingError::InvalidConfig`], which
/// is only returned while constructing a provider.
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    /// The provider could not be constructed from its configuration (bad or
    /// blocked base URL, empty model, zero batch size, …).
    #[error("invalid embedding provider configuration: {reason}")]
    InvalidConfig { reason: String },

    /// The request did not complete within the configured timeout.
    #[error("embedding request timed out after {timeout:?}")]
    Timeout { timeout: Duration },

    /// The request failed before an HTTP response arrived (connect, TLS, …).
    #[error("embedding request failed: {reason}")]
    RequestFailed { reason: String },

    /// The endpoint rejected the credentials (HTTP 401/403).
    #[error("embedding provider rejected the credentials (HTTP {status})")]
    AuthFailed { status: u16 },

    /// The endpoint is rate limiting this caller (HTTP 429).
    #[error("embedding provider rate limited the request; retry after {retry_after:?}")]
    RateLimited { retry_after: Duration },

    /// Any other non-success HTTP status. `body` is truncated.
    #[error("embedding provider returned HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },

    /// The response was not the documented shape (unparseable JSON, missing or
    /// duplicated indices, wrong number of vectors, empty vectors).
    #[error("invalid embedding response: {reason}")]
    InvalidResponse { reason: String },

    /// A returned vector's length differs from the configured dimension or,
    /// when none was configured, from the first dimension this provider saw.
    #[error("embedding dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    /// One input exceeds the provider's per-input byte budget.
    #[error("embedding input {index} is {length} bytes, over the {max}-byte limit")]
    InputTooLong {
        index: usize,
        length: usize,
        max: usize,
    },
}

/// Port for turning text into embedding vectors.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Provider id the client was built for (`openai`,
    /// `openai_compatible`, …). Callers that persist vectors key them by it,
    /// with the model and dimension, so vectors from different providers
    /// are never mixed. Override it: the default names no provider.
    fn provider_id(&self) -> &str {
        ""
    }

    /// Model identifier sent to the endpoint.
    fn model_name(&self) -> &str;

    /// Vector dimension: the configured one, else the first one observed, else
    /// `None` before the first successful call.
    fn dimension(&self) -> Option<usize>;

    /// Embed `texts`, returning exactly one vector per input, in input order.
    ///
    /// An empty slice returns an empty vector without any I/O. Implementations
    /// split large batches into endpoint-sized requests themselves, and every
    /// returned vector has the same length ([`EmbeddingProvider::dimension`]).
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError>;
}
