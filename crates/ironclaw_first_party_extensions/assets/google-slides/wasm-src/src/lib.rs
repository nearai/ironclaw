//! Google Slides WASM Tool for IronClaw.
//!
//! Provides Google Slides integration for creating, reading, editing,
//! and formatting presentations. Use Google Drive tool to search for
//! existing presentations by name.
//!
//! # Capabilities Required
//!
//! - HTTP: `slides.googleapis.com/v1/presentations*`
//! - Credentials: staged Google product-auth account token injected by the host.
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

use types::{GoogleSlidesAction, ToolContext};

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../../../wit/tool.wit",
});

struct GoogleSlidesTool;

impl exports::near::agent::tool::Guest for GoogleSlidesTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params, req.context.as_deref()) {
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
        // Derived from `GoogleSlidesAction` via `schemars::JsonSchema` so the
        // advertised schema can never drift from the serde contract.
        let schema = schemars::schema_for!(types::GoogleSlidesAction);
        serde_json::to_string(&schema).unwrap_or_else(|_| "{}".to_string())
    }

    fn description() -> String {
        "Google Slides integration for creating, reading, editing, and formatting presentations. \
         Supports slide management (create, delete, reorder), text operations (insert, delete, \
         find-replace), shapes and text boxes, image insertion, text formatting (bold, italic, \
         font, color, size), paragraph alignment, thumbnails, and template-based image replacement. \
         Also provides a batch_update action for complex multi-step edits executed atomically. \
         Positions and sizes use points (standard slide is 720x405 pt). Presentation IDs are the \
         same as Google Drive file IDs, so use the google-drive tool to search for existing \
         presentations. The host injects a Google product-auth credential with the presentations \
         scope. \
         To discover all available API operations, use http GET to fetch \
         <https://www.googleapis.com/discovery/v1/apis/slides/v1/rest> (public, no auth needed)."
            .to_string()
    }
}

fn execute_inner(params: &str, context: Option<&str>) -> Result<String, String> {
    let action_name = action_from_context(context)?;
    let params = params_with_action(params, action_name)?;
    let action: GoogleSlidesAction =
        serde_json::from_value(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Debug,
        &format!("Executing Google Slides action: {action_name}"),
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

fn action_from_context(context: Option<&str>) -> Result<&'static str, String> {
    let context = context.ok_or_else(|| "missing_invocation_context".to_string())?;
    let context: ToolContext =
        serde_json::from_str(context).map_err(|_| "invalid_invocation_context".to_string())?;
    match context.capability_id.as_str() {
        "google-slides.create_presentation" => Ok("create_presentation"),
        "google-slides.get_presentation" => Ok("get_presentation"),
        "google-slides.get_thumbnail" => Ok("get_thumbnail"),
        "google-slides.create_slide" => Ok("create_slide"),
        "google-slides.delete_object" => Ok("delete_object"),
        "google-slides.insert_text" => Ok("insert_text"),
        "google-slides.delete_text" => Ok("delete_text"),
        "google-slides.replace_all_text" => Ok("replace_all_text"),
        "google-slides.create_shape" => Ok("create_shape"),
        "google-slides.insert_image" => Ok("insert_image"),
        "google-slides.format_text" => Ok("format_text"),
        "google-slides.format_paragraph" => Ok("format_paragraph"),
        "google-slides.replace_shapes_with_image" => Ok("replace_shapes_with_image"),
        "google-slides.batch_update" => Ok("batch_update"),
        _ => Err("unsupported_google_slides_capability".to_string()),
    }
}

fn params_with_action(params: &str, action: &str) -> Result<serde_json::Value, String> {
    let mut params: serde_json::Value = if params.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(params).map_err(|_| "invalid_parameters".to_string())?
    };
    let obj = params
        .as_object_mut()
        .ok_or_else(|| "invalid_parameters".to_string())?;
    if obj.contains_key("action") {
        return Err("invalid_parameters".to_string());
    }
    obj.insert(
        "action".to_string(),
        serde_json::Value::String(action.to_string()),
    );
    Ok(params)
}

export!(GoogleSlidesTool);
