//! Google Sheets API v4 implementation.
//!
//! All API calls go through the host's HTTP capability, which handles
//! credential injection and rate limiting. The WASM tool never sees
//! the actual OAuth token.

use crate::exports::near::agent::tool::{ErrorKind, GuestFailure};
use crate::near::agent::host;
use crate::types::*;

const SHEETS_API_BASE: &str = "https://sheets.googleapis.com/v4/spreadsheets";
const GOOGLE_API_AUTH_REQUIRED_ERROR: &str = "google_api_error_status_401";

/// Bound a free-text message to a sane length before it rides in a
/// `guest-failure`; the host re-bounds and scrubs downstream, but the guest
/// should never hand over an unbounded string in the first place.
pub(crate) fn bounded_message(message: &str) -> String {
    const MAX_MESSAGE_CHARS: usize = 512;
    message.chars().take(MAX_MESSAGE_CHARS).collect()
}

/// A guest-side JSON encode/decode of an already-well-formed payload failed
/// — a tool-side processing failure, not anything the caller did wrong.
pub(crate) fn serialization_failure(error: &serde_json::Error) -> GuestFailure {
    GuestFailure {
        kind: ErrorKind::Executor,
        code: Some("serialization_failed".to_string()),
        message: Some(bounded_message(&error.to_string())),
        retry_after_ms: None,
    }
}

/// A host-level transport failure (network denial, timeout, credential
/// unavailable, ...): the guest has no closed vocabulary for the host's
/// free-text transport message, so it rides the `operation_failed` bucket
/// with the raw text bounded as `message`.
fn transport_failure(message: &str) -> GuestFailure {
    GuestFailure {
        kind: ErrorKind::OperationFailed,
        code: Some("google_api_transport_error".to_string()),
        message: Some(bounded_message(message)),
        retry_after_ms: None,
    }
}

/// Build an `input`-kind guest failure for a caller-supplied argument that
/// fails local validation before any egress.
fn input_failure(code: &'static str, message: impl Into<String>) -> GuestFailure {
    GuestFailure {
        kind: ErrorKind::Input,
        code: Some(code.to_string()),
        message: Some(bounded_message(&message.into())),
        retry_after_ms: None,
    }
}

/// Make a Google Sheets API call.
fn api_call(method: &str, path: &str, body: Option<&str>) -> Result<String, GuestFailure> {
    let url = if path.is_empty() {
        SHEETS_API_BASE.to_string()
    } else {
        format!("{}/{}", SHEETS_API_BASE, path)
    };

    let headers = if body.is_some() {
        r#"{"Content-Type": "application/json"}"#
    } else {
        "{}"
    };

    let body_bytes = body.map(|b| b.as_bytes().to_vec());

    host::log(
        host::LogLevel::Debug,
        &format!("Google Sheets API: {} {}", method, url),
    );

    let response = host::http_request(method, &url, headers, body_bytes.as_deref(), None)
        .map_err(|e| transport_failure(&e))?;

    if response.status < 200 || response.status >= 300 {
        return Err(api_status_error(
            "Google Sheets",
            response.status,
            &response.body,
        ));
    }

    if response.body.is_empty() {
        return Ok(String::new());
    }

    String::from_utf8(response.body).map_err(|e| GuestFailure {
        kind: ErrorKind::Executor,
        code: Some("invalid_utf8_response".to_string()),
        message: Some(bounded_message(&e.to_string())),
        retry_after_ms: None,
    })
}

fn api_status_error(service: &str, status: u16, body: &[u8]) -> GuestFailure {
    if status == 401 {
        return GuestFailure {
            kind: ErrorKind::AuthRequired,
            code: Some(GOOGLE_API_AUTH_REQUIRED_ERROR.to_string()),
            message: None,
            retry_after_ms: None,
        };
    }
    let body_text = String::from_utf8_lossy(body);
    GuestFailure {
        kind: ErrorKind::Client,
        code: Some(format!("api_status_{status}")),
        message: Some(bounded_message(&format!(
            "{service} API returned status {status}: {body_text}"
        ))),
        retry_after_ms: None,
    }
}

/// Parse sheet info from the API's JSON.
fn parse_sheet_info(v: &serde_json::Value) -> SheetInfo {
    let props = &v["properties"];
    let grid = &props["gridProperties"];
    SheetInfo {
        sheet_id: props["sheetId"].as_i64().unwrap_or(0),
        title: props["title"].as_str().unwrap_or("").to_string(),
        index: props["index"].as_i64().unwrap_or(0),
        row_count: grid["rowCount"].as_i64().unwrap_or(0),
        column_count: grid["columnCount"].as_i64().unwrap_or(0),
    }
}

/// Parse a named range from the API's JSON.
fn parse_named_range(v: &serde_json::Value) -> NamedRange {
    let range = &v["range"];
    let range_str = format_grid_range(range);
    NamedRange {
        named_range_id: v["namedRangeId"].as_str().unwrap_or("").to_string(),
        name: v["name"].as_str().unwrap_or("").to_string(),
        range: range_str,
    }
}

/// Format a GridRange into a human-readable string.
fn format_grid_range(v: &serde_json::Value) -> String {
    let sheet_id = v["sheetId"].as_i64().unwrap_or(0);
    let start_row = v["startRowIndex"].as_i64().unwrap_or(0);
    let end_row = v["endRowIndex"].as_i64().unwrap_or(0);
    let start_col = v["startColumnIndex"].as_i64().unwrap_or(0);
    let end_col = v["endColumnIndex"].as_i64().unwrap_or(0);
    format!(
        "sheetId={}, rows {}:{}, cols {}:{}",
        sheet_id, start_row, end_row, start_col, end_col
    )
}

/// Create a new spreadsheet.
pub fn create_spreadsheet(
    title: &str,
    sheet_names: &[String],
) -> Result<CreateSpreadsheetResult, GuestFailure> {
    let sheets: Vec<serde_json::Value> = if sheet_names.is_empty() {
        vec![serde_json::json!({"properties": {"title": "Sheet1"}})]
    } else {
        sheet_names
            .iter()
            .map(|name| serde_json::json!({"properties": {"title": name}}))
            .collect()
    };

    let body = serde_json::json!({
        "properties": {"title": title},
        "sheets": sheets,
    });

    let body_str = serde_json::to_string(&body).map_err(|e| serialization_failure(&e))?;
    let response = api_call("POST", "", Some(&body_str))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| serialization_failure(&e))?;

    Ok(CreateSpreadsheetResult {
        spreadsheet_id: parsed["spreadsheetId"].as_str().unwrap_or("").to_string(),
        title: parsed["properties"]["title"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        url: parsed["spreadsheetUrl"].as_str().unwrap_or("").to_string(),
        sheets: parsed["sheets"]
            .as_array()
            .map(|arr| arr.iter().map(parse_sheet_info).collect())
            .unwrap_or_default(),
    })
}

/// Get spreadsheet metadata.
pub fn get_spreadsheet(spreadsheet_id: &str) -> Result<SpreadsheetMetadata, GuestFailure> {
    let path = format!(
        "{}?fields=spreadsheetId,properties.title,spreadsheetUrl,sheets.properties,namedRanges",
        url_encode(spreadsheet_id)
    );

    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| serialization_failure(&e))?;

    Ok(SpreadsheetMetadata {
        spreadsheet_id: parsed["spreadsheetId"].as_str().unwrap_or("").to_string(),
        title: parsed["properties"]["title"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        url: parsed["spreadsheetUrl"].as_str().unwrap_or("").to_string(),
        sheets: parsed["sheets"]
            .as_array()
            .map(|arr| arr.iter().map(parse_sheet_info).collect())
            .unwrap_or_default(),
        named_ranges: parsed["namedRanges"]
            .as_array()
            .map(|arr| arr.iter().map(parse_named_range).collect())
            .unwrap_or_default(),
    })
}

/// Read values from a single range.
pub fn read_values(spreadsheet_id: &str, range: &str) -> Result<ValuesResult, GuestFailure> {
    let path = format!(
        "{}/values/{}",
        url_encode(spreadsheet_id),
        url_encode(range)
    );

    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| serialization_failure(&e))?;

    Ok(ValuesResult {
        range: parsed["range"].as_str().unwrap_or("").to_string(),
        values: parsed["values"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| row.as_array().map(|cols| cols.to_vec()).unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Read values from multiple ranges at once.
pub fn batch_read_values(
    spreadsheet_id: &str,
    ranges: &[String],
) -> Result<BatchValuesResult, GuestFailure> {
    let range_params: Vec<String> = ranges
        .iter()
        .map(|r| format!("ranges={}", url_encode(r)))
        .collect();

    let path = format!(
        "{}/values:batchGet?{}",
        url_encode(spreadsheet_id),
        range_params.join("&")
    );

    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| serialization_failure(&e))?;

    let value_ranges = parsed["valueRanges"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|vr| ValuesResult {
                    range: vr["range"].as_str().unwrap_or("").to_string(),
                    values: vr["values"]
                        .as_array()
                        .map(|rows| {
                            rows.iter()
                                .map(|row| {
                                    row.as_array().map(|cols| cols.to_vec()).unwrap_or_default()
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(BatchValuesResult { value_ranges })
}

/// Write values to a range.
pub fn write_values(
    spreadsheet_id: &str,
    range: &str,
    values: &[Vec<serde_json::Value>],
    value_input_option: &str,
) -> Result<UpdateResult, GuestFailure> {
    let path = format!(
        "{}/values/{}?valueInputOption={}",
        url_encode(spreadsheet_id),
        url_encode(range),
        url_encode(value_input_option)
    );

    let body = serde_json::json!({
        "range": range,
        "majorDimension": "ROWS",
        "values": values,
    });

    let body_str = serde_json::to_string(&body).map_err(|e| serialization_failure(&e))?;
    let response = api_call("PUT", &path, Some(&body_str))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| serialization_failure(&e))?;

    Ok(UpdateResult {
        updated_range: parsed["updatedRange"].as_str().unwrap_or("").to_string(),
        updated_rows: parsed["updatedRows"].as_i64().unwrap_or(0),
        updated_columns: parsed["updatedColumns"].as_i64().unwrap_or(0),
        updated_cells: parsed["updatedCells"].as_i64().unwrap_or(0),
    })
}

/// Append rows after existing data.
pub fn append_values(
    spreadsheet_id: &str,
    range: &str,
    values: &[Vec<serde_json::Value>],
    value_input_option: &str,
) -> Result<UpdateResult, GuestFailure> {
    let path = format!(
        "{}/values/{}:append?valueInputOption={}&insertDataOption=INSERT_ROWS",
        url_encode(spreadsheet_id),
        url_encode(range),
        url_encode(value_input_option)
    );

    let body = serde_json::json!({
        "range": range,
        "majorDimension": "ROWS",
        "values": values,
    });

    let body_str = serde_json::to_string(&body).map_err(|e| serialization_failure(&e))?;
    let response = api_call("POST", &path, Some(&body_str))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| serialization_failure(&e))?;

    let updates = &parsed["updates"];
    Ok(UpdateResult {
        updated_range: updates["updatedRange"].as_str().unwrap_or("").to_string(),
        updated_rows: updates["updatedRows"].as_i64().unwrap_or(0),
        updated_columns: updates["updatedColumns"].as_i64().unwrap_or(0),
        updated_cells: updates["updatedCells"].as_i64().unwrap_or(0),
    })
}

/// Clear values from a range.
pub fn clear_values(spreadsheet_id: &str, range: &str) -> Result<ClearResult, GuestFailure> {
    let path = format!(
        "{}/values/{}:clear",
        url_encode(spreadsheet_id),
        url_encode(range)
    );

    let response = api_call("POST", &path, Some("{}"))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| serialization_failure(&e))?;

    Ok(ClearResult {
        cleared_range: parsed["clearedRange"].as_str().unwrap_or("").to_string(),
    })
}

/// Send a batchUpdate request to the spreadsheet.
fn batch_update(
    spreadsheet_id: &str,
    requests: Vec<serde_json::Value>,
) -> Result<serde_json::Value, GuestFailure> {
    let path = format!("{}:batchUpdate", url_encode(spreadsheet_id));

    let body = serde_json::json!({ "requests": requests });
    let body_str = serde_json::to_string(&body).map_err(|e| serialization_failure(&e))?;

    let response = api_call("POST", &path, Some(&body_str))?;
    serde_json::from_str(&response).map_err(|e| serialization_failure(&e))
}

/// Add a new sheet (tab) to the spreadsheet.
pub fn add_sheet(spreadsheet_id: &str, title: &str) -> Result<AddSheetResult, GuestFailure> {
    let requests = vec![serde_json::json!({
        "addSheet": {
            "properties": {
                "title": title
            }
        }
    })];

    let parsed = batch_update(spreadsheet_id, requests)?;

    let reply = parsed["replies"]
        .as_array()
        .and_then(|arr| arr.first())
        .map(|r| &r["addSheet"]["properties"]);

    let reply = reply.ok_or_else(|| GuestFailure {
        kind: ErrorKind::Executor,
        code: Some("batch_update_missing_reply".to_string()),
        message: Some("No reply from batch update".to_string()),
        retry_after_ms: None,
    })?;

    Ok(AddSheetResult {
        sheet: SheetInfo {
            sheet_id: reply["sheetId"].as_i64().unwrap_or(0),
            title: reply["title"].as_str().unwrap_or("").to_string(),
            index: reply["index"].as_i64().unwrap_or(0),
            row_count: reply["gridProperties"]["rowCount"].as_i64().unwrap_or(1000),
            column_count: reply["gridProperties"]["columnCount"]
                .as_i64()
                .unwrap_or(26),
        },
    })
}

/// Delete a sheet (tab) from the spreadsheet.
pub fn delete_sheet(
    spreadsheet_id: &str,
    sheet_id: i64,
) -> Result<SheetOperationResult, GuestFailure> {
    let requests = vec![serde_json::json!({
        "deleteSheet": {
            "sheetId": sheet_id
        }
    })];

    batch_update(spreadsheet_id, requests)?;

    Ok(SheetOperationResult {
        spreadsheet_id: spreadsheet_id.to_string(),
        success: true,
    })
}

/// Rename a sheet (tab).
pub fn rename_sheet(
    spreadsheet_id: &str,
    sheet_id: i64,
    title: &str,
) -> Result<SheetOperationResult, GuestFailure> {
    let requests = vec![serde_json::json!({
        "updateSheetProperties": {
            "properties": {
                "sheetId": sheet_id,
                "title": title
            },
            "fields": "title"
        }
    })];

    batch_update(spreadsheet_id, requests)?;

    Ok(SheetOperationResult {
        spreadsheet_id: spreadsheet_id.to_string(),
        success: true,
    })
}

/// Parse a hex color like "#FF0000" into Sheets API color (0.0-1.0 floats).
fn parse_hex_color(hex: &str) -> Option<serde_json::Value> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(serde_json::json!({
        "red": r as f64 / 255.0,
        "green": g as f64 / 255.0,
        "blue": b as f64 / 255.0,
    }))
}

/// Parameters for cell formatting.
pub struct FormatOptions<'a> {
    pub spreadsheet_id: &'a str,
    pub sheet_id: i64,
    pub start_row: i64,
    pub end_row: i64,
    pub start_column: i64,
    pub end_column: i64,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub font_size: Option<i64>,
    pub text_color: Option<&'a str>,
    pub background_color: Option<&'a str>,
    pub horizontal_alignment: Option<&'a str>,
    pub number_format: Option<&'a str>,
    pub number_format_type: Option<&'a str>,
}

/// Format cells in a range.
pub fn format_cells(opts: FormatOptions<'_>) -> Result<FormatResult, GuestFailure> {
    let mut format = serde_json::json!({});
    let mut fields = Vec::new();

    // Text format
    let mut text_format = serde_json::json!({});
    let mut has_text_format = false;

    if let Some(b) = opts.bold {
        text_format["bold"] = serde_json::Value::Bool(b);
        has_text_format = true;
    }
    if let Some(i) = opts.italic {
        text_format["italic"] = serde_json::Value::Bool(i);
        has_text_format = true;
    }
    if let Some(size) = opts.font_size {
        text_format["fontSize"] = serde_json::json!(size);
        has_text_format = true;
    }
    if let Some(color) = opts.text_color {
        if let Some(c) = parse_hex_color(color) {
            text_format["foregroundColor"] = c;
            has_text_format = true;
        }
    }

    if has_text_format {
        format["textFormat"] = text_format;
        fields.push("userEnteredFormat.textFormat");
    }

    // Background color
    if let Some(color) = opts.background_color {
        if let Some(c) = parse_hex_color(color) {
            format["backgroundColor"] = c;
            fields.push("userEnteredFormat.backgroundColor");
        }
    }

    // Horizontal alignment
    if let Some(align) = opts.horizontal_alignment {
        format["horizontalAlignment"] = serde_json::Value::String(align.to_string());
        fields.push("userEnteredFormat.horizontalAlignment");
    }

    // Number format
    if let Some(pattern) = opts.number_format {
        let fmt_type = opts.number_format_type.unwrap_or("NUMBER");
        format["numberFormat"] = serde_json::json!({
            "type": fmt_type,
            "pattern": pattern,
        });
        fields.push("userEnteredFormat.numberFormat");
    }

    if fields.is_empty() {
        return Err(input_failure(
            "no_formatting_options",
            "No formatting options specified",
        ));
    }

    let requests = vec![serde_json::json!({
        "repeatCell": {
            "range": {
                "sheetId": opts.sheet_id,
                "startRowIndex": opts.start_row,
                "endRowIndex": opts.end_row,
                "startColumnIndex": opts.start_column,
                "endColumnIndex": opts.end_column,
            },
            "cell": {
                "userEnteredFormat": format,
            },
            "fields": fields.join(","),
        }
    })];

    batch_update(opts.spreadsheet_id, requests)?;

    Ok(FormatResult {
        spreadsheet_id: opts.spreadsheet_id.to_string(),
        success: true,
    })
}

fn url_encode(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}
