//! OpenAI Whisper transcription provider.

use async_trait::async_trait;
use reqwest::multipart;
use secrecy::{ExposeSecret, SecretString};

use super::{AudioFormat, TranscriptionError, TranscriptionProvider};

/// OpenAI Whisper speech-to-text provider.
///
/// Uses the `/v1/audio/transcriptions` endpoint.
pub struct OpenAiWhisperProvider {
    client: reqwest::Client,
    api_key: SecretString,
    model: String,
    base_url: String,
}

impl OpenAiWhisperProvider {
    /// Create a new Whisper provider with the given API key.
    pub fn new(api_key: SecretString) -> Self {
        Self {
            client: match reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(
                        "Failed to build HTTP client with timeout, falling back to default: {e}"
                    );
                    reqwest::Client::default()
                }
            },
            api_key,
            model: "whisper-1".to_string(),
            base_url: "https://api.openai.com".to_string(),
        }
    }

    /// Override the base URL (for proxied or compatible endpoints).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        let mut url = base_url.into();
        // Normalize: strip trailing slash to avoid double-slash in URL construction
        while url.ends_with('/') {
            url.pop();
        }
        self.base_url = url;
        self
    }

    /// Override the model name.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }
}

#[async_trait]
impl TranscriptionProvider for OpenAiWhisperProvider {
    async fn transcribe(
        &self,
        audio_data: &[u8],
        format: AudioFormat,
    ) -> Result<String, TranscriptionError> {
        if audio_data.is_empty() {
            return Err(TranscriptionError::EmptyAudio);
        }

        let filename = format!("audio.{}", format.extension());
        let mime_str = match format {
            AudioFormat::Ogg => "audio/ogg",
            AudioFormat::Mp3 => "audio/mpeg",
            AudioFormat::Mp4 => "audio/mp4",
            AudioFormat::Wav => "audio/wav",
            AudioFormat::Webm => "audio/webm",
            AudioFormat::Flac => "audio/flac",
            AudioFormat::M4a => "audio/m4a",
        };

        let file_part = multipart::Part::bytes(audio_data.to_vec())
            .file_name(filename)
            .mime_str(mime_str)
            .map_err(|e| TranscriptionError::RequestFailed(e.to_string()))?;

        let form = multipart::Form::new()
            .text("model", self.model.clone())
            .text("response_format", "text")
            .part("file", file_part);

        let url = format!("{}/v1/audio/transcriptions", self.base_url);

        let response = self
            .client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .multipart(form)
            .send()
            .await
            .map_err(|e| TranscriptionError::RequestFailed(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(TranscriptionError::RequestFailed(format!(
                "HTTP {}: {}",
                status, body
            )));
        }

        let text = response
            .text()
            .await
            .map_err(|e| TranscriptionError::RequestFailed(e.to_string()))?;

        Ok(text.trim().to_string())
    }
}
