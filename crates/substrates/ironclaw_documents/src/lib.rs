//! Structure-preserving document editing.
//!
//! Companion to `ironclaw_extractors`, which turns a document's bytes into flat
//! text for reading. This crate exists for the case extraction cannot serve:
//! *changing* a document and handing it back intact.
//!
//! The governing rule, and the reason this is not "add a docx writer":
//!
//! > Rewrite only the parts an edit actually targets. Copy everything else
//! > through byte-for-byte.
//!
//! A generator that rebuilds a document from the text a model saw drops
//! everything the model never saw — styles, numbering, headers, images,
//! embedded objects, comments. The file still opens, so the loss is invisible
//! until someone notices their formatting is gone. Copy-through makes that
//! class of loss impossible by construction rather than by diligence.
//!
//! PDF is the deliberate exception. There is no reliable structured edit for
//! an arbitrary PDF, so this crate does not pretend to offer one: it renders
//! *new* PDFs from an HTML subset ([`html_to_pdf`]) and leaves existing PDFs
//! alone.

mod error;
mod ooxml;

pub mod docx;
pub mod html_pdf;
pub mod pptx;
pub mod xlsx;

#[cfg(test)]
mod test_fixtures;

pub use error::DocumentError;
pub use html_pdf::{PdfOptions, html_to_pdf};

/// The document formats this crate can edit structurally.
///
/// Deliberately not a general "is this binary" test — a format belongs here
/// only once there is an editor that preserves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentFormat {
    Docx,
    Xlsx,
    Pptx,
}

impl DocumentFormat {
    /// Recognize a format from a path's extension, case-insensitively.
    ///
    /// Returns `None` for `.pdf` and legacy binary Office formats (`.doc`,
    /// `.xls`, `.ppt`) — all of which remain read-only, and none of which may
    /// be written through the text tools either (see the `write_file` guard in
    /// `ironclaw_extension_support::coding`).
    pub fn from_path(path: &str) -> Option<Self> {
        let extension = path.rsplit('.').next()?.to_ascii_lowercase();
        match extension.as_str() {
            "docx" => Some(Self::Docx),
            "xlsx" => Some(Self::Xlsx),
            "pptx" => Some(Self::Pptx),
            _ => None,
        }
    }

    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_recognition_is_case_insensitive_and_excludes_unsupported_formats() {
        assert_eq!(
            DocumentFormat::from_path("/w/Report.DOCX"),
            Some(DocumentFormat::Docx)
        );
        assert_eq!(
            DocumentFormat::from_path("/w/book.xlsx"),
            Some(DocumentFormat::Xlsx)
        );
        // PDF and the legacy binary formats have no structured editor, so they
        // must not resolve here — a `Some` would route them into an editor
        // that cannot preserve them.
        assert_eq!(DocumentFormat::from_path("/w/paper.pdf"), None);
        assert_eq!(DocumentFormat::from_path("/w/old.doc"), None);
        assert_eq!(DocumentFormat::from_path("/w/notes.txt"), None);
        assert_eq!(DocumentFormat::from_path("/w/noextension"), None);
    }
}
