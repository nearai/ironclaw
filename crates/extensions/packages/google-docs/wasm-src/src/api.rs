//! Google Docs API v1 implementation.
//!
//! All API calls go through the host's HTTP capability, which handles
//! credential injection and rate limiting. The WASM tool never sees
//! the actual OAuth token.

use crate::exports::near::agent::tool::{ErrorKind, GuestFailure};
use crate::near::agent::host;
use crate::types::*;

const DOCS_API_BASE: &str = "https://docs.googleapis.com/v1/documents";
const GOOGLE_API_AUTH_REQUIRED_ERROR: &str = "google_api_error_status_401";

/// Bound a free-text message to a sane length before it rides in a
/// `guest-failure`; the host re-bounds and scrubs downstream, but the guest
/// should never hand over an unbounded string in the first place.
pub(crate) fn bounded_message(message: &str) -> String {
    const MAX_MESSAGE_CHARS: usize = 512;
    message.chars().take(MAX_MESSAGE_CHARS).collect()
}

// ponytail: duplicated across isolated WASM guest packages; consolidate only
// when guests share a supported contracts crate.
fn transport_failure(failure: &host::HttpFailure) -> GuestFailure {
    let kind = match failure.kind {
        host::HttpErrorKind::AuthRequired => ErrorKind::AuthRequired,
        host::HttpErrorKind::Input => ErrorKind::Input,
        host::HttpErrorKind::OutputTooLarge => ErrorKind::OutputTooLarge,
        host::HttpErrorKind::Executor => ErrorKind::Executor,
        host::HttpErrorKind::NetworkDenied => ErrorKind::NetworkDenied,
        host::HttpErrorKind::Client => ErrorKind::Client,
        host::HttpErrorKind::OperationFailed => ErrorKind::OperationFailed,
    };
    GuestFailure {
        kind,
        code: failure
            .code
            .clone()
            .or_else(|| Some("google_api_transport_error".into())),
        message: failure.message.as_deref().map(bounded_message),
    }
}

/// A guest-side JSON encode/decode of an already-well-formed payload failed
/// — a tool-side processing failure, not anything the caller did wrong.
pub(crate) fn serialization_failure(error: &serde_json::Error) -> GuestFailure {
    GuestFailure {
        kind: ErrorKind::Executor,
        code: Some("serialization_failed".to_string()),
        message: Some(bounded_message(&error.to_string())),
    }
}

/// Build an `input`-kind guest failure for a caller-supplied argument that
/// fails local validation before any egress.
fn input_failure(code: &'static str, message: impl Into<String>) -> GuestFailure {
    GuestFailure {
        kind: ErrorKind::Input,
        code: Some(code.to_string()),
        message: Some(bounded_message(&message.into())),
    }
}

fn operation_failure(code: &'static str, message: impl Into<String>) -> GuestFailure {
    GuestFailure {
        kind: ErrorKind::OperationFailed,
        code: Some(code.to_string()),
        message: Some(bounded_message(&message.into())),
    }
}

fn failure_reason(failure: &GuestFailure) -> String {
    failure
        .message
        .clone()
        .or_else(|| failure.code.clone())
        .unwrap_or_else(|| "google_docs_operation_failed".to_string())
}

fn utf8_decode_failure(error: &std::string::FromUtf8Error) -> GuestFailure {
    GuestFailure {
        kind: ErrorKind::Executor,
        code: Some("invalid_utf8_response".to_string()),
        message: Some(bounded_message(&error.to_string())),
    }
}

/// Make a Google Docs API call.
fn api_call(method: &str, path: &str, body: Option<&str>) -> Result<String, GuestFailure> {
    let url = if path.is_empty() {
        DOCS_API_BASE.to_string()
    } else {
        format!("{}/{}", DOCS_API_BASE, path)
    };

    let headers = if body.is_some() {
        r#"{"Content-Type": "application/json"}"#
    } else {
        "{}"
    };

    let body_bytes = body.map(|b| b.as_bytes().to_vec());

    host::log(
        host::LogLevel::Debug,
        &format!("Google Docs API: {} {}", method, url),
    );

    let response = host::http_request(method, &url, headers, body_bytes.as_deref(), None)
        .map_err(|e| transport_failure(&e))?;

    if response.status < 200 || response.status >= 300 {
        return Err(api_status_error(
            "Google Docs",
            response.status,
            &response.body,
        ));
    }

    if response.body.is_empty() {
        return Ok(String::new());
    }

    String::from_utf8(response.body).map_err(|e| utf8_decode_failure(&e))
}

fn api_status_error(service: &str, status: u16, body: &[u8]) -> GuestFailure {
    if status == 401 {
        return GuestFailure {
            kind: ErrorKind::AuthRequired,
            code: Some(GOOGLE_API_AUTH_REQUIRED_ERROR.to_string()),
            message: None,
        };
    }
    let body_text = String::from_utf8_lossy(body);
    GuestFailure {
        kind: ErrorKind::Client,
        code: Some(format!("api_status_{status}")),
        message: Some(bounded_message(&format!(
            "{service} API returned status {status}: {body_text}"
        ))),
    }
}

/// Send a batchUpdate to the document and return the parsed response.
fn batch_update_raw(
    document_id: &str,
    requests: Vec<serde_json::Value>,
) -> Result<serde_json::Value, GuestFailure> {
    batch_update_raw_with_revision(document_id, requests, None)
}

fn batch_update_raw_with_revision(
    document_id: &str,
    requests: Vec<serde_json::Value>,
    required_revision_id: Option<&str>,
) -> Result<serde_json::Value, GuestFailure> {
    let path = format!("{}:batchUpdate", url_encode(document_id));

    let body = batch_update_body(requests, required_revision_id);
    let body_str = serde_json::to_string(&body).map_err(|e| serialization_failure(&e))?;

    let response = api_call("POST", &path, Some(&body_str))?;
    serde_json::from_str(&response).map_err(|e| serialization_failure(&e))
}

fn batch_update_body(
    requests: Vec<serde_json::Value>,
    required_revision_id: Option<&str>,
) -> serde_json::Value {
    let mut body = serde_json::json!({ "requests": requests });
    if let Some(revision_id) = required_revision_id.filter(|revision_id| !revision_id.is_empty()) {
        body["writeControl"] = serde_json::json!({
            "requiredRevisionId": revision_id,
        });
    }
    body
}

fn required_revision<'a>(
    document: &'a serde_json::Value,
    operation: &str,
) -> Result<&'a str, String> {
    document["revisionId"]
        .as_str()
        .filter(|revision_id| !revision_id.is_empty())
        .ok_or_else(|| {
            format!(
                "{operation} requires a document revision; the read response did not include revisionId"
            )
        })
}

/// Extract revision ID from a batchUpdate response.
fn extract_revision_id(parsed: &serde_json::Value) -> String {
    parsed["writeControl"]["requiredRevisionId"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn fetch_document(document_id: &str) -> Result<serde_json::Value, GuestFailure> {
    let path = format!("{}?includeTabsContent=true", url_encode(document_id));
    let response = api_call("GET", &path, None)?;
    let mut document: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| serialization_failure(&e))?;
    normalize_first_tab(&mut document);
    Ok(document)
}

fn normalize_first_tab(document: &mut serde_json::Value) {
    let Some(document_tab) = document["tabs"]
        .as_array()
        .and_then(|tabs| tabs.first())
        .and_then(|tab| tab.get("documentTab"))
        .cloned()
    else {
        return;
    };
    if let Some(body) = document_tab.get("body") {
        document["body"] = body.clone();
    }
    if let Some(named_ranges) = document_tab.get("namedRanges") {
        document["namedRanges"] = named_ranges.clone();
    }
}

fn first_tab_id(document: &serde_json::Value) -> Option<&str> {
    document["tabs"].as_array()?.first()?["tabProperties"]["tabId"].as_str()
}

/// Create a new document.
pub fn create_document(title: &str) -> Result<CreateDocumentResult, GuestFailure> {
    let body = serde_json::json!({ "title": title });
    let body_str = serde_json::to_string(&body).map_err(|e| serialization_failure(&e))?;

    let response = api_call("POST", "", Some(&body_str))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| serialization_failure(&e))?;

    Ok(CreateDocumentResult {
        document_id: parsed["documentId"].as_str().unwrap_or("").to_string(),
        title: parsed["title"].as_str().unwrap_or("").to_string(),
    })
}

/// Get document metadata.
pub fn get_document(document_id: &str) -> Result<DocumentMetadata, GuestFailure> {
    let parsed = fetch_document(document_id)?;

    // Calculate body length from the last element's endIndex
    let body_length = parsed["body"]["content"]
        .as_array()
        .and_then(|arr| arr.last())
        .and_then(|el| el["endIndex"].as_i64())
        .unwrap_or(1);

    // Extract named ranges
    let mut named_ranges = Vec::new();
    if let Some(nr_map) = parsed["namedRanges"].as_object() {
        for (_name, nr_group) in nr_map {
            if let Some(ranges) = nr_group["namedRanges"].as_array() {
                for nr in ranges {
                    let name = nr["name"].as_str().unwrap_or("").to_string();
                    let id = nr["namedRangeId"].as_str().unwrap_or("").to_string();
                    if let Some(range_list) = nr["ranges"].as_array() {
                        for range in range_list {
                            named_ranges.push(DocumentNamedRange {
                                name: name.clone(),
                                named_range_id: id.clone(),
                                start_index: range["startIndex"].as_i64().unwrap_or(0),
                                end_index: range["endIndex"].as_i64().unwrap_or(0),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(DocumentMetadata {
        document_id: parsed["documentId"].as_str().unwrap_or("").to_string(),
        title: parsed["title"].as_str().unwrap_or("").to_string(),
        revision_id: parsed["revisionId"].as_str().unwrap_or("").to_string(),
        body_length,
        named_ranges,
    })
}

/// Read the document body as plain text by walking the structural elements.
pub fn read_content(document_id: &str) -> Result<ReadContentResult, GuestFailure> {
    let parsed = fetch_document(document_id)?;

    let mut text = String::new();
    if let Some(content) = parsed["body"]["content"].as_array() {
        extract_text_from_elements(content, &mut text);
    }

    Ok(ReadContentResult {
        document_id: parsed["documentId"].as_str().unwrap_or("").to_string(),
        title: parsed["title"].as_str().unwrap_or("").to_string(),
        content: text,
    })
}

/// Inspect the document without flattening structural elements.
pub fn inspect_document(document_id: &str) -> Result<InspectDocumentResult, GuestFailure> {
    let parsed = fetch_document(document_id)?;
    Ok(parse_inspection(&parsed))
}

fn parse_inspection(document: &serde_json::Value) -> InspectDocumentResult {
    let elements = document["body"]["content"]
        .as_array()
        .map(|content| parse_structural_elements(content))
        .unwrap_or_default();
    let body_length = document["body"]["content"]
        .as_array()
        .and_then(|content| content.last())
        .and_then(|element| element["endIndex"].as_i64())
        .unwrap_or(1);

    InspectDocumentResult {
        document_id: document["documentId"].as_str().unwrap_or("").to_string(),
        title: document["title"].as_str().unwrap_or("").to_string(),
        revision_id: document["revisionId"].as_str().unwrap_or("").to_string(),
        body_length,
        elements,
    }
}

fn parse_structural_elements(elements: &[serde_json::Value]) -> Vec<DocumentElement> {
    let mut parsed = Vec::new();
    for element in elements {
        if let Some(paragraph) = element.get("paragraph") {
            let mut text = String::new();
            extract_text_from_elements(std::slice::from_ref(element), &mut text);
            parsed.push(DocumentElement::Paragraph(ParagraphElement {
                start_index: element["startIndex"].as_i64().unwrap_or(0),
                end_index: element["endIndex"].as_i64().unwrap_or(0),
                text,
                named_style: paragraph["paragraphStyle"]["namedStyleType"]
                    .as_str()
                    .map(str::to_string),
            }));
        } else if let Some(table) = element.get("table") {
            let rows = table["tableRows"]
                .as_array()
                .map(|table_rows| {
                    table_rows
                        .iter()
                        .map(|row| {
                            row["tableCells"]
                                .as_array()
                                .map(|cells| {
                                    cells
                                        .iter()
                                        .map(|cell| {
                                            let mut text = String::new();
                                            if let Some(content) = cell["content"].as_array() {
                                                extract_text_from_elements(content, &mut text);
                                            }
                                            TableCell {
                                                start_index: cell["startIndex"]
                                                    .as_i64()
                                                    .unwrap_or(0),
                                                end_index: cell["endIndex"].as_i64().unwrap_or(0),
                                                text,
                                            }
                                        })
                                        .collect()
                                })
                                .unwrap_or_default()
                        })
                        .collect()
                })
                .unwrap_or_default();
            parsed.push(DocumentElement::Table(TableElement {
                start_index: element["startIndex"].as_i64().unwrap_or(0),
                end_index: element["endIndex"].as_i64().unwrap_or(0),
                rows,
            }));
        }
    }
    parsed
}

/// Apply validated, text-anchored edits in one provider batch and read back the result.
pub fn apply_text_edits(
    document_id: &str,
    edits: &[AnchoredTextEdit],
) -> Result<ApplyTextEditsResult, GuestFailure> {
    if edits.is_empty() {
        return Err(input_failure(
            "invalid_parameters",
            "edits must contain at least one replacement",
        ));
    }
    if edits.len() > 100 {
        return Err(input_failure(
            "invalid_parameters",
            "edits may contain at most 100 replacements",
        ));
    }

    let before = fetch_document(document_id)?;
    let before_revision = required_revision(&before, "apply_text_edits")
        .map_err(|reason| operation_failure("missing_revision", reason))?;
    let before_text = document_text(&before);
    let (requests, expected_text) =
        build_anchored_edit_requests(&before_text, first_tab_id(&before), edits)
            .map_err(|reason| input_failure("invalid_parameters", reason))?;
    let updated = batch_update_raw_with_revision(document_id, requests, Some(before_revision))?;
    let occurrences_changed = updated["replies"]
        .as_array()
        .map(|replies| {
            replies
                .iter()
                .map(|reply| {
                    reply["replaceAllText"]["occurrencesChanged"]
                        .as_i64()
                        .unwrap_or(0)
                })
                .sum()
        })
        .unwrap_or(0);
    let after = fetch_document(document_id)?;

    Ok(ApplyTextEditsResult {
        document_id: after["documentId"]
            .as_str()
            .unwrap_or(document_id)
            .to_string(),
        revision_id: after["revisionId"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| extract_revision_id(&updated)),
        edits_applied: edits.len(),
        occurrences_changed,
        verified: document_text(&after) == expected_text,
    })
}

fn build_anchored_edit_requests(
    text: &str,
    tab_id: Option<&str>,
    edits: &[AnchoredTextEdit],
) -> Result<(Vec<serde_json::Value>, String), String> {
    let mut current = text.to_string();
    let mut requests = Vec::with_capacity(edits.len());
    for edit in edits {
        if edit.find.is_empty() {
            return Err("edit find text must not be empty".to_string());
        }
        if edit.find.len() > 10_000 || edit.replace.len() > 10_000 {
            return Err("edit find and replace text are limited to 10000 bytes".to_string());
        }
        if !edit.match_case && !edit.find.is_ascii() {
            return Err("case-insensitive semantic edits currently require ASCII text".to_string());
        }
        let occurrences = match_count(&current, &edit.find, edit.match_case);
        if occurrences == 0 {
            return Err(format!("edit anchor {:?} was not found", edit.find));
        }
        if !edit.replace_all && occurrences != 1 {
            return Err(format!(
                "edit anchor {:?} matched {occurrences} times; set replace_all=true or use a unique anchor",
                edit.find
            ));
        }

        let mut request = serde_json::json!({
            "replaceAllText": {
                "containsText": {
                    "text": edit.find,
                    "matchCase": edit.match_case,
                },
                "replaceText": edit.replace,
            }
        });
        if let Some(tab_id) = tab_id {
            request["replaceAllText"]["tabsCriteria"] = serde_json::json!({
                "tabIds": [tab_id],
            });
        }
        requests.push(request);
        current = replace_matches(&current, &edit.find, &edit.replace, edit.match_case);
    }
    Ok((requests, current))
}

fn match_count(text: &str, find: &str, match_case: bool) -> usize {
    if match_case {
        text.matches(find).count()
    } else {
        text.to_ascii_lowercase()
            .matches(&find.to_ascii_lowercase())
            .count()
    }
}

fn replace_matches(text: &str, find: &str, replace: &str, match_case: bool) -> String {
    if match_case {
        text.replace(find, replace)
    } else {
        let lower_text = text.to_ascii_lowercase();
        let lower_find = find.to_ascii_lowercase();
        let mut output = String::new();
        let mut cursor = 0;
        for (offset, _) in lower_text.match_indices(&lower_find) {
            let end = offset + find.len();
            let (Some(prefix), Some(_matched)) = (text.get(cursor..offset), text.get(offset..end))
            else {
                continue;
            };
            output.push_str(prefix);
            output.push_str(replace);
            cursor = end;
        }
        if let Some(suffix) = text.get(cursor..) {
            output.push_str(suffix);
        }
        output
    }
}

fn document_text(document: &serde_json::Value) -> String {
    let mut text = String::new();
    if let Some(content) = document["body"]["content"].as_array() {
        extract_text_from_elements(content, &mut text);
    }
    text
}

/// Recursively extract plain text from structural elements.
fn extract_text_from_elements(elements: &[serde_json::Value], out: &mut String) {
    for el in elements {
        // Paragraph
        if let Some(para) = el.get("paragraph") {
            if let Some(para_elements) = para["elements"].as_array() {
                for pe in para_elements {
                    if let Some(text_run) = pe.get("textRun") {
                        if let Some(content) = text_run["content"].as_str() {
                            out.push_str(content);
                        }
                    }
                }
            }
        }
        // Table: recurse into cells
        if let Some(table) = el.get("table") {
            if let Some(rows) = table["tableRows"].as_array() {
                for row in rows {
                    if let Some(cells) = row["tableCells"].as_array() {
                        for cell in cells {
                            if let Some(cell_content) = cell["content"].as_array() {
                                extract_text_from_elements(cell_content, out);
                            }
                        }
                    }
                }
            }
        }
        if let Some(table_of_contents) = el.get("tableOfContents") {
            if let Some(content) = table_of_contents["content"].as_array() {
                extract_text_from_elements(content, out);
            }
        }
    }
}

/// Insert text at a position.
pub fn insert_text(
    document_id: &str,
    text: &str,
    index: i64,
    segment_id: &str,
) -> Result<UpdateResult, GuestFailure> {
    let request = if index == -1 {
        // Append at end of segment
        let mut loc = serde_json::json!({});
        if !segment_id.is_empty() {
            loc["segmentId"] = serde_json::Value::String(segment_id.to_string());
        }
        serde_json::json!({
            "insertText": {
                "text": text,
                "endOfSegmentLocation": loc,
            }
        })
    } else if index < 0 {
        return Err(input_failure(
            "invalid_index",
            format!("invalid index {index}: only -1 (append) or non-negative indexes are accepted"),
        ));
    } else {
        let mut loc = serde_json::json!({ "index": index });
        if !segment_id.is_empty() {
            loc["segmentId"] = serde_json::Value::String(segment_id.to_string());
        }
        serde_json::json!({
            "insertText": {
                "text": text,
                "location": loc,
            }
        })
    };

    let parsed = batch_update_raw(document_id, vec![request])?;

    Ok(UpdateResult {
        document_id: parsed["documentId"].as_str().unwrap_or("").to_string(),
        revision_id: extract_revision_id(&parsed),
    })
}

/// Delete content in a range.
pub fn delete_content(
    document_id: &str,
    start_index: i64,
    end_index: i64,
    segment_id: &str,
) -> Result<UpdateResult, GuestFailure> {
    let mut range = serde_json::json!({
        "startIndex": start_index,
        "endIndex": end_index,
    });
    if !segment_id.is_empty() {
        range["segmentId"] = serde_json::Value::String(segment_id.to_string());
    }

    let request = serde_json::json!({
        "deleteContentRange": { "range": range }
    });

    let parsed = batch_update_raw(document_id, vec![request])?;

    Ok(UpdateResult {
        document_id: parsed["documentId"].as_str().unwrap_or("").to_string(),
        revision_id: extract_revision_id(&parsed),
    })
}

/// Find and replace all occurrences of text.
pub fn replace_text(
    document_id: &str,
    find: &str,
    replace: &str,
    match_case: bool,
) -> Result<ReplaceResult, GuestFailure> {
    let request = serde_json::json!({
        "replaceAllText": {
            "containsText": {
                "text": find,
                "matchCase": match_case,
            },
            "replaceText": replace,
        }
    });

    let parsed = batch_update_raw(document_id, vec![request])?;

    let first_reply = parsed["replies"].as_array().and_then(|arr| arr.first());
    let occurrences = first_reply
        .map(|r| {
            r["replaceAllText"]["occurrencesChanged"]
                .as_i64()
                .unwrap_or(0)
        })
        .unwrap_or(0);

    Ok(ReplaceResult {
        document_id: parsed["documentId"].as_str().unwrap_or("").to_string(),
        revision_id: extract_revision_id(&parsed),
        occurrences_changed: occurrences,
    })
}

/// Parse a hex color like "#FF0000" into Docs API color format.
fn parse_hex_color(hex: &str) -> Option<serde_json::Value> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(serde_json::json!({
        "color": {
            "rgbColor": {
                "red": r as f64 / 255.0,
                "green": g as f64 / 255.0,
                "blue": b as f64 / 255.0,
            }
        }
    }))
}

/// Parameters for text formatting.
pub struct FormatTextOptions<'a> {
    pub document_id: &'a str,
    pub start_index: i64,
    pub end_index: i64,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
    pub font_size: Option<f64>,
    pub font_family: Option<&'a str>,
    pub foreground_color: Option<&'a str>,
    pub background_color: Option<&'a str>,
}

/// Format text in a range.
pub fn format_text(opts: FormatTextOptions<'_>) -> Result<UpdateResult, GuestFailure> {
    let mut style = serde_json::json!({});
    let mut fields = Vec::new();

    if let Some(b) = opts.bold {
        style["bold"] = serde_json::Value::Bool(b);
        fields.push("bold");
    }
    if let Some(i) = opts.italic {
        style["italic"] = serde_json::Value::Bool(i);
        fields.push("italic");
    }
    if let Some(u) = opts.underline {
        style["underline"] = serde_json::Value::Bool(u);
        fields.push("underline");
    }
    if let Some(s) = opts.strikethrough {
        style["strikethrough"] = serde_json::Value::Bool(s);
        fields.push("strikethrough");
    }
    if let Some(size) = opts.font_size {
        style["fontSize"] = serde_json::json!({ "magnitude": size, "unit": "PT" });
        fields.push("fontSize");
    }
    if let Some(family) = opts.font_family {
        style["weightedFontFamily"] = serde_json::json!({ "fontFamily": family });
        fields.push("weightedFontFamily");
    }
    if let Some(color) = opts.foreground_color {
        let Some(c) = parse_hex_color(color) else {
            return Err(input_failure(
                "invalid_color",
                format!("invalid foreground_color hex: {color}"),
            ));
        };
        style["foregroundColor"] = c;
        fields.push("foregroundColor");
    }
    if let Some(color) = opts.background_color {
        let Some(c) = parse_hex_color(color) else {
            return Err(input_failure(
                "invalid_color",
                format!("invalid background_color hex: {color}"),
            ));
        };
        style["backgroundColor"] = c;
        fields.push("backgroundColor");
    }

    if fields.is_empty() {
        return Err(input_failure(
            "no_formatting_options",
            "No formatting options specified",
        ));
    }

    let request = serde_json::json!({
        "updateTextStyle": {
            "range": {
                "startIndex": opts.start_index,
                "endIndex": opts.end_index,
            },
            "textStyle": style,
            "fields": fields.join(","),
        }
    });

    let parsed = batch_update_raw(opts.document_id, vec![request])?;

    Ok(UpdateResult {
        document_id: parsed["documentId"].as_str().unwrap_or("").to_string(),
        revision_id: extract_revision_id(&parsed),
    })
}

/// Format paragraph style.
pub fn format_paragraph(
    document_id: &str,
    start_index: i64,
    end_index: i64,
    named_style: Option<&str>,
    alignment: Option<&str>,
    line_spacing: Option<f64>,
) -> Result<UpdateResult, GuestFailure> {
    let mut para_style = serde_json::json!({});
    let mut fields = Vec::new();

    if let Some(style) = named_style {
        para_style["namedStyleType"] = serde_json::Value::String(style.to_string());
        fields.push("namedStyleType");
    }
    if let Some(align) = alignment {
        para_style["alignment"] = serde_json::Value::String(align.to_string());
        fields.push("alignment");
    }
    if let Some(spacing) = line_spacing {
        para_style["lineSpacing"] = serde_json::json!(spacing);
        fields.push("lineSpacing");
    }

    if fields.is_empty() {
        return Err(input_failure(
            "no_paragraph_style_options",
            "No paragraph style options specified",
        ));
    }

    let request = serde_json::json!({
        "updateParagraphStyle": {
            "range": {
                "startIndex": start_index,
                "endIndex": end_index,
            },
            "paragraphStyle": para_style,
            "fields": fields.join(","),
        }
    });

    let parsed = batch_update_raw(document_id, vec![request])?;

    Ok(UpdateResult {
        document_id: parsed["documentId"].as_str().unwrap_or("").to_string(),
        revision_id: extract_revision_id(&parsed),
    })
}

/// Insert a table at a position.
pub fn insert_table(
    document_id: &str,
    rows: i64,
    columns: i64,
    index: i64,
) -> Result<UpdateResult, GuestFailure> {
    let request = serde_json::json!({
        "insertTable": {
            "rows": rows,
            "columns": columns,
            "location": { "index": index },
        }
    });

    let parsed = batch_update_raw(document_id, vec![request])?;

    Ok(UpdateResult {
        document_id: parsed["documentId"].as_str().unwrap_or("").to_string(),
        revision_id: extract_revision_id(&parsed),
    })
}

/// Insert a table, populate every cell, optionally style its header, and verify provider state.
pub fn create_table_with_data(
    document_id: &str,
    index: i64,
    table_data: &[Vec<String>],
    bold_header: bool,
) -> Result<CreateTableWithDataResult, GuestFailure> {
    if index < 1 {
        return Err(input_failure(
            "invalid_parameters",
            "table insertion index must be at least 1",
        ));
    }
    let columns = validate_table_data(table_data)
        .map_err(|reason| input_failure("invalid_parameters", reason))?;
    let rows = table_data.len();
    let preferred_table_index = preferred_inserted_table_index(index)
        .map_err(|reason| input_failure("invalid_parameters", reason))?;
    let populated_cells = table_data
        .iter()
        .flatten()
        .filter(|cell| !cell.is_empty())
        .count();
    let before = fetch_document(document_id)?;
    let before_revision = required_revision(&before, "create_table_with_data")
        .map_err(|reason| operation_failure("missing_revision", reason))?;
    let insert_response = batch_update_raw_with_revision(
        document_id,
        vec![serde_json::json!({
            "insertTable": {
                "rows": rows,
                "columns": columns,
                "location": { "index": index },
            }
        })],
        Some(before_revision),
    )?;

    let insert_revision = extract_revision_id(&insert_response);
    let mut latest_revision = insert_revision.clone();
    let result_document_id = before["documentId"]
        .as_str()
        .unwrap_or(document_id)
        .to_string();
    let inserted = match fetch_document(document_id) {
        Ok(document) => document,
        Err(reason) => {
            return Ok(partial_table_result(
                &result_document_id,
                &latest_revision,
                rows,
                columns,
                0,
                CreateTableStage::TableInserted,
                failure_reason(&reason),
            ));
        }
    };
    if let Some(revision_id) = inserted["revisionId"].as_str() {
        latest_revision = revision_id.to_string();
    }
    if let Err(reason) = validate_read_revision(&insert_revision, &inserted, "table insertion") {
        return Ok(partial_table_result(
            &result_document_id,
            &latest_revision,
            rows,
            columns,
            0,
            CreateTableStage::TableInserted,
            reason,
        ));
    }
    let table_index = preferred_table_index;
    let population_requests =
        match build_table_population_requests(&inserted, table_index, table_data) {
            Ok(requests) => requests,
            Err(reason) => {
                return Ok(partial_table_result(
                    &result_document_id,
                    &latest_revision,
                    rows,
                    columns,
                    0,
                    CreateTableStage::TableInserted,
                    reason,
                ));
            }
        };
    if !population_requests.is_empty() {
        let populated = match batch_update_raw_with_revision(
            document_id,
            population_requests,
            inserted["revisionId"].as_str(),
        ) {
            Ok(response) => response,
            Err(reason) => {
                return Ok(partial_table_result(
                    &result_document_id,
                    &latest_revision,
                    rows,
                    columns,
                    0,
                    CreateTableStage::TableInserted,
                    failure_reason(&reason),
                ));
            }
        };
        latest_revision = extract_revision_id(&populated);
    }

    let mut completed_stage = CreateTableStage::TablePopulated;
    if bold_header {
        let populated = match fetch_document(document_id) {
            Ok(document) => document,
            Err(reason) => {
                return Ok(partial_table_result(
                    &result_document_id,
                    &latest_revision,
                    rows,
                    columns,
                    populated_cells,
                    completed_stage,
                    failure_reason(&reason),
                ));
            }
        };
        let expected_revision = latest_revision.clone();
        if let Some(revision_id) = populated["revisionId"].as_str() {
            latest_revision = revision_id.to_string();
        }
        if let Err(reason) =
            validate_read_revision(&expected_revision, &populated, "table population")
        {
            return Ok(partial_table_result(
                &result_document_id,
                &latest_revision,
                rows,
                columns,
                populated_cells,
                completed_stage,
                reason,
            ));
        }
        let header_requests = match build_header_style_requests(&populated, table_index) {
            Ok(requests) => requests,
            Err(reason) => {
                return Ok(partial_table_result(
                    &result_document_id,
                    &latest_revision,
                    rows,
                    columns,
                    populated_cells,
                    completed_stage,
                    reason,
                ));
            }
        };
        if !header_requests.is_empty() {
            let styled = match batch_update_raw_with_revision(
                document_id,
                header_requests,
                populated["revisionId"].as_str(),
            ) {
                Ok(response) => response,
                Err(reason) => {
                    return Ok(partial_table_result(
                        &result_document_id,
                        &latest_revision,
                        rows,
                        columns,
                        populated_cells,
                        completed_stage,
                        failure_reason(&reason),
                    ));
                }
            };
            latest_revision = extract_revision_id(&styled);
        }
        completed_stage = CreateTableStage::HeaderStyled;
    }

    let verified_document = match fetch_document(document_id) {
        Ok(document) => document,
        Err(reason) => {
            return Ok(partial_table_result(
                &result_document_id,
                &latest_revision,
                rows,
                columns,
                populated_cells,
                completed_stage,
                failure_reason(&reason),
            ));
        }
    };
    let expected_revision = latest_revision.clone();
    if let Some(revision_id) = verified_document["revisionId"].as_str() {
        latest_revision = revision_id.to_string();
    }
    let last_mutation = if completed_stage == CreateTableStage::HeaderStyled {
        "header styling"
    } else {
        "table population"
    };
    if let Err(reason) =
        validate_read_revision(&expected_revision, &verified_document, last_mutation)
    {
        return Ok(partial_table_result(
            &result_document_id,
            &latest_revision,
            rows,
            columns,
            populated_cells,
            completed_stage,
            reason,
        ));
    }
    let verified = table_at_index(&verified_document, table_index)
        .is_some_and(|table_element| table_element_matches(table_element, table_data));

    Ok(CreateTableWithDataResult {
        document_id: verified_document["documentId"]
            .as_str()
            .unwrap_or(document_id)
            .to_string(),
        revision_id: latest_revision,
        rows,
        columns,
        populated_cells,
        verified,
        stage: CreateTableStage::Verified,
        failure: None,
    })
}

fn preferred_inserted_table_index(requested_index: i64) -> Result<i64, String> {
    requested_index
        .checked_add(1)
        .ok_or_else(|| "table insertion index is too large".to_string())
}

fn validate_read_revision(
    expected_revision: &str,
    document: &serde_json::Value,
    mutation: &str,
) -> Result<(), String> {
    if expected_revision.is_empty() {
        return Err(format!("{mutation} response did not include a revision ID"));
    }
    let actual_revision = document["revisionId"]
        .as_str()
        .ok_or_else(|| format!("document read after {mutation} did not include a revision ID"))?;
    if actual_revision != expected_revision {
        return Err(format!(
            "document revision changed after {mutation}: expected {expected_revision}, found {actual_revision}"
        ));
    }
    Ok(())
}

fn partial_table_result(
    document_id: &str,
    revision_id: &str,
    rows: usize,
    columns: usize,
    populated_cells: usize,
    stage: CreateTableStage,
    failure: String,
) -> CreateTableWithDataResult {
    CreateTableWithDataResult {
        document_id: document_id.to_string(),
        revision_id: revision_id.to_string(),
        rows,
        columns,
        populated_cells,
        verified: false,
        stage,
        failure: Some(failure),
    }
}

fn validate_table_data(table_data: &[Vec<String>]) -> Result<usize, String> {
    let Some(first_row) = table_data.first() else {
        return Err("table_data must contain at least one row".to_string());
    };
    if first_row.is_empty() {
        return Err("table_data rows must contain at least one cell".to_string());
    }
    if table_data.len() > 100 || first_row.len() > 20 {
        return Err("table_data is limited to 100 rows and 20 columns".to_string());
    }
    if table_data.iter().any(|row| row.len() != first_row.len()) {
        return Err("table_data must be rectangular".to_string());
    }
    if table_data.iter().flatten().any(|cell| cell.len() > 10_000) {
        return Err("table cells are limited to 10000 bytes".to_string());
    }
    Ok(first_row.len())
}

fn table_at_index(document: &serde_json::Value, index: i64) -> Option<&serde_json::Value> {
    document["body"]["content"]
        .as_array()?
        .iter()
        .find(|element| {
            element.get("table").is_some() && element["startIndex"].as_i64() == Some(index)
        })
}

fn build_table_population_requests(
    document: &serde_json::Value,
    index: i64,
    table_data: &[Vec<String>],
) -> Result<Vec<serde_json::Value>, String> {
    let table_element = table_at_index(document, index)
        .ok_or_else(|| format!("inserted table was not found at index {index}"))?;
    let table_rows = table_element["table"]["tableRows"]
        .as_array()
        .ok_or_else(|| "inserted table did not contain rows".to_string())?;
    if table_rows.len() != table_data.len() {
        return Err("inserted table row count did not match table_data".to_string());
    }

    let mut cells = Vec::new();
    for (row_index, expected_row) in table_data.iter().enumerate() {
        let provider_cells = table_rows[row_index]["tableCells"]
            .as_array()
            .ok_or_else(|| format!("inserted table row {row_index} did not contain cells"))?;
        if provider_cells.len() != expected_row.len() {
            return Err(format!(
                "inserted table row {row_index} column count did not match table_data"
            ));
        }
        for (column_index, text) in expected_row.iter().enumerate() {
            if text.is_empty() {
                continue;
            }
            let insertion_index = provider_cells[column_index]["content"]
                .as_array()
                .and_then(|content| content.first())
                .and_then(|paragraph| paragraph["startIndex"].as_i64())
                .ok_or_else(|| {
                    format!("table cell {row_index},{column_index} had no insertion index")
                })?;
            cells.push((insertion_index, text));
        }
    }
    cells.sort_by_key(|cell| std::cmp::Reverse(cell.0));
    Ok(cells
        .into_iter()
        .map(|(cell_index, text)| {
            serde_json::json!({
                "insertText": {
                    "location": { "index": cell_index },
                    "text": text,
                }
            })
        })
        .collect())
}

fn build_header_style_requests(
    document: &serde_json::Value,
    index: i64,
) -> Result<Vec<serde_json::Value>, String> {
    let table_element = table_at_index(document, index)
        .ok_or_else(|| format!("populated table was not found at index {index}"))?;
    let first_row = table_element["table"]["tableRows"]
        .as_array()
        .and_then(|rows| rows.first())
        .ok_or_else(|| "populated table did not contain a header row".to_string())?;
    let cells = first_row["tableCells"]
        .as_array()
        .ok_or_else(|| "populated table header did not contain cells".to_string())?;
    let mut requests = Vec::new();
    for cell in cells {
        let start_index = cell["content"]
            .as_array()
            .and_then(|content| content.first())
            .and_then(|paragraph| paragraph["startIndex"].as_i64())
            .ok_or_else(|| "header cell had no start index".to_string())?;
        let text = cell_text(cell);
        let text_length = text.trim_end_matches('\n').encode_utf16().count() as i64;
        if text_length > 0 {
            requests.push(serde_json::json!({
                "updateTextStyle": {
                    "range": {
                        "startIndex": start_index,
                        "endIndex": start_index + text_length,
                    },
                    "textStyle": { "bold": true },
                    "fields": "bold",
                }
            }));
        }
    }
    Ok(requests)
}

fn cell_text(cell: &serde_json::Value) -> String {
    let mut text = String::new();
    if let Some(content) = cell["content"].as_array() {
        extract_text_from_elements(content, &mut text);
    }
    text
}

fn table_matches(
    document: &serde_json::Value,
    table_index: usize,
    expected: &[Vec<String>],
) -> bool {
    let tables: Vec<&serde_json::Value> = document["body"]["content"]
        .as_array()
        .map(|content| {
            content
                .iter()
                .filter(|element| element.get("table").is_some())
                .collect()
        })
        .unwrap_or_default();
    let Some(table_element) = tables.get(table_index) else {
        return false;
    };
    table_element_matches(table_element, expected)
}

fn table_element_matches(table_element: &serde_json::Value, expected: &[Vec<String>]) -> bool {
    let Some(rows) = table_element["table"]["tableRows"].as_array() else {
        return false;
    };
    rows.len() == expected.len()
        && rows.iter().zip(expected).all(|(row, expected_row)| {
            row["tableCells"].as_array().is_some_and(|cells| {
                cells.len() == expected_row.len()
                    && cells.iter().zip(expected_row).all(|(cell, expected_cell)| {
                        cell_text(cell).trim_end_matches('\n') == expected_cell
                    })
            })
        })
}

/// Verify document expectations without mutating provider state.
pub fn verify_document(
    document_id: &str,
    expected_text: &[String],
    expected_tables: &[TableExpectation],
) -> Result<VerifyDocumentResult, GuestFailure> {
    let document = fetch_document(document_id)?;
    verify_parsed_document(&document, expected_text, expected_tables)
        .map_err(|reason| input_failure("invalid_parameters", reason))
}

fn verify_parsed_document(
    document: &serde_json::Value,
    expected_text: &[String],
    expected_tables: &[TableExpectation],
) -> Result<VerifyDocumentResult, String> {
    if expected_text.is_empty() && expected_tables.is_empty() {
        return Err("at least one document expectation is required".to_string());
    }
    if expected_text.len() > 100 || expected_tables.len() > 20 {
        return Err("verification is limited to 100 text and 20 table expectations".to_string());
    }
    if expected_text
        .iter()
        .any(|fragment| fragment.is_empty() || fragment.len() > 10_000)
    {
        return Err("expected text must contain 1 to 10000 bytes".to_string());
    }
    let text = document_text(document);
    let mut checks = Vec::with_capacity(expected_text.len() + expected_tables.len());
    for fragment in expected_text {
        checks.push(VerificationCheck {
            expectation: format!("document contains text {fragment:?}"),
            passed: text.contains(fragment),
        });
    }
    for (index, expectation) in expected_tables.iter().enumerate() {
        validate_table_data(&expectation.table_data)?;
        let table_index = expectation.table_index.unwrap_or(index);
        checks.push(VerificationCheck {
            expectation: format!("table {table_index} matches expected data"),
            passed: table_matches(document, table_index, &expectation.table_data),
        });
    }
    let verified = checks.iter().all(|check| check.passed);
    Ok(VerifyDocumentResult {
        document_id: document["documentId"].as_str().unwrap_or("").to_string(),
        revision_id: document["revisionId"].as_str().unwrap_or("").to_string(),
        verified,
        checks,
    })
}

/// Create a bulleted or numbered list from paragraphs in a range.
pub fn create_list(
    document_id: &str,
    start_index: i64,
    end_index: i64,
    bullet_preset: &str,
) -> Result<UpdateResult, GuestFailure> {
    let request = serde_json::json!({
        "createParagraphBullets": {
            "range": {
                "startIndex": start_index,
                "endIndex": end_index,
            },
            "bulletPreset": bullet_preset,
        }
    });

    let parsed = batch_update_raw(document_id, vec![request])?;

    Ok(UpdateResult {
        document_id: parsed["documentId"].as_str().unwrap_or("").to_string(),
        revision_id: extract_revision_id(&parsed),
    })
}

/// Execute a raw batch update with arbitrary requests.
pub fn batch_update(
    document_id: &str,
    requests: Vec<serde_json::Value>,
) -> Result<BatchUpdateResult, GuestFailure> {
    let parsed = batch_update_raw(document_id, requests)?;

    let replies = parsed["replies"]
        .as_array()
        .map(|arr| arr.to_vec())
        .unwrap_or_default();

    Ok(BatchUpdateResult {
        document_id: parsed["documentId"].as_str().unwrap_or("").to_string(),
        revision_id: extract_revision_id(&parsed),
        replies,
    })
}

fn url_encode(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_text_from_elements_includes_table_of_contents_content() {
        let elements = vec![serde_json::json!({
            "tableOfContents": {
                "content": [
                    {
                        "paragraph": {
                            "elements": [
                                { "textRun": { "content": "Heading 1\n" } }
                            ]
                        }
                    }
                ]
            }
        })];
        let mut text = String::new();

        extract_text_from_elements(&elements, &mut text);

        assert_eq!(text, "Heading 1\n");
    }

    #[test]
    fn api_status_error_401_maps_to_auth_required() {
        let err = api_status_error("Google Docs", 401, b"{\"error\":\"invalid_token\"}");

        assert_eq!(err.kind, ErrorKind::AuthRequired);
        assert_eq!(err.code.as_deref(), Some(GOOGLE_API_AUTH_REQUIRED_ERROR));
    }

    #[test]
    fn api_status_error_non_401_maps_to_client() {
        let err = api_status_error("Google Docs", 429, b"rate limited");

        assert_eq!(err.kind, ErrorKind::Client);
        assert_eq!(err.code.as_deref(), Some("api_status_429"));
        assert!(
            err.message
                .as_deref()
                .is_some_and(|message| message.contains("rate limited")),
            "{err:?}"
        );
    }

    #[test]
    fn insert_text_rejects_negative_indexes_other_than_append() {
        let err = insert_text("doc-1", "text", -2, "").unwrap_err();

        assert_eq!(err.kind, ErrorKind::Input);
        assert_eq!(err.code.as_deref(), Some("invalid_index"));
        assert!(
            err.message
                .as_deref()
                .is_some_and(|message| message.contains("invalid index -2")),
            "{err:?}"
        );
    }

    #[test]
    fn format_text_rejects_invalid_hex_colors() {
        let err = format_text(FormatTextOptions {
            document_id: "doc-1",
            start_index: 1,
            end_index: 2,
            bold: None,
            italic: None,
            underline: None,
            strikethrough: None,
            font_size: None,
            font_family: None,
            foreground_color: Some("#GG0000"),
            background_color: None,
        })
        .unwrap_err();

        assert_eq!(err.kind, ErrorKind::Input);
        assert_eq!(err.code.as_deref(), Some("invalid_color"));
        assert_eq!(
            err.message.as_deref(),
            Some("invalid foreground_color hex: #GG0000")
        );
    }

    #[test]
    fn parse_document_preserves_paragraph_and_table_structure() {
        let document = serde_json::json!({
            "documentId": "doc-1",
            "title": "Plan",
            "revisionId": "7",
            "body": { "content": [
                {
                    "startIndex": 1,
                    "endIndex": 7,
                    "paragraph": {
                        "paragraphStyle": { "namedStyleType": "HEADING_1" },
                        "elements": [{ "textRun": { "content": "Plan\n" } }]
                    }
                },
                {
                    "startIndex": 7,
                    "endIndex": 17,
                    "table": { "tableRows": [{ "tableCells": [
                        {
                            "startIndex": 8,
                            "endIndex": 12,
                            "content": [{ "paragraph": { "elements": [
                                { "textRun": { "content": "Owner\n" } }
                            ] } }]
                        },
                        {
                            "startIndex": 12,
                            "endIndex": 16,
                            "content": [{ "paragraph": { "elements": [
                                { "textRun": { "content": "Ada\n" } }
                            ] } }]
                        }
                    ] }] }
                }
            ] }
        });

        let inspected = parse_inspection(&document);
        let rendered = serde_json::to_value(inspected).unwrap();

        assert_eq!(rendered["elements"][0]["kind"], "paragraph");
        assert_eq!(rendered["elements"][0]["named_style"], "HEADING_1");
        assert_eq!(rendered["elements"][1]["kind"], "table");
        assert_eq!(rendered["elements"][1]["rows"][0][1]["text"], "Ada\n");
    }

    #[test]
    fn anchored_edit_rejects_ambiguous_text_unless_replace_all_is_explicit() {
        let edits = vec![AnchoredTextEdit {
            find: "owner".to_string(),
            replace: "lead".to_string(),
            replace_all: false,
            match_case: true,
        }];

        let err = build_anchored_edit_requests("owner and owner", None, &edits).unwrap_err();

        assert!(err.contains("matched 2 times"), "{err}");
    }

    #[test]
    fn anchored_edits_are_scoped_to_the_inspected_tab() {
        let edits = vec![AnchoredTextEdit {
            find: "owner".to_string(),
            replace: "lead".to_string(),
            replace_all: false,
            match_case: true,
        }];

        let (requests, _) = build_anchored_edit_requests("owner", Some("tab-1"), &edits).unwrap();

        assert_eq!(
            requests[0]["replaceAllText"]["tabsCriteria"]["tabIds"],
            serde_json::json!(["tab-1"])
        );
    }

    #[test]
    fn tabbed_documents_are_normalized_to_the_first_tab_for_semantic_reads() {
        let mut document = serde_json::json!({
            "tabs": [{
                "tabProperties": { "tabId": "tab-1" },
                "documentTab": { "body": { "content": [{
                    "paragraph": { "elements": [{
                        "textRun": { "content": "first tab" }
                    }] }
                }] } }
            }]
        });

        normalize_first_tab(&mut document);

        assert_eq!(first_tab_id(&document), Some("tab-1"));
        assert_eq!(document_text(&document), "first tab");
    }

    #[test]
    fn table_population_requests_are_emitted_from_last_cell_to_first() {
        let document = serde_json::json!({
            "body": { "content": [{
                "startIndex": 5,
                "endIndex": 20,
                "table": { "tableRows": [{ "tableCells": [
                    { "startIndex": 6, "endIndex": 10, "content": [{
                        "startIndex": 7, "paragraph": { "elements": [] }
                    }] },
                    { "startIndex": 10, "endIndex": 14, "content": [{
                        "startIndex": 11, "paragraph": { "elements": [] }
                    }] }
                ] }] }
            }] }
        });
        let data = vec![vec!["Owner".to_string(), "Ada".to_string()]];

        let requests = build_table_population_requests(&document, 5, &data).unwrap();

        assert_eq!(requests[0]["insertText"]["location"]["index"], 11);
        assert_eq!(requests[0]["insertText"]["text"], "Ada");
        assert_eq!(requests[1]["insertText"]["location"]["index"], 7);
    }

    #[test]
    fn inserted_table_uses_provider_index_after_leading_newline() {
        assert_eq!(preferred_inserted_table_index(5).unwrap(), 6);
        assert!(preferred_inserted_table_index(i64::MAX).is_err());
    }

    #[test]
    fn table_population_rejects_revision_drift_after_insertion() {
        let concurrent_document = serde_json::json!({ "revisionId": "3" });

        let error =
            validate_read_revision("2", &concurrent_document, "table insertion").unwrap_err();

        assert_eq!(
            error,
            "document revision changed after table insertion: expected 2, found 3"
        );
    }

    #[test]
    fn create_table_stage_round_trips_its_snake_case_wire_value() {
        let stage: CreateTableStage = serde_json::from_str("\"header_styled\"").unwrap();

        assert_eq!(stage, CreateTableStage::HeaderStyled);
        assert_eq!(serde_json::to_string(&stage).unwrap(), "\"header_styled\"");
    }

    #[test]
    fn verification_reports_each_failed_expectation_without_erroring() {
        let document = serde_json::json!({
            "documentId": "doc-1",
            "revisionId": "3",
            "body": { "content": [{
                "startIndex": 1,
                "endIndex": 5,
                "paragraph": { "elements": [{ "textRun": { "content": "Plan" } }] }
            }] }
        });
        let tables = vec![TableExpectation {
            table_index: None,
            table_data: vec![vec!["Owner".to_string()]],
        }];

        let result = verify_parsed_document(&document, &["Plan".to_string()], &tables).unwrap();

        assert!(!result.verified);
        assert_eq!(result.checks.len(), 2);
        assert!(result.checks[0].passed);
        assert!(!result.checks[1].passed);
    }

    #[test]
    fn verification_can_target_a_later_table_without_padding_expectations() {
        let table = |start_index: i64, value: &str| {
            serde_json::json!({
                "startIndex": start_index,
                "table": { "tableRows": [{ "tableCells": [{ "content": [{
                    "paragraph": { "elements": [{ "textRun": { "content": value } }] }
                }] }] }] }
            })
        };
        let document = serde_json::json!({
            "documentId": "doc-1",
            "revisionId": "3",
            "body": { "content": [table(2, "first\n"), table(8, "second\n")] }
        });
        let tables = vec![TableExpectation {
            table_index: Some(1),
            table_data: vec![vec!["second".to_string()]],
        }];

        let result = verify_parsed_document(&document, &[], &tables).unwrap();

        assert!(result.verified);
        assert_eq!(
            result.checks[0].expectation,
            "table 1 matches expected data"
        );
    }

    #[test]
    fn partial_table_result_reports_the_last_completed_stage() {
        let result = partial_table_result(
            "doc-1",
            "revision-2",
            2,
            2,
            0,
            CreateTableStage::TableInserted,
            "provider unavailable".to_string(),
        );

        assert!(!result.verified);
        assert_eq!(result.stage, CreateTableStage::TableInserted);
        assert_eq!(result.failure.as_deref(), Some("provider unavailable"));
    }

    #[test]
    fn semantic_input_schemas_bound_document_ids() {
        let schemas = [
            include_str!("../../schemas/google-docs/inspect_document.input.v1.json"),
            include_str!("../../schemas/google-docs/apply_text_edits.input.v1.json"),
            include_str!("../../schemas/google-docs/create_table_with_data.input.v1.json"),
            include_str!("../../schemas/google-docs/verify_document.input.v1.json"),
        ];

        for schema in schemas {
            let parsed: serde_json::Value = serde_json::from_str(schema).unwrap();
            assert_eq!(parsed["properties"]["document_id"]["maxLength"], 256);
        }
    }

    #[test]
    fn semantic_batch_updates_require_the_revision_that_was_inspected() {
        let body = batch_update_body(
            vec![serde_json::json!({"replaceAllText": {}})],
            Some("revision-7"),
        );

        assert_eq!(body["writeControl"]["requiredRevisionId"], "revision-7");
    }
}
