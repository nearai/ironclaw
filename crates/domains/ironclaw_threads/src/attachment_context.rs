//! Render attachment references into model-visible message content.
//!
//! When a transcript message carries [`AttachmentRef`]s, the model-visible
//! projection of that message ([`crate::ContextMessage`]) appends a rendered
//! `<attachments>` block so the model can reason about the files: documents and
//! audio contribute their extracted text / transcript, and every attachment
//! contributes its stored project path (`storage_key`) so the agent can
//! `file_read` it. Image pixels reach a vision-capable model through the
//! multimodal path (the model port reads the bytes back and the gateway sends
//! them as `ContentPart::ImageUrl`; see `ContextMessage::image_attachments`);
//! this textual block contributes a pointer to the stored file, which is the
//! fallback a text-only model relies on. Video and other binary media are
//! represented as typed stored-file pointers without pretending their bytes
//! were extracted into text.

use ironclaw_common::{AttachmentKind, AttachmentRef};

use crate::contract::ContextImageAttachment;

/// Append a rendered `<attachments>` block to `content` when `attachments` is
/// non-empty; otherwise return `content` unchanged.
pub(crate) fn augment_model_content(content: String, attachments: &[AttachmentRef]) -> String {
    match render_attachments_block(attachments) {
        Some(block) => format!("{content}\n\n{block}"),
        None => content,
    }
}

/// Remove a host-owned model-context attachment suffix for the same
/// authoritative attachment refs.
///
/// Models can see [`augment_model_content`] output in prior transcript
/// messages and may copy that internal context serialization into a final
/// answer. Final assistant-message projection uses this inverse operation
/// after sealing the run's typed attachment intents.
///
/// This is deliberately a narrow grammar check, not general output
/// normalization: the suffix must have the host-owned `<attachments>` shape,
/// contain one entry per authoritative ref in the same order, and match each
/// ref's escaped filename, MIME type, storage key, and formatted size exactly.
/// The presentational `type` and body may differ because a model can copy an
/// older context block and adapt it while reattaching the same file. Markdown,
/// standalone paths, malformed envelopes, and envelopes for any other file are
/// left untouched.
pub fn deproject_model_attachment_context(
    mut content: String,
    attachments: &[AttachmentRef],
) -> String {
    if attachments.is_empty() {
        return content;
    }

    let Some(block_start) = model_attachment_block_suffix_start(&content) else {
        return content;
    };
    if !model_attachment_block_matches(&content[block_start..], attachments) {
        return content;
    }

    let prose_end = if block_start == 0 {
        0
    } else {
        let Some(separator_start) = block_start.checked_sub(2) else {
            return content;
        };
        if content.get(separator_start..block_start) != Some("\n\n") {
            return content;
        }
        separator_start
    };
    content.truncate(prose_end);
    content
}

fn model_attachment_block_suffix_start(content: &str) -> Option<usize> {
    const BLOCK_START: &str = "<attachments>";
    const BLOCK_END: &str = "</attachments>";

    if !content.ends_with(BLOCK_END) {
        return None;
    }
    if content.starts_with(BLOCK_START) {
        return Some(0);
    }
    content
        .rfind("\n\n<attachments>")
        .map(|separator_start| separator_start + 2)
}

fn model_attachment_block_matches(block: &str, attachments: &[AttachmentRef]) -> bool {
    let mut lines = block.lines();
    if lines.next() != Some("<attachments>") {
        return false;
    }

    for (index, attachment) in attachments.iter().enumerate() {
        let Some(header) = lines.next() else {
            return false;
        };
        if !attachment_header_matches(header, index + 1, attachment) {
            return false;
        }

        let mut saw_body = false;
        loop {
            let Some(line) = lines.next() else {
                return false;
            };
            if line == "</attachment>" {
                break;
            }
            if line.starts_with("<attachment ") || line == "</attachments>" {
                return false;
            }
            saw_body = true;
        }
        if !saw_body {
            return false;
        }
    }

    lines.next() == Some("</attachments>") && lines.next().is_none()
}

fn attachment_header_matches(header: &str, index: usize, attachment: &AttachmentRef) -> bool {
    [
        AttachmentKind::Audio,
        AttachmentKind::Image,
        AttachmentKind::Video,
        AttachmentKind::Document,
        AttachmentKind::Other,
    ]
    .into_iter()
    .any(|kind| render_attachment_header(index, attachment, kind) == header)
}

/// The image attachments a vision-capable model could view as multimodal parts:
/// `kind == Image` with a landed `storage_key`. Only the reference is carried —
/// the bytes are read later (and only for a vision model), so a text-only model
/// pays nothing here. The textual pointer from [`augment_model_content`] stays
/// as the fallback either way.
pub(crate) fn model_image_attachments(
    attachments: &[AttachmentRef],
) -> Vec<ContextImageAttachment> {
    attachments
        .iter()
        .filter(|attachment| attachment.kind == AttachmentKind::Image)
        .filter_map(|attachment| {
            attachment
                .storage_key
                .as_ref()
                .map(|storage_key| ContextImageAttachment {
                    mime_type: attachment.mime_type.clone(),
                    storage_key: storage_key.clone(),
                })
        })
        .collect()
}

fn render_attachments_block(attachments: &[AttachmentRef]) -> Option<String> {
    if attachments.is_empty() {
        return None;
    }
    let mut out = String::from("<attachments>");
    for (index, attachment) in attachments.iter().enumerate() {
        out.push('\n');
        out.push_str(&render_attachment(index + 1, attachment));
    }
    out.push_str("\n</attachments>");
    Some(out)
}

fn render_attachment(index: usize, attachment: &AttachmentRef) -> String {
    let header = render_attachment_header(index, attachment, attachment.kind);
    let body = body_text(attachment, attachment.storage_key.is_some());
    let body = match attachment.storage_key.as_deref() {
        Some(path) => format!("Saved to project file: {}\n{}", escape_xml_text(path), body),
        None => body,
    };

    format!("{header}\n{body}\n</attachment>")
}

fn render_attachment_header(
    index: usize,
    attachment: &AttachmentRef,
    kind: AttachmentKind,
) -> String {
    let filename = escape_xml_attr(attachment.filename.as_deref().unwrap_or("unknown"));
    let mime = escape_xml_attr(&attachment.mime_type);
    let type_label = match kind {
        AttachmentKind::Audio => "audio",
        AttachmentKind::Image => "image",
        AttachmentKind::Video => "video",
        AttachmentKind::Document => "document",
        AttachmentKind::Other => "other",
    };
    let project_path_attr = attachment
        .storage_key
        .as_deref()
        .map(|path| format!(" project_path=\"{}\"", escape_xml_attr(path)))
        .unwrap_or_default();
    let size_attr = attachment
        .size_bytes
        .map(|size| format!(" size=\"{}\"", format_size(size)))
        .unwrap_or_default();

    format!(
        "<attachment index=\"{index}\" type=\"{type_label}\" filename=\"{filename}\" mime=\"{mime}\"{project_path_attr}{size_attr}>"
    )
}

fn body_text(attachment: &AttachmentRef, has_project_path: bool) -> String {
    match attachment.kind {
        AttachmentKind::Audio => match &attachment.extracted_text {
            Some(text) => format!("Transcript: {}", model_safe_extracted_text(text)),
            None => "Audio transcript unavailable.".to_string(),
        },
        AttachmentKind::Document => match &attachment.extracted_text {
            Some(text) => model_safe_extracted_text(text),
            None => "[Document attached — text extraction unavailable]".to_string(),
        },
        // An image's pixels reach the model through the multimodal path; here it
        // only contributes a pointer to its stored file, so the body is useful
        // only when the file was actually landed.
        AttachmentKind::Image if has_project_path => {
            "[Image attached — the file is saved at the project path above; read it from there to view it.]"
                .to_string()
        }
        AttachmentKind::Image => "[Image attached — not yet stored.]".to_string(),
        AttachmentKind::Video if has_project_path => {
            "[Video attached — the file is saved at the project path above.]".to_string()
        }
        AttachmentKind::Video => "[Video attached — not yet stored.]".to_string(),
        AttachmentKind::Other if has_project_path => {
            "[Binary attachment — the file is saved at the project path above.]".to_string()
        }
        AttachmentKind::Other => "[Binary attachment — not yet stored.]".to_string(),
    }
}

fn model_safe_extracted_text(value: &str) -> String {
    escape_xml_text(ironclaw_safety::redact_model_input_text(value).text())
}

fn escape_xml_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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

    fn doc_ref(extracted: Option<&str>) -> AttachmentRef {
        AttachmentRef {
            id: "att-0".to_string(),
            kind: AttachmentKind::Document,
            mime_type: "application/pdf".to_string(),
            filename: Some("report.pdf".to_string()),
            size_bytes: Some(2048),
            storage_key: Some("/workspace/attachments/2026-06-09/m1-0-report.pdf".to_string()),
            extracted_text: extracted.map(str::to_string),
        }
    }

    #[test]
    fn empty_attachments_leave_content_unchanged() {
        assert_eq!(augment_model_content("hello".to_string(), &[]), "hello");
    }

    #[test]
    fn exact_model_context_suffix_is_deprojected_from_final_text() {
        let attachments = [doc_ref(None)];
        let projected = augment_model_content("plain reply".to_string(), &attachments);

        assert_eq!(
            deproject_model_attachment_context(projected, &attachments),
            "plain reply"
        );
    }

    #[test]
    fn deprojection_accepts_presentational_kind_drift_for_the_same_typed_ref() {
        let image = AttachmentRef {
            id: "att-image".to_string(),
            kind: AttachmentKind::Image,
            mime_type: "image/jpeg".to_string(),
            filename: Some("photo.jpg".to_string()),
            size_bytes: Some(3714),
            storage_key: Some("/workspace/attachments/photo.jpg".to_string()),
            extracted_text: None,
        };
        let mut copied_as_document = image.clone();
        copied_as_document.kind = AttachmentKind::Document;
        let projected =
            augment_model_content("Reattached the image.".to_string(), &[copied_as_document]);

        assert_eq!(
            deproject_model_attachment_context(projected, &[image]),
            "Reattached the image."
        );
    }

    #[test]
    fn deprojection_does_not_rewrite_noncanonical_or_different_attachment_text() {
        let attachments = [doc_ref(None)];
        let similar = "plain reply\n\n<attachments>\nnot the canonical envelope\n</attachments>";
        let markdown_path = "Created [the report](/workspace/report.pdf).";
        let different_path = augment_model_content(
            "plain reply".to_string(),
            &[AttachmentRef {
                storage_key: Some("/workspace/different.pdf".to_string()),
                ..doc_ref(None)
            }],
        );

        assert_eq!(
            deproject_model_attachment_context(similar.to_string(), &attachments),
            similar
        );
        assert_eq!(
            deproject_model_attachment_context(markdown_path.to_string(), &attachments),
            markdown_path
        );
        assert_eq!(
            deproject_model_attachment_context(different_path.clone(), &attachments),
            different_path
        );
    }

    #[test]
    fn attachment_only_context_deprojects_to_empty_prose() {
        let attachments = [doc_ref(None)];
        let block = render_attachments_block(&attachments).expect("attachment context");

        assert_eq!(deproject_model_attachment_context(block, &attachments), "");
    }

    #[test]
    fn document_extracted_text_is_folded_into_content() {
        let out = augment_model_content(
            "see attached".to_string(),
            &[doc_ref(Some("Quarterly revenue up 12%"))],
        );
        assert!(out.starts_with("see attached\n\n<attachments>"));
        assert!(out.contains("type=\"document\""));
        assert!(out.contains("filename=\"report.pdf\""));
        assert!(out.contains("project_path=\"/workspace/attachments/2026-06-09/m1-0-report.pdf\""));
        assert!(out.contains("size=\"2KB\""));
        assert!(
            out.contains(
                "Saved to project file: /workspace/attachments/2026-06-09/m1-0-report.pdf"
            )
        );
        assert!(out.contains("Quarterly revenue up 12%"));
        assert!(out.ends_with("</attachments>"));
    }

    #[test]
    fn document_extracted_text_redacts_credentials_but_keeps_benign_context() {
        let secret = "attachment canary,with;delimiters";
        let out = augment_model_content(
            "see attached".to_string(),
            &[doc_ref(Some(&format!(
                "Marker ZAFFRE. password: \"{secret}\"; secretary: Treasury contact."
            )))],
        );

        assert!(!out.contains(secret));
        assert!(out.contains("[REDACTED_SECRET]"));
        assert!(out.contains("Marker ZAFFRE"));
        assert!(out.contains("secretary: Treasury contact"));
    }

    #[test]
    fn document_without_text_notes_unavailable() {
        let out = augment_model_content("x".to_string(), &[doc_ref(None)]);
        assert!(out.contains("[Document attached — text extraction unavailable]"));
    }

    #[test]
    fn image_points_at_stored_file() {
        let image = AttachmentRef {
            id: "att-1".to_string(),
            kind: AttachmentKind::Image,
            mime_type: "image/png".to_string(),
            filename: Some("diagram.png".to_string()),
            size_bytes: Some(4096),
            storage_key: Some("/workspace/attachments/2026-06-09/m1-1-diagram.png".to_string()),
            extracted_text: None,
        };
        let out = augment_model_content("look".to_string(), &[image]);
        assert!(out.contains("type=\"image\""));
        assert!(out.contains("the file is saved at the project path above"));
    }

    #[test]
    fn image_without_storage_key_does_not_claim_a_project_path() {
        let image = AttachmentRef {
            id: "att-1".to_string(),
            kind: AttachmentKind::Image,
            mime_type: "image/png".to_string(),
            filename: Some("diagram.png".to_string()),
            size_bytes: Some(4096),
            storage_key: None,
            extracted_text: None,
        };
        let out = augment_model_content("look".to_string(), &[image]);
        assert!(out.contains("type=\"image\""));
        // No `project_path` attr and no "saved at the project path above" claim
        // when the image was never landed.
        assert!(!out.contains("project_path="));
        assert!(!out.contains("saved at the project path above"));
        assert!(out.contains("not yet stored"));
    }

    #[test]
    fn audio_transcript_is_rendered() {
        let audio = AttachmentRef {
            id: "att-2".to_string(),
            kind: AttachmentKind::Audio,
            mime_type: "audio/ogg".to_string(),
            filename: Some("voice.ogg".to_string()),
            size_bytes: Some(1024),
            storage_key: Some("/workspace/attachments/2026-06-09/m1-2-voice.ogg".to_string()),
            extracted_text: Some("hello can you help".to_string()),
        };
        let out = augment_model_content("".to_string(), &[audio]);
        assert!(out.contains("type=\"audio\""));
        assert!(out.contains("Transcript: hello can you help"));
    }

    #[test]
    fn special_characters_are_escaped() {
        let mut att = doc_ref(Some("a < b & c > d"));
        att.filename = Some("a\"&<>.txt".to_string());
        let out = augment_model_content("x".to_string(), &[att]);
        assert!(out.contains("filename=\"a&quot;&amp;&lt;&gt;.txt\""));
        assert!(out.contains("a &lt; b &amp; c &gt; d"));
    }

    fn image_ref(id: &str, storage_key: Option<&str>) -> AttachmentRef {
        AttachmentRef {
            id: id.to_string(),
            kind: AttachmentKind::Image,
            mime_type: "image/png".to_string(),
            filename: Some("diagram.png".to_string()),
            size_bytes: Some(4),
            storage_key: storage_key.map(str::to_string),
            extracted_text: None,
        }
    }

    #[test]
    fn model_image_attachments_keeps_only_landed_images() {
        let attachments = vec![
            image_ref(
                "img-landed",
                Some("/workspace/attachments/2026-06-14/m1-0.png"),
            ),
            // An image that never landed (no storage_key) has no bytes to read.
            image_ref("img-unlanded", None),
            // A non-image landed attachment is not part of the multimodal path.
            doc_ref(Some("text")),
        ];

        let images = model_image_attachments(&attachments);

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].mime_type, "image/png");
        assert_eq!(
            images[0].storage_key,
            "/workspace/attachments/2026-06-14/m1-0.png"
        );
    }

    #[test]
    fn model_image_attachments_empty_when_no_images() {
        assert!(model_image_attachments(&[doc_ref(None)]).is_empty());
        assert!(model_image_attachments(&[]).is_empty());
    }
}
