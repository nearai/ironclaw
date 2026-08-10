use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ironclaw_host_api::{
    dispatch::RuntimeDispatchErrorKind, http::RuntimeHttpEgressResponse, resource::ResourceUsage,
    result_meta::MODEL_DIAGNOSTIC_MAX_BYTES,
};
use serde_json::{Map, Value, json};

use crate::FirstPartyCapabilityError;

use super::model_visible_output::{
    max_binary_bytes_for_base64_budget, serialized_json_content_len, serialized_json_len,
    truncate_str_for_json_content_budget, truncate_string_for_json_content_budget,
};

const MODEL_VISIBLE_HTTP_OUTPUT_OVERHEAD_BYTES: usize = 2 * 1024;
const MODEL_VISIBLE_HTTP_HEADER_BYTES: usize = 8 * 1024;
const MODEL_VISIBLE_HTTP_TRUNCATION_ENVELOPE_BYTES: usize = 1024;
/// Fixed overhead of `ironclaw_safety::wrap_external_content` (the SECURITY
/// NOTICE fence) plus margin for the source name and escape expansion.
/// Reserving it keeps the fenced failure diagnostic within
/// `MODEL_DIAGNOSTIC_MAX_BYTES` at the observation boundary.
const MODEL_DIAGNOSTIC_FENCE_HEADROOM_BYTES: usize = 768;
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
    if let Some(hint) = unauthorized_extension_hint(response.status) {
        output.insert("auth_hint".to_string(), json!(hint));
    }
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

/// HTTP statuses that classify as capability failures. Drawn deliberately at
/// 400 so rate-limit and overload responses (429/503) are never reported as
/// success; all statuses outside 400..=599 (informational, successful,
/// redirects, and out-of-spec 600+) stay inspectable successful results.
/// Single source of truth shared by the dispatch shape-limit selection and
/// the classifier.
pub(super) fn is_error_status(status: u16) -> bool {
    (400..=599).contains(&status)
}

/// Transport completion is not capability success: HTTP 4xx/5xx responses are
/// model-visible, recoverable `OperationFailed` outcomes carrying the bounded,
/// sanitized response as diagnostic context. Informational, successful, and
/// redirect responses stay inspectable successful results; redirects are
/// returned, never followed. Boundary rationale: see [`is_error_status`].
pub(super) fn classify_status(
    shaped: HttpDispatchOutput,
    status: u16,
    wall_clock_ms: u64,
) -> Result<HttpDispatchOutput, FirstPartyCapabilityError> {
    if !is_error_status(status) {
        return Ok(shaped);
    }
    tracing::debug!(
        http_status = status,
        dispatch_error_kind = RuntimeDispatchErrorKind::OperationFailed.as_str(),
        "first-party HTTP response status classified as capability failure"
    );
    let usage = ResourceUsage::default()
        .set_network_egress_bytes(shaped.network_egress_bytes)
        .set_wall_clock_ms(wall_clock_ms);
    Err(FirstPartyCapabilityError::dispatch_with_diagnostic(
        RuntimeDispatchErrorKind::OperationFailed,
        Some(format!("HTTP request returned status {status}")),
        bounded_failure_diagnostic(shaped.output, status),
    )
    .with_usage(usage))
}

/// Serialize the shaped output as the failure diagnostic, trimming the response
/// body so the serialized diagnostic stays within the model-visible diagnostic
/// budget. The resolution boundary truncates the whole diagnostic string at
/// `MODEL_DIAGNOSTIC_MAX_BYTES` keeping the head, and `serde_json::Map`
/// serializes keys in sorted order, so an untrimmed diagnostic would cut
/// `status` and the truncation envelope (the last-sorted keys) out of the
/// model-visible text. Trimming here keeps the verdict fields intact and the
/// diagnostic valid JSON.
///
/// The budget also reserves headroom for the loop-host external-content fence:
/// when an error body trips the injection scan, the seam wraps the whole
/// diagnostic in `ironclaw_safety::wrap_external_content` before the
/// observation boundary re-applies `MODEL_DIAGNOSTIC_MAX_BYTES`. Without the
/// reservation the fenced diagnostic would exceed the boundary and the tail
/// (status, truncation envelope) would be cut.
fn bounded_failure_diagnostic(output: Value, status: u16) -> String {
    let Value::Object(mut output) = output else {
        return fallback_diagnostic(status, None);
    };
    if serialized_output_len(&output) <= MODEL_DIAGNOSTIC_MAX_BYTES {
        return scrub_model_diagnostic_controls(serialize_diagnostic(&output, status));
    }
    // Pre-account the truncation envelope and the injection-fence headroom,
    // exactly like `enforce_final_model_visible_output_budget` pre-accounts
    // the envelope; the fixed verdict fields alone are always far below the
    // budget.
    let final_budget = MODEL_DIAGNOSTIC_MAX_BYTES
        .saturating_sub(MODEL_VISIBLE_HTTP_TRUNCATION_ENVELOPE_BYTES)
        .saturating_sub(MODEL_DIAGNOSTIC_FENCE_HEADROOM_BYTES);
    let trim = fit_output_to_budget(&mut output, final_budget);
    if serialized_output_len(&output) > final_budget {
        return fallback_diagnostic(status, saved_body_evidence(&output));
    }
    // The envelope must reflect truncation state from BOTH stages: the shape
    // stage may have already marked headers/body as truncated (e.g. more than
    // 32 headers, or a body trimmed at shape time), and the diagnostic-budget
    // stage may have trimmed only one of them. OR the surviving keys in so the
    // model never sees headers_truncated:true alongside an envelope that
    // claims headers:false.
    let headers_truncated = trim.headers_truncated || output.contains_key("headers_truncated");
    let body_bytes_returned = trim.body_bytes_returned.or_else(|| {
        output
            .get("body_bytes_returned")
            .and_then(Value::as_u64)
            .map(|bytes| bytes as usize)
    });
    insert_truncation_envelope(&mut output, headers_truncated, body_bytes_returned);
    scrub_model_diagnostic_controls(serialize_diagnostic(&output, status))
}

/// `ModelDiagnostic` validation rejects raw control characters other than
/// line breaks and tabs. `serde_json` escapes U+0000..U+001F, but DEL/C1
/// control bytes (U+007F..U+009F) survive serialization raw and would
/// otherwise fail the whole diagnostic at the resolution boundary, replacing
/// it with the fixed "no additional diagnostic detail" sentence.
fn scrub_model_diagnostic_controls(diagnostic: String) -> String {
    if !diagnostic
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return diagnostic;
    }
    diagnostic
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
        .collect()
}

/// The diagnostic string is produced from a `serde_json::Value`. Serialization
/// of a `Value` cannot fail for any string a safe API can produce: escaping
/// does not revalidate UTF-8 (probed: even an unsafe lone-surrogate string
/// serializes `Ok`), and all other `Value` variants are infallible to emit.
/// The fallback is a pure defensive guard for future `Value` shapes.
fn serialize_diagnostic(output: &Map<String, Value>, status: u16) -> String {
    match serde_json::to_string(output) {
        Ok(diagnostic) => diagnostic,
        Err(error) => {
            tracing::debug!(%error, "failed to serialize HTTP failure diagnostic");
            fallback_diagnostic(status, saved_body_evidence(output))
        }
    }
}

/// Compact `saved_body` evidence extracted from a shaped output, used to keep
/// the save destination visible even when the diagnostic falls back.
fn saved_body_evidence(output: &Map<String, Value>) -> Option<(&str, u64)> {
    let saved_body = output.get("saved_body")?.as_object()?;
    let path = saved_body.get("path")?.as_str()?;
    let bytes_written = saved_body.get("bytes_written")?.as_u64()?;
    Some((path, bytes_written))
}

/// Long save paths are accepted by the tool input limit; keep only a bounded
/// prefix in the fallback so the evidence payload stays far below the
/// diagnostic budget.
const FALLBACK_SAVED_BODY_PATH_BUDGET: usize = 1024;

fn fallback_diagnostic(status: u16, saved_body: Option<(&str, u64)>) -> String {
    let mut output = Map::new();
    output.insert("status".to_string(), json!(status));
    if let Some((path, bytes_written)) = saved_body {
        let (path, _) = truncate_str_for_json_content_budget(path, FALLBACK_SAVED_BODY_PATH_BUDGET);
        output.insert(
            "saved_body".to_string(),
            json!({ "path": path, "bytes_written": bytes_written }),
        );
    }
    output.insert("error".to_string(), json!("diagnostic unavailable"));
    serde_json::to_string(&output)
        .unwrap_or_else(|_| format!("{{\"status\":{status},\"error\":\"diagnostic unavailable\"}}"))
}

/// When an outbound `builtin.http` request is rejected for missing or invalid
/// authorization, nudge the model toward the extension that can inject
/// credentials for the host, rather than concluding the resource is
/// inaccessible and giving up. Hint-only: the model still drives install and
/// the manifest-declared readiness flow.
fn unauthorized_extension_hint(status: u16) -> Option<&'static str> {
    matches!(status, 401 | 403 | 407).then_some(
        "This request was rejected for authentication/authorization. If the request targeted a \
         public human-facing webpage rather than an authenticated API, retry with an available \
         web_search tool. If this host is served by an installable extension, that extension \
         injects the required credentials for you: \
         search for it with builtin.extension_search (by the service or domain name), then \
         builtin.extension_install, and retry through the \
         extension's tools instead of an unauthenticated builtin.http call.",
    )
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
    fit_output_to_budget(output, final_budget)
}

/// Shared budget trim: shrink the inline body (text then base64), then pop
/// headers until the serialized output fits `final_budget`. Each branch
/// re-measures the running serialized length incrementally, so full-output
/// serializations stay bounded by the number of trim passes rather than one
/// per trimmed key. Used by the success-path final budget and by the failure
/// diagnostic budget trim, which differ only in the target size.
///
/// Convergence: the first pass trims the body by the full excess, leaving at
/// most the truncation-marker overhead (~a few hundred serialized bytes); the
/// second pass absorbs that remainder, so the loop completes in at most two
/// or three passes for any body size (it exits early once the output fits,
/// and an empty body falls through to the base64/headers branches).
fn fit_output_to_budget(output: &mut Map<String, Value>, final_budget: usize) -> FinalBudgetTrim {
    let mut trim = FinalBudgetTrim::default();
    let mut current_len = serialized_output_len(output);

    // Each trim inserts truncation markers that grow the output again, so
    // re-measure and keep trimming the same body until it fits or is empty.
    // The loop makes strict progress (target is always smaller than the
    // current body) and an empty body falls through to the base64/headers
    // branches instead of spinning.
    while current_len > final_budget
        && let Some(body_text) = output.get("body_text").and_then(Value::as_str)
        && !body_text.is_empty()
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

    while current_len > final_budget
        && let Some(body_base64) = output.get("body_base64").and_then(Value::as_str)
        && !body_base64.is_empty()
    {
        let excess_bytes = current_len.saturating_sub(final_budget);
        let target_len = body_base64.len().saturating_sub(excess_bytes) / 4 * 4;
        // safety: base64 text is ASCII and `target_len` is a multiple of 4
        // no larger than `body_base64.len()`, so the slice lands on a
        // UTF-8 boundary.
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

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{
        http::{RuntimeHttpEgressResponse, RuntimeHttpSavedBody},
        path::ScopedPath,
    };
    use serde_json::json;

    use super::*;

    fn response_with_status(status: u16) -> RuntimeHttpEgressResponse {
        RuntimeHttpEgressResponse {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: false,
        }
    }

    #[test]
    fn unauthorized_responses_carry_web_search_and_extension_recovery_hints() {
        for status in [401, 403, 407] {
            let shaped = shape_response(response_with_status(status), 1024);
            let hint = shaped.output.get("auth_hint").and_then(Value::as_str);
            assert!(
                hint.is_some_and(|hint| {
                    hint.contains("web_search") && hint.contains("builtin.extension_search")
                }),
                "status {status} should nudge the model toward web search for public pages or an \
                 authenticated extension for APIs"
            );
        }
    }

    #[test]
    fn successful_and_not_found_responses_have_no_auth_hint() {
        for status in [200, 204, 404, 500] {
            let shaped = shape_response(response_with_status(status), 1024);
            assert!(
                shaped.output.get("auth_hint").is_none(),
                "status {status} must not carry an auth hint"
            );
        }
    }

    #[test]
    fn failure_diagnostic_trims_large_bodies_while_preserving_verdict_fields() {
        let shaped = shape_response(
            RuntimeHttpEgressResponse {
                status: 403,
                headers: Vec::new(),
                body: vec![b'a'; 32 * 1024],
                saved_body: None,
                request_bytes: 42,
                response_bytes: 32 * 1024,
                redaction_applied: false,
            },
            48 * 1024,
        );

        let diagnostic = bounded_failure_diagnostic(shaped.output, 403);

        assert!(
            diagnostic.len() <= MODEL_DIAGNOSTIC_MAX_BYTES,
            "diagnostic must fit the model-visible budget, got {} bytes",
            diagnostic.len()
        );
        let parsed: Value =
            serde_json::from_str(&diagnostic).expect("trimmed diagnostic must stay valid JSON");
        assert_eq!(parsed["status"], json!(403));
        assert!(
            parsed["auth_hint"].as_str().is_some(),
            "auth_hint must survive the budget trim"
        );
        assert_eq!(parsed["truncation"]["body"], json!(true));
        assert!(
            parsed["body_text"].as_str().is_some(),
            "trimmed body must remain visible"
        );
    }

    #[test]
    fn failure_diagnostic_trims_headers_when_body_is_empty_or_absent() {
        // Empty text body: shape_response inserts an empty body_text key, which
        // must not block header trimming on the failure-diagnostic path.
        let shaped = shape_response(
            RuntimeHttpEgressResponse {
                status: 403,
                headers: (0..32)
                    .map(|i| (format!("x-header-{i}"), "h".repeat(1024)))
                    .collect(),
                body: Vec::new(),
                saved_body: None,
                request_bytes: 0,
                response_bytes: 0,
                redaction_applied: false,
            },
            48 * 1024,
        );

        let diagnostic = bounded_failure_diagnostic(shaped.output, 403);
        let parsed: Value =
            serde_json::from_str(&diagnostic).expect("diagnostic must be valid JSON");
        assert_eq!(parsed["status"], json!(403));
        assert_eq!(parsed["truncation"]["headers"], json!(true));
        assert!(
            diagnostic.len() <= MODEL_DIAGNOSTIC_MAX_BYTES,
            "diagnostic must fit the budget, got {} bytes",
            diagnostic.len()
        );
        assert!(
            parsed["auth_hint"].as_str().is_some(),
            "auth_hint must survive header trimming"
        );

        // Save-mode shape: no inline body keys at all, oversized headers.
        let shaped = shape_response(
            RuntimeHttpEgressResponse {
                status: 503,
                headers: (0..32)
                    .map(|i| (format!("x-header-{i}"), "h".repeat(1024)))
                    .collect(),
                body: Vec::new(),
                saved_body: Some(RuntimeHttpSavedBody {
                    path: ScopedPath::new("/workspace/x.bin").unwrap(),
                    bytes_written: 10,
                }),
                request_bytes: 0,
                response_bytes: 10,
                redaction_applied: false,
            },
            48 * 1024,
        );

        let diagnostic = bounded_failure_diagnostic(shaped.output, 503);
        let parsed: Value =
            serde_json::from_str(&diagnostic).expect("diagnostic must be valid JSON");
        assert_eq!(parsed["status"], json!(503));
        assert_eq!(parsed["truncation"]["headers"], json!(true));
        assert_eq!(parsed["saved_body"]["bytes_written"], json!(10));
        assert!(
            diagnostic.len() <= MODEL_DIAGNOSTIC_MAX_BYTES,
            "diagnostic must fit the budget, got {} bytes",
            diagnostic.len()
        );
    }

    #[test]
    fn failure_diagnostic_trims_base64_body_with_large_headers() {
        // Binary inline body (base64 branch) pushed over budget by large
        // headers: the base64 slice must stay multiple-of-4 aligned and the
        // verdict fields must survive.
        let shaped = shape_response(
            RuntimeHttpEgressResponse {
                status: 403,
                headers: (0..32)
                    .map(|i| (format!("x-header-{i}"), "h".repeat(1024)))
                    .collect(),
                body: vec![0xFF; 512],
                saved_body: None,
                request_bytes: 0,
                response_bytes: 512,
                redaction_applied: false,
            },
            48 * 1024,
        );
        assert!(
            shaped
                .output
                .get("body_base64")
                .and_then(Value::as_str)
                .is_some(),
            "512-byte binary body must inline as body_base64"
        );

        let diagnostic = bounded_failure_diagnostic(shaped.output, 403);
        assert!(
            diagnostic.len() <= MODEL_DIAGNOSTIC_MAX_BYTES,
            "diagnostic must fit the budget, got {} bytes",
            diagnostic.len()
        );
        let parsed: Value =
            serde_json::from_str(&diagnostic).expect("diagnostic must be valid JSON");
        assert_eq!(parsed["status"], json!(403));
        assert_eq!(parsed["truncation"]["headers"], json!(true));
        let trimmed_base64 = parsed["body_base64"]
            .as_str()
            .expect("base64 body survives");
        assert_eq!(
            trimmed_base64.len() % 4,
            0,
            "trimmed base64 must stay multiple-of-4 aligned"
        );
    }

    #[test]
    fn failure_diagnostic_falls_back_when_fit_cannot_reach_budget() {
        // The post-fit safety valve: an untrimmable oversized key (nothing
        // fit_output_to_budget trims) must never produce an over-budget
        // diagnostic — the fixed verdict payload is returned instead.
        let mut output = Map::new();
        output.insert("status".to_string(), json!(403));
        output.insert(
            "huge_untrimmed".to_string(),
            Value::String("x".repeat(64 * 1024)),
        );
        let diagnostic = bounded_failure_diagnostic(Value::Object(output), 403);
        assert!(
            diagnostic.len() <= MODEL_DIAGNOSTIC_MAX_BYTES,
            "diagnostic must never exceed the budget, got {} bytes",
            diagnostic.len()
        );
        assert_eq!(diagnostic, fallback_diagnostic(403, None));
    }

    #[test]
    fn failure_diagnostic_non_object_output_falls_back_to_verdict() {
        // A non-object output funnels to the fixed verdict payload; the
        // fallback shape is part of the contract. (serialize_diagnostic's
        // serde-failure arm is a pure defensive guard: serde_json serializes
        // every Value string without revalidating UTF-8, probed empirically,
        // and no safe API can construct text serde would reject.)
        let diagnostic = bounded_failure_diagnostic(Value::Null, 403);
        assert_eq!(
            diagnostic,
            "{\"error\":\"diagnostic unavailable\",\"status\":403}"
        );
    }

    #[test]
    fn failure_diagnostic_fallback_retains_saved_body_evidence() {
        // A save path longer than the diagnostic budget must not erase the
        // saved-body evidence: the fallback keeps a bounded path prefix and
        // bytes_written so the model can still inspect the saved response.
        let mut output = Map::new();
        output.insert("status".to_string(), json!(403));
        output.insert(
            "saved_body".to_string(),
            json!({"path": format!("/workspace/{}", "x".repeat(8 * 1024)), "bytes_written": 42}),
        );
        output.insert(
            "huge_untrimmed".to_string(),
            Value::String("x".repeat(64 * 1024)),
        );
        let diagnostic = bounded_failure_diagnostic(Value::Object(output), 403);
        assert!(
            diagnostic.len() <= MODEL_DIAGNOSTIC_MAX_BYTES,
            "diagnostic must never exceed the budget, got {} bytes",
            diagnostic.len()
        );
        let parsed: Value =
            serde_json::from_str(&diagnostic).expect("fallback must stay valid JSON");
        assert_eq!(parsed["status"], json!(403));
        let saved = parsed["saved_body"]
            .as_object()
            .expect("saved_body evidence retained");
        assert_eq!(saved["bytes_written"], json!(42));
        let path = saved["path"].as_str().expect("path retained");
        assert!(
            path.len() <= FALLBACK_SAVED_BODY_PATH_BUDGET,
            "fallback path must stay bounded"
        );
        assert!(path.starts_with("/workspace/"));
    }

    #[test]
    fn failure_diagnostic_stays_within_budget_when_fenced() {
        // The loop-host seam wraps injection-shaped diagnostics in the
        // external-content fence before the observation boundary; the reserved
        // headroom must keep the fenced diagnostic within the budget.
        let shaped = shape_response(
            RuntimeHttpEgressResponse {
                status: 403,
                headers: (0..32)
                    .map(|i| (format!("x-header-{i}"), "h".repeat(1024)))
                    .collect(),
                body: vec![b'a'; 32 * 1024],
                saved_body: None,
                request_bytes: 0,
                response_bytes: 32 * 1024,
                redaction_applied: false,
            },
            48 * 1024,
        );
        let diagnostic = bounded_failure_diagnostic(shaped.output, 403);
        assert!(
            diagnostic.len() <= MODEL_DIAGNOSTIC_MAX_BYTES,
            "unfenced diagnostic must fit, got {} bytes",
            diagnostic.len()
        );
        let fenced = ironclaw_safety::wrap_external_content("http", &diagnostic);
        assert!(
            fenced.len() <= MODEL_DIAGNOSTIC_MAX_BYTES,
            "fenced diagnostic must stay within the observation budget, got {} bytes",
            fenced.len()
        );
    }

    #[test]
    fn classify_status_failure_carries_egress_and_wall_clock_usage() {
        // The OperationFailed outcome must carry the same usage accounting as
        // the sibling first-party dispatch paths: egress bytes and wall time.
        let shaped = shape_response(response_with_status(403), 48 * 1024);
        let error = match classify_status(shaped, 403, 1_234) {
            Err(error) => error,
            Ok(_) => panic!("4xx must classify as a capability failure"),
        };
        let usage = error.usage().expect("failure must carry usage");
        assert_eq!(usage.wall_clock_ms, 1_234);
        assert_eq!(
            usage.network_egress_bytes, 0,
            "empty GET request carries no egress bytes"
        );

        let shaped = shape_response(response_with_status(200), 48 * 1024);
        assert!(
            classify_status(shaped, 200, 1_234).is_ok(),
            "2xx must remain a successful result"
        );
    }

    #[test]
    fn failure_diagnostic_envelope_keeps_shape_stage_truncation_flags() {
        // 33 headers trip the shape-stage header cap (MAX_MODEL_VISIBLE_RESPONSE_HEADERS)
        // while the body alone absorbs the diagnostic-budget trim, so the
        // re-inserted envelope must OR the shape-stage truncation state in.
        let shaped = shape_response(
            RuntimeHttpEgressResponse {
                status: 403,
                headers: (0..33)
                    .map(|i| (format!("x-header-{i}"), "value".to_string()))
                    .collect(),
                body: vec![b'x'; 8 * 1024],
                saved_body: None,
                request_bytes: 0,
                response_bytes: 8 * 1024,
                redaction_applied: false,
            },
            48 * 1024,
        );
        assert_eq!(shaped.output["headers_truncated"], json!(true));

        let diagnostic = bounded_failure_diagnostic(shaped.output, 403);
        let parsed: Value =
            serde_json::from_str(&diagnostic).expect("diagnostic must be valid JSON");
        assert_eq!(parsed["truncation"]["headers"], json!(true));
        assert_eq!(parsed["headers_truncated"], json!(true));
        assert!(diagnostic.len() <= MODEL_DIAGNOSTIC_MAX_BYTES);
    }

    #[test]
    fn truncation_envelope_fits_its_reserved_budget() {
        // The failure diagnostic trims to MODEL_DIAGNOSTIC_MAX_BYTES minus
        // MODEL_VISIBLE_HTTP_TRUNCATION_ENVELOPE_BYTES and inserts the
        // envelope afterwards; the envelope must never exceed the reserved
        // bytes or the diagnostic would blow the 4 KiB budget at the
        // resolution boundary.
        let mut output = Map::new();
        insert_truncation_envelope(&mut output, true, Some(usize::MAX));
        assert!(
            serialized_output_len(&output) < MODEL_VISIBLE_HTTP_TRUNCATION_ENVELOPE_BYTES,
            "truncation envelope must fit its reserved budget"
        );
    }

    #[test]
    fn failure_diagnostic_scrubs_disallowed_control_characters() {
        // DEL/C1 control bytes are not escaped by serde_json and would fail
        // ModelDiagnostic validation at the resolution boundary.
        let mut body = vec![b'a'; 256];
        // DEL (U+007F) and C1 control (U+0081) are valid UTF-8 but are not
        // escaped by serde_json; they must be scrubbed from the diagnostic.
        body.extend_from_slice(&[0x7F, 0xC2, 0x81, b'b']);
        let shaped = shape_response(
            RuntimeHttpEgressResponse {
                status: 403,
                headers: Vec::new(),
                body,
                saved_body: None,
                request_bytes: 0,
                response_bytes: 259,
                redaction_applied: false,
            },
            48 * 1024,
        );

        let diagnostic = bounded_failure_diagnostic(shaped.output, 403);
        assert!(
            !diagnostic
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')),
            "diagnostic must not carry raw control bytes"
        );
        let parsed: Value =
            serde_json::from_str(&diagnostic).expect("diagnostic must be valid JSON");
        assert_eq!(parsed["status"], json!(403));
    }

    #[test]
    fn failure_diagnostic_preserves_saved_body_metadata_without_inline_body() {
        let shaped = shape_response(
            RuntimeHttpEgressResponse {
                status: 403,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: Vec::new(),
                saved_body: Some(RuntimeHttpSavedBody {
                    path: ScopedPath::new("/workspace/x.json").unwrap(),
                    bytes_written: 10,
                }),
                request_bytes: 0,
                response_bytes: 10,
                redaction_applied: false,
            },
            1024,
        );

        let diagnostic = bounded_failure_diagnostic(shaped.output, 403);
        let parsed: Value =
            serde_json::from_str(&diagnostic).expect("diagnostic must be valid JSON");
        assert_eq!(parsed["status"], json!(403));
        assert_eq!(
            parsed["saved_body"],
            json!({"path": "/workspace/x.json", "bytes_written": 10})
        );
        assert!(
            parsed.get("body_text").is_none(),
            "save-mode diagnostics carry saved_body metadata, not the inline body"
        );
    }

    #[test]
    fn small_failure_diagnostic_is_serialized_untrimmed() {
        let shaped = shape_response(response_with_status(403), 1024);
        let diagnostic = bounded_failure_diagnostic(shaped.output, 403);
        let parsed: Value =
            serde_json::from_str(&diagnostic).expect("diagnostic must be valid JSON");
        assert_eq!(parsed["status"], json!(403));
        assert!(diagnostic.len() <= MODEL_DIAGNOSTIC_MAX_BYTES);
    }
}
