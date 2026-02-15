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
    path: "../../wit/tool.wit",
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
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create_presentation", "get_presentation", "get_thumbnail", "create_slide", "delete_object", "insert_text", "delete_text", "replace_all_text", "create_shape", "insert_image", "format_text", "format_paragraph", "replace_shapes_with_image", "batch_update"],
                    "description": "The Google Slides operation to perform"
                },
                "title": {
                    "type": "string",
                    "description": "Presentation title. Required for: create_presentation"
                },
                "presentation_id": {
                    "type": "string",
                    "description": "Presentation ID (same as Google Drive file ID). Required for all actions except create_presentation"
                },
                "slide_object_id": {
                    "type": "string",
                    "description": "Slide object ID. Required for: get_thumbnail, create_shape, insert_image"
                },
                "object_id": {
                    "type": "string",
                    "description": "Object ID of a slide element. Required for: delete_object, insert_text, delete_text, format_text, format_paragraph"
                },
                "text": {
                    "type": "string",
                    "description": "Text to insert. Required for: insert_text"
                },
                "insertion_index": {
                    "type": "integer",
                    "description": "Position to insert at (0-based). Used by: create_slide (omit to append at end), insert_text (default: 0)"
                },
                "layout": {
                    "type": "string",
                    "enum": ["BLANK", "TITLE", "TITLE_AND_BODY", "TITLE_AND_TWO_COLUMNS", "TITLE_ONLY", "SECTION_HEADER", "CAPTION_ONLY", "BIG_NUMBER", "ONE_COLUMN_TEXT", "MAIN_POINT"],
                    "description": "Predefined slide layout (default: BLANK). Used by: create_slide",
                    "default": "BLANK"
                },
                "start_index": {
                    "type": "integer",
                    "description": "Start index (inclusive, 0-based). Used by: delete_text, format_text, format_paragraph"
                },
                "end_index": {
                    "type": "integer",
                    "description": "End index (exclusive). Used by: delete_text, format_text, format_paragraph"
                },
                "find": {
                    "type": "string",
                    "description": "Text to search for. Required for: replace_all_text, replace_shapes_with_image"
                },
                "replace": {
                    "type": "string",
                    "description": "Replacement text. Required for: replace_all_text"
                },
                "match_case": {
                    "type": "boolean",
                    "description": "Case-sensitive match (default: true). Used by: replace_all_text, replace_shapes_with_image",
                    "default": true
                },
                "shape_type": {
                    "type": "string",
                    "enum": ["TEXT_BOX", "RECTANGLE", "ROUND_RECTANGLE", "ELLIPSE"],
                    "description": "Shape type (default: TEXT_BOX). Used by: create_shape",
                    "default": "TEXT_BOX"
                },
                "x": {
                    "type": "number",
                    "description": "X position in points from left edge. Required for: create_shape, insert_image"
                },
                "y": {
                    "type": "number",
                    "description": "Y position in points from top edge. Required for: create_shape, insert_image"
                },
                "width": {
                    "type": "number",
                    "description": "Width in points. Required for: create_shape, insert_image"
                },
                "height": {
                    "type": "number",
                    "description": "Height in points. Required for: create_shape, insert_image"
                },
                "image_url": {
                    "type": "string",
                    "description": "Publicly accessible image URL. Required for: insert_image, replace_shapes_with_image"
                },
                "bold": {
                    "type": "boolean",
                    "description": "Make text bold. Used by: format_text"
                },
                "italic": {
                    "type": "boolean",
                    "description": "Make text italic. Used by: format_text"
                },
                "underline": {
                    "type": "boolean",
                    "description": "Underline text. Used by: format_text"
                },
                "font_size": {
                    "type": "number",
                    "description": "Font size in points (e.g., 12, 18, 24). Used by: format_text"
                },
                "font_family": {
                    "type": "string",
                    "description": "Font family (e.g., 'Arial', 'Roboto'). Used by: format_text"
                },
                "foreground_color": {
                    "type": "string",
                    "description": "Text color as hex (e.g., '#FF0000'). Used by: format_text"
                },
                "alignment": {
                    "type": "string",
                    "enum": ["START", "CENTER", "END", "JUSTIFIED"],
                    "description": "Paragraph alignment. Required for: format_paragraph"
                },
                "requests": {
                    "type": "array",
                    "items": { "type": "object" },
                    "description": "Array of raw Slides API batchUpdate request objects. Required for: batch_update"
                }
            }
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
