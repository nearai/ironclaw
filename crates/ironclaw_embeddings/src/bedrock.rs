//! AWS Bedrock embedding provider (Titan Text Embeddings V2).
//!
//! Entire file is gated on the `bedrock` feature; when disabled, only
//! [`BedrockEmbeddingSetup`] remains so callers can construct one without
//! depending on the feature flag (it is then ignored by the factory).

/// AWS Bedrock parameters needed by the embedding provider.
///
/// Defined here rather than re-using `ironclaw_llm::BedrockConfig` so the
/// embeddings layer does not couple to LLM-side config types. Callers
/// (which already hold an `LlmConfig`) translate at the boundary.
#[derive(Debug, Clone)]
pub struct BedrockEmbeddingSetup {
    pub region: String,
    pub profile: Option<String>,
}

#[cfg(feature = "bedrock")]
mod imp {
    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};

    use crate::provider::{EmbeddingError, EmbeddingProvider};

    use super::BedrockEmbeddingSetup;

    /// AWS Bedrock embedding provider using Titan Text Embeddings V2.
    pub struct BedrockEmbeddings {
        client: aws_sdk_bedrockruntime::Client,
        model: String,
        dimension: usize,
    }

    impl BedrockEmbeddings {
        /// Create a new Bedrock embedding provider.
        pub async fn new(
            setup: &BedrockEmbeddingSetup,
            model: impl Into<String>,
            dimension: usize,
        ) -> Result<Self, EmbeddingError> {
            let mut builder = aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(aws_config::Region::new(setup.region.clone()));
            if let Some(ref profile) = setup.profile {
                builder = builder.profile_name(profile);
            }

            let sdk_config = builder.load().await;
            Ok(Self {
                client: aws_sdk_bedrockruntime::Client::new(&sdk_config),
                model: model.into(),
                dimension,
            })
        }
    }

    #[derive(Debug, Serialize)]
    struct BedrockTitanEmbeddingRequest<'a> {
        #[serde(rename = "inputText")]
        input_text: &'a str,
        dimensions: usize,
        normalize: bool,
    }

    #[derive(Debug, Deserialize)]
    struct BedrockTitanEmbeddingResponse {
        embedding: Vec<f32>,
    }

    fn map_bedrock_invoke_model_error<R: std::fmt::Debug>(
        error: &aws_sdk_bedrockruntime::error::SdkError<
            aws_sdk_bedrockruntime::operation::invoke_model::InvokeModelError,
            R,
        >,
    ) -> EmbeddingError {
        use aws_sdk_bedrockruntime::error::SdkError;
        use aws_sdk_bedrockruntime::operation::invoke_model::InvokeModelError;

        match error {
            SdkError::ServiceError(service_err) => match service_err.err() {
                InvokeModelError::ThrottlingException(_) => {
                    EmbeddingError::RateLimited { retry_after: None }
                }
                InvokeModelError::AccessDeniedException(_) => EmbeddingError::AuthFailed,
                InvokeModelError::ValidationException(e) => {
                    EmbeddingError::InvalidResponse(format!(
                        "Bedrock validation error: {}",
                        e.message().unwrap_or("unknown")
                    ))
                }
                InvokeModelError::ModelNotReadyException(e) => EmbeddingError::HttpError(format!(
                    "Bedrock model not ready: {}",
                    e.message().unwrap_or("unknown")
                )),
                other => EmbeddingError::HttpError(format!("Bedrock service error: {other:?}")),
            },
            SdkError::TimeoutError(_) => {
                EmbeddingError::HttpError("Bedrock request timed out".to_string())
            }
            other => EmbeddingError::HttpError(format!("Bedrock request failed: {other:?}")),
        }
    }

    #[async_trait]
    impl EmbeddingProvider for BedrockEmbeddings {
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

            let request = BedrockTitanEmbeddingRequest {
                input_text: text,
                dimensions: self.dimension,
                normalize: true,
            };

            let body = serde_json::to_vec(&request).map_err(|e| {
                EmbeddingError::InvalidResponse(format!("Failed to serialize request: {}", e))
            })?;

            let response = self
                .client
                .invoke_model()
                .model_id(&self.model)
                .content_type("application/json")
                .accept("application/json")
                .body(aws_smithy_types::Blob::new(body))
                .send()
                .await
                .map_err(|e| map_bedrock_invoke_model_error(&e))?;

            let result: BedrockTitanEmbeddingResponse =
                serde_json::from_slice(response.body.as_ref()).map_err(|e| {
                    EmbeddingError::InvalidResponse(format!("Failed to parse response: {}", e))
                })?;

            if result.embedding.len() != self.dimension {
                return Err(EmbeddingError::InvalidResponse(format!(
                    "Bedrock returned embedding of dimension {}, expected {}",
                    result.embedding.len(),
                    self.dimension,
                )));
            }

            Ok(result.embedding)
        }
    }
}

#[cfg(feature = "bedrock")]
pub use imp::BedrockEmbeddings;
