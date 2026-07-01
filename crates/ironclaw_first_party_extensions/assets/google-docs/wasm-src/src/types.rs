//! Types for Google Docs API requests and responses.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub(crate) struct ToolContext {
    pub(crate) capability_id: String,
}

/// Input parameters for the Google Docs tool.
///
/// `JsonSchema` is derived so the advertised tool schema mirrors the
/// serde-enforced contract: each variant becomes a `oneOf` entry with
/// its own `required` array.
#[derive(Debug, Deserialize, JsonSchema)]
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

    /// Read a bounded excerpt from the document body as plain text.
    ReadExcerpt {
        /// The document ID.
        document_id: String,
        /// Optional text to search for. When present, the excerpt starts near the first match.
        #[serde(default)]
        query: Option<String>,
        /// Character offset to start at when query is omitted or not found.
        #[serde(default)]
        start_char: usize,
        /// Maximum excerpt characters (default: 4000, max: 20000).
        #[serde(default = "default_excerpt_chars")]
        max_chars: usize,
        /// Include compact heading outline.
        #[serde(default = "default_true")]
        include_outline: bool,
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

fn default_excerpt_chars() -> usize {
    4_000
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

/// Result from read_excerpt.
#[derive(Debug, Serialize)]
pub struct ReadExcerptResult {
    pub document_id: String,
    pub title: String,
    pub excerpt: String,
    pub start_char: usize,
    pub end_char: usize,
    pub total_chars: usize,
    pub truncated_before: bool,
    pub truncated_after: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub outline: Vec<DocumentOutlineItem>,
}

/// Compact heading outline entry.
#[derive(Debug, Serialize)]
pub struct DocumentOutlineItem {
    pub title: String,
    pub style: DocumentOutlineStyle,
    pub char_offset: usize,
}

/// Supported Google Docs outline styles.
#[derive(Debug, Serialize)]
pub enum DocumentOutlineStyle {
    #[serde(rename = "TITLE")]
    Title,
    #[serde(rename = "SUBTITLE")]
    Subtitle,
    #[serde(rename = "HEADING_1")]
    Heading1,
    #[serde(rename = "HEADING_2")]
    Heading2,
    #[serde(rename = "HEADING_3")]
    Heading3,
    #[serde(rename = "HEADING_4")]
    Heading4,
    #[serde(rename = "HEADING_5")]
    Heading5,
    #[serde(rename = "HEADING_6")]
    Heading6,
}

impl DocumentOutlineStyle {
    pub(crate) fn from_named_style(value: &str) -> Option<Self> {
        match value {
            "TITLE" => Some(Self::Title),
            "SUBTITLE" => Some(Self::Subtitle),
            "HEADING_1" => Some(Self::Heading1),
            "HEADING_2" => Some(Self::Heading2),
            "HEADING_3" => Some(Self::Heading3),
            "HEADING_4" => Some(Self::Heading4),
            "HEADING_5" => Some(Self::Heading5),
            "HEADING_6" => Some(Self::Heading6),
            _ => None,
        }
    }
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
