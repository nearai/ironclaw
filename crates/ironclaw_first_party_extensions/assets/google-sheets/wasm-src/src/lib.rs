//! Google Sheets WASM Tool for IronClaw.
//!
//! Provides Google Sheets integration for creating, reading, writing,
//! and formatting spreadsheets. Use Google Drive tool to search for
//! existing spreadsheets by name.
//!
//! # Capabilities Required
//!
//! - HTTP: `sheets.googleapis.com/v4/spreadsheets*`
//! - Credentials: staged Google product-auth account token injected by the host.
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

use types::{GoogleSheetsAction, ToolContext};

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../../../wit/tool.wit",
});

struct GoogleSheetsTool;

impl exports::near::agent::tool::Guest for GoogleSheetsTool {
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
        // Derived from `GoogleSheetsAction` via `schemars::JsonSchema` so the
        // advertised schema can never drift from the serde contract.
        let schema = schemars::schema_for!(types::GoogleSheetsAction);
        serde_json::to_string(&schema).unwrap_or_else(|_| "{}".to_string())
    }

    fn description() -> String {
        "Google Sheets integration for creating, reading, writing, and formatting spreadsheets. \
         Supports cell value operations (read, write, append, clear) using A1 notation, sheet \
         (tab) management (add, delete, rename), and cell formatting (bold, colors, alignment, \
         number formats). Spreadsheet IDs are the same as Google Drive file IDs, so use the \
         google-drive tool to search for existing spreadsheets. The host injects a Google \
         product-auth credential with the spreadsheets scope. \
         To discover all available API operations, use http GET to fetch \
         <https://www.googleapis.com/discovery/v1/apis/sheets/v4/rest> (public, no auth needed)."
            .to_string()
    }
}

fn execute_inner(params: &str, context: Option<&str>) -> Result<String, String> {
    let action_name = action_from_context(context)?;
    let params = params_with_action(params, action_name)?;
    let action: GoogleSheetsAction =
        serde_json::from_value(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Debug,
        &format!("Executing Google Sheets action: {action_name}"),
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

fn action_from_context(context: Option<&str>) -> Result<&'static str, String> {
    let context = context.ok_or_else(|| "missing_invocation_context".to_string())?;
    let context: ToolContext =
        serde_json::from_str(context).map_err(|_| "invalid_invocation_context".to_string())?;
    match context.capability_id.as_str() {
        "google-sheets.create_spreadsheet" => Ok("create_spreadsheet"),
        "google-sheets.get_spreadsheet" => Ok("get_spreadsheet"),
        "google-sheets.read_values" => Ok("read_values"),
        "google-sheets.batch_read_values" => Ok("batch_read_values"),
        "google-sheets.write_values" => Ok("write_values"),
        "google-sheets.append_values" => Ok("append_values"),
        "google-sheets.clear_values" => Ok("clear_values"),
        "google-sheets.add_sheet" => Ok("add_sheet"),
        "google-sheets.delete_sheet" => Ok("delete_sheet"),
        "google-sheets.rename_sheet" => Ok("rename_sheet"),
        "google-sheets.format_cells" => Ok("format_cells"),
        _ => Err("unsupported_google_sheets_capability".to_string()),
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

export!(GoogleSheetsTool);
