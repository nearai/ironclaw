//! Audio transcription pipeline.
//!
//! Provides a [`TranscriptionProvider`] trait for pluggable speech-to-text
//! backends and a [`TranscriptionMiddleware`] that detects audio attachments
//! on incoming messages and replaces them with transcribed text.

mod chat_completions;
mod openai;

pub use self::chat_completions::ChatCompletionsTranscriptionProvider;
pub use self::openai::OpenAiWhisperProvider;

use async_trait::async_trait;

/// Supported audio formats for transcription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Ogg,
    Mp3,
    Mp4,
    Wav,
    Webm,
    Flac,
    M4a,
}

impl AudioFormat {
    /// Infer audio format from MIME type. Returns `None` for unsupported types.
    pub fn from_mime_type(mime: &str) -> Option<Self> {
        let base = mime.split(';').next().unwrap_or(mime).trim();
        match base {
            "audio/ogg" | "audio/opus" => Some(Self::Ogg),
            "audio/mpeg" | "audio/mp3" => Some(Self::Mp3),
            "audio/mp4" => Some(Self::Mp4),
            "audio/wav" | "audio/x-wav" => Some(Self::Wav),
            "audio/webm" => Some(Self::Webm),
            "audio/flac" | "audio/x-flac" => Some(Self::Flac),
            "audio/m4a" | "audio/x-m4a" | "audio/aac" => Some(Self::M4a),
            _ => None,
        }
    }

    /// File extension for this format (used as the filename in multipart uploads).
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Ogg => "ogg",
            Self::Mp3 => "mp3",
            Self::Mp4 => "mp4",
            Self::Wav => "wav",
            Self::Webm => "webm",
            Self::Flac => "flac",
            Self::M4a => "m4a",
        }
    }
}

/// Errors from the transcription pipeline.
#[derive(Debug, thiserror::Error)]
pub enum TranscriptionError {
    #[error("Transcription request failed: {0}")]
    RequestFailed(String),

    #[error("Unsupported audio format: {mime_type}")]
    UnsupportedFormat { mime_type: String },

    #[error("Audio data is empty")]
    EmptyAudio,
}

/// Trait for speech-to-text providers.
#[async_trait]
pub trait TranscriptionProvider: Send + Sync {
    /// Transcribe audio bytes into text.
    async fn transcribe(
        &self,
        audio_data: &[u8],
        format: AudioFormat,
    ) -> Result<String, TranscriptionError>;
}

/// Middleware that processes audio attachments on incoming messages.
///
/// When an incoming message has audio attachments with inline data,
/// the middleware transcribes them and sets `extracted_text` on the attachment.
/// If the message has no text content, the transcription becomes the message content.
pub struct TranscriptionMiddleware {
    provider: Box<dyn TranscriptionProvider>,
}

impl TranscriptionMiddleware {
    /// Create a new middleware with the given transcription provider.
    pub fn new(provider: Box<dyn TranscriptionProvider>) -> Self {
        Self { provider }
    }

    /// Process an incoming message, transcribing any audio attachments with data.
    ///
    /// Modifies the message in place:
    /// - Sets `extracted_text` on audio attachments that have inline data
    /// - If the message content is empty, sets it to the transcription
    pub async fn process(&self, msg: &mut crate::channels::IncomingMessage) {
        use crate::channels::AttachmentKind;

        let mut transcriptions = Vec::new();

        for (i, attachment) in msg.attachments.iter().enumerate() {
            if attachment.kind != AttachmentKind::Audio {
                continue;
            }
            if attachment.data.is_empty() {
                continue;
            }
            // Already transcribed
            if attachment.extracted_text.is_some() {
                continue;
            }

            let format = match AudioFormat::from_mime_type(&attachment.mime_type) {
                Some(f) => f,
                None => {
                    tracing::warn!(
                        mime = %attachment.mime_type,
                        "Skipping audio attachment with unsupported format"
                    );
                    continue;
                }
            };

            match self.provider.transcribe(&attachment.data, format).await {
                Ok(text) => {
                    tracing::info!(
                        attachment_id = %attachment.id,
                        text_len = text.len(),
                        "Transcribed audio attachment"
                    );
                    transcriptions.push((i, text));
                }
                Err(e) => {
                    tracing::error!(
                        attachment_id = %attachment.id,
                        error = %e,
                        "Failed to transcribe audio attachment"
                    );
                    transcriptions.push((i, format!("[Transcription failed: {}]", e)));
                }
            }
        }

        for (i, text) in &transcriptions {
            msg.attachments[*i].extracted_text = Some(text.clone());
        }

        // If message has no text content, use the first successful transcription
        if (msg.content.is_empty() || msg.content == "[Voice note]")
            && let Some((_, text)) = transcriptions
                .iter()
                .find(|(_, t)| !t.starts_with("[Transcription failed"))
        {
            msg.content = text.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::{AttachmentKind, IncomingAttachment, IncomingMessage};

    struct MockProvider {
        result: Result<String, TranscriptionError>,
    }

    #[async_trait]
    impl TranscriptionProvider for MockProvider {
        async fn transcribe(
            &self,
            _audio_data: &[u8],
            _format: AudioFormat,
        ) -> Result<String, TranscriptionError> {
            match &self.result {
                Ok(text) => Ok(text.clone()),
                Err(_) => Err(TranscriptionError::RequestFailed("mock error".into())),
            }
        }
    }

    fn voice_attachment(data: Vec<u8>) -> IncomingAttachment {
        IncomingAttachment {
            id: "voice_123".to_string(),
            kind: AttachmentKind::Audio,
            mime_type: "audio/ogg".to_string(),
            filename: Some("voice.ogg".to_string()),
            size_bytes: Some(data.len() as u64),
            source_url: None,
            storage_key: None,
            local_path: None,
            extracted_text: None,
            data,
            duration_secs: Some(5),
        }
    }

    #[tokio::test]
    async fn middleware_transcribes_audio_attachment() {
        let middleware = TranscriptionMiddleware::new(Box::new(MockProvider {
            result: Ok("Hello world".to_string()),
        }));

        let mut msg = IncomingMessage::new("telegram", "user1", "[Voice note]")
            .with_attachments(vec![voice_attachment(vec![1, 2, 3])]);

        middleware.process(&mut msg).await;

        assert_eq!(
            msg.attachments[0].extracted_text.as_deref(),
            Some("Hello world")
        );
        assert_eq!(msg.content, "Hello world");
    }

    #[tokio::test]
    async fn middleware_skips_empty_audio_data() {
        let middleware = TranscriptionMiddleware::new(Box::new(MockProvider {
            result: Ok("Should not be called".to_string()),
        }));

        let mut msg = IncomingMessage::new("telegram", "user1", "text message")
            .with_attachments(vec![voice_attachment(Vec::new())]);

        middleware.process(&mut msg).await;

        assert!(msg.attachments[0].extracted_text.is_none());
        assert_eq!(msg.content, "text message");
    }

    #[tokio::test]
    async fn middleware_skips_already_transcribed() {
        let middleware = TranscriptionMiddleware::new(Box::new(MockProvider {
            result: Ok("New transcription".to_string()),
        }));

        let mut attachment = voice_attachment(vec![1, 2, 3]);
        attachment.extracted_text = Some("Already done".to_string());

        let mut msg =
            IncomingMessage::new("telegram", "user1", "").with_attachments(vec![attachment]);

        middleware.process(&mut msg).await;

        assert_eq!(
            msg.attachments[0].extracted_text.as_deref(),
            Some("Already done")
        );
    }

    #[tokio::test]
    async fn middleware_preserves_existing_content() {
        let middleware = TranscriptionMiddleware::new(Box::new(MockProvider {
            result: Ok("Transcription".to_string()),
        }));

        let mut msg = IncomingMessage::new("telegram", "user1", "User typed this")
            .with_attachments(vec![voice_attachment(vec![1, 2, 3])]);

        middleware.process(&mut msg).await;

        assert_eq!(
            msg.attachments[0].extracted_text.as_deref(),
            Some("Transcription")
        );
        assert_eq!(msg.content, "User typed this");
    }

    #[test]
    fn audio_format_from_mime() {
        assert_eq!(
            AudioFormat::from_mime_type("audio/ogg"),
            Some(AudioFormat::Ogg)
        );
        assert_eq!(
            AudioFormat::from_mime_type("audio/mpeg"),
            Some(AudioFormat::Mp3)
        );
        assert_eq!(
            AudioFormat::from_mime_type("audio/ogg; codecs=opus"),
            Some(AudioFormat::Ogg)
        );
        assert_eq!(AudioFormat::from_mime_type("image/jpeg"), None);
    }
}
