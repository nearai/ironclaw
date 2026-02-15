//! Google Sheets WASM Tool for IronClaw.
//!
//! Provides Google Sheets integration for creating, reading, writing,
//! and formatting spreadsheets. Use Google Drive tool to search for
//! existing spreadsheets by name.
//!
//! # Capabilities Required
//!
//! - HTTP: `sheets.googleapis.com/v4/spreadsheets*`
//! - Secrets: `google_oauth_token` (shared OAuth 2.0 token, injected automatically)
//!
//! # Supported Actions
//!
//! - `create_spreadsheet`: Create a new spreadsheet with optional sheet names
//! - `get_spreadsheet`: Get metadata (title, sheets, named ranges)
//! - `read_values`: Read cell values from a range (A1 notation)
//! - `batch_read_values`: Read from multiple ranges at once
//! - `write_values`: Write values to a range (overwrites)
//! - `append_values`: Append rows after existing data
//! - `clear_values`: Clear values from a range (keeps formatting)
//! - `add_sheet`: Add a new sheet (tab)
//! - `delete_sheet`: Delete a sheet (tab)
//! - `rename_sheet`: Rename a sheet (tab)
//! - `format_cells`: Format cells (bold, colors, alignment, number format)
//!
//! # Tips
//!
//! - Spreadsheet IDs are the same as Google Drive file IDs. Use google-drive
//!   tool's list_files to find spreadsheets.
//! - Use A1 notation for ranges: "Sheet1!A1:D10", "A1:B5", "Sheet1!A:E"
//! - Sheet IDs (numeric) are different from sheet names. Get them via get_spreadsheet.
//!
//! # Example Usage
//!
//! ```json
//! {"action": "create_spreadsheet", "title": "Q1 Report", "sheet_names": ["Revenue", "Expenses"]}
//! {"action": "read_values", "spreadsheet_id": "abc123", "range": "Sheet1!A1:D10"}
//! {"action": "write_values", "spreadsheet_id": "abc123", "range": "Sheet1!A1", "values": [["Name", "Age"], ["Alice", 30]]}
//! {"action": "append_values", "spreadsheet_id": "abc123", "range": "Sheet1!A:B", "values": [["Bob", 25]]}
//! {"action": "format_cells", "spreadsheet_id": "abc123", "sheet_id": 0, "start_row": 0, "end_row": 1, "start_column": 0, "end_column": 4, "bold": true, "background_color": "#4285F4", "text_color": "#FFFFFF"}
//! ```

mod api;
mod types;

use types::GoogleSheetsAction;

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

struct GoogleSheetsTool;

impl exports::near::agent::tool::Guest for GoogleSheetsTool {
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
                    "enum": ["create_spreadsheet", "get_spreadsheet", "read_values", "batch_read_values", "write_values", "append_values", "clear_values", "add_sheet", "delete_sheet", "rename_sheet", "format_cells"],
                    "description": "The Google Sheets operation to perform"
                },
                "spreadsheet_id": {
                    "type": "string",
                    "description": "Spreadsheet ID (same as Google Drive file ID). Required for all actions except create_spreadsheet"
                },
                "title": {
                    "type": "string",
                    "description": "Title/name. Required for: create_spreadsheet, add_sheet, rename_sheet"
                },
                "sheet_names": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Names for sheets (tabs, defaults to ['Sheet1']). Used by: create_spreadsheet"
                },
                "range": {
                    "type": "string",
                    "description": "A1 notation range (e.g., 'Sheet1!A1:D10'). Required for: read_values, write_values, append_values, clear_values"
                },
                "ranges": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of A1 notation ranges. Required for: batch_read_values"
                },
                "values": {
                    "type": "array",
                    "items": { "type": "array" },
                    "description": "2D array of values (rows of columns). Required for: write_values, append_values"
                },
                "value_input_option": {
                    "type": "string",
                    "enum": ["RAW", "USER_ENTERED"],
                    "description": "How to interpret input (USER_ENTERED parses like the UI, RAW stores as-is, default: USER_ENTERED). Used by: write_values, append_values",
                    "default": "USER_ENTERED"
                },
                "sheet_id": {
                    "type": "integer",
                    "description": "Numeric sheet ID (from get_spreadsheet, NOT the sheet name). Required for: delete_sheet, rename_sheet, format_cells"
                },
                "start_row": {
                    "type": "integer",
                    "description": "Start row (0-indexed, inclusive). Required for: format_cells"
                },
                "end_row": {
                    "type": "integer",
                    "description": "End row (0-indexed, exclusive). Required for: format_cells"
                },
                "start_column": {
                    "type": "integer",
                    "description": "Start column (0-indexed, inclusive). Required for: format_cells"
                },
                "end_column": {
                    "type": "integer",
                    "description": "End column (0-indexed, exclusive). Required for: format_cells"
                },
                "bold": {
                    "type": "boolean",
                    "description": "Make text bold. Used by: format_cells"
                },
                "italic": {
                    "type": "boolean",
                    "description": "Make text italic. Used by: format_cells"
                },
                "font_size": {
                    "type": "integer",
                    "description": "Font size in points. Used by: format_cells"
                },
                "text_color": {
                    "type": "string",
                    "description": "Text color as hex (e.g., '#FF0000'). Used by: format_cells"
                },
                "background_color": {
                    "type": "string",
                    "description": "Cell background color as hex (e.g., '#FFFF00'). Used by: format_cells"
                },
                "horizontal_alignment": {
                    "type": "string",
                    "enum": ["LEFT", "CENTER", "RIGHT"],
                    "description": "Horizontal text alignment. Used by: format_cells"
                },
                "number_format": {
                    "type": "string",
                    "description": "Number format pattern (e.g., '#,##0.00', 'yyyy-mm-dd'). Used by: format_cells"
                },
                "number_format_type": {
                    "type": "string",
                    "enum": ["NUMBER", "CURRENCY", "PERCENT", "DATE", "TIME", "TEXT"],
                    "description": "Type of number format (default: NUMBER). Used by: format_cells"
                }
            }
        }"#
        .to_string()
    }

    fn description() -> String {
        "Google Sheets integration for creating, reading, writing, and formatting spreadsheets. \
         Supports cell value operations (read, write, append, clear) using A1 notation, sheet \
         (tab) management (add, delete, rename), and cell formatting (bold, colors, alignment, \
         number formats). Spreadsheet IDs are the same as Google Drive file IDs, so use the \
         google-drive tool to search for existing spreadsheets. Requires a Google OAuth token \
         with the spreadsheets scope."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    if !crate::near::agent::host::secret_exists("google_oauth_token") {
        return Err(
            "Google OAuth token not configured. Run `ironclaw tool auth google-sheets` to set up \
             OAuth, or set the GOOGLE_OAUTH_TOKEN environment variable."
                .to_string(),
        );
    }

    let action: GoogleSheetsAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Info,
        &format!("Executing Google Sheets action: {:?}", action),
    );

    let result = match action {
        GoogleSheetsAction::CreateSpreadsheet { title, sheet_names } => {
            let result = api::create_spreadsheet(&title, &sheet_names)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSheetsAction::GetSpreadsheet { spreadsheet_id } => {
            let result = api::get_spreadsheet(&spreadsheet_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSheetsAction::ReadValues {
            spreadsheet_id,
            range,
        } => {
            let result = api::read_values(&spreadsheet_id, &range)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSheetsAction::BatchReadValues {
            spreadsheet_id,
            ranges,
        } => {
            let result = api::batch_read_values(&spreadsheet_id, &ranges)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSheetsAction::WriteValues {
            spreadsheet_id,
            range,
            values,
            value_input_option,
        } => {
            let result = api::write_values(&spreadsheet_id, &range, &values, &value_input_option)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSheetsAction::AppendValues {
            spreadsheet_id,
            range,
            values,
            value_input_option,
        } => {
            let result = api::append_values(&spreadsheet_id, &range, &values, &value_input_option)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSheetsAction::ClearValues {
            spreadsheet_id,
            range,
        } => {
            let result = api::clear_values(&spreadsheet_id, &range)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSheetsAction::AddSheet {
            spreadsheet_id,
            title,
        } => {
            let result = api::add_sheet(&spreadsheet_id, &title)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSheetsAction::DeleteSheet {
            spreadsheet_id,
            sheet_id,
        } => {
            let result = api::delete_sheet(&spreadsheet_id, sheet_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSheetsAction::RenameSheet {
            spreadsheet_id,
            sheet_id,
            title,
        } => {
            let result = api::rename_sheet(&spreadsheet_id, sheet_id, &title)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GoogleSheetsAction::FormatCells {
            spreadsheet_id,
            sheet_id,
            start_row,
            end_row,
            start_column,
            end_column,
            bold,
            italic,
            font_size,
            text_color,
            background_color,
            horizontal_alignment,
            number_format,
            number_format_type,
        } => {
            let result = api::format_cells(api::FormatOptions {
                spreadsheet_id: &spreadsheet_id,
                sheet_id,
                start_row,
                end_row,
                start_column,
                end_column,
                bold,
                italic,
                font_size,
                text_color: text_color.as_deref(),
                background_color: background_color.as_deref(),
                horizontal_alignment: horizontal_alignment.as_deref(),
                number_format: number_format.as_deref(),
                number_format_type: number_format_type.as_deref(),
            })?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }
    };

    Ok(result)
}

export!(GoogleSheetsTool);
