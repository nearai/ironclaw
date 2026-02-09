//! Types for Google Docs API requests and responses.

use serde::{Deserialize, Serialize};

/// Input parameters for the Google Docs tool.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GoogleDocsAction {
    /// Create a new document.
    CreateDocument {
        /// Document title.
        title: String,
    },

    /// Get document metadata and structure (title, body text, named ranges).
    GetDocument {
        /// The document ID (same as Google Drive file ID).
        document_id: String,
    },

    /// Read the document body as plain text.
    ReadContent {
        /// The document ID.
        document_id: String,
    },

    /// Insert text at a position.
    InsertText {
        /// The document ID.
        document_id: String,
        /// Text to insert.
        text: String,
        /// Character index to insert at (1-based, since 0 is before the body).
        /// Use -1 to append at end.
        #[serde(default = "default_insert_index")]
        index: i64,
        /// Segment ID ("" for body, or a header/footer ID).
        #[serde(default)]
        segment_id: String,
    },

    /// Delete content in a range.
    DeleteContent {
        /// The document ID.
        document_id: String,
        /// Start index (inclusive).
        start_index: i64,
        /// End index (exclusive).
        end_index: i64,
        /// Segment ID ("" for body).
        #[serde(default)]
        segment_id: String,
    },

    /// Find and replace all occurrences of text.
    ReplaceText {
        /// The document ID.
        document_id: String,
        /// Text to search for.
        find: String,
        /// Replacement text.
        replace: String,
        /// Case-sensitive match (default: true).
        #[serde(default = "default_true")]
        match_case: bool,
    },

    /// Format text in a range (bold, italic, font size, color, etc.).
    FormatText {
        /// The document ID.
        document_id: String,
        /// Start index (inclusive).
        start_index: i64,
        /// End index (exclusive).
        end_index: i64,
        /// Make text bold.
        #[serde(default)]
        bold: Option<bool>,
        /// Make text italic.
        #[serde(default)]
        italic: Option<bool>,
        /// Underline text.
        #[serde(default)]
        underline: Option<bool>,
        /// Strikethrough text.
        #[serde(default)]
        strikethrough: Option<bool>,
        /// Font size in points.
        #[serde(default)]
        font_size: Option<f64>,
        /// Font family name (e.g., "Arial", "Times New Roman").
        #[serde(default)]
        font_family: Option<String>,
        /// Text color as hex (e.g., "#FF0000").
        #[serde(default)]
        foreground_color: Option<String>,
        /// Text background color as hex.
        #[serde(default)]
        background_color: Option<String>,
    },

    /// Set paragraph style (heading level, alignment, spacing).
    FormatParagraph {
        /// The document ID.
        document_id: String,
        /// Start index (inclusive).
        start_index: i64,
        /// End index (exclusive).
        end_index: i64,
        /// Named style: "NORMAL_TEXT", "TITLE", "SUBTITLE", "HEADING_1" through "HEADING_6".
        #[serde(default)]
        named_style: Option<String>,
        /// Alignment: "START", "CENTER", "END", "JUSTIFIED".
        #[serde(default)]
        alignment: Option<String>,
        /// Line spacing as percentage (e.g., 115 for 1.15x).
        #[serde(default)]
        line_spacing: Option<f64>,
    },

    /// Insert a table at a position.
    InsertTable {
        /// The document ID.
        document_id: String,
        /// Number of rows.
        rows: i64,
        /// Number of columns.
        columns: i64,
        /// Character index to insert at.
        index: i64,
    },

    /// Create a bulleted or numbered list from a range of paragraphs.
    CreateList {
        /// The document ID.
        document_id: String,
        /// Start index (inclusive).
        start_index: i64,
        /// End index (exclusive).
        end_index: i64,
        /// Bullet preset. Bulleted: "BULLET_DISC_CIRCLE_SQUARE" (default).
        /// Numbered: "NUMBERED_DECIMAL_ALPHA_ROMAN".
        #[serde(default = "default_bullet_preset")]
        bullet_preset: String,
    },

    /// Execute multiple operations in a single atomic batch.
    /// Each operation is an object with one key (the request type name)
    /// and a value matching the Docs API batchUpdate request format.
    BatchUpdate {
        /// The document ID.
        document_id: String,
        /// Array of raw request objects as per Google Docs API.
        requests: Vec<serde_json::Value>,
    },
}

fn default_insert_index() -> i64 {
    -1
}

fn default_true() -> bool {
    true
}

fn default_bullet_preset() -> String {
    "BULLET_DISC_CIRCLE_SQUARE".to_string()
}

/// Result from create_document.
#[derive(Debug, Serialize)]
pub struct CreateDocumentResult {
    pub document_id: String,
    pub title: String,
}

/// Result from get_document.
#[derive(Debug, Serialize)]
pub struct DocumentMetadata {
    pub document_id: String,
    pub title: String,
    pub revision_id: String,
    pub body_length: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub named_ranges: Vec<DocumentNamedRange>,
}

/// Named range within a document.
#[derive(Debug, Serialize)]
pub struct DocumentNamedRange {
    pub name: String,
    pub named_range_id: String,
    pub start_index: i64,
    pub end_index: i64,
}

/// Result from read_content.
#[derive(Debug, Serialize)]
pub struct ReadContentResult {
    pub document_id: String,
    pub title: String,
    pub content: String,
}

/// Result from insert_text, delete_content, replace_text.
#[derive(Debug, Serialize)]
pub struct UpdateResult {
    pub document_id: String,
    pub revision_id: String,
}

/// Result from replace_text with occurrence count.
#[derive(Debug, Serialize)]
pub struct ReplaceResult {
    pub document_id: String,
    pub revision_id: String,
    pub occurrences_changed: i64,
}

/// Result from batch_update.
#[derive(Debug, Serialize)]
pub struct BatchUpdateResult {
    pub document_id: String,
    pub revision_id: String,
    pub replies: Vec<serde_json::Value>,
}
