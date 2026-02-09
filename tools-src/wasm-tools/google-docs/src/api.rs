//! Google Docs API v1 implementation.
//!
//! All API calls go through the host's HTTP capability, which handles
//! credential injection and rate limiting. The WASM tool never sees
//! the actual OAuth token.

use crate::near::agent::host;
use crate::types::*;

const DOCS_API_BASE: &str = "https://docs.googleapis.com/v1/documents";

/// Make a Google Docs API call.
fn api_call(method: &str, path: &str, body: Option<&str>) -> Result<String, String> {
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

    let response = host::http_request(method, &url, headers, body_bytes.as_deref())?;

    if response.status < 200 || response.status >= 300 {
        let body_text = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Google Docs API returned status {}: {}",
            response.status, body_text
        ));
    }

    if response.body.is_empty() {
        return Ok(String::new());
    }

    String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 in response: {}", e))
}

/// Send a batchUpdate to the document and return the parsed response.
fn batch_update_raw(
    document_id: &str,
    requests: Vec<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let path = format!("{}:batchUpdate", url_encode(document_id));

    let body = serde_json::json!({ "requests": requests });
    let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;

    let response = api_call("POST", &path, Some(&body_str))?;
    serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))
}

/// Extract revision ID from a batchUpdate response.
fn extract_revision_id(parsed: &serde_json::Value) -> String {
    parsed["writeControl"]["requiredRevisionId"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

/// Create a new document.
pub fn create_document(title: &str) -> Result<CreateDocumentResult, String> {
    let body = serde_json::json!({ "title": title });
    let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;

    let response = api_call("POST", "", Some(&body_str))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(CreateDocumentResult {
        document_id: parsed["documentId"].as_str().unwrap_or("").to_string(),
        title: parsed["title"].as_str().unwrap_or("").to_string(),
    })
}

/// Get document metadata.
pub fn get_document(document_id: &str) -> Result<DocumentMetadata, String> {
    let path = url_encode(document_id);

    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

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
pub fn read_content(document_id: &str) -> Result<ReadContentResult, String> {
    let path = url_encode(document_id);

    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

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
    }
}

/// Insert text at a position.
pub fn insert_text(
    document_id: &str,
    text: &str,
    index: i64,
    segment_id: &str,
) -> Result<UpdateResult, String> {
    let request = if index < 0 {
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
) -> Result<UpdateResult, String> {
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
) -> Result<ReplaceResult, String> {
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

    let occurrences = parsed["replies"][0]["replaceAllText"]["occurrencesChanged"]
        .as_i64()
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
pub fn format_text(opts: FormatTextOptions<'_>) -> Result<UpdateResult, String> {
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
        if let Some(c) = parse_hex_color(color) {
            style["foregroundColor"] = c;
            fields.push("foregroundColor");
        }
    }
    if let Some(color) = opts.background_color {
        if let Some(c) = parse_hex_color(color) {
            style["backgroundColor"] = c;
            fields.push("backgroundColor");
        }
    }

    if fields.is_empty() {
        return Err("No formatting options specified".to_string());
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
) -> Result<UpdateResult, String> {
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
        return Err("No paragraph style options specified".to_string());
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
) -> Result<UpdateResult, String> {
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

/// Create a bulleted or numbered list from paragraphs in a range.
pub fn create_list(
    document_id: &str,
    start_index: i64,
    end_index: i64,
    bullet_preset: &str,
) -> Result<UpdateResult, String> {
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
) -> Result<BatchUpdateResult, String> {
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

/// Minimal percent-encoding for URL path segments.
fn url_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b as char);
            }
            _ => {
                encoded.push('%');
                encoded.push(char::from(HEX[(b >> 4) as usize]));
                encoded.push(char::from(HEX[(b & 0x0F) as usize]));
            }
        }
    }
    encoded
}

const HEX: [u8; 16] = *b"0123456789ABCDEF";
