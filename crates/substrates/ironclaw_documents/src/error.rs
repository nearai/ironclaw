//! Error type for structured document reads and edits.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DocumentError {
    #[error("not a readable OOXML package: {0}")]
    NotAnOoxmlPackage(String),

    #[error("package is too large to edit safely: {detail}")]
    PackageTooLarge { detail: String },

    #[error("document is missing the required part {part}")]
    MissingPart { part: String },

    #[error("part {part} is malformed: {detail}")]
    MalformedPart { part: String, detail: String },

    /// The edit named an address (`p7`, `Sheet1!B4`, slide index) that the
    /// document does not contain. Distinct from `MalformedPart` because it is
    /// a caller error the model can correct by re-reading, not a broken file.
    #[error("{address} does not exist in this document")]
    UnknownAddress { address: String },

    /// The edit is well-formed but cannot apply to the addressed content — for
    /// example accepting a revision on a paragraph that carries none.
    #[error("edit does not apply at {address}: {detail}")]
    InapplicableEdit { address: String, detail: String },

    #[error("failed to write the edited document: {0}")]
    Write(String),

    #[error("unsupported document format: {0}")]
    UnsupportedFormat(String),

    #[error("HTML could not be rendered: {0}")]
    Html(String),
}
