use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ironclaw_host_api::RuntimeHttpEgressResponse;
use serde_json::{Map, Value, json};

use super::model_visible_output::{
    max_binary_bytes_for_base64_budget, serialized_json_content_len, serialized_json_len,
    truncate_str_for_json_content_budget, truncate_string_for_json_content_budget,
};

const MODEL_VISIBLE_HTTP_OUTPUT_OVERHEAD_BYTES: usize = 2 * 1024;
const MODEL_VISIBLE_HTTP_HEADER_BYTES: usize = 8 * 1024;
const MODEL_VISIBLE_HTTP_TRUNCATION_ENVELOPE_BYTES: usize = 1024;
const MAX_MODEL_VISIBLE_BINARY_INLINE_BYTES: usize = 512;
const MAX_MODEL_VISIBLE_RESPONSE_HEADERS: usize = 32;
const MAX_MODEL_VISIBLE_RESPONSE_HEADER_NAME_BYTES: usize = 128;
const MAX_MODEL_VISIBLE_RESPONSE_HEADER_VALUE_BYTES: usize = 1024;
const HTTP_TRUNCATION_HINT: &str = "Response body was truncated for the model-visible budget. Use builtin.http.save with save_to, then builtin.read_file with offsets, to inspect the full sanitized body.";

pub(super) struct HttpDispatchOutput {
    pub output: Value,
    pub network_egress_bytes: u64,
}

pub(super) fn shape_response(
    response: RuntimeHttpEgressResponse,
    response_body_limit: u64,
) -> HttpDispatchOutput {
    let body_was_truncated_by_egress = response.response_bytes > response_body_limit;
    let mut output = Map::new();
    output.insert("status".to_string(), json!(response.status));
    let (headers, headers_truncated) = response_headers(response.headers);
    let inline_body_budget = inline_body_budget(response_body_limit, &headers);
    output.insert("headers".to_string(), headers);
    if headers_truncated {
        output.insert("headers_truncated".to_string(), json!(true));
    }
    let mut body_bytes_returned = if let Some(saved_body) = response.saved_body {
        output.insert(
            "saved_body".to_string(),
            json!({
                "path": saved_body.path.as_str(),
                "bytes_written": saved_body.bytes_written,
            }),
        );
        None
    } else {
        insert_inline_body(
            &mut output,
            response.body,
            inline_body_budget,
            body_was_truncated_by_egress,
        )
    };
    output.insert("request_bytes".to_string(), json!(response.request_bytes));
    output.insert("response_bytes".to_string(), json!(response.response_bytes));
    output.insert(
        "redaction_applied".to_string(),
        json!(response.redaction_applied),
    );
    let final_budget_trim =
        enforce_final_model_visible_output_budget(&mut output, response_body_limit);
    if let Some(final_body_bytes_returned) = final_budget_trim.body_bytes_returned {
        body_bytes_returned = Some(final_body_bytes_returned);
    }
    let headers_truncated = headers_truncated || final_budget_trim.headers_truncated;
    insert_truncation_envelope(&mut output, headers_truncated, body_bytes_returned);
    HttpDispatchOutput {
        output: Value::Object(output),
        network_egress_bytes: response.request_bytes,
    }
}

fn response_headers(headers: Vec<(String, String)>) -> (Value, bool) {
    let mut headers_truncated = headers.len() > MAX_MODEL_VISIBLE_RESPONSE_HEADERS;
    let mut value_truncated = false;
    let mut visible_headers = Vec::new();
    let mut serialized_content_len = 0_usize;
    for (index, (name, value)) in headers.into_iter().enumerate() {
        if index >= MAX_MODEL_VISIBLE_RESPONSE_HEADERS {
            break;
        }
        let (name, name_truncated) = truncate_string_for_json_content_budget(
            name,
            MAX_MODEL_VISIBLE_RESPONSE_HEADER_NAME_BYTES,
        );
        let (value, header_value_truncated) = truncate_string_for_json_content_budget(
            value,
            MAX_MODEL_VISIBLE_RESPONSE_HEADER_VALUE_BYTES,
        );
        value_truncated |= name_truncated || header_value_truncated;
        let mut header = Map::new();
        header.insert("name".to_string(), Value::String(name));
        header.insert("value".to_string(), Value::String(value));
        if name_truncated || header_value_truncated {
            header.insert("truncated".to_string(), json!(true));
        }
        let header = Value::Object(header);
        let candidate_content_len =
            serialized_content_len.saturating_add(serialized_json_len(&header));
        let candidate_separator_bytes = visible_headers.len();
        let candidate_array_len = candidate_content_len
            .saturating_add(candidate_separator_bytes)
            .saturating_add(2);
        if candidate_array_len > MODEL_VISIBLE_HTTP_HEADER_BYTES {
            headers_truncated = true;
            break;
        }
        serialized_content_len = candidate_content_len;
        visible_headers.push(header);
    }
    (
        Value::Array(visible_headers),
        headers_truncated || value_truncated,
    )
}

fn inline_body_budget(response_body_limit: u64, headers: &Value) -> u64 {
    let response_body_limit = usize::try_from(response_body_limit).unwrap_or(usize::MAX);
    let header_bytes = serialized_json_len(headers);
    let excess_header_bytes = header_bytes.saturating_sub(MODEL_VISIBLE_HTTP_HEADER_BYTES);
    let body_budget = response_body_limit
        .saturating_sub(excess_header_bytes)
        .max(1);
    u64::try_from(body_budget).unwrap_or(u64::MAX)
}

fn insert_inline_body(
    output: &mut Map<String, Value>,
    body: Vec<u8>,
    response_body_limit: u64,
    body_was_truncated_by_egress: bool,
) -> Option<usize> {
    let limit = usize::try_from(response_body_limit).unwrap_or(usize::MAX);
    let returned_body_bytes;
    let mut body_truncated = body_was_truncated_by_egress;

    match String::from_utf8(body) {
        Ok(body_text) => {
            let (returned_len, truncated) = if body_text.len() <= limit / 6 {
                (body_text.len(), false)
            } else {
                let (body_text, truncated) =
                    truncate_str_for_json_content_budget(&body_text, limit);
                (body_text.len(), truncated)
            };
            returned_body_bytes = returned_len;
            body_truncated |= truncated;
            let body_text = if truncated {
                body_text[..returned_len].to_string()
            } else {
                body_text
            };
            output.insert("body_text".to_string(), Value::String(body_text));
        }
        Err(error) => {
            let body = error.into_bytes();
            if body.len() > MAX_MODEL_VISIBLE_BINARY_INLINE_BYTES {
                body_truncated = true;
                returned_body_bytes = 0;
                output.insert("body_base64_omitted".to_string(), json!(true));
            } else {
                let binary_limit = max_binary_bytes_for_base64_budget(limit);
                let returned = body.len().min(binary_limit);
                body_truncated |= returned < body.len();
                returned_body_bytes = returned;
                output.insert(
                    "body_base64".to_string(),
                    Value::String(BASE64_STANDARD.encode(&body[..returned])),
                );
            }
        }
    }

    if body_truncated {
        output.insert("body_truncated".to_string(), json!(true));
        output.insert(
            "body_bytes_returned".to_string(),
            json!(returned_body_bytes),
        );
        output.insert(
            "body_truncation_hint".to_string(),
            Value::String(HTTP_TRUNCATION_HINT.to_string()),
        );
        return Some(returned_body_bytes);
    }
    None
}

fn insert_truncation_envelope(
    output: &mut Map<String, Value>,
    headers_truncated: bool,
    body_bytes_returned: Option<usize>,
) {
    if !headers_truncated && body_bytes_returned.is_none() {
        return;
    }
    let mut truncation = Map::new();
    truncation.insert("body".to_string(), json!(body_bytes_returned.is_some()));
    truncation.insert("headers".to_string(), json!(headers_truncated));
    if let Some(body_bytes_returned) = body_bytes_returned {
        truncation.insert("bytes_returned".to_string(), json!(body_bytes_returned));
    }
    truncation.insert(
        "reason".to_string(),
        Value::String("model_visible_budget".to_string()),
    );
    truncation.insert(
        "next_step".to_string(),
        Value::String(HTTP_TRUNCATION_HINT.to_string()),
    );
    output.insert("truncation".to_string(), Value::Object(truncation));
}

#[derive(Debug, Default)]
struct FinalBudgetTrim {
    body_bytes_returned: Option<usize>,
    headers_truncated: bool,
}

fn enforce_final_model_visible_output_budget(
    output: &mut Map<String, Value>,
    response_body_limit: u64,
) -> FinalBudgetTrim {
    let response_body_limit = usize::try_from(response_body_limit).unwrap_or(usize::MAX);
    let final_budget = response_body_limit
        .saturating_add(MODEL_VISIBLE_HTTP_OUTPUT_OVERHEAD_BYTES)
        .saturating_sub(MODEL_VISIBLE_HTTP_TRUNCATION_ENVELOPE_BYTES);
    let mut trim = FinalBudgetTrim::default();
    let mut current_len = serialized_output_len(output);

    if current_len > final_budget
        && let Some(body_text) = output.get("body_text").and_then(Value::as_str)
    {
        let excess_bytes = current_len.saturating_sub(final_budget);
        let current_body_budget = serialized_json_content_len(body_text);
        let target_body_budget = current_body_budget.saturating_sub(excess_bytes);
        let (body_text, _) = truncate_str_for_json_content_budget(body_text, target_body_budget);
        let returned_body_bytes = body_text.len();
        output.insert(
            "body_text".to_string(),
            Value::String(body_text.to_string()),
        );
        mark_inline_body_truncated(output, returned_body_bytes);
        trim.body_bytes_returned = Some(returned_body_bytes);
        current_len = serialized_output_len(output);
    }

    if current_len > final_budget
        && let Some(body_base64) = output.get("body_base64").and_then(Value::as_str)
    {
        let excess_bytes = current_len.saturating_sub(final_budget);
        let target_len = body_base64.len().saturating_sub(excess_bytes) / 4 * 4;
        let body_base64 = body_base64[..target_len].to_string();
        let returned_body_bytes = max_binary_bytes_for_base64_budget(target_len);
        output.insert("body_base64".to_string(), Value::String(body_base64));
        mark_inline_body_truncated(output, returned_body_bytes);
        trim.body_bytes_returned = Some(returned_body_bytes);
        current_len = serialized_output_len(output);
    }

    trim.headers_truncated = trim_headers_for_final_budget(output, final_budget, current_len);
    trim
}

fn serialized_output_len(output: &Map<String, Value>) -> usize {
    serde_json::to_vec(output).map_or(usize::MAX, |serialized| serialized.len())
}

fn trim_headers_for_final_budget(
    output: &mut Map<String, Value>,
    final_budget: usize,
    mut current_len: usize,
) -> bool {
    let mut trimmed = false;
    let mut headers_truncated_marked = output.contains_key("headers_truncated");
    loop {
        if current_len <= final_budget {
            return trimmed;
        }
        let Some(headers) = output.get_mut("headers").and_then(Value::as_array_mut) else {
            return trimmed;
        };
        let Some(popped) = headers.pop() else {
            return trimmed;
        };
        let separator_bytes = usize::from(!headers.is_empty());
        current_len = current_len
            .saturating_sub(serialized_json_len(&popped).saturating_add(separator_bytes));
        if !headers_truncated_marked {
            output.insert("headers_truncated".to_string(), json!(true));
            current_len = serialized_output_len(output);
            headers_truncated_marked = true;
        }
        trimmed = true;
    }
}

fn mark_inline_body_truncated(output: &mut Map<String, Value>, returned_body_bytes: usize) {
    output.insert("body_truncated".to_string(), json!(true));
    output.insert(
        "body_bytes_returned".to_string(),
        json!(returned_body_bytes),
    );
    output.insert(
        "body_truncation_hint".to_string(),
        Value::String(HTTP_TRUNCATION_HINT.to_string()),
    );
}
