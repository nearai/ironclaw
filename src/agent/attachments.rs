//! Augment user message content with structured attachment context.

use base64::Engine;

use crate::channels::{AttachmentKind, IncomingAttachment};
use crate::llm::{ContentPart, ImageUrl};

/// Result of processing attachments for the LLM pipeline.
pub struct AugmentResult {
    /// Augmented text content with attachment metadata appended.
    pub text: String,
    /// Image content parts to include as multimodal input.
    pub image_parts: Vec<ContentPart>,
}

/// Process attachments into augmented text and multimodal image parts.
///
/// Returns `None` if `attachments` is empty (caller should use original content).
/// Returns `Some(AugmentResult)` with:
/// - `text`: original content + `<attachments>` block (metadata, transcripts, etc.)
/// - `image_parts`: `ContentPart::ImageUrl` entries for images with data
pub fn augment_with_attachments(
    content: &str,
    attachments: &[IncomingAttachment],
) -> Option<AugmentResult> {
    if attachments.is_empty() {
        return None;
    }

    let mut text = content.to_string();
    text.push_str("\n\n<attachments>");

    let mut image_parts = Vec::new();

    for (i, att) in attachments.iter().enumerate() {
        text.push('\n');
        text.push_str(&format_attachment(i + 1, att));

        // Build multimodal image part when image data is available
        if att.kind == AttachmentKind::Image && !att.data.is_empty() {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&att.data);
            let data_url = format!("data:{};base64,{}", att.mime_type, b64);
            image_parts.push(ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: data_url,
                    detail: Some("auto".to_string()),
                },
            });
        }
    }

    text.push_str("\n</attachments>");
    Some(AugmentResult { text, image_parts })
}

/// Escape a string for use as an XML attribute value.
fn escape_xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape a string for use as XML text content.
fn escape_xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn format_attachment(index: usize, att: &IncomingAttachment) -> String {
    let filename = escape_xml_attr(att.filename.as_deref().unwrap_or("unknown"));
    let mime = escape_xml_attr(&att.mime_type);

    match &att.kind {
        AttachmentKind::Audio => {
            let duration_attr = att
                .duration_secs
                .map(|d| format!(" duration=\"{d}s\""))
                .unwrap_or_default();

            let body = match &att.extracted_text {
                Some(text) => format!("Transcript: {}", escape_xml_text(text)),
                None => "Audio transcript unavailable.".to_string(),
            };

            format!(
                "<attachment index=\"{index}\" type=\"audio\" filename=\"{filename}\"{duration_attr}>\n\
                 {body}\n\
                 </attachment>"
            )
        }
        AttachmentKind::Image => {
            let size_attr = att
                .size_bytes
                .map(|s| format!(" size=\"{}\"", format_size(s)))
                .unwrap_or_default();

            let body = if att.data.is_empty() {
                "[Image attached — visual content not available in this conversation.]"
            } else {
                "[Image attached — you can already see this image directly in the conversation. Do NOT use image_analyze or try to find this file on disk — it exists only in memory. Analyze it using your vision capabilities.]"
            };

            format!(
                "<attachment index=\"{index}\" type=\"image\" filename=\"{filename}\" mime=\"{mime}\"{size_attr}>\n\
                 {body}\n\
                 </attachment>"
            )
        }
        AttachmentKind::Document => {
            let body: String = match &att.extracted_text {
                Some(text) => escape_xml_text(text),
                None => {
                    let size_info = att
                        .size_bytes
                        .map(|s| format!(" size=\"{}\"", format_size(s)))
                        .unwrap_or_default();
                    return format!(
                        "<attachment index=\"{index}\" type=\"document\" filename=\"{filename}\" mime=\"{mime}\"{size_info}>\n\
                         [Document attached — text extraction unavailable]\n\
                         </attachment>"
                    );
                }
            };

            let size_attr = att
                .size_bytes
                .map(|s| format!(" size=\"{}\"", format_size(s)))
                .unwrap_or_default();

            format!(
                "<attachment index=\"{index}\" type=\"document\" filename=\"{filename}\" mime=\"{mime}\"{size_attr}>\n\
                 {body}\n\
                 </attachment>"
            )
        }
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_attachment(kind: AttachmentKind) -> IncomingAttachment {
        IncomingAttachment {
            id: "test-id".to_string(),
            kind,
            mime_type: "application/octet-stream".to_string(),
            filename: None,
            size_bytes: None,
            source_url: None,
            storage_key: None,
            extracted_text: None,
            data: vec![],
            duration_secs: None,
        }
    }

    #[test]
    fn empty_attachments_returns_none() {
        assert!(augment_with_attachments("hello", &[]).is_none());
    }

    #[test]
    fn audio_with_transcript() {
        let mut att = make_attachment(AttachmentKind::Audio);
        att.filename = Some("voice.ogg".to_string());
        att.extracted_text = Some("Hello, can you help me?".to_string());
        att.duration_secs = Some(5);

        let result = augment_with_attachments("hi", &[att]).unwrap();
        assert!(result.text.starts_with("hi\n\n<attachments>"));
        assert!(result.text.contains("type=\"audio\""));
        assert!(result.text.contains("filename=\"voice.ogg\""));
        assert!(result.text.contains("duration=\"5s\""));
        assert!(result.text.contains("Transcript: Hello, can you help me?"));
        assert!(result.text.ends_with("</attachments>"));
        assert!(result.image_parts.is_empty());
    }

    #[test]
    fn audio_without_transcript() {
        let mut att = make_attachment(AttachmentKind::Audio);
        att.filename = Some("voice.ogg".to_string());
        att.duration_secs = Some(10);

        let result = augment_with_attachments("hi", &[att]).unwrap();
        assert!(result.text.contains("Audio transcript unavailable."));
        assert!(result.text.contains("duration=\"10s\""));
    }

    #[test]
    fn image_without_data_no_visual() {
        let mut att = make_attachment(AttachmentKind::Image);
        att.filename = Some("screenshot.png".to_string());
        att.mime_type = "image/png".to_string();
        att.size_bytes = Some(245_000);

        let result = augment_with_attachments("check this", &[att]).unwrap();
        assert!(result.text.contains("type=\"image\""));
        assert!(result.text.contains("filename=\"screenshot.png\""));
        assert!(result.text.contains("mime=\"image/png\""));
        assert!(result.text.contains("size=\"239KB\""));
        assert!(
            result
                .text
                .contains("[Image attached — visual content not available in this conversation.]")
        );
        assert!(result.image_parts.is_empty());
    }

    #[test]
    fn image_with_data_produces_content_part() {
        let mut att = make_attachment(AttachmentKind::Image);
        att.filename = Some("photo.jpg".to_string());
        att.mime_type = "image/jpeg".to_string();
        att.data = vec![0xFF, 0xD8, 0xFF]; // fake JPEG header

        let result = augment_with_attachments("look", &[att]).unwrap();
        assert!(
            result
                .text
                .contains("you can already see this image directly")
        );
        assert_eq!(result.image_parts.len(), 1);
        match &result.image_parts[0] {
            ContentPart::ImageUrl { image_url } => {
                assert!(image_url.url.starts_with("data:image/jpeg;base64,"));
            }
            other => panic!("Expected ImageUrl, got: {:?}", other),
        }
    }

    #[test]
    fn image_url_includes_detail_auto() {
        let mut att = make_attachment(AttachmentKind::Image);
        att.mime_type = "image/png".to_string();
        att.data = vec![0x89, 0x50, 0x4E, 0x47]; // fake PNG header

        let result = augment_with_attachments("check", &[att]).unwrap();
        assert_eq!(result.image_parts.len(), 1);
        match &result.image_parts[0] {
            ContentPart::ImageUrl { image_url } => {
                assert_eq!(
                    image_url.detail.as_deref(),
                    Some("auto"),
                    "detail field must be set to 'auto' for provider compatibility"
                );
            }
            other => panic!("Expected ImageUrl, got: {:?}", other),
        }
    }

    #[test]
    fn document_with_extracted_text() {
        let mut att = make_attachment(AttachmentKind::Document);
        att.filename = Some("report.pdf".to_string());
        att.extracted_text = Some("Executive summary: Q3 results".to_string());

        let result = augment_with_attachments("review", &[att]).unwrap();
        assert!(result.text.contains("type=\"document\""));
        assert!(result.text.contains("filename=\"report.pdf\""));
        assert!(result.text.contains("Executive summary: Q3 results"));
    }

    #[test]
    fn document_without_extracted_text() {
        let mut att = make_attachment(AttachmentKind::Document);
        att.filename = Some("data.csv".to_string());
        att.mime_type = "text/csv".to_string();
        att.size_bytes = Some(1024);

        let result = augment_with_attachments("analyze", &[att]).unwrap();
        assert!(result.text.contains("type=\"document\""));
        assert!(result.text.contains("mime=\"text/csv\""));
        assert!(
            result
                .text
                .contains("[Document attached — text extraction unavailable]")
        );
    }

    #[test]
    fn multiple_attachments_with_mixed_images() {
        let mut audio = make_attachment(AttachmentKind::Audio);
        audio.filename = Some("voice.ogg".to_string());
        audio.extracted_text = Some("Hello".to_string());

        let mut image_with_data = make_attachment(AttachmentKind::Image);
        image_with_data.filename = Some("photo.jpg".to_string());
        image_with_data.mime_type = "image/jpeg".to_string();
        image_with_data.data = vec![0xFF, 0xD8];

        let mut image_no_data = make_attachment(AttachmentKind::Image);
        image_no_data.filename = Some("remote.png".to_string());
        image_no_data.mime_type = "image/png".to_string();

        let result =
            augment_with_attachments("msg", &[audio, image_with_data, image_no_data]).unwrap();
        assert!(result.text.contains("index=\"1\""));
        assert!(result.text.contains("index=\"2\""));
        assert!(result.text.contains("index=\"3\""));
        // Only the image with data produces a content part
        assert_eq!(result.image_parts.len(), 1);
    }

    #[test]
    fn original_content_preserved() {
        let original = "Please help me with this task";
        let mut att = make_attachment(AttachmentKind::Audio);
        att.extracted_text = Some("transcript".to_string());

        let result = augment_with_attachments(original, &[att]).unwrap();
        assert!(result.text.starts_with(original));
    }
}
