//! Stable, bounded failure tokens for the MCP lane.
//!
//! Every reason string the lane surfaces is built here, from one of the three
//! cause enums below. Modules classify a failure; this module is the only one
//! that names it. That is what keeps the model-visible token set enumerable in
//! one file and every untrusted fragment bounded.

/// Maximum byte length for a diagnostic reason string surfaced to the
/// runtime/model. These tokens carry protocol codes, HTTP statuses, and
/// bounded JSON-RPC messages through a private cause channel. They are still
/// untrusted and may contain secrets until the downstream model-visible scrub
/// seam processes them, so every reason is capped here as defense in depth.
pub(crate) const MAX_MCP_REASON_BYTES: usize = 512;

/// Bound an untrusted diagnostic fragment to [`MAX_MCP_REASON_BYTES`],
/// truncating on a char boundary and appending an ellipsis marker so the
/// reader knows the value was clipped.
pub(crate) fn bound_mcp_reason_detail(detail: &str) -> String {
    const ELLIPSIS: &str = "...";
    let normalized: String = detail
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if normalized.len() <= MAX_MCP_REASON_BYTES {
        return normalized;
    }
    let budget = MAX_MCP_REASON_BYTES.saturating_sub(ELLIPSIS.len());
    let mut end = budget;
    while end > 0 && !normalized.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{ELLIPSIS}", &normalized[..end])
}

/// Per-cause request-side (pre-send / planning) failure tokens. Each carries
/// a stable prefix so callers and the model can classify the failure, plus
/// bounded diagnostic detail where available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum McpRequestDeniedCause {
    /// JSON-RPC request body could not be encoded.
    EncodeFailed(String),
    /// The planned request has no target URL.
    MissingUrl,
    /// The requested transport is not host-mediated HTTP/SSE.
    UnsupportedTransport,
    /// A credential injection used a denied source over this boundary.
    DeniedCredentialSource,
    /// The in-memory session map lock was poisoned.
    SessionStatePoisoned,
}

impl McpRequestDeniedCause {
    fn into_reason(self) -> String {
        match self {
            Self::EncodeFailed(detail) => {
                format!(
                    "mcp_request_encode_failed: {}",
                    bound_mcp_reason_detail(&detail)
                )
            }
            Self::MissingUrl => "mcp_missing_url".to_string(),
            Self::UnsupportedTransport => "mcp_unsupported_transport".to_string(),
            Self::DeniedCredentialSource => "mcp_denied_credential_source".to_string(),
            Self::SessionStatePoisoned => "mcp_session_state_poisoned".to_string(),
        }
    }
}

/// Per-cause response-side failure tokens. Each carries a stable prefix plus
/// bounded diagnostic detail (HTTP status, JSON-RPC code/message,
/// parse-failure cause) for the private model-visible cause channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum McpResponseErrorCause {
    /// Non-2xx HTTP status from the MCP endpoint.
    HttpStatus(u16),
    /// JSON-RPC `error` object with code and bounded message.
    JsonRpcError {
        code: Option<i64>,
        message: Option<String>,
    },
    /// Response body failed JSON parsing.
    ParseFailed(String),
    /// A successful response carried no `result` field.
    MissingResult,
    /// The endpoint returned an unsafe/oversized `Mcp-Session-Id`.
    InvalidSessionId,
    /// The `initialize` response carried an unsafe/missing protocol version.
    InvalidProtocolVersion,
    /// JSON-RPC response `id` did not match the request id.
    IdMismatch,
    /// Response did not contain a usable JSON-RPC payload (e.g. SSE with no
    /// matching data frame).
    NoPayload,
    /// Discovered `tools/list` result was malformed (shape/limits violation).
    InvalidToolList(McpInvalidToolListCause),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum McpInvalidToolListCause {
    MissingToolsArray,
    TooManyTools,
    InvalidToolName,
    InvalidDescription,
    MissingInputSchema,
    UnsafeInputSchema,
    InvalidAnnotations,
    InvalidCursor,
    TooManyPages,
    CatalogTooLarge,
}

impl McpInvalidToolListCause {
    pub(crate) const fn stable_token(self) -> &'static str {
        match self {
            Self::MissingToolsArray => "missing_tools_array",
            Self::TooManyTools => "too_many_tools",
            Self::InvalidToolName => "invalid_tool_name",
            Self::InvalidDescription => "invalid_description",
            Self::MissingInputSchema => "missing_input_schema",
            Self::UnsafeInputSchema => "unsafe_input_schema",
            Self::InvalidAnnotations => "invalid_annotations",
            Self::InvalidCursor => "invalid_cursor",
            Self::TooManyPages => "too_many_pages",
            Self::CatalogTooLarge => "catalog_too_large",
        }
    }
}

impl McpResponseErrorCause {
    fn into_reason(self) -> String {
        match self {
            Self::HttpStatus(status) => format!("mcp_http_status_{status}"),
            Self::JsonRpcError { code, message } => {
                let mut reason = String::from("mcp_jsonrpc_error");
                if let Some(code) = code {
                    reason.push_str(&format!(" code={code}"));
                }
                if let Some(message) = message {
                    reason.push_str(": ");
                    reason.push_str(&message);
                }
                reason
            }
            Self::ParseFailed(detail) => {
                format!("mcp_parse_failed: {}", bound_mcp_reason_detail(&detail))
            }
            Self::MissingResult => "mcp_missing_result".to_string(),
            Self::InvalidSessionId => "mcp_invalid_session_id".to_string(),
            Self::InvalidProtocolVersion => "mcp_invalid_protocol_version".to_string(),
            Self::IdMismatch => "mcp_jsonrpc_id_mismatch".to_string(),
            Self::NoPayload => "mcp_no_payload".to_string(),
            Self::InvalidToolList(cause) => {
                format!("mcp_invalid_tool_list: {}", cause.stable_token())
            }
        }
    }
}

pub(crate) fn request_denied(cause: McpRequestDeniedCause) -> String {
    cause.into_reason()
}

pub(crate) fn response_error(cause: McpResponseErrorCause) -> String {
    cause.into_reason()
}

pub(crate) fn invalid_tool_list(cause: McpInvalidToolListCause) -> String {
    response_error(McpResponseErrorCause::InvalidToolList(cause))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_denied_causes_map_to_stable_tokens() {
        assert_eq!(
            request_denied(McpRequestDeniedCause::MissingUrl),
            "mcp_missing_url"
        );
        assert_eq!(
            request_denied(McpRequestDeniedCause::UnsupportedTransport),
            "mcp_unsupported_transport"
        );
        assert_eq!(
            request_denied(McpRequestDeniedCause::DeniedCredentialSource),
            "mcp_denied_credential_source"
        );
        assert_eq!(
            request_denied(McpRequestDeniedCause::SessionStatePoisoned),
            "mcp_session_state_poisoned"
        );
        let encode = request_denied(McpRequestDeniedCause::EncodeFailed("eof".to_string()));
        assert!(encode.starts_with("mcp_request_encode_failed: "));
        assert!(encode.contains("eof"));
    }

    #[test]
    fn reason_detail_is_bounded_and_strips_control_chars() {
        let long = "a".repeat(10_000);
        let bounded = bound_mcp_reason_detail(&long);
        assert!(bounded.len() <= MAX_MCP_REASON_BYTES);
        assert!(bounded.ends_with("..."));

        let with_control = bound_mcp_reason_detail("line\nbreak\u{0000}null");
        assert!(!with_control.contains('\n'));
        assert!(!with_control.contains('\u{0000}'));
    }
}
