//! Google Docs WASM Tool for IronClaw.
//!
//! Provides Google Docs integration for creating, reading, editing,
//! and formatting documents. Use Google Drive tool to search for
//! existing documents by name.
//!
//! # Capabilities Required
//!
//! - HTTP: `docs.googleapis.com/v1/documents*`
//! - Credentials: staged Google product-auth account token injected by the host.
//!
//! # Supported Actions
//!
//! - `create_document`: Create a new blank document
//! - `get_document`: Get document metadata (title, length, named ranges)
//! - `read_content`: Read entire document body as plain text
//! - `insert_text`: Insert text at a position (or append at end)
//! - `delete_content`: Delete text in a range
//! - `replace_text`: Find and replace all occurrences
//! - `format_text`: Format text (bold, italic, font, color, size)
//! - `format_paragraph`: Set heading level, alignment, spacing
//! - `insert_table`: Insert a table at a position
//! - `create_list`: Create bulleted/numbered list from paragraphs
//! - `batch_update`: Execute multiple raw Docs API operations atomically
//!
//! # Tips
//!
//! - Document IDs are the same as Google Drive file IDs. Use google-drive
//!   tool's list_files to find documents.
//! - Indexes are 0-based character offsets. An empty document body starts
//!   with a newline at index 0, so insert at index 1 to prepend text.
//! - Use index -1 to append at the end of the document.
//! - When doing multiple edits, process from highest index to lowest
//!   to avoid index shifting issues.
//!
//! # Example Usage
//!
//! ```json
//! {"action": "create_document", "title": "Meeting Notes"}
//! {"action": "read_content", "document_id": "abc123"}
//! {"action": "insert_text", "document_id": "abc123", "text": "Hello World\n", "index": 1}
//! {"action": "replace_text", "document_id": "abc123", "find": "Hello", "replace": "Hi"}
//! {"action": "format_text", "document_id": "abc123", "start_index": 1, "end_index": 12, "bold": true, "font_size": 18}
//! {"action": "format_paragraph", "document_id": "abc123", "start_index": 1, "end_index": 12, "named_style": "HEADING_1"}
//! ```

mod api;
mod types;

use types::{GoogleDocsAction, ToolContext};

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../../lanes/ironclaw_wasm/wit/tool.wit",
});

use exports::near::agent::tool::{ErrorKind, GuestFailure, Response};

struct GoogleDocsTool;

impl exports::near::agent::tool::Guest for GoogleDocsTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params, req.context.as_deref()) {
            Ok(result) => Response::Success(result),
            Err(failure) => Response::Failure(failure),
        }
    }

    fn schema() -> String {
        // Derived from `GoogleDocsAction` via `schemars::JsonSchema` so the
        // advertised schema can never drift from the serde contract.
        let schema = schemars::schema_for!(types::GoogleDocsAction);
        serde_json::to_string(&schema).unwrap_or_else(|_| "{}".to_string())
    }

    fn description() -> String {
        "Google Docs integration for creating, reading, editing, and formatting documents. \
         Supports text operations (insert, delete, find-replace), text formatting (bold, italic, \
         font, color, size), paragraph styling (headings, alignment, spacing), tables, and \
         bulleted/numbered lists. Also provides a batch_update action for complex multi-step \
         edits executed atomically. Document IDs are the same as Google Drive file IDs, so use \
         the google-drive tool to search for existing documents. The host injects a Google \
         product-auth credential with the documents scope. \
         To discover all available API operations, use http GET to fetch \
         <https://www.googleapis.com/discovery/v1/apis/docs/v1/rest> (public, no auth needed)."
            .to_string()
    }
}

/// Build an `input`-kind guest failure with a stable code and no free-text
/// message (the code alone is the actionable signal for these host-plumbing
/// failures).
fn input_failure(code: &'static str) -> GuestFailure {
    GuestFailure {
        kind: ErrorKind::Input,
        code: Some(code.to_string()),
        message: None,
    }
}

fn execute_inner(params: &str, context: Option<&str>) -> Result<String, GuestFailure> {
    let action_name = action_from_context(context)?;
    let params = params_with_action(params, action_name)?;
    let action: GoogleDocsAction = serde_json::from_value(params).map_err(|e| GuestFailure {
        kind: ErrorKind::Input,
        code: Some("invalid_parameters".to_string()),
        message: Some(api::bounded_message(&e.to_string())),
    })?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Debug,
        &format!("Executing Google Docs action: {action_name}"),
    );

    let result = match action {
        GoogleDocsAction::CreateDocument { title } => {
            let result = api::create_document(&title)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        GoogleDocsAction::GetDocument { document_id } => {
            let result = api::get_document(&document_id)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        GoogleDocsAction::ReadContent { document_id } => {
            let result = api::read_content(&document_id)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        GoogleDocsAction::InsertText {
            document_id,
            text,
            index,
            segment_id,
        } => {
            let result = api::insert_text(&document_id, &text, index, &segment_id)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        GoogleDocsAction::DeleteContent {
            document_id,
            start_index,
            end_index,
            segment_id,
        } => {
            let result = api::delete_content(&document_id, start_index, end_index, &segment_id)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        GoogleDocsAction::ReplaceText {
            document_id,
            find,
            replace,
            match_case,
        } => {
            let result = api::replace_text(&document_id, &find, &replace, match_case)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        GoogleDocsAction::FormatText {
            document_id,
            start_index,
            end_index,
            bold,
            italic,
            underline,
            strikethrough,
            font_size,
            font_family,
            foreground_color,
            background_color,
        } => {
            let result = api::format_text(api::FormatTextOptions {
                document_id: &document_id,
                start_index,
                end_index,
                bold,
                italic,
                underline,
                strikethrough,
                font_size,
                font_family: font_family.as_deref(),
                foreground_color: foreground_color.as_deref(),
                background_color: background_color.as_deref(),
            })?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        GoogleDocsAction::FormatParagraph {
            document_id,
            start_index,
            end_index,
            named_style,
            alignment,
            line_spacing,
        } => {
            let result = api::format_paragraph(
                &document_id,
                start_index,
                end_index,
                named_style.as_deref(),
                alignment.as_deref(),
                line_spacing,
            )?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        GoogleDocsAction::InsertTable {
            document_id,
            rows,
            columns,
            index,
        } => {
            let result = api::insert_table(&document_id, rows, columns, index)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        GoogleDocsAction::CreateList {
            document_id,
            start_index,
            end_index,
            bullet_preset,
        } => {
            let result = api::create_list(&document_id, start_index, end_index, &bullet_preset)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        GoogleDocsAction::BatchUpdate {
            document_id,
            requests,
        } => {
            let result = api::batch_update(&document_id, requests)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }
    };

    Ok(result)
}

fn action_from_context(context: Option<&str>) -> Result<&'static str, GuestFailure> {
    let context = context.ok_or_else(|| input_failure("missing_invocation_context"))?;
    let context: ToolContext =
        serde_json::from_str(context).map_err(|_| input_failure("invalid_invocation_context"))?;
    match context.capability_id.as_str() {
        "google-docs.create_document" => Ok("create_document"),
        "google-docs.get_document" => Ok("get_document"),
        "google-docs.read_content" => Ok("read_content"),
        "google-docs.insert_text" => Ok("insert_text"),
        "google-docs.delete_content" => Ok("delete_content"),
        "google-docs.replace_text" => Ok("replace_text"),
        "google-docs.format_text" => Ok("format_text"),
        "google-docs.format_paragraph" => Ok("format_paragraph"),
        "google-docs.insert_table" => Ok("insert_table"),
        "google-docs.create_list" => Ok("create_list"),
        "google-docs.batch_update" => Ok("batch_update"),
        _ => Err(input_failure("unsupported_google_docs_capability")),
    }
}

fn params_with_action(params: &str, action: &str) -> Result<serde_json::Value, GuestFailure> {
    let mut params: serde_json::Value = if params.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(params).map_err(|_| input_failure("invalid_parameters"))?
    };
    let obj = params
        .as_object_mut()
        .ok_or_else(|| input_failure("invalid_parameters"))?;
    if obj.contains_key("action") {
        return Err(input_failure("invalid_parameters"));
    }
    obj.insert(
        "action".to_string(),
        serde_json::Value::String(action.to_string()),
    );
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_with_action_rejects_caller_supplied_action() {
        let error = params_with_action(
            r#"{"action":"delete_all","document_id":"doc-1"}"#,
            "get_document",
        )
        .unwrap_err();

        assert_eq!(error.kind, ErrorKind::Input);
        assert_eq!(error.code.as_deref(), Some("invalid_parameters"));
    }
}

export!(GoogleDocsTool);
