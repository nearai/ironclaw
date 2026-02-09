//! Types for Google Slides API requests and responses.

use serde::{Deserialize, Serialize};

/// Input parameters for the Google Slides tool.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GoogleSlidesAction {
    /// Create a new presentation.
    CreatePresentation {
        /// Presentation title.
        title: String,
    },

    /// Get presentation metadata (slides, elements, text content).
    GetPresentation {
        /// The presentation ID (same as Google Drive file ID).
        presentation_id: String,
    },

    /// Get a thumbnail image URL for a specific slide.
    GetThumbnail {
        /// The presentation ID.
        presentation_id: String,
        /// The slide's object ID.
        slide_object_id: String,
    },

    /// Create a new slide.
    CreateSlide {
        /// The presentation ID.
        presentation_id: String,
        /// Position to insert (0-based). Omit to append at end.
        #[serde(default)]
        insertion_index: Option<i64>,
        /// Predefined layout: "BLANK", "TITLE", "TITLE_AND_BODY",
        /// "TITLE_AND_TWO_COLUMNS", "TITLE_ONLY", "SECTION_HEADER",
        /// "CAPTION_ONLY", "BIG_NUMBER", "ONE_COLUMN_TEXT", "MAIN_POINT".
        #[serde(default = "default_layout")]
        layout: String,
    },

    /// Delete a slide or page element.
    DeleteObject {
        /// The presentation ID.
        presentation_id: String,
        /// Object ID of the slide or element to delete.
        object_id: String,
    },

    /// Insert text into a shape or text box.
    InsertText {
        /// The presentation ID.
        presentation_id: String,
        /// Object ID of the shape/text box.
        object_id: String,
        /// Text to insert.
        text: String,
        /// Character index to insert at (0-based). Default: 0.
        #[serde(default)]
        insertion_index: i64,
    },

    /// Delete text from a shape.
    DeleteText {
        /// The presentation ID.
        presentation_id: String,
        /// Object ID of the shape.
        object_id: String,
        /// Start index (inclusive). Use 0 for start.
        #[serde(default)]
        start_index: i64,
        /// End index (exclusive). Omit to delete to end.
        #[serde(default)]
        end_index: Option<i64>,
    },

    /// Find and replace text across the entire presentation.
    ReplaceAllText {
        /// The presentation ID.
        presentation_id: String,
        /// Text to find.
        find: String,
        /// Replacement text.
        replace: String,
        /// Case-sensitive match (default: true).
        #[serde(default = "default_true")]
        match_case: bool,
    },

    /// Create a text box or shape on a slide.
    CreateShape {
        /// The presentation ID.
        presentation_id: String,
        /// Slide object ID to place the shape on.
        slide_object_id: String,
        /// Shape type: "TEXT_BOX", "RECTANGLE", "ROUND_RECTANGLE", "ELLIPSE".
        #[serde(default = "default_shape_type")]
        shape_type: String,
        /// X position in points from left edge.
        x: f64,
        /// Y position in points from top edge.
        y: f64,
        /// Width in points.
        width: f64,
        /// Height in points.
        height: f64,
    },

    /// Insert an image on a slide.
    InsertImage {
        /// The presentation ID.
        presentation_id: String,
        /// Slide object ID to place the image on.
        slide_object_id: String,
        /// Publicly accessible image URL.
        image_url: String,
        /// X position in points.
        x: f64,
        /// Y position in points.
        y: f64,
        /// Width in points.
        width: f64,
        /// Height in points.
        height: f64,
    },

    /// Format text in a shape (bold, italic, font, color, size).
    FormatText {
        /// The presentation ID.
        presentation_id: String,
        /// Object ID of the shape.
        object_id: String,
        /// Start index (inclusive). Use 0 for start.
        #[serde(default)]
        start_index: Option<i64>,
        /// End index (exclusive). Omit to format all text.
        #[serde(default)]
        end_index: Option<i64>,
        /// Make text bold.
        #[serde(default)]
        bold: Option<bool>,
        /// Make text italic.
        #[serde(default)]
        italic: Option<bool>,
        /// Underline text.
        #[serde(default)]
        underline: Option<bool>,
        /// Font size in points.
        #[serde(default)]
        font_size: Option<f64>,
        /// Font family name (e.g., "Arial").
        #[serde(default)]
        font_family: Option<String>,
        /// Text color as hex (e.g., "#FF0000").
        #[serde(default)]
        foreground_color: Option<String>,
    },

    /// Set paragraph alignment for text in a shape.
    FormatParagraph {
        /// The presentation ID.
        presentation_id: String,
        /// Object ID of the shape.
        object_id: String,
        /// Alignment: "START", "CENTER", "END", "JUSTIFIED".
        alignment: String,
        /// Start index (inclusive).
        #[serde(default)]
        start_index: Option<i64>,
        /// End index (exclusive). Omit to format all.
        #[serde(default)]
        end_index: Option<i64>,
    },

    /// Replace all shapes containing specific text with an image.
    ReplaceShapesWithImage {
        /// The presentation ID.
        presentation_id: String,
        /// Text to match in shapes.
        find: String,
        /// Image URL to replace shapes with.
        image_url: String,
        /// Case-sensitive match (default: true).
        #[serde(default = "default_true")]
        match_case: bool,
    },

    /// Execute multiple raw Slides API operations atomically.
    BatchUpdate {
        /// The presentation ID.
        presentation_id: String,
        /// Array of raw request objects as per Google Slides API.
        requests: Vec<serde_json::Value>,
    },
}

fn default_layout() -> String {
    "BLANK".to_string()
}

fn default_true() -> bool {
    true
}

fn default_shape_type() -> String {
    "TEXT_BOX".to_string()
}

/// Slide info.
#[derive(Debug, Serialize)]
pub struct SlideInfo {
    pub object_id: String,
    pub layout_object_id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub elements: Vec<ElementInfo>,
}

/// Page element info.
#[derive(Debug, Serialize)]
pub struct ElementInfo {
    pub object_id: String,
    pub element_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder_type: Option<String>,
}

/// Result from create_presentation.
#[derive(Debug, Serialize)]
pub struct CreatePresentationResult {
    pub presentation_id: String,
    pub title: String,
}

/// Result from get_presentation.
#[derive(Debug, Serialize)]
pub struct PresentationMetadata {
    pub presentation_id: String,
    pub title: String,
    pub revision_id: String,
    pub slide_count: usize,
    pub slides: Vec<SlideInfo>,
}

/// Result from get_thumbnail.
#[derive(Debug, Serialize)]
pub struct ThumbnailResult {
    pub content_url: String,
    pub width: i64,
    pub height: i64,
}

/// Result from a batchUpdate operation.
#[derive(Debug, Serialize)]
pub struct UpdateResult {
    pub presentation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_object_id: Option<String>,
}

/// Result from replace_all_text.
#[derive(Debug, Serialize)]
pub struct ReplaceResult {
    pub presentation_id: String,
    pub occurrences_changed: i64,
}

/// Result from batch_update.
#[derive(Debug, Serialize)]
pub struct BatchUpdateResult {
    pub presentation_id: String,
    pub replies: Vec<serde_json::Value>,
}
