//! Document text extraction pipeline.
//!
//! Provides a [`DocumentExtractionMiddleware`] that detects document attachments
//! on incoming messages and extracts text content so the LLM can reason about them.
//!
//! Supported formats:
//! - **PDF** — via `pdf-extract`
//! - **Office XML** (DOCX, PPTX, XLSX) — ZIP + XML text extraction
//! - **Plain text** (TXT, CSV, JSON, XML, Markdown, code) — UTF-8 decode

mod extractors;

use crate::channels::{AttachmentKind, IncomingMessage};

/// Maximum document size to download/extract (10 MB).
const MAX_DOCUMENT_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum extracted text length to keep (100K chars ≈ ~25K tokens).
const MAX_EXTRACTED_TEXT_LEN: usize = 100_000;

/// Middleware that processes document attachments on incoming messages.
///
/// For each document attachment, attempts to:
/// 1. Download bytes from `source_url` if `data` is empty
/// 2. Extract text based on MIME type
/// 3. Set `extracted_text` on the attachment
#[derive(Default)]
pub struct DocumentExtractionMiddleware;

impl DocumentExtractionMiddleware {
    pub fn new() -> Self {
        Self
    }

    /// Process an incoming message, extracting text from document attachments.
    pub async fn process(&self, msg: &mut IncomingMessage) {
        let mut extractions = Vec::new();

        for (i, attachment) in msg.attachments.iter().enumerate() {
            if attachment.kind != AttachmentKind::Document {
                continue;
            }
            if attachment.extracted_text.is_some() {
                continue;
            }

            // Check if too large
            if let Some(size) = attachment.size_bytes.filter(|&s| s > MAX_DOCUMENT_SIZE) {
                tracing::warn!(
                    attachment_id = %attachment.id,
                    size,
                    "Document too large for extraction, skipping"
                );
                let mb = size as f64 / (1024.0 * 1024.0);
                let max_mb = MAX_DOCUMENT_SIZE as f64 / (1024.0 * 1024.0);
                extractions.push((
                    i,
                    format!(
                        "[Document too large for text extraction: {mb:.1} MB exceeds {max_mb:.0} MB limit. \
                         Please send a smaller file or copy-paste the relevant text.]"
                    ),
                ));
                continue;
            }

            // Use inline data only — downloading from source_url is intentionally
            // not supported to prevent SSRF. Channels must populate attachment.data
            // via store_attachment_data before emitting the message.
            let data = if !attachment.data.is_empty() {
                attachment.data.clone()
            } else {
                extractions.push((
                    i,
                    "[Document has no data and no download URL. \
                     Please try sending the file again.]"
                        .to_string(),
                ));
                continue;
            };

            // Enforce size limit on actual data (size_bytes may be missing or underreported)
            if data.len() as u64 > MAX_DOCUMENT_SIZE {
                let mb = data.len() as f64 / (1024.0 * 1024.0);
                let max_mb = MAX_DOCUMENT_SIZE as f64 / (1024.0 * 1024.0);
                extractions.push((
                    i,
                    format!(
                        "[Document too large for text extraction: {mb:.1} MB exceeds {max_mb:.0} MB limit. \
                         Please send a smaller file or copy-paste the relevant text.]"
                    ),
                ));
                continue;
            }

            let mime = &attachment.mime_type;
            let filename = attachment.filename.as_deref();
            match extractors::extract_text(&data, mime, filename) {
                Ok(text) => {
                    // Truncate at a char boundary to avoid panicking on multi-byte UTF-8
                    let text = if text.len() > MAX_EXTRACTED_TEXT_LEN {
                        let boundary = text
                            .char_indices()
                            .map(|(i, _)| i)
                            .take_while(|&i| i <= MAX_EXTRACTED_TEXT_LEN)
                            .last()
                            .unwrap_or(0);
                        let mut truncated = text[..boundary].to_string();
                        truncated.push_str("\n\n[... truncated, document too long ...]");
                        truncated
                    } else {
                        text
                    };
                    tracing::info!(
                        attachment_id = %attachment.id,
                        mime_type = %mime,
                        text_len = text.len(),
                        "Extracted text from document"
                    );
                    extractions.push((i, text));
                }
                Err(e) => {
                    tracing::warn!(
                        attachment_id = %attachment.id,
                        mime_type = %mime,
                        error = %e,
                        "Failed to extract text from document"
                    );
                    let name = filename.unwrap_or("document");
                    extractions.push((
                        i,
                        format!(
                            "[Failed to extract text from '{name}' ({mime}): {e}. \
                             The file format may not be supported.]"
                        ),
                    ));
                }
            }
        }

        for (i, text) in extractions {
            msg.attachments[i].extracted_text = Some(text);
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::IncomingAttachment;

    fn doc_attachment(mime: &str, filename: &str, data: Vec<u8>) -> IncomingAttachment {
        IncomingAttachment {
            id: "doc_1".to_string(),
            kind: AttachmentKind::Document,
            mime_type: mime.to_string(),
            filename: Some(filename.to_string()),
            size_bytes: Some(data.len() as u64),
            source_url: None,
            storage_key: None,
            extracted_text: None,
            data,
            duration_secs: None,
        }
    }

    #[tokio::test]
    async fn extracts_plain_text() {
        let middleware = DocumentExtractionMiddleware::new();
        let mut msg = IncomingMessage::new("test", "user1", "check this").with_attachments(vec![
            doc_attachment("text/plain", "notes.txt", b"Hello world".to_vec()),
        ]);

        middleware.process(&mut msg).await;
        assert_eq!(
            msg.attachments[0].extracted_text.as_deref(),
            Some("Hello world")
        );
    }

    #[tokio::test]
    async fn extracts_csv() {
        let middleware = DocumentExtractionMiddleware::new();
        let mut msg = IncomingMessage::new("test", "user1", "analyze").with_attachments(vec![
            doc_attachment("text/csv", "data.csv", b"name,age\nAlice,30".to_vec()),
        ]);

        middleware.process(&mut msg).await;
        assert_eq!(
            msg.attachments[0].extracted_text.as_deref(),
            Some("name,age\nAlice,30")
        );
    }

    #[tokio::test]
    async fn extracts_json() {
        let middleware = DocumentExtractionMiddleware::new();
        let data = br#"{"key": "value"}"#.to_vec();
        let mut msg = IncomingMessage::new("test", "user1", "parse")
            .with_attachments(vec![doc_attachment("application/json", "data.json", data)]);

        middleware.process(&mut msg).await;
        assert!(msg.attachments[0].extracted_text.is_some());
    }

    #[tokio::test]
    async fn skips_already_extracted() {
        let middleware = DocumentExtractionMiddleware::new();
        let mut att = doc_attachment("text/plain", "test.txt", b"data".to_vec());
        att.extracted_text = Some("Already done".to_string());
        let mut msg = IncomingMessage::new("test", "user1", "").with_attachments(vec![att]);

        middleware.process(&mut msg).await;
        assert_eq!(
            msg.attachments[0].extracted_text.as_deref(),
            Some("Already done")
        );
    }

    #[tokio::test]
    async fn skips_audio_attachments() {
        let middleware = DocumentExtractionMiddleware::new();
        let mut att = doc_attachment("text/plain", "test.txt", b"data".to_vec());
        att.kind = AttachmentKind::Audio;
        let mut msg = IncomingMessage::new("test", "user1", "").with_attachments(vec![att]);

        middleware.process(&mut msg).await;
        assert!(msg.attachments[0].extracted_text.is_none());
    }

    #[tokio::test]
    async fn reports_oversized_documents() {
        let middleware = DocumentExtractionMiddleware::new();
        let mut att = doc_attachment("text/plain", "huge.txt", vec![]);
        att.size_bytes = Some(MAX_DOCUMENT_SIZE + 1);
        let mut msg = IncomingMessage::new("test", "user1", "").with_attachments(vec![att]);

        middleware.process(&mut msg).await;
        let text = msg.attachments[0].extracted_text.as_deref().unwrap();
        assert!(
            text.contains("too large"),
            "Expected 'too large' error, got: {text}"
        );
    }

    #[tokio::test]
    async fn truncates_long_text() {
        let middleware = DocumentExtractionMiddleware::new();
        let long_text = "x".repeat(MAX_EXTRACTED_TEXT_LEN + 1000);
        let mut msg =
            IncomingMessage::new("test", "user1", "read").with_attachments(vec![doc_attachment(
                "text/plain",
                "long.txt",
                long_text.into_bytes(),
            )]);

        middleware.process(&mut msg).await;
        let extracted = msg.attachments[0].extracted_text.as_ref().unwrap();
        assert!(extracted.len() < MAX_EXTRACTED_TEXT_LEN + 100);
        assert!(extracted.ends_with("[... truncated, document too long ...]"));
    }

    #[tokio::test]
    async fn extracts_pdf_text() {
        // Minimal valid PDF with text "Hello World"
        let pdf_bytes = include_bytes!("../../tests/fixtures/hello.pdf");
        let middleware = DocumentExtractionMiddleware::new();
        let mut msg =
            IncomingMessage::new("test", "user1", "review").with_attachments(vec![doc_attachment(
                "application/pdf",
                "hello.pdf",
                pdf_bytes.to_vec(),
            )]);

        middleware.process(&mut msg).await;
        let text = msg.attachments[0].extracted_text.as_deref().unwrap_or("");
        assert!(
            text.contains("Hello"),
            "PDF extraction should contain 'Hello', got: {text}"
        );
    }
}
