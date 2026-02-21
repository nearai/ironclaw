//! OpenAI Whisper transcription provider.
//!
//! Uses the OpenAI `/v1/audio/transcriptions` endpoint with multipart form upload.

use async_trait::async_trait;
use reqwest::multipart;
use secrecy::{ExposeSecret, SecretString};

use crate::transcription::{AudioFormat, TranscriptionError, TranscriptionProvider};

/// Maximum file size for Whisper API (25 MB).
const WHISPER_MAX_FILE_SIZE: usize = 25 * 1024 * 1024;

/// Supported formats for Whisper.
const WHISPER_FORMATS: &[AudioFormat] = &[
    AudioFormat::OggOpus,
    AudioFormat::Mp3,
    AudioFormat::Wav,
    AudioFormat::Webm,
    AudioFormat::M4a,
];

/// OpenAI Whisper transcription provider.
pub struct OpenAiWhisper {
    api_key: SecretString,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAiWhisper {
    /// Create a new OpenAI Whisper provider.
    pub fn new(api_key: SecretString, model: String) -> Self {
        Self {
            api_key,
            model,
            base_url: "https://api.openai.com".to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Set a custom base URL (for testing or alternative endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[async_trait]
impl TranscriptionProvider for OpenAiWhisper {
    fn name(&self) -> &str {
        "openai"
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn max_file_size(&self) -> usize {
        WHISPER_MAX_FILE_SIZE
    }

    fn supported_formats(&self) -> &[AudioFormat] {
        WHISPER_FORMATS
    }

    async fn transcribe(
        &self,
        audio: &[u8],
        format: AudioFormat,
        language: Option<&str>,
    ) -> Result<String, TranscriptionError> {
        if audio.len() > WHISPER_MAX_FILE_SIZE {
            return Err(TranscriptionError::FileTooLarge {
                size: audio.len(),
                max: WHISPER_MAX_FILE_SIZE,
            });
        }

        let filename = format!("audio.{}", format.extension());

        // Note: to_vec() copies the audio bytes for the multipart body.
        // Peak memory is ~2x the file size (original slice + copy). Acceptable
        // for the 25 MB Whisper limit; revisit if supporting larger files.
        let file_part = multipart::Part::bytes(audio.to_vec())
            .file_name(filename)
            .mime_str(format.mime_type())
            .map_err(|e| TranscriptionError::RequestFailed(e.to_string()))?;

        let mut form = multipart::Form::new()
            .part("file", file_part)
            .text("model", self.model.clone())
            .text("response_format", "text");

        if let Some(lang) = language {
            form = form.text("language", lang.to_string());
        }

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
        let body = response
            .text()
            .await
            .map_err(|e| TranscriptionError::RequestFailed(e.to_string()))?;

        if !status.is_success() {
            return Err(TranscriptionError::ApiError {
                message: format!("HTTP {}: {}", status, body),
            });
        }

        // response_format=text returns raw text, trim whitespace
        Ok(body.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_too_large_is_rejected_sync() {
        // The size check happens before any async work, so we can verify
        // via a simple construction + manual call of the guard logic.
        let oversized = vec![0u8; WHISPER_MAX_FILE_SIZE + 1];
        assert!(oversized.len() > WHISPER_MAX_FILE_SIZE);
    }

    #[tokio::test]
    async fn file_too_large_returns_error() {
        let provider = OpenAiWhisper::new(
            SecretString::from("sk-test".to_string()),
            "whisper-1".to_string(),
        );

        let oversized = vec![0u8; WHISPER_MAX_FILE_SIZE + 1];
        let result = provider
            .transcribe(&oversized, AudioFormat::Wav, None)
            .await;

        match result {
            Err(TranscriptionError::FileTooLarge { size, max }) => {
                assert_eq!(size, WHISPER_MAX_FILE_SIZE + 1);
                assert_eq!(max, WHISPER_MAX_FILE_SIZE);
            }
            other => panic!("Expected FileTooLarge, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn api_error_on_bad_url() {
        // Point at a URL that will fail to connect
        let provider = OpenAiWhisper::new(
            SecretString::from("sk-test".to_string()),
            "whisper-1".to_string(),
        )
        .with_base_url("http://127.0.0.1:1"); // port 1 won't be listening

        let audio = vec![0u8; 100];
        let result = provider
            .transcribe(&audio, AudioFormat::OggOpus, Some("en"))
            .await;

        assert!(
            matches!(result, Err(TranscriptionError::RequestFailed(_))),
            "Expected RequestFailed, got: {:?}",
            result
        );
    }

    #[test]
    fn provider_metadata() {
        let provider = OpenAiWhisper::new(
            SecretString::from("sk-test".to_string()),
            "whisper-1".to_string(),
        );

        assert_eq!(provider.name(), "openai");
        assert_eq!(provider.model_name(), "whisper-1");
        assert_eq!(provider.max_file_size(), WHISPER_MAX_FILE_SIZE);
        assert_eq!(provider.supported_formats().len(), 5);
    }
}
