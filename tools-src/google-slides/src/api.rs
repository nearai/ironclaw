//! Google Slides API v1 implementation.
//!
//! All API calls go through the host's HTTP capability, which handles
//! credential injection and rate limiting. The WASM tool never sees
//! the actual OAuth token.

use crate::near::agent::host;
use crate::types::*;

const SLIDES_API_BASE: &str = "https://slides.googleapis.com/v1/presentations";

/// Make a Google Slides API call.
fn api_call(method: &str, path: &str, body: Option<&str>) -> Result<String, String> {
    let url = if path.is_empty() {
        SLIDES_API_BASE.to_string()
    } else {
        format!("{}/{}", SLIDES_API_BASE, path)
    };

    let headers = if body.is_some() {
        r#"{"Content-Type": "application/json"}"#
    } else {
        "{}"
    };

    let body_bytes = body.map(|b| b.as_bytes().to_vec());

    host::log(
        host::LogLevel::Debug,
        &format!("Google Slides API: {} {}", method, url),
    );

    let response = host::http_request(method, &url, headers, body_bytes.as_deref(), None)?;

    if response.status < 200 || response.status >= 300 {
        let body_text = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Google Slides API returned status {}: {}",
            response.status, body_text
        ));
    }

    if response.body.is_empty() {
        return Ok(String::new());
    }

    String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 in response: {}", e))
}

/// Send a batchUpdate to the presentation.
fn batch_update_raw(
    presentation_id: &str,
    requests: Vec<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let path = format!("{}:batchUpdate", url_encode(presentation_id));

    let body = serde_json::json!({ "requests": requests });
    let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;

    let response = api_call("POST", &path, Some(&body_str))?;
    serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))
}

/// Extract text content from a shape's textElements array.
fn extract_text_from_shape(shape: &serde_json::Value) -> Option<String> {
    let text_elements = shape["text"]["textElements"].as_array()?;
    let mut text = String::new();
    for el in text_elements {
        if let Some(content) = el["textRun"]["content"].as_str() {
            text.push_str(content);
        }
    }
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Parse a page element into ElementInfo.
fn parse_element(el: &serde_json::Value) -> ElementInfo {
    let object_id = el["objectId"].as_str().unwrap_or("").to_string();

    let (element_type, text_content, placeholder_type) = if el.get("shape").is_some() {
        let pt = el["shape"]["placeholder"]["type"]
            .as_str()
            .map(|s| s.to_string());
        let text = extract_text_from_shape(&el["shape"]);
        ("shape".to_string(), text, pt)
    } else if el.get("image").is_some() {
        ("image".to_string(), None, None)
    } else if el.get("table").is_some() {
        ("table".to_string(), None, None)
    } else if el.get("line").is_some() {
        ("line".to_string(), None, None)
    } else if el.get("video").is_some() {
        ("video".to_string(), None, None)
    } else if el.get("elementGroup").is_some() {
        ("group".to_string(), None, None)
    } else {
        ("unknown".to_string(), None, None)
    };

    ElementInfo {
        object_id,
        element_type,
        text_content,
        placeholder_type,
    }
}

/// Create a new presentation.
pub fn create_presentation(title: &str) -> Result<CreatePresentationResult, String> {
    let body = serde_json::json!({ "title": title });
    let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;

    let response = api_call("POST", "", Some(&body_str))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(CreatePresentationResult {
        presentation_id: parsed["presentationId"].as_str().unwrap_or("").to_string(),
        title: parsed["title"].as_str().unwrap_or("").to_string(),
    })
}

/// Get presentation metadata and slides.
pub fn get_presentation(presentation_id: &str) -> Result<PresentationMetadata, String> {
    let path = url_encode(presentation_id);

    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    let slides: Vec<SlideInfo> = parsed["slides"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|slide| {
                    let elements = slide["pageElements"]
                        .as_array()
                        .map(|els| els.iter().map(parse_element).collect())
                        .unwrap_or_default();

                    SlideInfo {
                        object_id: slide["objectId"].as_str().unwrap_or("").to_string(),
                        layout_object_id: slide["slideProperties"]["layoutObjectId"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                        elements,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let slide_count = slides.len();

    Ok(PresentationMetadata {
        presentation_id: parsed["presentationId"].as_str().unwrap_or("").to_string(),
        title: parsed["title"].as_str().unwrap_or("").to_string(),
        revision_id: parsed["revisionId"].as_str().unwrap_or("").to_string(),
        slide_count,
        slides,
    })
}

/// Get a thumbnail URL for a slide.
pub fn get_thumbnail(
    presentation_id: &str,
    slide_object_id: &str,
) -> Result<ThumbnailResult, String> {
    let path = format!(
        "{}/pages/{}/thumbnail",
        url_encode(presentation_id),
        url_encode(slide_object_id)
    );

    let response = api_call("GET", &path, None)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(ThumbnailResult {
        content_url: parsed["contentUrl"].as_str().unwrap_or("").to_string(),
        width: parsed["width"].as_i64().unwrap_or(0),
        height: parsed["height"].as_i64().unwrap_or(0),
    })
}

/// Create a new slide.
pub fn create_slide(
    presentation_id: &str,
    insertion_index: Option<i64>,
    layout: &str,
) -> Result<UpdateResult, String> {
    let mut request = serde_json::json!({
        "createSlide": {
            "slideLayoutReference": {
                "predefinedLayout": layout,
            }
        }
    });

    if let Some(idx) = insertion_index {
        request["createSlide"]["insertionIndex"] = serde_json::json!(idx);
    }

    let parsed = batch_update_raw(presentation_id, vec![request])?;

    let created_id = parsed["replies"][0]["createSlide"]["objectId"]
        .as_str()
        .map(|s| s.to_string());

    Ok(UpdateResult {
        presentation_id: parsed["presentationId"].as_str().unwrap_or("").to_string(),
        created_object_id: created_id,
    })
}

/// Delete a slide or page element.
pub fn delete_object(presentation_id: &str, object_id: &str) -> Result<UpdateResult, String> {
    let request = serde_json::json!({
        "deleteObject": { "objectId": object_id }
    });

    let parsed = batch_update_raw(presentation_id, vec![request])?;

    Ok(UpdateResult {
        presentation_id: parsed["presentationId"].as_str().unwrap_or("").to_string(),
        created_object_id: None,
    })
}

/// Insert text into a shape.
pub fn insert_text(
    presentation_id: &str,
    object_id: &str,
    text: &str,
    insertion_index: i64,
) -> Result<UpdateResult, String> {
    let request = serde_json::json!({
        "insertText": {
            "objectId": object_id,
            "text": text,
            "insertionIndex": insertion_index,
        }
    });

    let parsed = batch_update_raw(presentation_id, vec![request])?;

    Ok(UpdateResult {
        presentation_id: parsed["presentationId"].as_str().unwrap_or("").to_string(),
        created_object_id: None,
    })
}

/// Delete text from a shape.
pub fn delete_text(
    presentation_id: &str,
    object_id: &str,
    start_index: i64,
    end_index: Option<i64>,
) -> Result<UpdateResult, String> {
    let text_range = if let Some(end) = end_index {
        serde_json::json!({
            "type": "FIXED_RANGE",
            "startIndex": start_index,
            "endIndex": end,
        })
    } else {
        serde_json::json!({
            "type": "FROM_START_INDEX",
            "startIndex": start_index,
        })
    };

    let request = serde_json::json!({
        "deleteText": {
            "objectId": object_id,
            "textRange": text_range,
        }
    });

    let parsed = batch_update_raw(presentation_id, vec![request])?;

    Ok(UpdateResult {
        presentation_id: parsed["presentationId"].as_str().unwrap_or("").to_string(),
        created_object_id: None,
    })
}

/// Find and replace text across the presentation.
pub fn replace_all_text(
    presentation_id: &str,
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

    let parsed = batch_update_raw(presentation_id, vec![request])?;

    let occurrences = parsed["replies"][0]["replaceAllText"]["occurrencesChanged"]
        .as_i64()
        .unwrap_or(0);

    Ok(ReplaceResult {
        presentation_id: parsed["presentationId"].as_str().unwrap_or("").to_string(),
        occurrences_changed: occurrences,
    })
}

/// Points to EMU (English Metric Units). 1 point = 12700 EMU.
fn pt_to_emu(pt: f64) -> f64 {
    pt * 12700.0
}

/// Create a shape on a slide.
pub fn create_shape(
    presentation_id: &str,
    slide_object_id: &str,
    shape_type: &str,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<UpdateResult, String> {
    let request = serde_json::json!({
        "createShape": {
            "shapeType": shape_type,
            "elementProperties": {
                "pageObjectId": slide_object_id,
                "size": {
                    "width": { "magnitude": pt_to_emu(width), "unit": "EMU" },
                    "height": { "magnitude": pt_to_emu(height), "unit": "EMU" },
                },
                "transform": {
                    "scaleX": 1.0,
                    "scaleY": 1.0,
                    "shearX": 0.0,
                    "shearY": 0.0,
                    "translateX": pt_to_emu(x),
                    "translateY": pt_to_emu(y),
                    "unit": "EMU",
                },
            },
        }
    });

    let parsed = batch_update_raw(presentation_id, vec![request])?;

    let created_id = parsed["replies"][0]["createShape"]["objectId"]
        .as_str()
        .map(|s| s.to_string());

    Ok(UpdateResult {
        presentation_id: parsed["presentationId"].as_str().unwrap_or("").to_string(),
        created_object_id: created_id,
    })
}

/// Insert an image on a slide.
pub fn insert_image(
    presentation_id: &str,
    slide_object_id: &str,
    image_url: &str,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<UpdateResult, String> {
    let request = serde_json::json!({
        "createImage": {
            "url": image_url,
            "elementProperties": {
                "pageObjectId": slide_object_id,
                "size": {
                    "width": { "magnitude": pt_to_emu(width), "unit": "EMU" },
                    "height": { "magnitude": pt_to_emu(height), "unit": "EMU" },
                },
                "transform": {
                    "scaleX": 1.0,
                    "scaleY": 1.0,
                    "shearX": 0.0,
                    "shearY": 0.0,
                    "translateX": pt_to_emu(x),
                    "translateY": pt_to_emu(y),
                    "unit": "EMU",
                },
            },
        }
    });

    let parsed = batch_update_raw(presentation_id, vec![request])?;

    let created_id = parsed["replies"][0]["createImage"]["objectId"]
        .as_str()
        .map(|s| s.to_string());

    Ok(UpdateResult {
        presentation_id: parsed["presentationId"].as_str().unwrap_or("").to_string(),
        created_object_id: created_id,
    })
}

/// Parse a hex color like "#FF0000" into Slides API color format.
fn parse_hex_color(hex: &str) -> Option<serde_json::Value> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(serde_json::json!({
        "opaqueColor": {
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
    pub presentation_id: &'a str,
    pub object_id: &'a str,
    pub start_index: Option<i64>,
    pub end_index: Option<i64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub font_size: Option<f64>,
    pub font_family: Option<&'a str>,
    pub foreground_color: Option<&'a str>,
}

/// Format text in a shape.
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
    if let Some(size) = opts.font_size {
        style["fontSize"] = serde_json::json!({ "magnitude": size, "unit": "PT" });
        fields.push("fontSize");
    }
    if let Some(family) = opts.font_family {
        style["fontFamily"] = serde_json::Value::String(family.to_string());
        fields.push("fontFamily");
    }
    if let Some(color) = opts.foreground_color {
        if let Some(c) = parse_hex_color(color) {
            style["foregroundColor"] = c;
            fields.push("foregroundColor");
        }
    }

    if fields.is_empty() {
        return Err("No formatting options specified".to_string());
    }

    let text_range = match (opts.start_index, opts.end_index) {
        (Some(start), Some(end)) => serde_json::json!({
            "type": "FIXED_RANGE",
            "startIndex": start,
            "endIndex": end,
        }),
        (Some(start), None) => serde_json::json!({
            "type": "FROM_START_INDEX",
            "startIndex": start,
        }),
        _ => serde_json::json!({ "type": "ALL" }),
    };

    let request = serde_json::json!({
        "updateTextStyle": {
            "objectId": opts.object_id,
            "textRange": text_range,
            "style": style,
            "fields": fields.join(","),
        }
    });

    let parsed = batch_update_raw(opts.presentation_id, vec![request])?;

    Ok(UpdateResult {
        presentation_id: parsed["presentationId"].as_str().unwrap_or("").to_string(),
        created_object_id: None,
    })
}

/// Format paragraph alignment in a shape.
pub fn format_paragraph(
    presentation_id: &str,
    object_id: &str,
    alignment: &str,
    start_index: Option<i64>,
    end_index: Option<i64>,
) -> Result<UpdateResult, String> {
    let text_range = match (start_index, end_index) {
        (Some(start), Some(end)) => serde_json::json!({
            "type": "FIXED_RANGE",
            "startIndex": start,
            "endIndex": end,
        }),
        (Some(start), None) => serde_json::json!({
            "type": "FROM_START_INDEX",
            "startIndex": start,
        }),
        _ => serde_json::json!({ "type": "ALL" }),
    };

    let request = serde_json::json!({
        "updateParagraphStyle": {
            "objectId": object_id,
            "textRange": text_range,
            "style": { "alignment": alignment },
            "fields": "alignment",
        }
    });

    let parsed = batch_update_raw(presentation_id, vec![request])?;

    Ok(UpdateResult {
        presentation_id: parsed["presentationId"].as_str().unwrap_or("").to_string(),
        created_object_id: None,
    })
}

/// Replace all shapes containing text with an image.
pub fn replace_shapes_with_image(
    presentation_id: &str,
    find: &str,
    image_url: &str,
    match_case: bool,
) -> Result<ReplaceResult, String> {
    let request = serde_json::json!({
        "replaceAllShapesWithImage": {
            "containsText": {
                "text": find,
                "matchCase": match_case,
            },
            "imageUrl": image_url,
            "imageReplaceMethod": "CENTER_INSIDE",
        }
    });

    let parsed = batch_update_raw(presentation_id, vec![request])?;

    let occurrences = parsed["replies"][0]["replaceAllShapesWithImage"]["occurrencesChanged"]
        .as_i64()
        .unwrap_or(0);

    Ok(ReplaceResult {
        presentation_id: parsed["presentationId"].as_str().unwrap_or("").to_string(),
        occurrences_changed: occurrences,
    })
}

/// Execute a raw batch update with arbitrary requests.
pub fn batch_update(
    presentation_id: &str,
    requests: Vec<serde_json::Value>,
) -> Result<BatchUpdateResult, String> {
    let parsed = batch_update_raw(presentation_id, requests)?;

    let replies = parsed["replies"]
        .as_array()
        .map(|arr| arr.to_vec())
        .unwrap_or_default();

    Ok(BatchUpdateResult {
        presentation_id: parsed["presentationId"].as_str().unwrap_or("").to_string(),
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
