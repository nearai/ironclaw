//! Shared attachment helpers for channel ingestion and persistence.

/// Maximum decoded size per inline attachment.
pub(crate) const MAX_INLINE_ATTACHMENT_BYTES: usize = 5 * 1024 * 1024;
/// Maximum total decoded size across all inline attachments in a message.
pub(crate) const MAX_INLINE_TOTAL_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
/// Maximum number of inline attachments in a single message.
pub(crate) const MAX_INLINE_ATTACHMENTS: usize = 5;

fn base_mime_type(mime: &str) -> &str {
    mime.split(';').next().unwrap_or(mime).trim()
}

pub(crate) fn attachment_extension_for_mime(mime: &str) -> &'static str {
    match base_mime_type(mime) {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/markdown" => "md",
        "text/csv" => "csv",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/ogg" => "ogg",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/vnd.ms-powerpoint" => "ppt",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        other if other.starts_with("image/") => "jpg",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn attachment_extension_handles_common_types_and_parameters() {
        assert_eq!(
            super::attachment_extension_for_mime("text/plain; charset=utf-8"),
            "txt"
        );
        assert_eq!(
            super::attachment_extension_for_mime(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            ),
            "docx"
        );
        assert_eq!(
            super::attachment_extension_for_mime(
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            ),
            "xlsx"
        );
        assert_eq!(super::attachment_extension_for_mime("audio/x-wav"), "wav");
    }
}
