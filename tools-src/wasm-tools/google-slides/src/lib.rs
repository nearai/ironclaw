//! Google Slides WASM Tool for IronClaw.
//!
//! Provides Google Slides integration for creating, reading, editing,
//! and formatting presentations. Use Google Drive tool to search for
//! existing presentations by name.
//!
//! # Capabilities Required
//!
//! - HTTP: `slides.googleapis.com/v1/presentations*`
//! - Secrets: `google_oauth_token` (shared OAuth 2.0 token, injected automatically)
//!
//! # Supported Actions
//!
//! - `create_presentation`: Create a new blank presentation
//! - `get_presentation`: Get presentation metadata (slides, elements, text)
//! - `get_thumbnail`: Get a thumbnail image URL for a slide
//! - `create_slide`: Add a new slide with a predefined layout
//! - `delete_object`: Delete a slide or page element
//! - `insert_text`: Insert text into a shape or text box
//! - `delete_text`: Delete text from a shape
//! - `replace_all_text`: Find and replace text across the presentation
//! - `create_shape`: Create a text box or shape on a slide
//! - `insert_image`: Insert an image on a slide
//! - `format_text`: Format text (bold, italic, font, color, size)
//! - `format_paragraph`: Set paragraph alignment
//! - `replace_shapes_with_image`: Replace placeholder shapes with an image
//! - `batch_update`: Execute multiple raw Slides API operations atomically
//!
//! # Tips
//!
//! - Presentation IDs are the same as Google Drive file IDs. Use
//!   google-drive tool's list_files to find presentations.
//! - Positions and sizes are specified in points (1 inch = 72 points).
//!   A standard slide is 720x405 points (10x5.625 inches).
//! - To add text to a slide: first create_shape (TEXT_BOX), then
//!   insert_text into the returned object_id.
//! - Use get_presentation to discover object IDs for existing elements.
//! - For template workflows: create shapes with placeholder text, then
//!   use replace_all_text or replace_shapes_with_image.
//!
//! # Example Usage
//!
//! ```json
//! {"action": "create_presentation", "title": "Q1 Report"}
//! {"action": "create_slide", "presentation_id": "abc123", "layout": "TITLE_AND_BODY"}
//! {"action": "get_presentation", "presentation_id": "abc123"}
//! {"action": "create_shape", "presentation_id": "abc123", "slide_object_id": "slide1", "shape_type": "TEXT_BOX", "x": 50, "y": 50, "width": 300, "height": 40}
//! {"action": "insert_text", "presentation_id": "abc123", "object_id": "shape1", "text": "Hello World"}
//! {"action": "format_text", "presentation_id": "abc123", "object_id": "shape1", "bold": true, "font_size": 24}
//! ```

mod api;
mod types;

use types::GoogleSlidesAction;

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../wit/tool.wit",
});

struct GoogleSlidesTool;

impl exports::near::agent::tool::Guest for GoogleSlidesTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        r#"{
            "type": "object",
            "required": ["action"],
            "oneOf": [
                {
                    "properties": {
                        "action": { "const": "create_presentation" },
                        "title": {
                            "type": "string",
                            "description": "Presentation title"
                        }
                    },
                    "required": ["action", "title"]
                },
                {
                    "properties": {
                        "action": { "const": "get_presentation" },
                        "presentation_id": {
                            "type": "string",
                            "description": "The presentation ID (same as Google Drive file ID)"
                        }
                    },
                    "required": ["action", "presentation_id"]
                },
                {
                    "properties": {
                        "action": { "const": "get_thumbnail" },
                        "presentation_id": {
                            "type": "string",
                            "description": "The presentation ID"
                        },
                        "slide_object_id": {
                            "type": "string",
                            "description": "The slide's object ID"
                        }
                    },
                    "required": ["action", "presentation_id", "slide_object_id"]
                },
                {
                    "properties": {
                        "action": { "const": "create_slide" },
                        "presentation_id": {
                            "type": "string",
                            "description": "The presentation ID"
                        },
                        "insertion_index": {
                            "type": "integer",
                            "description": "Position to insert (0-based). Omit to append at end."
                        },
                        "layout": {
                            "type": "string",
                            "enum": ["BLANK", "TITLE", "TITLE_AND_BODY", "TITLE_AND_TWO_COLUMNS", "TITLE_ONLY", "SECTION_HEADER", "CAPTION_ONLY", "BIG_NUMBER", "ONE_COLUMN_TEXT", "MAIN_POINT"],
                            "description": "Predefined layout (default: BLANK)",
                            "default": "BLANK"
                        }
                    },
                    "required": ["action", "presentation_id"]
                },
                {
                    "properties": {
                        "action": { "const": "delete_object" },
                        "presentation_id": {
                            "type": "string",
                            "description": "The presentation ID"
                        },
                        "object_id": {
                            "type": "string",
                            "description": "Object ID of the slide or element to delete"
                        }
                    },
                    "required": ["action", "presentation_id", "object_id"]
                },
                {
                    "properties": {
                        "action": { "const": "insert_text" },
                        "presentation_id": {
                            "type": "string",
                            "description": "The presentation ID"
                        },
                        "object_id": {
                            "type": "string",
                            "description": "Object ID of the shape or text box"
                        },
                        "text": {
                            "type": "string",
                            "description": "Text to insert"
                        },
                        "insertion_index": {
                            "type": "integer",
                            "description": "Character index to insert at (0-based). Default: 0.",
                            "default": 0
                        }
                    },
                    "required": ["action", "presentation_id", "object_id", "text"]
                },
                {
                    "properties": {
                        "action": { "const": "delete_text" },
                        "presentation_id": {
                            "type": "string",
                            "description": "The presentation ID"
                        },
                        "object_id": {
                            "type": "string",
                            "description": "Object ID of the shape"
                        },
                        "start_index": {
                            "type": "integer",
                            "description": "Start index (inclusive, 0-based)",
                            "default": 0
                        },
                        "end_index": {
                            "type": "integer",
                            "description": "End index (exclusive). Omit to delete from start_index to end."
                        }
                    },
                    "required": ["action", "presentation_id", "object_id"]
                },
                {
                    "properties": {
                        "action": { "const": "replace_all_text" },
                        "presentation_id": {
                            "type": "string",
                            "description": "The presentation ID"
                        },
                        "find": {
                            "type": "string",
                            "description": "Text to search for"
                        },
                        "replace": {
                            "type": "string",
                            "description": "Replacement text"
                        },
                        "match_case": {
                            "type": "boolean",
                            "description": "Case-sensitive match (default: true)",
                            "default": true
                        }
                    },
                    "required": ["action", "presentation_id", "find", "replace"]
                },
                {
                    "properties": {
                        "action": { "const": "create_shape" },
                        "presentation_id": {
                            "type": "string",
                            "description": "The presentation ID"
                        },
                        "slide_object_id": {
                            "type": "string",
                            "description": "Slide object ID to place the shape on"
                        },
                        "shape_type": {
                            "type": "string",
                            "enum": ["TEXT_BOX", "RECTANGLE", "ROUND_RECTANGLE", "ELLIPSE"],
                            "description": "Shape type (default: TEXT_BOX)",
                            "default": "TEXT_BOX"
                        },
                        "x": {
                            "type": "number",
                            "description": "X position in points from left edge"
                        },
                        "y": {
                            "type": "number",
                            "description": "Y position in points from top edge"
                        },
                        "width": {
                            "type": "number",
                            "description": "Width in points"
                        },
                        "height": {
                            "type": "number",
                            "description": "Height in points"
                        }
                    },
                    "required": ["action", "presentation_id", "slide_object_id", "x", "y", "width", "height"]
                },
                {
                    "properties": {
                        "action": { "const": "insert_image" },
                        "presentation_id": {
                            "type": "string",
                            "description": "The presentation ID"
                        },
                        "slide_object_id": {
                            "type": "string",
                            "description": "Slide object ID to place the image on"
                        },
                        "image_url": {
                            "type": "string",
                            "description": "Publicly accessible image URL"
                        },
                        "x": {
                            "type": "number",
                            "description": "X position in points"
                        },
                        "y": {
                            "type": "number",
                            "description": "Y position in points"
                        },
                        "width": {
                            "type": "number",
                            "description": "Width in points"
                        },
                        "height": {
                            "type": "number",
                            "description": "Height in points"
                        }
                    },
                    "required": ["action", "presentation_id", "slide_object_id", "image_url", "x", "y", "width", "height"]
                },
                {
                    "properties": {
                        "action": { "const": "format_text" },
                        "presentation_id": {
                            "type": "string",
                            "description": "The presentation ID"
                        },
                        "object_id": {
                            "type": "string",
                            "description": "Object ID of the shape"
                        },
                        "start_index": {
                            "type": "integer",
                            "description": "Start index (inclusive). Omit to format all text."
                        },
                        "end_index": {
                            "type": "integer",
                            "description": "End index (exclusive). Omit to format to end."
                        },
                        "bold": {
                            "type": "boolean",
                            "description": "Make text bold"
                        },
                        "italic": {
                            "type": "boolean",
                            "description": "Make text italic"
                        },
                        "underline": {
                            "type": "boolean",
                            "description": "Underline text"
                        },
                        "font_size": {
                            "type": "number",
                            "description": "Font size in points (e.g., 12, 18, 24)"
                        },
                        "font_family": {
                            "type": "string",
                            "description": "Font family (e.g., 'Arial', 'Roboto', 'Times New Roman')"
                        },
                        "foreground_color": {
                            "type": "string",
                            "description": "Text color as hex (e.g., '#FF0000' for red)"
                        }
                    },
                    "required": ["action", "presentation_id", "object_id"]
                },
                {
                    "properties": {
                        "action": { "const": "format_paragraph" },
                        "presentation_id": {
                            "type": "string",
                            "description": "The presentation ID"
                        },
                        "object_id": {
                            "type": "string",
                            "description": "Object ID of the shape"
                        },
                        "alignment": {
                            "type": "string",
                            "enum": ["START", "CENTER", "END", "JUSTIFIED"],
                            "description": "Paragraph alignment"
                        },
                        "start_index": {
                            "type": "integer",
                            "description": "Start index (inclusive). Omit to format all."
                        },
                        "end_index": {
                            "type": "integer",
                            "description": "End index (exclusive). Omit to format to end."
                        }
                    },
                    "required": ["action", "presentation_id", "object_id", "alignment"]
                },
                {
                    "properties": {
                        "action": { "const": "replace_shapes_with_image" },
                        "presentation_id": {
                            "type": "string",
                            "description": "The presentation ID"
                        },
                        "find": {
                            "type": "string",
                            "description": "Text to match in shapes"
                        },
                        "image_url": {
                            "type": "string",
                            "description": "Image URL to replace matched shapes with"
                        },
                        "match_case": {
                            "type": "boolean",
                            "description": "Case-sensitive match (default: true)",
                            "default": true
                        }
                    },
                    "required": ["action", "presentation_id", "find", "image_url"]
                },
                {
                    "properties": {
                        "action": { "const": "batch_update" },
                        "presentation_id": {
                            "type": "string",
                            "description": "The presentation ID"
                        },
                        "requests": {
                            "type": "array",
                            "items": { "type": "object" },
                            "description": "Array of raw Slides API batchUpdate request objects"
                        }
                    },
                    "required": ["action", "presentation_id", "requests"]
                }
            ]
        }"#
        .to_string()
    }

    fn description() -> String {
        "Google Slides integration for creating, reading, editing, and formatting presentations. \
         Supports slide management (create, delete, reorder), text operations (insert, delete, \
         find-replace), shapes and text boxes, image insertion, text formatting (bold, italic, \
         font, color, size), paragraph alignment, thumbnails, and template-based image replacement. \
         Also provides a batch_update action for complex multi-step edits executed atomically. \
         Positions and sizes use points (standard slide is 720x405 pt). Presentation IDs are the \
         same as Google Drive file IDs, so use the google-drive tool to search for existing \
         presentations. Requires a Google OAuth token with the presentations scope."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    if !crate::near::agent::host::secret_exists("google_oauth_token") {
        return Err(
            "Google OAuth token not configured. Run `ironclaw tool auth google-slides` to set up \
             OAuth, or set the GOOGLE_OAUTH_TOKEN environment variable."
                .to_string(),
        );
    }

    let action: GoogleSlidesAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Info,
        &format!("Executing Google Slides action: {:?}", action),
    );

    let result = match action {
        GoogleSlidesAction::CreatePresentation { title } => {
            let result = api::create_presentation(&title)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSlidesAction::GetPresentation { presentation_id } => {
            let result = api::get_presentation(&presentation_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSlidesAction::GetThumbnail {
            presentation_id,
            slide_object_id,
        } => {
            let result = api::get_thumbnail(&presentation_id, &slide_object_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSlidesAction::CreateSlide {
            presentation_id,
            insertion_index,
            layout,
        } => {
            let result = api::create_slide(&presentation_id, insertion_index, &layout)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSlidesAction::DeleteObject {
            presentation_id,
            object_id,
        } => {
            let result = api::delete_object(&presentation_id, &object_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSlidesAction::InsertText {
            presentation_id,
            object_id,
            text,
            insertion_index,
        } => {
            let result = api::insert_text(&presentation_id, &object_id, &text, insertion_index)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSlidesAction::DeleteText {
            presentation_id,
            object_id,
            start_index,
            end_index,
        } => {
            let result = api::delete_text(&presentation_id, &object_id, start_index, end_index)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSlidesAction::ReplaceAllText {
            presentation_id,
            find,
            replace,
            match_case,
        } => {
            let result = api::replace_all_text(&presentation_id, &find, &replace, match_case)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSlidesAction::CreateShape {
            presentation_id,
            slide_object_id,
            shape_type,
            x,
            y,
            width,
            height,
        } => {
            let result = api::create_shape(
                &presentation_id,
                &slide_object_id,
                &shape_type,
                x,
                y,
                width,
                height,
            )?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSlidesAction::InsertImage {
            presentation_id,
            slide_object_id,
            image_url,
            x,
            y,
            width,
            height,
        } => {
            let result = api::insert_image(
                &presentation_id,
                &slide_object_id,
                &image_url,
                x,
                y,
                width,
                height,
            )?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSlidesAction::FormatText {
            presentation_id,
            object_id,
            start_index,
            end_index,
            bold,
            italic,
            underline,
            font_size,
            font_family,
            foreground_color,
        } => {
            let result = api::format_text(api::FormatTextOptions {
                presentation_id: &presentation_id,
                object_id: &object_id,
                start_index,
                end_index,
                bold,
                italic,
                underline,
                font_size,
                font_family: font_family.as_deref(),
                foreground_color: foreground_color.as_deref(),
            })?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSlidesAction::FormatParagraph {
            presentation_id,
            object_id,
            alignment,
            start_index,
            end_index,
        } => {
            let result = api::format_paragraph(
                &presentation_id,
                &object_id,
                &alignment,
                start_index,
                end_index,
            )?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSlidesAction::ReplaceShapesWithImage {
            presentation_id,
            find,
            image_url,
            match_case,
        } => {
            let result =
                api::replace_shapes_with_image(&presentation_id, &find, &image_url, match_case)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSlidesAction::BatchUpdate {
            presentation_id,
            requests,
        } => {
            let result = api::batch_update(&presentation_id, requests)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }
    };

    Ok(result)
}

export!(GoogleSlidesTool);
