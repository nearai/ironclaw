//! Embedding provider trait and embedding-vector helpers.

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::VirtualPath;

use crate::path::{MemoryDocumentScope, memory_error, valid_memory_path};

/// Error returned by memory embedding providers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddingError {
    ProviderUnavailable { reason: String },
    InvalidVector { expected: usize, actual: usize },
    TextTooLong { length: usize, max: usize },
}

impl std::fmt::Display for EmbeddingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbeddingError::ProviderUnavailable { reason } => {
                write!(formatter, "embedding provider unavailable: {reason}")
            }
            EmbeddingError::InvalidVector { expected, actual } => {
                write!(
                    formatter,
                    "embedding vector dimension mismatch: expected {expected}, got {actual}"
                )
            }
            EmbeddingError::TextTooLong { length, max } => {
                write!(formatter, "embedding input too long: {length} > {max}")
            }
        }
    }
}

impl std::error::Error for EmbeddingError {}

/// Memory-owned embedding-provider seam.
///
/// Concrete HTTP/provider integrations belong outside this core crate and can
/// implement this trait after resolving credentials/network policy at the host
/// boundary.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn dimension(&self) -> usize;

    fn model_name(&self) -> &str;

    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let mut embeddings = Vec::with_capacity(texts.len());
        for text in texts {
            embeddings.push(self.embed(text).await?);
        }
        Ok(embeddings)
    }
}

pub(crate) fn validate_embedding_dimension(
    expected: usize,
    actual: usize,
) -> Result<(), EmbeddingError> {
    if expected == 0 || actual != expected {
        Err(EmbeddingError::InvalidVector { expected, actual })
    } else {
        Ok(())
    }
}

pub(crate) fn embedding_filesystem_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    error: EmbeddingError,
) -> FilesystemError {
    memory_error(path, operation, error.to_string())
}

pub(crate) async fn embed_text(
    provider: &dyn EmbeddingProvider,
    scope: &MemoryDocumentScope,
    text: &str,
) -> Result<Vec<f32>, FilesystemError> {
    let embedding = provider.embed(text).await.map_err(|error| {
        embedding_filesystem_error(
            scope
                .virtual_prefix()
                .unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::ReadFile,
            error,
        )
    })?;
    validate_embedding_dimension(provider.dimension(), embedding.len()).map_err(|error| {
        embedding_filesystem_error(
            scope
                .virtual_prefix()
                .unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::ReadFile,
            error,
        )
    })?;
    Ok(embedding)
}

#[cfg(feature = "libsql")]
pub(crate) fn encode_embedding_blob(embedding: &[f32]) -> Vec<u8> {
    embedding
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

#[cfg(feature = "libsql")]
pub(crate) fn decode_embedding_blob(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(std::mem::size_of::<f32>()) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(std::mem::size_of::<f32>())
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

#[cfg(feature = "libsql")]
pub(crate) fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;
    for (left, right) in left.iter().zip(right.iter()) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    if left_norm <= 0.0 || right_norm <= 0.0 {
        return None;
    }
    let score = dot / (left_norm.sqrt() * right_norm.sqrt());
    if score.is_finite() { Some(score) } else { None }
}
