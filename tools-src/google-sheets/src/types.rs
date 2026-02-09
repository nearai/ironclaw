//! Types for Google Sheets API requests and responses.

use serde::{Deserialize, Serialize};

/// Input parameters for the Google Sheets tool.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GoogleSheetsAction {
    /// Create a new spreadsheet.
    CreateSpreadsheet {
        /// Spreadsheet title.
        title: String,
        /// Names of sheets (tabs) to create. Defaults to one sheet named "Sheet1".
        #[serde(default)]
        sheet_names: Vec<String>,
    },

    /// Get spreadsheet metadata (title, sheets, named ranges).
    GetSpreadsheet {
        /// The spreadsheet ID (same as Google Drive file ID).
        spreadsheet_id: String,
    },

    /// Read cell values from a range.
    ReadValues {
        /// The spreadsheet ID.
        spreadsheet_id: String,
        /// A1 notation range (e.g., "Sheet1!A1:D10", "A1:B5").
        range: String,
    },

    /// Read values from multiple ranges at once.
    BatchReadValues {
        /// The spreadsheet ID.
        spreadsheet_id: String,
        /// List of A1 notation ranges.
        ranges: Vec<String>,
    },

    /// Write values to a range (overwrites existing data).
    WriteValues {
        /// The spreadsheet ID.
        spreadsheet_id: String,
        /// A1 notation range (e.g., "Sheet1!A1:D10").
        range: String,
        /// 2D array of values (rows of columns).
        values: Vec<Vec<serde_json::Value>>,
        /// How to interpret input: "RAW" or "USER_ENTERED" (default).
        #[serde(default = "default_value_input_option")]
        value_input_option: String,
    },

    /// Append rows after existing data in a range.
    AppendValues {
        /// The spreadsheet ID.
        spreadsheet_id: String,
        /// A1 notation range to search for a table (e.g., "Sheet1!A:E").
        range: String,
        /// Rows to append (2D array).
        values: Vec<Vec<serde_json::Value>>,
        /// How to interpret input: "RAW" or "USER_ENTERED" (default).
        #[serde(default = "default_value_input_option")]
        value_input_option: String,
    },

    /// Clear values from a range (keeps formatting).
    ClearValues {
        /// The spreadsheet ID.
        spreadsheet_id: String,
        /// A1 notation range to clear.
        range: String,
    },

    /// Add a new sheet (tab) to the spreadsheet.
    AddSheet {
        /// The spreadsheet ID.
        spreadsheet_id: String,
        /// Name for the new sheet.
        title: String,
    },

    /// Delete a sheet (tab) from the spreadsheet.
    DeleteSheet {
        /// The spreadsheet ID.
        spreadsheet_id: String,
        /// Numeric sheet ID (from get_spreadsheet, NOT the sheet name).
        sheet_id: i64,
    },

    /// Rename a sheet (tab).
    RenameSheet {
        /// The spreadsheet ID.
        spreadsheet_id: String,
        /// Numeric sheet ID.
        sheet_id: i64,
        /// New name for the sheet.
        title: String,
    },

    /// Format cells in a range (bold, colors, number format, borders, alignment).
    FormatCells {
        /// The spreadsheet ID.
        spreadsheet_id: String,
        /// Numeric sheet ID.
        sheet_id: i64,
        /// Start row (0-indexed, inclusive).
        start_row: i64,
        /// End row (0-indexed, exclusive).
        end_row: i64,
        /// Start column (0-indexed, inclusive).
        start_column: i64,
        /// End column (0-indexed, exclusive).
        end_column: i64,
        /// Bold text.
        #[serde(default)]
        bold: Option<bool>,
        /// Italic text.
        #[serde(default)]
        italic: Option<bool>,
        /// Font size.
        #[serde(default)]
        font_size: Option<i64>,
        /// Text color as hex (e.g., "#FF0000").
        #[serde(default)]
        text_color: Option<String>,
        /// Background color as hex (e.g., "#FFFF00").
        #[serde(default)]
        background_color: Option<String>,
        /// Horizontal alignment: "LEFT", "CENTER", "RIGHT".
        #[serde(default)]
        horizontal_alignment: Option<String>,
        /// Number format pattern (e.g., "#,##0.00", "yyyy-mm-dd").
        #[serde(default)]
        number_format: Option<String>,
        /// Number format type: "NUMBER", "CURRENCY", "PERCENT", "DATE", "TIME", "TEXT".
        #[serde(default)]
        number_format_type: Option<String>,
    },
}

fn default_value_input_option() -> String {
    "USER_ENTERED".to_string()
}

/// Sheet (tab) info within a spreadsheet.
#[derive(Debug, Serialize)]
pub struct SheetInfo {
    pub sheet_id: i64,
    pub title: String,
    pub index: i64,
    pub row_count: i64,
    pub column_count: i64,
}

/// Named range within a spreadsheet.
#[derive(Debug, Serialize)]
pub struct NamedRange {
    pub named_range_id: String,
    pub name: String,
    pub range: String,
}

/// Result from create_spreadsheet.
#[derive(Debug, Serialize)]
pub struct CreateSpreadsheetResult {
    pub spreadsheet_id: String,
    pub title: String,
    pub url: String,
    pub sheets: Vec<SheetInfo>,
}

/// Result from get_spreadsheet.
#[derive(Debug, Serialize)]
pub struct SpreadsheetMetadata {
    pub spreadsheet_id: String,
    pub title: String,
    pub url: String,
    pub sheets: Vec<SheetInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub named_ranges: Vec<NamedRange>,
}

/// Result from read_values.
#[derive(Debug, Serialize)]
pub struct ValuesResult {
    pub range: String,
    pub values: Vec<Vec<serde_json::Value>>,
}

/// Result from batch_read_values.
#[derive(Debug, Serialize)]
pub struct BatchValuesResult {
    pub value_ranges: Vec<ValuesResult>,
}

/// Result from write_values or append_values.
#[derive(Debug, Serialize)]
pub struct UpdateResult {
    pub updated_range: String,
    pub updated_rows: i64,
    pub updated_columns: i64,
    pub updated_cells: i64,
}

/// Result from clear_values.
#[derive(Debug, Serialize)]
pub struct ClearResult {
    pub cleared_range: String,
}

/// Result from add_sheet.
#[derive(Debug, Serialize)]
pub struct AddSheetResult {
    pub sheet: SheetInfo,
}

/// Result from delete_sheet or rename_sheet.
#[derive(Debug, Serialize)]
pub struct SheetOperationResult {
    pub spreadsheet_id: String,
    pub success: bool,
}

/// Result from format_cells.
#[derive(Debug, Serialize)]
pub struct FormatResult {
    pub spreadsheet_id: String,
    pub success: bool,
}
