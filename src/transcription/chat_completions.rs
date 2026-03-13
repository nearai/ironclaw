//! Chat Completions-based transcription provider.
//!
//! Uses the `/v1/chat/completions` endpoint with `input_audio` content type
//! to transcribe audio. Compatible with OpenRouter, OpenAI GPT-4o-audio, and
//! any provider that supports audio input via the Chat Completions API.

use async_trait::async_trait;
use base64::Engine;
use secrecy::{ExposeSecret, SecretString};

use super::{AudioFormat, TranscriptionError, TranscriptionProvider};

/// Transcription provider that sends audio via the Chat Completions API.
///
/// Unlike the Whisper provider (which uses `/v1/audio/transcriptions` with
/// multipart upload), this provider sends base64-encoded audio as an
/// `input_audio` content part in a chat message, enabling use with
/// OpenRouter and other providers that only expose audio through the
/// Chat Completions API.
pub struct ChatCompletionsTranscriptionProvider {
    client: reqwest::Client,
    api_key: SecretString,
    model: String,
    base_url: String,
}

impl ChatCompletionsTranscriptionProvider {
    /// Create a new provider with the given API key.
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
            model: "google/gemini-2.0-flash-001".to_string(),
            base_url: "https://openrouter.ai/api".to_string(),
        }
    }

    /// Override the base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        let mut url = base_url.into();
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

/// Map [`AudioFormat`] to the format string expected by the Chat Completions API.
fn audio_format_str(format: AudioFormat) -> &'static str {
    match format {
        AudioFormat::Ogg => "ogg",
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Mp4 => "mp4",
        AudioFormat::Wav => "wav",
        AudioFormat::Webm => "webm",
        AudioFormat::Flac => "flac",
        AudioFormat::M4a => "m4a",
    }
}

#[async_trait]
impl TranscriptionProvider for ChatCompletionsTranscriptionProvider {
    async fn transcribe(
        &self,
        audio_data: &[u8],
        format: AudioFormat,
    ) -> Result<String, TranscriptionError> {
        if audio_data.is_empty() {
            return Err(TranscriptionError::EmptyAudio);
        }

        let b64 = base64::engine::general_purpose::STANDARD.encode(audio_data);

        let body = serde_json::json!({
            "model": self.model,
            "messages": [{
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "Transcribe this audio. Return only the transcript text, nothing else."
                    },
                    {
                        "type": "input_audio",
                        "input_audio": {
                            "data": b64,
                            "format": audio_format_str(format)
                        }
                    }
                ]
            }]
        });

        let url = format!("{}/v1/chat/completions", self.base_url);

        let response = self
            .client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .json(&body)
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

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| TranscriptionError::RequestFailed(e.to_string()))?;

        // Extract text from the standard Chat Completions response format:
        // { "choices": [{ "message": { "content": "..." } }] }
        let text = json
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| {
                TranscriptionError::RequestFailed(
                    "unexpected response format: missing choices[0].message.content".to_string(),
                )
            })?;

        Ok(text.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_format_str_maps_all_variants() {
        assert_eq!(audio_format_str(AudioFormat::Ogg), "ogg");
        assert_eq!(audio_format_str(AudioFormat::Mp3), "mp3");
        assert_eq!(audio_format_str(AudioFormat::Mp4), "mp4");
        assert_eq!(audio_format_str(AudioFormat::Wav), "wav");
        assert_eq!(audio_format_str(AudioFormat::Webm), "webm");
        assert_eq!(audio_format_str(AudioFormat::Flac), "flac");
        assert_eq!(audio_format_str(AudioFormat::M4a), "m4a");
    }

    #[tokio::test]
    async fn rejects_empty_audio() {
        let provider =
            ChatCompletionsTranscriptionProvider::new(SecretString::from("test-key".to_string()));
        let result = provider.transcribe(&[], AudioFormat::Ogg).await;
        assert!(matches!(result, Err(TranscriptionError::EmptyAudio)));
    }
}
