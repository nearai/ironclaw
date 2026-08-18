use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_llm::transcription::{
    AudioFormat, OpenAiWhisperProvider, TranscriptionError, TranscriptionErrorKind,
    TranscriptionProvider,
};
use ironclaw_product_contracts::surface::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
    ProductSurfaceValidationCode,
};
use ironclaw_product_contracts::transcription::TranscriptionService;

/// The model this deployment transcribes with when nothing overrides it.
///
/// NEAR AI serves `openai/whisper-large-v3` on the OpenAI-compatible
/// `/v1/audio/transcriptions` endpoint of the same `cloud-api.near.ai` host and
/// the same `NEARAI_API_KEY` the chat backend already uses.
const DEFAULT_TRANSCRIPTION_MODEL: &str = "openai/whisper-large-v3";

/// Environment override for the served transcription model, so a deployment can
/// point at a different model on the same endpoint without a rebuild.
const TRANSCRIPTION_MODEL_ENV: &str = "IRONCLAW_TRANSCRIPTION_MODEL";

/// Build the deployment's transcription provider, or `None` when this
/// deployment has no transcription-capable backend.
///
/// `None` is a first-class outcome, not a failure: the product surface reports
/// voice input unavailable and the WebUI hides its microphone button. Only the
/// NEAR AI backend is wired today — it is the one backend confirmed to serve
/// `/v1/audio/transcriptions` with the credential the runtime already holds.
/// Any other backend would need its own endpoint check before being claimed
/// here, and claiming it early would ship a button that always fails.
pub(crate) fn build_transcription_provider(
    config: &ironclaw_llm::LlmConfig,
) -> Option<Arc<dyn TranscriptionProvider>> {
    if !matches!(config.backend.as_str(), "nearai" | "near_ai" | "near") {
        tracing::debug!(
            backend = %config.backend,
            "transcription: only the nearai backend serves an audio endpoint today; voice input disabled"
        );
        return None;
    }
    // Session-token auth (no API key) cannot sign a bare multipart POST to the
    // audio endpoint, so a keyless NEAR AI deployment has no transcription.
    let Some(api_key) = config.nearai.api_key.clone() else {
        tracing::debug!("transcription: nearai backend has no API key; voice input disabled");
        return None;
    };

    let model = std::env::var(TRANSCRIPTION_MODEL_ENV)
        .ok()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| DEFAULT_TRANSCRIPTION_MODEL.to_string());

    tracing::debug!(%model, "transcription: voice input enabled");
    Some(Arc::new(
        OpenAiWhisperProvider::new(api_key)
            .with_base_url(config.nearai.base_url.clone())
            .with_model(model),
    ))
}

/// Adapts an `ironclaw_llm` transcription provider to the product-tier port.
///
/// The whole job is the error boundary: a provider failure carries a status and
/// a provider body, and neither may reach the browser. The classification
/// crosses; the detail is logged host-side and dropped.
pub(crate) struct LlmTranscriptionService {
    provider: Arc<dyn TranscriptionProvider>,
}

impl LlmTranscriptionService {
    pub(crate) fn new(provider: Arc<dyn TranscriptionProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl TranscriptionService for LlmTranscriptionService {
    async fn transcribe(
        &self,
        caller: &ProductSurfaceCaller,
        audio: &[u8],
        mime_type: &str,
    ) -> Result<String, ProductSurfaceError> {
        // The product service already checked this against the shared registry;
        // a `None` here would mean the registry and the provider's container
        // table have drifted apart, which is a host fault, not a client one.
        let Some(format) = AudioFormat::from_mime_type(mime_type) else {
            tracing::error!(
                %mime_type,
                "transcription: registry-accepted media type has no provider container"
            );
            return Err(ProductSurfaceError::internal());
        };

        match self.provider.transcribe(audio, format).await {
            Ok(text) => Ok(text),
            Err(error) => Err(map_transcription_error(caller, error)),
        }
    }
}

/// Map a provider failure to a stable product error, logging the provider
/// detail host-side exactly once.
fn map_transcription_error(
    caller: &ProductSurfaceCaller,
    error: TranscriptionError,
) -> ProductSurfaceError {
    let kind = error.kind();
    match kind {
        TranscriptionErrorKind::Transient => {
            tracing::debug!(
                tenant_id = %caller.tenant_id,
                %error,
                "transcription failed transiently"
            );
            ProductSurfaceError::service_unavailable(true)
        }
        TranscriptionErrorKind::Misconfigured => {
            // An operator problem, reported to the browser as a non-retryable
            // outage: the user cannot fix a credential, and retrying only
            // burns their rate-limit budget.
            tracing::error!(
                tenant_id = %caller.tenant_id,
                %error,
                "transcription is misconfigured (credentials or model id)"
            );
            ProductSurfaceError::service_unavailable(false)
        }
        TranscriptionErrorKind::Permanent => {
            // The clip itself is the problem. Reported against the audio field
            // so the composer can say "that recording could not be
            // transcribed" instead of blaming the service.
            tracing::debug!(
                tenant_id = %caller.tenant_id,
                %error,
                "transcription rejected the clip"
            );
            ProductSurfaceError {
                code: ProductSurfaceErrorCode::InvalidRequest,
                kind: ProductSurfaceErrorKind::Validation,
                status_code: 400,
                retryable: false,
                field: Some("audio_base64".to_string()),
                validation_code: Some(ProductSurfaceValidationCode::InvalidValue),
            }
        }
    }
}
