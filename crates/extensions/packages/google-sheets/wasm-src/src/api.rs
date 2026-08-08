//! Google Sheets API v4 implementation.
//!
//! All API calls go through the host's HTTP capability, which handles
//! credential injection and rate limiting. The WASM tool never sees
//! the actual OAuth token.

use crate::near::agent::host;
use crate::types::*;

const SHEETS_API_BASE: &str = "https://sheets.googleapis.com/v4/spreadsheets";
const GOOGLE_API_AUTH_REQUIRED_ERROR: &str = "google_api_error_status_401";
const MAX_PREVIEW_ROWS: usize = 100;
const MAX_PREVIEW_COLUMNS: usize = 30;

/// Make a Google Sheets API call.
fn api_call(method: &str, path: &str, body: Option<&str>) -> Result<String, String> {
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

    #[cfg(test)]
    if let Some(response) = TEST_API_RESPONSES.with(|queue| queue.borrow_mut().pop_front()) {
        TEST_API_CALLS.with(|calls| calls.borrow_mut().push((method.to_string(), url)));
        return response;
    }

    host::log(
        host::LogLevel::Debug,
        &format!("Google Sheets API: {} {}", method, url),
    );

    let response = host::http_request(method, &url, headers, body_bytes.as_deref(), None)?;

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

    String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 in response: {}", e))
}

fn api_status_error(service: &str, status: u16, body: &[u8]) -> String {
    if status == 401 {
        return serde_json::json!({
            "code": GOOGLE_API_AUTH_REQUIRED_ERROR,
            "kind": "auth_required",
        })
        .to_string();
    }
    let body_text = String::from_utf8_lossy(body);
    format!("{service} API returned status {status}: {body_text}")
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
) -> Result<CreateSpreadsheetResult, String> {
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

    let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let response = api_call("POST", "", Some(&body_str))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

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
pub fn get_spreadsheet(spreadsheet_id: &str) -> Result<SpreadsheetMetadata, String> {
    let path = format!(
        "{}?fields=spreadsheetId,properties.title,spreadsheetUrl,sheets.properties,namedRanges",
        url_encode(spreadsheet_id)
    );

    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

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
pub fn read_values(spreadsheet_id: &str, range: &str) -> Result<ValuesResult, String> {
    let path = format!(
        "{}/values/{}",
        url_encode(spreadsheet_id),
        url_encode(range)
    );

    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

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

/// Preview a bounded sheet range with headers and sample rows.
pub fn preview(
    spreadsheet_id: &str,
    sheet_name: Option<&str>,
    range: Option<&str>,
    max_rows: usize,
    max_columns: usize,
) -> Result<SheetPreviewResult, String> {
    let metadata = get_spreadsheet(spreadsheet_id)?;
    let selected_sheet = select_sheet(&metadata, sheet_name)?;
    let max_rows = max_rows.clamp(1, MAX_PREVIEW_ROWS);
    let max_columns = max_columns.clamp(1, MAX_PREVIEW_COLUMNS);
    let has_explicit_range = range.is_some();
    let range = match range {
        Some(range) => bound_explicit_preview_range(range, max_rows, max_columns)?,
        None => preview_range(&selected_sheet.title, max_rows, max_columns),
    };
    let values = read_values(spreadsheet_id, &range)?;
    let returned_range = values.range.clone();
    let effective_sheet = range_sheet_name(&returned_range)
        .or_else(|| range_sheet_name(&range))
        .map(|name| select_sheet(&metadata, Some(&name)))
        .transpose()?
        .unwrap_or(selected_sheet);
    let mut rows = values.values;
    let headers: Vec<String> = if has_explicit_range {
        Vec::new()
    } else {
        let headers = rows
            .first()
            .map(|row| row.iter().map(cell_to_string).collect())
            .unwrap_or_default();
        if !rows.is_empty() {
            rows.remove(0);
        }
        headers
    };
    let sampled_column_count = rows
        .iter()
        .map(Vec::len)
        .chain(std::iter::once(headers.len()))
        .max()
        .unwrap_or(0);
    let sampled_row_count = rows.len();

    Ok(SheetPreviewResult {
        spreadsheet_id: metadata.spreadsheet_id,
        title: metadata.title,
        url: metadata.url,
        sheet_name: effective_sheet.title,
        range,
        row_count_estimate: effective_sheet.row_count,
        column_count_estimate: effective_sheet.column_count,
        headers,
        rows,
        sampled_row_count,
        sampled_column_count,
    })
}

fn select_sheet(
    metadata: &SpreadsheetMetadata,
    sheet_name: Option<&str>,
) -> Result<SheetInfo, String> {
    if let Some(name) = sheet_name {
        return metadata
            .sheets
            .iter()
            .find(|sheet| sheet.title == name)
            .cloned()
            .ok_or_else(|| format!("sheet_not_found: {name}"));
    }
    metadata
        .sheets
        .first()
        .cloned()
        .ok_or_else(|| "spreadsheet_has_no_sheets".to_string())
}

fn preview_range(sheet_name: &str, max_rows: usize, max_columns: usize) -> String {
    format!(
        "{}!A1:{}{}",
        quote_sheet_name(sheet_name),
        column_name(max_columns),
        max_rows
    )
}

fn range_sheet_name(range: &str) -> Option<String> {
    let (sheet, _) = range.split_once('!')?;
    if sheet.is_empty() {
        return None;
    }
    if let Some(quoted) = sheet.strip_prefix('\'').and_then(|value| value.strip_suffix('\'')) {
        return Some(quoted.replace("''", "'"));
    }
    Some(sheet.to_string())
}

fn quote_sheet_name(sheet_name: &str) -> String {
    format!("'{}'", sheet_name.replace('\'', "''"))
}

/// A parsed A1 cell reference: 0-based column/row indices.
struct CellRef {
    col: usize,
    row: usize,
}

fn parse_cell_ref(reference: &str) -> Option<CellRef> {
    let reference = reference.trim();
    let split = reference
        .find(|character: char| character.is_ascii_digit())
        .unwrap_or(reference.len());
    if split == 0 || split == reference.len() {
        // "A" (whole column) and "1" (whole row) cannot be bounded.
        return None;
    }
    let (letters, digits) = reference.split_at(split);
    if !letters.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    let mut col = 0usize;
    for byte in letters.bytes() {
        col = col
            .checked_mul(26)
            .and_then(|value| value.checked_add(usize::from(byte.to_ascii_uppercase() - b'A' + 1)))
            .and_then(|value| value.checked_sub(1))?;
    }
    let row = digits.parse::<usize>().ok()?.checked_sub(1)?;
    Some(CellRef { col, row })
}

/// Clamp an explicit A1 preview range to `max_rows` x `max_columns` so the
/// compact-preview contract holds even when the caller names a huge range.
/// Unbounded forms (whole columns/rows, open-ended ranges) are rejected: the
/// tool cannot promise a bounded preview it cannot bound.
fn bound_explicit_preview_range(
    range: &str,
    max_rows: usize,
    max_columns: usize,
) -> Result<String, String> {
    let (sheet_prefix, cells) = match range.split_once('!') {
        Some((prefix, cells)) => (Some(prefix), cells),
        None => (None, range),
    };
    let (start_ref, end_ref) = match cells.split_once(':') {
        Some((start, end)) => (start, Some(end)),
        None => (cells, None),
    };
    let start = parse_cell_ref(start_ref)
        .ok_or_else(|| format!("unbounded preview range: {range}"))?;
    let Some(end_ref) = end_ref else {
        // A single-cell reference is already bounded.
        return Ok(range.to_string());
    };
    let end = parse_cell_ref(end_ref)
        .ok_or_else(|| format!("unbounded preview range: {range}"))?;
    let max_end_col = start.col.saturating_add(max_columns.saturating_sub(1));
    let max_end_row = start.row.saturating_add(max_rows.saturating_sub(1));
    let end_col = end.col.min(max_end_col);
    let end_row = end.row.min(max_end_row);
    if end.col == end_col && end.row == end_row {
        return Ok(range.to_string());
    }
    let prefix = sheet_prefix.map(|prefix| format!("{prefix}!")).unwrap_or_default();
    Ok(format!(
        "{prefix}{}{}:{}{}",
        column_name(start.col + 1),
        start.row + 1,
        column_name(end_col + 1),
        end_row + 1,
    ))
}

#[cfg(test)]
thread_local! {
    static TEST_API_RESPONSES: std::cell::RefCell<std::collections::VecDeque<Result<String, String>>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    static TEST_API_CALLS: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn stub_api_responses(responses: Vec<Result<String, String>>) {
    TEST_API_RESPONSES.with(|queue| *queue.borrow_mut() = responses.into());
    TEST_API_CALLS.with(|calls| calls.borrow_mut().clear());
}

#[cfg(test)]
fn take_test_api_calls() -> Vec<(String, String)> {
    TEST_API_CALLS.with(|calls| std::mem::take(&mut *calls.borrow_mut()))
}

fn column_name(mut one_based_column: usize) -> String {
    let mut name = String::new();
    while one_based_column > 0 {
        let rem = (one_based_column - 1) % 26;
        name.insert(0, (b'A' + rem as u8) as char);
        one_based_column = (one_based_column - 1) / 26;
    }
    name
}

fn cell_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Null => String::new(),
        value => value.to_string(),
    }
}

/// Read values from multiple ranges at once.
pub fn batch_read_values(
    spreadsheet_id: &str,
    ranges: &[String],
) -> Result<BatchValuesResult, String> {
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
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

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
) -> Result<UpdateResult, String> {
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

    let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let response = api_call("PUT", &path, Some(&body_str))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

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
) -> Result<UpdateResult, String> {
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

    let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let response = api_call("POST", &path, Some(&body_str))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    let updates = &parsed["updates"];
    Ok(UpdateResult {
        updated_range: updates["updatedRange"].as_str().unwrap_or("").to_string(),
        updated_rows: updates["updatedRows"].as_i64().unwrap_or(0),
        updated_columns: updates["updatedColumns"].as_i64().unwrap_or(0),
        updated_cells: updates["updatedCells"].as_i64().unwrap_or(0),
    })
}

/// Clear values from a range.
pub fn clear_values(spreadsheet_id: &str, range: &str) -> Result<ClearResult, String> {
    let path = format!(
        "{}/values/{}:clear",
        url_encode(spreadsheet_id),
        url_encode(range)
    );

    let response = api_call("POST", &path, Some("{}"))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(ClearResult {
        cleared_range: parsed["clearedRange"].as_str().unwrap_or("").to_string(),
    })
}

/// Send a batchUpdate request to the spreadsheet.
fn batch_update(
    spreadsheet_id: &str,
    requests: Vec<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let path = format!("{}:batchUpdate", url_encode(spreadsheet_id));

    let body = serde_json::json!({ "requests": requests });
    let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;

    let response = api_call("POST", &path, Some(&body_str))?;
    serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))
}

/// Add a new sheet (tab) to the spreadsheet.
pub fn add_sheet(spreadsheet_id: &str, title: &str) -> Result<AddSheetResult, String> {
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

    let reply = reply.ok_or_else(|| "No reply from batch update".to_string())?;

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
pub fn delete_sheet(spreadsheet_id: &str, sheet_id: i64) -> Result<SheetOperationResult, String> {
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
) -> Result<SheetOperationResult, String> {
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
pub fn format_cells(opts: FormatOptions<'_>) -> Result<FormatResult, String> {
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
        return Err("No formatting options specified".to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata_response() -> String {
        serde_json::json!({
            "spreadsheetId": "sheet-1",
            "properties": {"title": "Preview sheet"},
            "sheets": [
                {
                    "properties": {
                        "sheetId": 0,
                        "title": "Data",
                        "gridProperties": {"rowCount": 1000, "columnCount": 100}
                    }
                }
            ]
        })
        .to_string()
    }

    fn values_response(range: &str) -> String {
        serde_json::json!({
            "range": range,
            "values": [
                ["h1", "h2", "h3"],
                ["a", "b", "c"],
                ["d", "e", "f"]
            ]
        })
        .to_string()
    }

    #[test]
    fn explicit_oversized_range_is_clamped_to_preview_bounds() {
        // Metadata call, then the read_values call.
        stub_api_responses(vec![
            Ok(metadata_response()),
            Ok(values_response("Data!A1:Z1000")),
        ]);

        let result = preview("sheet-1", None, Some("Data!A1:Z1000"), 2, 2).unwrap();

        let calls = take_test_api_calls();
        assert_eq!(calls.len(), 2);
        assert!(
            calls[1].1.contains("Data%21A1%3AB2"),
            "explicit range must be clamped to 2x2, got: {}",
            calls[1].1
        );
        assert_eq!(result.range, "Data!A1:B2");
        assert_eq!(result.sampled_row_count, 3);
    }

    #[test]
    fn explicit_in_bounds_range_is_passed_through_unchanged() {
        stub_api_responses(vec![
            Ok(metadata_response()),
            Ok(values_response("Data!A1:C3")),
        ]);

        let result = preview("sheet-1", None, Some("Data!A1:C3"), 10, 10).unwrap();

        assert_eq!(result.range, "Data!A1:C3");
    }

    #[test]
    fn unbounded_explicit_range_is_rejected_before_the_read() {
        // Each attempt still needs its metadata call; the unbounded range must
        // fail before any read_values egress (one call per attempt, no more).
        stub_api_responses(vec![
            Ok(metadata_response()),
            Ok(metadata_response()),
            Ok(metadata_response()),
            Ok(metadata_response()),
        ]);

        for range in ["Data!A:A", "Data!1:5", "Data!A1:B", "Data!A"] {
            let error = preview("sheet-1", None, Some(range), 10, 10).unwrap_err();
            assert!(
                error.contains("unbounded preview range"),
                "{range}: expected rejection, got {error}"
            );
        }
        assert_eq!(take_test_api_calls().len(), 4);
    }

    #[test]
    fn single_cell_explicit_range_is_allowed() {
        stub_api_responses(vec![
            Ok(metadata_response()),
            Ok(values_response("Data!B2:B2")),
        ]);

        let result = preview("sheet-1", None, Some("Data!B2"), 10, 10).unwrap();

        assert_eq!(result.range, "Data!B2");
    }
}
