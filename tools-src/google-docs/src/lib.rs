//! Google Docs WASM Tool for IronClaw.
//!
//! Provides Google Docs integration for creating, reading, editing,
//! and formatting documents. Use Google Drive tool to search for
//! existing documents by name.
//!
//! # Capabilities Required
//!
//! - HTTP: `docs.googleapis.com/v1/documents*`
//! - Secrets: `google_oauth_token` (shared OAuth 2.0 token, injected automatically)
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

use types::GoogleDocsAction;

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

struct GoogleDocsTool;

impl exports::near::agent::tool::Guest for GoogleDocsTool {
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
                        "action": { "const": "create_document" },
                        "title": {
                            "type": "string",
                            "description": "Document title"
                        }
                    },
                    "required": ["action", "title"]
                },
                {
                    "properties": {
                        "action": { "const": "get_document" },
                        "document_id": {
                            "type": "string",
                            "description": "The document ID (same as Google Drive file ID)"
                        }
                    },
                    "required": ["action", "document_id"]
                },
                {
                    "properties": {
                        "action": { "const": "read_content" },
                        "document_id": {
                            "type": "string",
                            "description": "The document ID"
                        }
                    },
                    "required": ["action", "document_id"]
                },
                {
                    "properties": {
                        "action": { "const": "insert_text" },
                        "document_id": {
                            "type": "string",
                            "description": "The document ID"
                        },
                        "text": {
                            "type": "string",
                            "description": "Text to insert"
                        },
                        "index": {
                            "type": "integer",
                            "description": "Character index to insert at (1 for start of body). Use -1 to append at end.",
                            "default": -1
                        },
                        "segment_id": {
                            "type": "string",
                            "description": "Segment ID (empty string for body, or a header/footer ID)",
                            "default": ""
                        }
                    },
                    "required": ["action", "document_id", "text"]
                },
                {
                    "properties": {
                        "action": { "const": "delete_content" },
                        "document_id": {
                            "type": "string",
                            "description": "The document ID"
                        },
                        "start_index": {
                            "type": "integer",
                            "description": "Start index (inclusive)"
                        },
                        "end_index": {
                            "type": "integer",
                            "description": "End index (exclusive)"
                        },
                        "segment_id": {
                            "type": "string",
                            "description": "Segment ID (empty for body)",
                            "default": ""
                        }
                    },
                    "required": ["action", "document_id", "start_index", "end_index"]
                },
                {
                    "properties": {
                        "action": { "const": "replace_text" },
                        "document_id": {
                            "type": "string",
                            "description": "The document ID"
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
                    "required": ["action", "document_id", "find", "replace"]
                },
                {
                    "properties": {
                        "action": { "const": "format_text" },
                        "document_id": {
                            "type": "string",
                            "description": "The document ID"
                        },
                        "start_index": {
                            "type": "integer",
                            "description": "Start index (inclusive)"
                        },
                        "end_index": {
                            "type": "integer",
                            "description": "End index (exclusive)"
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
                        "strikethrough": {
                            "type": "boolean",
                            "description": "Strikethrough text"
                        },
                        "font_size": {
                            "type": "number",
                            "description": "Font size in points (e.g., 12, 14, 18)"
                        },
                        "font_family": {
                            "type": "string",
                            "description": "Font family (e.g., 'Arial', 'Times New Roman', 'Courier New')"
                        },
                        "foreground_color": {
                            "type": "string",
                            "description": "Text color as hex (e.g., '#FF0000' for red)"
                        },
                        "background_color": {
                            "type": "string",
                            "description": "Text background/highlight color as hex"
                        }
                    },
                    "required": ["action", "document_id", "start_index", "end_index"]
                },
                {
                    "properties": {
                        "action": { "const": "format_paragraph" },
                        "document_id": {
                            "type": "string",
                            "description": "The document ID"
                        },
                        "start_index": {
                            "type": "integer",
                            "description": "Start index (inclusive)"
                        },
                        "end_index": {
                            "type": "integer",
                            "description": "End index (exclusive)"
                        },
                        "named_style": {
                            "type": "string",
                            "enum": ["NORMAL_TEXT", "TITLE", "SUBTITLE", "HEADING_1", "HEADING_2", "HEADING_3", "HEADING_4", "HEADING_5", "HEADING_6"],
                            "description": "Paragraph style (heading level)"
                        },
                        "alignment": {
                            "type": "string",
                            "enum": ["START", "CENTER", "END", "JUSTIFIED"],
                            "description": "Text alignment"
                        },
                        "line_spacing": {
                            "type": "number",
                            "description": "Line spacing as percentage (e.g., 100 for single, 150 for 1.5x, 200 for double)"
                        }
                    },
                    "required": ["action", "document_id", "start_index", "end_index"]
                },
                {
                    "properties": {
                        "action": { "const": "insert_table" },
                        "document_id": {
                            "type": "string",
                            "description": "The document ID"
                        },
                        "rows": {
                            "type": "integer",
                            "description": "Number of rows"
                        },
                        "columns": {
                            "type": "integer",
                            "description": "Number of columns"
                        },
                        "index": {
                            "type": "integer",
                            "description": "Character index to insert the table at"
                        }
                    },
                    "required": ["action", "document_id", "rows", "columns", "index"]
                },
                {
                    "properties": {
                        "action": { "const": "create_list" },
                        "document_id": {
                            "type": "string",
                            "description": "The document ID"
                        },
                        "start_index": {
                            "type": "integer",
                            "description": "Start index (inclusive)"
                        },
                        "end_index": {
                            "type": "integer",
                            "description": "End index (exclusive)"
                        },
                        "bullet_preset": {
                            "type": "string",
                            "enum": ["BULLET_DISC_CIRCLE_SQUARE", "BULLET_CHECKBOX", "BULLET_ARROW_DIAMOND_DISC", "NUMBERED_DECIMAL_ALPHA_ROMAN", "NUMBERED_DECIMAL_NESTED", "NUMBERED_UPPERALPHA_ALPHA_ROMAN"],
                            "description": "Bullet style preset (default: BULLET_DISC_CIRCLE_SQUARE)",
                            "default": "BULLET_DISC_CIRCLE_SQUARE"
                        }
                    },
                    "required": ["action", "document_id", "start_index", "end_index"]
                },
                {
                    "properties": {
                        "action": { "const": "batch_update" },
                        "document_id": {
                            "type": "string",
                            "description": "The document ID"
                        },
                        "requests": {
                            "type": "array",
                            "items": { "type": "object" },
                            "description": "Array of raw Docs API batchUpdate request objects"
                        }
                    },
                    "required": ["action", "document_id", "requests"]
                }
            ]
        }"#
        .to_string()
    }

    fn description() -> String {
        "Google Docs integration for creating, reading, editing, and formatting documents. \
         Supports text operations (insert, delete, find-replace), text formatting (bold, italic, \
         font, color, size), paragraph styling (headings, alignment, spacing), tables, and \
         bulleted/numbered lists. Also provides a batch_update action for complex multi-step \
         edits executed atomically. Document IDs are the same as Google Drive file IDs, so use \
         the google-drive tool to search for existing documents. Requires a Google OAuth token \
         with the documents scope."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    if !crate::near::agent::host::secret_exists("google_oauth_token") {
        return Err(
            "Google OAuth token not configured. Run `ironclaw tool auth google-docs` to set up \
             OAuth, or set the GOOGLE_OAUTH_TOKEN environment variable."
                .to_string(),
        );
    }

    let action: GoogleDocsAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Info,
        &format!("Executing Google Docs action: {:?}", action),
    );

    let result = match action {
        GoogleDocsAction::CreateDocument { title } => {
            let result = api::create_document(&title)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDocsAction::GetDocument { document_id } => {
            let result = api::get_document(&document_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDocsAction::ReadContent { document_id } => {
            let result = api::read_content(&document_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDocsAction::InsertText {
            document_id,
            text,
            index,
            segment_id,
        } => {
            let result = api::insert_text(&document_id, &text, index, &segment_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDocsAction::DeleteContent {
            document_id,
            start_index,
            end_index,
            segment_id,
        } => {
            let result = api::delete_content(&document_id, start_index, end_index, &segment_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDocsAction::ReplaceText {
            document_id,
            find,
            replace,
            match_case,
        } => {
            let result = api::replace_text(&document_id, &find, &replace, match_case)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
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
            serde_json::to_string(&result).map_err(|e| e.to_string())?
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
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDocsAction::InsertTable {
            document_id,
            rows,
            columns,
            index,
        } => {
            let result = api::insert_table(&document_id, rows, columns, index)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDocsAction::CreateList {
            document_id,
            start_index,
            end_index,
            bullet_preset,
        } => {
            let result = api::create_list(&document_id, start_index, end_index, &bullet_preset)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleDocsAction::BatchUpdate {
            document_id,
            requests,
        } => {
            let result = api::batch_update(&document_id, requests)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }
    };

    Ok(result)
}

export!(GoogleDocsTool);
