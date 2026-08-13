//! Attachment vocabulary shared by product adapters, workflow, and landing.
//!
//! These are the byte-carrying attachment DTOs referenced by product-adapter
//! contracts (e.g. [`crate::product_adapter::ChannelAdapter`]); the landing
//! and budget logic that consumes them lives in `ironclaw_attachments`.

use std::fmt;

use crate::path::ScopedPath;

/// One inbound attachment with its raw bytes, ready to be landed and turned
/// into a transcript attachment reference.
///
/// The attachment kind and the fallback filename extension are *derived from*
/// `mime_type` against the attachment format registry by the landing routine —
/// the authoritative source — so callers cannot drift them out of sync with
/// the MIME type they pass.
#[derive(Clone, PartialEq, Eq)]
pub struct InboundAttachment {
    /// Stable identifier for this attachment within its message.
    pub id: String,
    /// MIME type as received at the ingress boundary. The attachment kind and
    /// fallback extension are derived from this.
    pub mime_type: String,
    /// Original filename, when the source provided one.
    pub filename: Option<String>,
    /// Raw attachment bytes to land in the project filesystem.
    pub bytes: Vec<u8>,
}

/// Raw bytes never reach a log line: every byte-carrying attachment type here
/// renders its length instead. `InboundAttachment` holds user documents
/// fetched from a vendor, so a derived `Debug` would dump whole files into any
/// diagnostic that formats a message.
impl fmt::Debug for InboundAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InboundAttachment")
            .field("id", &self.id)
            .field("mime_type", &self.mime_type)
            .field("filename", &self.filename)
            .field("size_bytes", &self.bytes.len())
            .finish()
    }
}

/// A trusted, in-memory file from a thread's scoped project workspace, whose
/// path has already been validated by its owning filesystem boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct WorkspaceFile {
    pub path: ScopedPath,
    pub filename: Option<String>,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for WorkspaceFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkspaceFile")
            .field("path", &self.path)
            .field("filename", &self.filename)
            .field("mime_type", &self.mime_type)
            .field("size_bytes", &self.bytes.len())
            .finish()
    }
}

impl WorkspaceFile {
    pub fn size_bytes(&self) -> u64 {
        self.bytes.len() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_bytes_are_not_rendered(rendered: &str, sentinel: &str) {
        assert!(rendered.contains("size_bytes"));
        assert!(
            !rendered.contains(sentinel),
            "raw bytes leaked into Debug: {rendered}"
        );
        // The byte-array rendering of the sentinel's first four characters.
        assert!(!rendered.contains("98, 121, 116, 101"));
    }

    #[test]
    fn debug_redacts_workspace_file_bytes() {
        let file = WorkspaceFile {
            path: ScopedPath::new("/workspace/report.txt").expect("scoped path"),
            filename: Some("report.txt".to_string()),
            mime_type: "text/plain".to_string(),
            bytes: b"byte-sentinel-must-not-leak".to_vec(),
        };

        let rendered = format!("{file:?}");
        assert!(rendered.contains("/workspace/report.txt"));
        assert_bytes_are_not_rendered(&rendered, "byte-sentinel-must-not-leak");
    }

    /// `InboundAttachment` carries user documents fetched from a vendor, so a
    /// derived `Debug` would dump whole files into any diagnostic that formats
    /// a message. Its byte-carrying sibling has always redacted; this pins the
    /// same contract on the type that holds the riskier content.
    #[test]
    fn debug_redacts_inbound_attachment_bytes() {
        let attachment = InboundAttachment {
            id: "file-1".to_string(),
            mime_type: "application/pdf".to_string(),
            filename: Some("report.pdf".to_string()),
            bytes: b"byte-sentinel-must-not-leak".to_vec(),
        };

        let rendered = format!("{attachment:?}");
        assert!(rendered.contains("file-1"));
        assert_bytes_are_not_rendered(&rendered, "byte-sentinel-must-not-leak");
    }
}
