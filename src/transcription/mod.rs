//! Audio transcription for voice notes and audio attachments.
//!
//! Provides a `TranscriptionProvider` trait and middleware that automatically
//! transcribes audio attachments on incoming messages before they reach the agent.
//!
//! # Architecture
//!
//! ```text
//! [WASM Channel] → emit-message { attachments=[audio bytes] }
//!     → [Host: EmittedMessage → IncomingMessage]
//!     → [TranscriptionMiddleware: detect audio, call provider, replace content]
//!     → [Agent Loop: sees plain text]
//! ```

pub mod openai;

use std::sync::Arc;

use async_trait::async_trait;

use crate::channels::{AttachmentKind, IncomingMessage};

/// Supported audio formats for transcription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// OGG with Opus codec (Telegram voice notes).
    OggOpus,
    /// MP3.
    Mp3,
    /// WAV.
    Wav,
    /// WebM.
    Webm,
    /// M4A / AAC.
    M4a,
}

impl AudioFormat {
    /// File extension for this format.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::OggOpus => "ogg",
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
            Self::Webm => "webm",
            Self::M4a => "m4a",
        }
    }

    /// MIME type for this format.
    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::OggOpus => "audio/ogg",
            Self::Mp3 => "audio/mpeg",
            Self::Wav => "audio/wav",
            Self::Webm => "audio/webm",
            Self::M4a => "audio/mp4",
        }
    }

    /// Detect format from MIME type string.
    pub fn from_mime_type(mime: &str) -> Option<Self> {
        // Normalize: strip parameters (e.g., "audio/ogg; codecs=opus" → "audio/ogg")
        let base = mime.split(';').next().unwrap_or(mime).trim();
        match base {
            "audio/ogg" | "audio/opus" => Some(Self::OggOpus),
            "audio/mpeg" | "audio/mp3" => Some(Self::Mp3),
            "audio/wav" | "audio/x-wav" | "audio/wave" => Some(Self::Wav),
            "audio/webm" => Some(Self::Webm),
            "audio/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac" => Some(Self::M4a),
            _ => None,
        }
    }
}

/// Errors from transcription operations.
#[derive(Debug, thiserror::Error)]
pub enum TranscriptionError {
    /// Unsupported audio format.
    #[error("unsupported audio format: {mime_type}")]
    UnsupportedFormat { mime_type: String },

    /// Audio file exceeds the provider's size limit.
    #[error("audio file too large: {size} bytes (max {max})")]
    FileTooLarge { size: usize, max: usize },

    /// Provider API returned an error.
    #[error("transcription API error: {message}")]
    ApiError { message: String },

    /// Network or HTTP error.
    #[error("transcription request failed: {0}")]
    RequestFailed(String),

    /// Provider is not configured.
    #[error("transcription provider not configured: {reason}")]
    NotConfigured { reason: String },
}

/// Trait for speech-to-text transcription providers.
#[async_trait]
pub trait TranscriptionProvider: Send + Sync {
    /// Provider name (e.g., "openai").
    fn name(&self) -> &str;

    /// Model name (e.g., "whisper-1").
    fn model_name(&self) -> &str;

    /// Maximum file size in bytes.
    fn max_file_size(&self) -> usize;

    /// Supported audio formats.
    fn supported_formats(&self) -> &[AudioFormat];

    /// Transcribe audio bytes to text.
    async fn transcribe(
        &self,
        audio: &[u8],
        format: AudioFormat,
        language: Option<&str>,
    ) -> Result<String, TranscriptionError>;
}

/// Middleware that detects audio attachments and transcribes them.
pub struct TranscriptionMiddleware {
    provider: Arc<dyn TranscriptionProvider>,
    language: Option<String>,
}

impl TranscriptionMiddleware {
    /// Create a new transcription middleware.
    pub fn new(provider: Arc<dyn TranscriptionProvider>, language: Option<String>) -> Self {
        Self { provider, language }
    }

    /// Process an incoming message, transcribing any audio attachments.
    ///
    /// If audio attachments are found and transcription succeeds, the message
    /// content is replaced with the transcribed text. On failure, a fallback
    /// message is used.
    pub async fn process(&self, mut msg: IncomingMessage) -> IncomingMessage {
        let audio_attachment = msg
            .attachments
            .iter()
            .find(|a| a.kind == AttachmentKind::Audio);

        let Some(attachment) = audio_attachment else {
            return msg;
        };

        let format = match AudioFormat::from_mime_type(&attachment.mime_type) {
            Some(f) => f,
            None => {
                tracing::warn!(
                    mime = %attachment.mime_type,
                    "Unsupported audio format for transcription"
                );
                if msg.content.is_empty() || msg.content == "[Voice note]" {
                    msg.content = "[Voice note: unsupported audio format]".to_string();
                }
                return msg;
            }
        };

        // Check size limit
        if attachment.data.len() > self.provider.max_file_size() {
            tracing::warn!(
                size = attachment.data.len(),
                max = self.provider.max_file_size(),
                "Audio attachment exceeds provider size limit"
            );
            if msg.content.is_empty() || msg.content == "[Voice note]" {
                msg.content = "[Voice note: file too large for transcription]".to_string();
            }
            return msg;
        }

        match self
            .provider
            .transcribe(&attachment.data, format, self.language.as_deref())
            .await
        {
            Ok(text) => {
                tracing::info!(
                    provider = %self.provider.name(),
                    text_len = text.len(),
                    "Audio transcription successful"
                );
                msg.content = text;
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    provider = %self.provider.name(),
                    "Audio transcription failed"
                );
                if msg.content.is_empty() || msg.content == "[Voice note]" {
                    msg.content = "[Voice note: transcription failed]".to_string();
                }
            }
        }

        msg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::Attachment;

    #[test]
    fn audio_format_from_mime_type() {
        assert_eq!(
            AudioFormat::from_mime_type("audio/ogg"),
            Some(AudioFormat::OggOpus)
        );
        assert_eq!(
            AudioFormat::from_mime_type("audio/ogg; codecs=opus"),
            Some(AudioFormat::OggOpus)
        );
        assert_eq!(
            AudioFormat::from_mime_type("audio/mpeg"),
            Some(AudioFormat::Mp3)
        );
        assert_eq!(
            AudioFormat::from_mime_type("audio/mp3"),
            Some(AudioFormat::Mp3)
        );
        assert_eq!(
            AudioFormat::from_mime_type("audio/wav"),
            Some(AudioFormat::Wav)
        );
        assert_eq!(
            AudioFormat::from_mime_type("audio/webm"),
            Some(AudioFormat::Webm)
        );
        assert_eq!(
            AudioFormat::from_mime_type("audio/mp4"),
            Some(AudioFormat::M4a)
        );
        assert_eq!(
            AudioFormat::from_mime_type("audio/m4a"),
            Some(AudioFormat::M4a)
        );
        assert_eq!(AudioFormat::from_mime_type("text/plain"), None);
        assert_eq!(AudioFormat::from_mime_type(""), None);
    }

    #[test]
    fn audio_format_extension() {
        assert_eq!(AudioFormat::OggOpus.extension(), "ogg");
        assert_eq!(AudioFormat::Mp3.extension(), "mp3");
        assert_eq!(AudioFormat::Wav.extension(), "wav");
        assert_eq!(AudioFormat::Webm.extension(), "webm");
        assert_eq!(AudioFormat::M4a.extension(), "m4a");
    }

    #[test]
    fn audio_format_mime_type() {
        assert_eq!(AudioFormat::OggOpus.mime_type(), "audio/ogg");
        assert_eq!(AudioFormat::Mp3.mime_type(), "audio/mpeg");
    }

    /// Mock provider for testing middleware.
    struct MockProvider {
        result: Result<String, TranscriptionError>,
    }

    #[async_trait]
    impl TranscriptionProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }
        fn model_name(&self) -> &str {
            "mock-1"
        }
        fn max_file_size(&self) -> usize {
            25 * 1024 * 1024
        }
        fn supported_formats(&self) -> &[AudioFormat] {
            &[AudioFormat::OggOpus, AudioFormat::Mp3]
        }
        async fn transcribe(
            &self,
            _audio: &[u8],
            _format: AudioFormat,
            _language: Option<&str>,
        ) -> Result<String, TranscriptionError> {
            match &self.result {
                Ok(text) => Ok(text.clone()),
                Err(_) => Err(TranscriptionError::ApiError {
                    message: "mock error".to_string(),
                }),
            }
        }
    }

    #[tokio::test]
    async fn middleware_transcribes_audio_attachment() {
        let provider = Arc::new(MockProvider {
            result: Ok("Hello, world!".to_string()),
        });
        let middleware = TranscriptionMiddleware::new(provider, None);

        let msg = IncomingMessage::new("telegram", "user1", "[Voice note]").with_attachments(vec![
            Attachment {
                kind: AttachmentKind::Audio,
                mime_type: "audio/ogg".to_string(),
                data: vec![0u8; 100],
                filename: None,
                duration_secs: Some(5),
            },
        ]);

        let result = middleware.process(msg).await;
        assert_eq!(result.content, "Hello, world!");
    }

    #[tokio::test]
    async fn middleware_skips_non_audio_messages() {
        let provider = Arc::new(MockProvider {
            result: Ok("transcribed".to_string()),
        });
        let middleware = TranscriptionMiddleware::new(provider, None);

        let msg = IncomingMessage::new("telegram", "user1", "regular text");
        let result = middleware.process(msg).await;
        assert_eq!(result.content, "regular text");
    }

    #[tokio::test]
    async fn middleware_handles_transcription_failure() {
        let provider = Arc::new(MockProvider {
            result: Err(TranscriptionError::ApiError {
                message: "test".to_string(),
            }),
        });
        let middleware = TranscriptionMiddleware::new(provider, None);

        let msg = IncomingMessage::new("telegram", "user1", "[Voice note]").with_attachments(vec![
            Attachment {
                kind: AttachmentKind::Audio,
                mime_type: "audio/ogg".to_string(),
                data: vec![0u8; 100],
                filename: None,
                duration_secs: Some(5),
            },
        ]);

        let result = middleware.process(msg).await;
        assert_eq!(result.content, "[Voice note: transcription failed]");
    }

    #[tokio::test]
    async fn middleware_handles_unsupported_format() {
        let provider = Arc::new(MockProvider {
            result: Ok("text".to_string()),
        });
        let middleware = TranscriptionMiddleware::new(provider, None);

        let msg = IncomingMessage::new("telegram", "user1", "[Voice note]").with_attachments(vec![
            Attachment {
                kind: AttachmentKind::Audio,
                mime_type: "audio/flac".to_string(),
                data: vec![0u8; 100],
                filename: None,
                duration_secs: None,
            },
        ]);

        let result = middleware.process(msg).await;
        assert_eq!(result.content, "[Voice note: unsupported audio format]");
    }
}
