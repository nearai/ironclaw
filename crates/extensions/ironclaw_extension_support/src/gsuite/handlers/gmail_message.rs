use base64::{
    Engine as _,
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
};
use encoding_rs::{Encoding, UTF_8};
use htmd::HtmlToMarkdown;
use ironclaw_common::normalize_mime_type;
use ironclaw_host_api::dispatch::RuntimeDispatchErrorKind;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::rc::Rc;

use super::GsuiteDispatchError;

const MAX_MIME_DEPTH: usize = 16;
const MAX_MIME_PARTS: usize = 256;
const MAX_ATTACHMENTS: usize = 64;
const MAX_BODY_BYTES: usize = 512 * 1024;
const MAX_HEADER_VALUE_BYTES: usize = 4 * 1024;
const MAX_ATTACHMENT_FIELD_BYTES: usize = 512;
const MAX_HTML_DEPTH: usize = 64;
const MAX_HTML_NODES: usize = 8_192;
const MAX_HTML_SIBLINGS: usize = 1_024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailMessage {
    id: String,
    thread_id: String,
    #[serde(default)]
    label_ids: Vec<String>,
    payload: MessagePart,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessagePart {
    #[serde(default)]
    mime_type: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    headers: Vec<MessageHeader>,
    #[serde(default)]
    body: MessagePartBody,
    #[serde(default)]
    parts: Vec<MessagePart>,
}

#[derive(Deserialize)]
struct MessageHeader {
    name: String,
    value: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessagePartBody {
    attachment_id: Option<String>,
    size: Option<u64>,
    data: Option<String>,
}

enum ReadableBody {
    Text {
        kind: &'static str,
        text: String,
        truncated: bool,
    },
    Encrypted,
}

struct TraversalBudget {
    parts_seen: usize,
}

enum BodySelectionError {
    Fatal(GsuiteDispatchError),
    Candidate(GsuiteDispatchError),
}

pub(super) fn normalize(value: Value) -> Result<Value, GsuiteDispatchError> {
    let message: GmailMessage = serde_json::from_value(value)
        .map_err(|error| output_decode_with("parse message JSON", error))?;
    let mut attachments = Vec::new();
    let mut attachments_truncated = false;
    collect_attachments(
        &message.payload,
        0,
        &mut TraversalBudget { parts_seen: 0 },
        &mut attachments,
        &mut attachments_truncated,
    )?;
    let selected = select_body(&message.payload, 0, &mut TraversalBudget { parts_seen: 0 })
        .map_err(|error| match error {
            BodySelectionError::Fatal(error) | BodySelectionError::Candidate(error) => error,
        })?;

    let mut output = Map::new();
    output.insert("id".to_string(), Value::String(message.id));
    output.insert("thread_id".to_string(), Value::String(message.thread_id));
    output.insert("label_ids".to_string(), json!(message.label_ids));
    output.insert(
        "headers".to_string(),
        selected_headers(&message.payload.headers),
    );
    output.insert("body".to_string(), readable_body_output(selected));
    output.insert("attachments".to_string(), Value::Array(attachments));
    output.insert(
        "attachments_truncated".to_string(),
        Value::Bool(attachments_truncated),
    );
    Ok(Value::Object(output))
}

fn selected_headers(headers: &[MessageHeader]) -> Value {
    let mut selected = Map::new();
    for (provider_name, output_name) in [
        ("From", "from"),
        ("To", "to"),
        ("Cc", "cc"),
        ("Reply-To", "reply_to"),
        ("Subject", "subject"),
        ("Date", "date"),
    ] {
        if let Some(value) = header(headers, provider_name) {
            selected.insert(
                output_name.to_string(),
                Value::String(truncate_utf8(value, MAX_HEADER_VALUE_BYTES).0),
            );
        }
    }
    Value::Object(selected)
}

fn readable_body_output(body: Option<ReadableBody>) -> Value {
    match body {
        Some(ReadableBody::Text {
            kind,
            text,
            truncated,
        }) => {
            let (text, output_truncated) = truncate_utf8(&text, MAX_BODY_BYTES);
            json!({ "kind": kind, "text": text, "truncated": truncated || output_truncated })
        }
        Some(ReadableBody::Encrypted) => json!({
            "kind": "encrypted",
            "reason": "encrypted content is not supported"
        }),
        None => json!({
            "kind": "unavailable",
            "reason": "no supported readable message body"
        }),
    }
}

fn collect_attachments(
    part: &MessagePart,
    depth: usize,
    budget: &mut TraversalBudget,
    attachments: &mut Vec<Value>,
    truncated: &mut bool,
) -> Result<(), GsuiteDispatchError> {
    visit_part(depth, budget)?;
    let is_attachment = is_attachment_part(part);
    if is_attachment {
        if attachments.len() == MAX_ATTACHMENTS {
            *truncated = true;
        } else if attachments.len() < MAX_ATTACHMENTS {
            let filename = truncate_utf8(&part.filename, MAX_ATTACHMENT_FIELD_BYTES).0;
            let mime_type = truncate_utf8(
                &normalize_mime_type(&part.mime_type),
                MAX_ATTACHMENT_FIELD_BYTES,
            )
            .0;
            let attachment_id = part
                .body
                .attachment_id
                .as_deref()
                .map(|value| truncate_utf8(value, MAX_ATTACHMENT_FIELD_BYTES).0);
            attachments.push(json!({
                "attachment_id": attachment_id,
                "filename": filename,
                "mime_type": mime_type,
                "size": part.body.size
            }));
        }
    }
    for child in &part.parts {
        collect_attachments(child, depth + 1, budget, attachments, truncated)?;
    }
    Ok(())
}

fn select_body(
    part: &MessagePart,
    depth: usize,
    budget: &mut TraversalBudget,
) -> Result<Option<ReadableBody>, BodySelectionError> {
    visit_part(depth, budget).map_err(BodySelectionError::Fatal)?;
    let mime_type = normalize_mime_type(&part.mime_type);
    if is_encrypted_mime(&mime_type) {
        return Ok(Some(ReadableBody::Encrypted));
    }
    if is_attachment_part(part) {
        return Ok(None);
    }
    if mime_type == "text/plain" || mime_type == "text/html" {
        if part.body.data.as_deref().is_none_or(str::is_empty) {
            return Ok(None);
        }
        return decode_text_part(part, &mime_type)
            .map(Some)
            .map_err(BodySelectionError::Candidate);
    }

    if mime_type == "multipart/alternative" {
        let mut html = None;
        let mut encrypted = None;
        let mut candidate_error = None;
        for child in &part.parts {
            let selected = match select_body(child, depth + 1, budget) {
                Ok(selected) => selected,
                Err(BodySelectionError::Fatal(error)) => {
                    return Err(BodySelectionError::Fatal(error));
                }
                Err(BodySelectionError::Candidate(error)) => {
                    candidate_error.get_or_insert(error);
                    continue;
                }
            };
            match selected {
                Some(body @ ReadableBody::Text { kind: "text", .. }) => return Ok(Some(body)),
                Some(body @ ReadableBody::Text { .. }) => html = html.or(Some(body)),
                Some(body @ ReadableBody::Encrypted) => encrypted = encrypted.or(Some(body)),
                None => {}
            }
        }
        return match html.or(encrypted) {
            Some(body) => Ok(Some(body)),
            None => candidate_error
                .map(BodySelectionError::Candidate)
                .map_or(Ok(None), Err),
        };
    }

    let mut encrypted = None;
    let mut candidate_error = None;
    for child in &part.parts {
        let selected = match select_body(child, depth + 1, budget) {
            Ok(selected) => selected,
            Err(BodySelectionError::Fatal(error)) => {
                return Err(BodySelectionError::Fatal(error));
            }
            Err(BodySelectionError::Candidate(error)) => {
                candidate_error.get_or_insert(error);
                continue;
            }
        };
        match selected {
            Some(body @ ReadableBody::Text { .. }) => return Ok(Some(body)),
            Some(body @ ReadableBody::Encrypted) => encrypted = encrypted.or(Some(body)),
            None => {}
        }
    }
    match encrypted {
        Some(body) => Ok(Some(body)),
        None => candidate_error
            .map(BodySelectionError::Candidate)
            .map_or(Ok(None), Err),
    }
}

fn decode_text_part(
    part: &MessagePart,
    mime_type: &str,
) -> Result<ReadableBody, GsuiteDispatchError> {
    let encoded = part.body.data.as_deref().ok_or_else(output_decode)?;
    let decoded = decode_base64url(encoded)?;
    let text = decode_declared_charset(part, decoded)?;
    if mime_type == "text/html" {
        let (text, input_truncated) = truncate_utf8(&text, MAX_BODY_BYTES);
        let converter = HtmlToMarkdown::builder()
            .skip_tags(vec!["script", "style", "noscript"])
            .add_handler(
                vec!["img"],
                |_handlers: &dyn htmd::element_handler::Handlers, _element: htmd::Element| {
                    Some("[inline image omitted]".into())
                },
            )
            .add_handler(
                vec!["a"],
                |handlers: &dyn htmd::element_handler::Handlers, element: htmd::Element| {
                    let has_inline_data = element.attrs.iter().any(|attribute| {
                        matches!(attribute.name.local.as_ref(), "href" | "title")
                            && attribute
                                .value
                                .trim_start()
                                .to_ascii_lowercase()
                                .starts_with("data:")
                    });
                    if has_inline_data {
                        Some(handlers.walk_children(element.node))
                    } else {
                        handlers.fallback(element)
                    }
                },
            )
            .build();
        let tree = converter
            .html_to_tree(&text)
            .map_err(|error| output_decode_with("parse message HTML", error))?;
        validate_html_tree(&tree)?;
        let markdown = converter.tree_to_markdown(&tree);
        Ok(ReadableBody::Text {
            kind: "markdown",
            text: markdown,
            truncated: input_truncated,
        })
    } else {
        Ok(ReadableBody::Text {
            kind: "text",
            text,
            truncated: false,
        })
    }
}

fn decode_declared_charset(
    part: &MessagePart,
    decoded: Vec<u8>,
) -> Result<String, GsuiteDispatchError> {
    let mut encoding = None;
    for content_type in [
        part.mime_type.as_str(),
        header(&part.headers, "Content-Type").unwrap_or(""),
    ] {
        if content_type.is_empty() {
            continue;
        }
        let parsed = match content_type.parse::<mime::Mime>() {
            Ok(parsed) => parsed,
            Err(error) => {
                tracing::debug!(%error, "ignoring malformed Gmail content type");
                continue;
            }
        };
        let Some(charset) = parsed.get_param(mime::CHARSET) else {
            continue;
        };
        match Encoding::for_label(charset.as_str().as_bytes()) {
            Some(declared) => {
                encoding = Some(declared);
                break;
            }
            None => tracing::debug!(charset = charset.as_str(), "unknown Gmail text charset"),
        }
    }
    let encoding = encoding.unwrap_or(UTF_8);

    if encoding == UTF_8 {
        return String::from_utf8(decoded)
            .map_err(|error| output_decode_with("decode message text as UTF-8", error));
    }

    encoding
        .decode_without_bom_handling_and_without_replacement(&decoded)
        .map(|text| text.into_owned())
        .ok_or_else(|| {
            output_decode_with(
                "decode message text using declared charset",
                format_args!("invalid {} byte sequence", encoding.name()),
            )
        })
}

fn decode_base64url(encoded: &str) -> Result<Vec<u8>, GsuiteDispatchError> {
    URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| URL_SAFE.decode(encoded))
        .map_err(|error| output_decode_with("decode Gmail body data as base64url", error))
}

fn header<'a>(headers: &'a [MessageHeader], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str())
}

fn is_attachment_part(part: &MessagePart) -> bool {
    !part.filename.is_empty()
        || part.body.attachment_id.is_some()
        || header(&part.headers, "Content-Disposition").is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|disposition| disposition.trim().eq_ignore_ascii_case("attachment"))
        })
}

fn is_encrypted_mime(mime_type: &str) -> bool {
    matches!(
        mime_type,
        "multipart/encrypted"
            | "application/pgp-encrypted"
            | "application/pkcs7-mime"
            | "application/x-pkcs7-mime"
    )
}

fn validate_html_tree(root: &Rc<htmd::Node>) -> Result<(), GsuiteDispatchError> {
    let mut nodes_seen = 0_usize;
    let mut pending = vec![(Rc::clone(root), 0_usize)];
    while let Some((node, depth)) = pending.pop() {
        nodes_seen = nodes_seen.saturating_add(1);
        let children = node.children.borrow();
        if depth > MAX_HTML_DEPTH
            || nodes_seen > MAX_HTML_NODES
            || children.len() > MAX_HTML_SIBLINGS
        {
            tracing::debug!(
                depth,
                nodes_seen,
                sibling_count = children.len(),
                "Gmail HTML exceeds conversion complexity limits"
            );
            return Err(output_decode());
        }
        pending.extend(
            children
                .iter()
                .rev()
                .map(|child| (Rc::clone(child), depth + 1)),
        );
    }
    Ok(())
}

fn visit_part(depth: usize, budget: &mut TraversalBudget) -> Result<(), GsuiteDispatchError> {
    budget.parts_seen = budget.parts_seen.saturating_add(1);
    if depth > MAX_MIME_DEPTH || budget.parts_seen > MAX_MIME_PARTS {
        Err(output_decode())
    } else {
        Ok(())
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_string(), false);
    }
    let mut prefix = String::with_capacity(max_bytes);
    for (index, character) in value.char_indices() {
        if index.saturating_add(character.len_utf8()) > max_bytes {
            break;
        }
        prefix.push(character);
    }
    (prefix, true)
}

fn output_decode() -> GsuiteDispatchError {
    GsuiteDispatchError::new(RuntimeDispatchErrorKind::OutputDecode)
}

fn output_decode_with(context: &'static str, error: impl std::fmt::Display) -> GsuiteDispatchError {
    tracing::debug!(context, error = %error, "Gmail response normalization failed");
    output_decode()
}
