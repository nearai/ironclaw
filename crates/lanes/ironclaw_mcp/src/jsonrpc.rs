//! The JSON-RPC 2.0 codec and MCP response hygiene.
//!
//! One request is encoded here and one response is parsed here, in either
//! framing the client advertises (`application/json` and `text/event-stream`).
//! Everything this module validates is untrusted remote input: the response
//! `id`, the `Mcp-Session-Id`, the negotiated protocol version, and the
//! auth-challenge headers. Session *state* belongs to `client`; tool-shape
//! rules belong to `discovery`.

use ironclaw_extension_contracts::hosted_mcp::McpAuthChallenge;
use ironclaw_host_api::{
    http::{RuntimeCredentialInjection, RuntimeCredentialSource},
    resource::ResourceUsage,
};
use serde_json::Value;

use crate::diagnostics::{
    McpRequestDeniedCause, McpResponseErrorCause, bound_mcp_reason_detail, request_denied,
    response_error,
};
use crate::egress::McpHostHttpResponse;

pub(crate) const STREAMABLE_HTTP_MCP_PROTOCOL_VERSION: &str = "2025-06-18";
pub(crate) const MCP_PROTOCOL_VERSION_HEADER: &str = "MCP-Protocol-Version";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct McpJsonRpcResponse {
    pub(crate) result: Option<Value>,
    pub(crate) error: Option<JsonRpcErrorInfo>,
}

/// Bounded view of a JSON-RPC `error` object surfaced through the private
/// model-visible cause channel. The server-provided `message` remains untrusted:
/// it is scrubbed at the model-visible diagnostic seam before reaching the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsonRpcErrorInfo {
    pub(crate) code: Option<i64>,
    pub(crate) message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct McpJsonRpcExchange {
    pub(crate) response: McpJsonRpcResponse,
    pub(crate) session_id: Option<String>,
    pub(crate) usage: ResourceUsage,
}

/// Known MCP JSON-RPC methods whose credential-routing behavior is host-owned.
///
/// Hosted MCP providers may require bearer authentication for the whole
/// JSON-RPC session, including `initialize` and notifications. The host egress
/// planner remains the source of truth for which staged credentials may be
/// sent to the provider URL, and direct secret-store leases are rejected before
/// outbound transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum McpJsonRpcMethod {
    Initialize,
    InitializedNotification,
    ToolsList,
    ToolsCall,
}

impl McpJsonRpcMethod {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Initialize => "initialize",
            Self::InitializedNotification => "notifications/initialized",
            Self::ToolsList => "tools/list",
            Self::ToolsCall => "tools/call",
        }
    }

    pub(crate) fn credential_injections(
        self,
        credential_injections: Vec<RuntimeCredentialInjection>,
    ) -> Result<Vec<RuntimeCredentialInjection>, String> {
        if credential_injections
            .iter()
            .any(|injection| matches!(injection.source, RuntimeCredentialSource::SecretStoreLease))
        {
            return Err(request_denied(
                McpRequestDeniedCause::DeniedCredentialSource,
            ));
        }
        Ok(credential_injections)
    }
}

/// Validate credential injections planned for a `tools/call` request without
/// consuming the list, so the caller can reuse it in the actual send.
///
/// Returns `Err(denied)` if any injection uses a [`RuntimeCredentialSource::SecretStoreLease`],
/// which is not permitted over the MCP `tools/call` boundary.
pub(crate) fn validate_tools_call_credential_injections(
    credential_injections: &[RuntimeCredentialInjection],
) -> Result<(), String> {
    validate_staged_credential_injections(credential_injections)
}

pub(crate) fn validate_staged_credential_injections(
    credential_injections: &[RuntimeCredentialInjection],
) -> Result<(), String> {
    if credential_injections
        .iter()
        .any(|injection| matches!(injection.source, RuntimeCredentialSource::SecretStoreLease))
    {
        return Err(request_denied(
            McpRequestDeniedCause::DeniedCredentialSource,
        ));
    }
    Ok(())
}

pub(crate) fn is_mcp_auth_response_status(status: u16) -> bool {
    matches!(status, 401 | 403)
}

pub(crate) fn mcp_auth_challenge_from_response(response: &McpHostHttpResponse) -> McpAuthChallenge {
    let mut www_authenticate_metadata = Vec::new();
    let mut protected_resource_metadata = Vec::new();
    for (name, value) in &response.headers {
        if name.eq_ignore_ascii_case("www-authenticate") {
            www_authenticate_metadata.extend(
                ironclaw_extension_contracts::hosted_mcp::extract_mcp_auth_metadata_locations(
                    value,
                ),
            );
        } else if name.eq_ignore_ascii_case("protected-resource-metadata") {
            protected_resource_metadata.extend(
                ironclaw_extension_contracts::hosted_mcp::extract_mcp_auth_metadata_locations(
                    value,
                ),
            );
        }
    }
    McpAuthChallenge {
        status: response.status,
        www_authenticate_metadata,
        protected_resource_metadata,
    }
}

fn is_safe_mcp_session_id(value: &str) -> bool {
    const MAX_MCP_SESSION_ID_BYTES: usize = 1024;
    !value.is_empty()
        && value.len() <= MAX_MCP_SESSION_ID_BYTES
        && value.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
}

pub(crate) fn mcp_session_id_from_response(
    response: &McpHostHttpResponse,
) -> Result<Option<String>, String> {
    let Some((_, value)) = response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Mcp-Session-Id"))
    else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if !is_safe_mcp_session_id(trimmed) {
        return Err(response_error(McpResponseErrorCause::InvalidSessionId));
    }
    Ok(Some(trimmed.to_string()))
}

fn is_safe_mcp_protocol_version(value: &str) -> bool {
    const MAX_MCP_PROTOCOL_VERSION_BYTES: usize = 64;
    !value.is_empty()
        && value.len() <= MAX_MCP_PROTOCOL_VERSION_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

pub(crate) fn protocol_version_from_initialize_response(
    response: &McpJsonRpcResponse,
) -> Result<String, String> {
    let Some(protocol_version) = response
        .result
        .as_ref()
        .and_then(|result| result.get("protocolVersion"))
        .and_then(Value::as_str)
    else {
        return Err(response_error(
            McpResponseErrorCause::InvalidProtocolVersion,
        ));
    };
    if !is_safe_mcp_protocol_version(protocol_version) {
        return Err(response_error(
            McpResponseErrorCause::InvalidProtocolVersion,
        ));
    }
    Ok(protocol_version.to_string())
}

pub(crate) fn encode_json_rpc_request(
    id: Option<u64>,
    method: &str,
    params: Option<Value>,
) -> Result<Vec<u8>, String> {
    let mut object = serde_json::Map::new();
    object.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
    if let Some(id) = id {
        object.insert(
            "id".to_string(),
            Value::Number(serde_json::Number::from(id)),
        );
    }
    object.insert("method".to_string(), Value::String(method.to_string()));
    if let Some(params) = params {
        object.insert("params".to_string(), params);
    }
    serde_json::to_vec(&Value::Object(object))
        .map_err(|err| request_denied(McpRequestDeniedCause::EncodeFailed(err.to_string())))
}

pub(crate) fn parse_mcp_response(
    response: &McpHostHttpResponse,
    expected_id: Option<u64>,
) -> Result<McpJsonRpcResponse, String> {
    if response_is_sse(response) {
        parse_mcp_sse_response(&response.body, expected_id)
    } else {
        let value = serde_json::from_slice::<Value>(&response.body)
            .map_err(|err| response_error(McpResponseErrorCause::ParseFailed(err.to_string())))?;
        parse_mcp_json_rpc_value(&value, expected_id)
    }
}

fn response_is_sse(response: &McpHostHttpResponse) -> bool {
    response.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type")
            && value.to_ascii_lowercase().contains("text/event-stream")
    })
}

fn parse_mcp_sse_response(
    body: &[u8],
    expected_id: Option<u64>,
) -> Result<McpJsonRpcResponse, String> {
    let text = std::str::from_utf8(body)
        .map_err(|err| response_error(McpResponseErrorCause::ParseFailed(err.to_string())))?;
    let mut event_data = String::new();
    for line in text.lines().chain(std::iter::once("")) {
        if !line.is_empty() {
            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.strip_prefix(' ').unwrap_or(payload);
            if !event_data.is_empty() {
                event_data.push('\n');
            }
            event_data.push_str(payload);
            continue;
        }
        if event_data.trim().is_empty() {
            event_data.clear();
            continue;
        }
        let value = serde_json::from_str::<Value>(&event_data);
        event_data.clear();
        let Ok(value) = value else {
            continue;
        };
        let parsed_id = json_rpc_id(&value);
        if expected_id.is_none() || parsed_id == expected_id {
            return parse_mcp_json_rpc_value(&value, expected_id);
        }
    }
    Err(response_error(McpResponseErrorCause::NoPayload))
}

fn parse_mcp_json_rpc_value(
    value: &Value,
    expected_id: Option<u64>,
) -> Result<McpJsonRpcResponse, String> {
    let parsed_id = json_rpc_id(value);
    if let Some(expected) = expected_id
        && parsed_id != Some(expected)
    {
        return Err(response_error(McpResponseErrorCause::IdMismatch));
    }
    Ok(McpJsonRpcResponse {
        result: value.get("result").cloned(),
        error: parse_json_rpc_error_info(value.get("error")),
    })
}

/// Extract a bounded view of a JSON-RPC `error` object. Returns
/// `None` when no `error` member is present. A non-object `error` member still
/// counts as an error, but carries no structured code/message.
fn parse_json_rpc_error_info(error: Option<&Value>) -> Option<JsonRpcErrorInfo> {
    let error = error?;
    let code = error.get("code").and_then(Value::as_i64);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(bound_mcp_reason_detail);
    Some(JsonRpcErrorInfo { code, message })
}

fn json_rpc_id(value: &Value) -> Option<u64> {
    match value.get("id") {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(value)) => value.parse::<u64>().ok(),
        _ => None,
    }
}

pub(crate) fn json_rpc_initialize_params() -> Value {
    serde_json::json!({
        "protocolVersion": STREAMABLE_HTTP_MCP_PROTOCOL_VERSION,
        "capabilities": {
            "roots": { "listChanged": false },
            "sampling": {}
        },
        "clientInfo": {
            "name": "ironclaw",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_mcp_sse_response_skips_empty_data_keepalives() {
        let body = b"event: ping\ndata:\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n";

        let response = parse_mcp_sse_response(body, Some(7))
            .expect("empty SSE data lines should not abort parsing");

        assert_eq!(response.result, Some(json!({"ok": true})));
        assert!(response.error.is_none());
    }

    #[test]
    fn parse_mcp_sse_response_joins_one_events_data_lines() {
        let body = br##"event: message
data: {
data:   "jsonrpc": "2.0",
data:   "id": 7,
data:   "result": {
data:     "content": [
data:       {"type": "text", "text": "# NEAR AI\nURL: https://cloud-api.near.ai"}
data:     ]
data:   }
data: }

"##;

        let response = parse_mcp_sse_response(body, Some(7))
            .expect("one SSE event may split its JSON over repeated data lines");

        assert_eq!(
            response.result,
            Some(json!({
                "content": [{
                    "type": "text",
                    "text": "# NEAR AI\nURL: https://cloud-api.near.ai"
                }]
            }))
        );
        assert!(response.error.is_none());
    }

    /// Build an `McpHostHttpResponse` with a caller-chosen `content-type` and
    /// raw body bytes — the two inputs `parse_mcp_response` sniffs to pick the
    /// SSE vs plain-JSON branch. Fixtures below are hand-authored (there are no
    /// live-captured MCP response bodies under `tests/fixtures/`), but their
    /// framings mirror what a spec-compliant Streamable-HTTP MCP server emits.
    fn mcp_response(content_type: &str, body: &[u8]) -> McpHostHttpResponse {
        McpHostHttpResponse {
            status: 200,
            headers: vec![("content-type".to_string(), content_type.to_string())],
            body: body.to_vec(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: body.len() as u64,
            redaction_applied: false,
        }
    }

    /// Format matrix for the single `parse_mcp_response` dispatch that every
    /// JSON-RPC leg (`initialize`/`tools/list`/`tools/call`) funnels through.
    /// The client advertises `Accept: application/json, text/event-stream`
    /// (two content types), so the parser must accept BOTH framings for the
    /// same logical response — this pins that parity at the dispatch entry
    /// point, not just at `parse_mcp_sse_response` (already covered above).
    #[test]
    fn parse_mcp_response_accepts_both_advertised_framings() {
        let id = Some(7u64);
        let ok_body = br#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#;

        // Plain JSON framing (content-type application/json).
        let json = parse_mcp_response(&mcp_response("application/json", ok_body), id)
            .expect("plain JSON framing parses");
        assert_eq!(json.result, Some(json!({"ok": true})));
        assert!(json.error.is_none());

        // SSE single-event framing (content-type text/event-stream).
        let sse_single = parse_mcp_response(
            &mcp_response(
                "text/event-stream",
                b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n",
            ),
            id,
        )
        .expect("SSE single-event framing parses");
        assert_eq!(sse_single.result, Some(json!({"ok": true})));

        // SSE multi-event framing with a leading keepalive ping — the real
        // frame ordering a streaming server emits.
        let sse_multi = parse_mcp_response(
            &mcp_response(
                "text/event-stream; charset=utf-8",
                b"event: ping\ndata:\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n",
            ),
            id,
        )
        .expect("SSE multi-event framing parses past the keepalive");
        assert_eq!(sse_multi.result, Some(json!({"ok": true})));
    }

    /// Error-object framing (a JSON-RPC `error` member) is surfaced as
    /// `error == true` — in BOTH framings — rather than mis-parsed as success
    /// or dropped. This is the recoverable, model-visible tool-error leg.
    #[test]
    fn parse_mcp_response_flags_error_object_in_both_framings() {
        let id = Some(3u64);
        let json_err = parse_mcp_response(
            &mcp_response(
                "application/json",
                br#"{"jsonrpc":"2.0","id":3,"error":{"code":-32602,"message":"bad"}}"#,
            ),
            id,
        )
        .expect("JSON error-object is a valid response, not a parse failure");
        assert!(
            json_err.error.is_some(),
            "plain-JSON error object flags error"
        );
        assert_eq!(json_err.result, None, "error object carries no result");

        let sse_err = parse_mcp_response(
            &mcp_response(
                "text/event-stream",
                b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":3,\"error\":{\"code\":-32602,\"message\":\"bad\"}}\n\n",
            ),
            id,
        )
        .expect("SSE error-object is a valid response, not a parse failure");
        assert!(
            sse_err.error.is_some(),
            "SSE-framed error object flags error"
        );
        assert_eq!(sse_err.result, None, "error object carries no result");
    }

    /// Empty / malformed bodies are rejected in both framings (mutation guard:
    /// a parser that returned an empty-`result` success here would flip these
    /// `Err`s to `Ok`). An empty plain-JSON body has no JSON value; an SSE body
    /// with only keepalives has no `data:` payload carrying the expected id.
    #[test]
    fn parse_mcp_response_rejects_empty_bodies_in_both_framings() {
        let id = Some(9u64);
        // Per-cause diagnostic tokens replaced the flat "response_error": an
        // unparseable JSON body reports `mcp_parse_failed` (with a bounded
        // serde detail), and an SSE stream with no id-matching data reports
        // `mcp_no_payload`. Both remain hard errors, not silent successes.
        let empty_json_err = parse_mcp_response(&mcp_response("application/json", b""), id)
            .expect_err("empty plain-JSON body must not parse as a success");
        assert!(
            empty_json_err.starts_with("mcp_parse_failed"),
            "empty plain-JSON body must report a parse failure, got {empty_json_err:?}"
        );
        assert_eq!(
            parse_mcp_response(
                &mcp_response("text/event-stream", b"event: ping\ndata:\n\n"),
                id,
            )
            .unwrap_err(),
            "mcp_no_payload",
            "SSE body with only keepalives (no id-matching data) must not parse"
        );
    }

    fn json_response(status: u16, body: Value) -> McpHostHttpResponse {
        McpHostHttpResponse {
            status,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: serde_json::to_vec(&body).expect("serialize test body"),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: false,
        }
    }

    #[test]
    fn json_rpc_error_response_reason_carries_code_and_message() {
        let response = json_response(
            200,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": { "code": -32601, "message": "Method not found" }
            }),
        );

        let parsed = parse_mcp_response(&response, Some(1)).expect("parse json-rpc error response");
        let error = parsed.error.expect("error object captured");

        // Drive the same reason construction the call sites use.
        let reason = response_error(McpResponseErrorCause::JsonRpcError {
            code: error.code,
            message: error.message,
        });
        assert!(
            reason.contains("-32601"),
            "reason should carry the standardized protocol code: {reason}"
        );
        assert!(
            reason.contains("Method not found"),
            "backend diagnostic should reach the private cause channel: {reason}"
        );
        assert!(reason.starts_with("mcp_jsonrpc_error"));
    }

    #[test]
    fn json_rpc_error_without_structured_fields_still_classifies() {
        let response = json_response(200, json!({ "jsonrpc": "2.0", "id": 4, "error": "boom" }));

        let parsed = parse_mcp_response(&response, Some(4)).expect("parse non-object error");
        let error = parsed.error.expect("error present even when non-object");
        assert_eq!(error.code, None);
        assert_eq!(error.message, None);
        let reason = response_error(McpResponseErrorCause::JsonRpcError {
            code: error.code,
            message: error.message,
        });
        assert_eq!(reason, "mcp_jsonrpc_error");
    }

    #[test]
    fn auth_challenge_redacts_response_body_and_preserves_only_metadata_locations() {
        let response = McpHostHttpResponse {
            status: 401,
            headers: vec![
                (
                    "WWW-Authenticate".to_string(),
                    "Bearer resource_metadata=\"https://issuer.example.test/.well-known/oauth-protected-resource?access_token=secret\"".to_string(),
                ),
                (
                    "protected-resource-metadata".to_string(),
                    "https://resource.example.test/.well-known/oauth-protected-resource#secret"
                        .to_string(),
                ),
            ],
            body: b"token=super-secret remote diagnostic".to_vec(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 42,
            redaction_applied: false,
        };

        let challenge = mcp_auth_challenge_from_response(&response);
        assert_eq!(challenge.status, 401);
        assert_eq!(
            challenge.www_authenticate_metadata[0].as_str(),
            "https://issuer.example.test/.well-known/oauth-protected-resource"
        );
        assert_eq!(
            challenge.protected_resource_metadata[0].as_str(),
            "https://resource.example.test/.well-known/oauth-protected-resource"
        );
        let rendered = format!("{challenge:?}");
        assert!(!rendered.contains("super-secret"));
        assert!(!rendered.contains("access_token"));
    }

    #[test]
    fn malformed_json_body_reason_names_parse_failure() {
        let response = McpHostHttpResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: b"{ this is not json".to_vec(),
            saved_body: None,
            request_bytes: 0,
            response_bytes: 0,
            redaction_applied: false,
        };

        let reason = parse_mcp_response(&response, Some(1)).expect_err("malformed body must fail");
        assert!(
            reason.starts_with("mcp_parse_failed:"),
            "reason should name parse failure: {reason}"
        );
    }

    #[test]
    fn successful_result_response_has_no_error() {
        let response = json_response(
            200,
            json!({ "jsonrpc": "2.0", "id": 9, "result": { "ok": true } }),
        );

        let parsed = parse_mcp_response(&response, Some(9)).expect("success path unchanged");
        assert_eq!(parsed.result, Some(json!({ "ok": true })));
        assert!(parsed.error.is_none());
    }

    #[test]
    fn id_mismatch_reason_is_stable_token() {
        let response = json_response(
            200,
            json!({ "jsonrpc": "2.0", "id": 2, "result": { "ok": true } }),
        );

        let reason = parse_mcp_response(&response, Some(1)).expect_err("id mismatch must fail");
        assert_eq!(reason, "mcp_jsonrpc_id_mismatch");
    }

    #[test]
    fn invalid_session_and_protocol_reasons_are_distinct_tokens() {
        assert_eq!(
            response_error(McpResponseErrorCause::InvalidSessionId),
            "mcp_invalid_session_id"
        );
        assert_eq!(
            response_error(McpResponseErrorCause::InvalidProtocolVersion),
            "mcp_invalid_protocol_version"
        );
        assert_eq!(
            response_error(McpResponseErrorCause::MissingResult),
            "mcp_missing_result"
        );
    }
}
