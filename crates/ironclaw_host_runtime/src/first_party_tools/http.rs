use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    EffectKind, NetworkMethod, NetworkPolicy, PermissionMode, ResourceCeiling, ResourceEstimate,
    ResourceProfile, ResourceUsage, RuntimeDispatchErrorKind, RuntimeHttpEgressError,
    RuntimeHttpEgressReasonCode, RuntimeHttpEgressRequest, RuntimeKind, SandboxQuota,
    valid_http_field_name,
};
use serde_json::{Map, Value, json};

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{first_party_capability_manifest, input_error};

pub const HTTP_CAPABILITY_ID: &str = "builtin.http";

const DEFAULT_HTTP_TIMEOUT_MS: u32 = 10_000;
const MAX_HTTP_TIMEOUT_MS: u32 = 30_000;
pub(super) const MAX_HTTP_OUTPUT_BYTES: u64 = 15 * 1024 * 1024;
const DEFAULT_RESPONSE_BODY_LIMIT: u64 = 10 * 1024 * 1024;
const MAX_RESPONSE_BODY_LIMIT: u64 = 10 * 1024 * 1024;
const DEFAULT_NETWORK_EGRESS_BYTES: u64 = 16 * 1024;
const MAX_NETWORK_EGRESS_BYTES: u64 = 256 * 1024;
const MAX_HTTP_HEADERS: usize = 64;
const MAX_HTTP_HEADER_NAME_BYTES: usize = 512;
const MAX_HTTP_HEADER_VALUE_BYTES: usize = 8 * 1024;
pub(super) struct HttpDispatchOutput {
    pub output: Value,
    pub network_egress_bytes: u64,
}

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        HTTP_CAPABILITY_ID,
        "Perform an outbound HTTP request through host egress. Redirect responses are returned; the host transport does not follow them.",
        vec![EffectKind::DispatchCapability, EffectKind::Network],
        PermissionMode::Ask,
        Some(ResourceProfile {
            default_estimate: ResourceEstimate {
                wall_clock_ms: Some(DEFAULT_HTTP_TIMEOUT_MS.into()),
                output_bytes: Some(DEFAULT_RESPONSE_BODY_LIMIT),
                network_egress_bytes: Some(DEFAULT_NETWORK_EGRESS_BYTES),
                ..ResourceEstimate::default()
            },
            hard_ceiling: Some(ResourceCeiling {
                max_usd: None,
                max_input_tokens: None,
                max_output_tokens: None,
                max_wall_clock_ms: Some(MAX_HTTP_TIMEOUT_MS.into()),
                max_output_bytes: Some(MAX_HTTP_OUTPUT_BYTES),
                sandbox: Some(SandboxQuota {
                    network_egress_bytes: Some(MAX_NETWORK_EGRESS_BYTES),
                    ..SandboxQuota::default()
                }),
            }),
        }),
    )
}

pub(super) async fn dispatch(
    request: &FirstPartyCapabilityRequest,
) -> Result<HttpDispatchOutput, FirstPartyCapabilityError> {
    let egress = request
        .services
        .runtime_http_egress
        .as_ref()
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::NetworkDenied))?
        .clone();
    // Keep this handler as a translator only: URL parsing, DNS/private-IP
    // enforcement, allowlists, transport, and credential injection remain in
    // HostHttpEgressService / ironclaw_network.
    let unsafe_raw_diagnostics_allowed = request.services.unsafe_raw_diagnostics_allowed;
    let mut headers = headers(&request.input).map_err(|error| {
        log_raw_http_input_error_for_local_diagnostics(
            unsafe_raw_diagnostics_allowed,
            &request.input,
            "headers",
            error,
        )
    })?;
    if json_body_needs_default_content_type(&request.input) && !has_header(&headers, "content-type")
    {
        headers.push(("content-type".to_string(), "application/json".to_string()));
    }
    let method = method(&request.input).map_err(|error| {
        log_raw_http_input_error_for_local_diagnostics(
            unsafe_raw_diagnostics_allowed,
            &request.input,
            "method",
            error,
        )
    })?;
    let url = required_string(&request.input, "url")
        .map_err(|error| {
            log_raw_http_input_error_for_local_diagnostics(
                unsafe_raw_diagnostics_allowed,
                &request.input,
                "url",
                error,
            )
        })?
        .to_string();
    let body = body(&request.input).map_err(|error| {
        log_raw_http_input_error_for_local_diagnostics(
            unsafe_raw_diagnostics_allowed,
            &request.input,
            "body",
            error,
        )
    })?;
    let response_body_limit = response_body_limit(&request.input).map_err(|error| {
        log_raw_http_input_error_for_local_diagnostics(
            unsafe_raw_diagnostics_allowed,
            &request.input,
            "response_body_limit",
            error,
        )
    })?;
    let timeout_ms = timeout_ms(&request.input).map_err(|error| {
        log_raw_http_input_error_for_local_diagnostics(
            unsafe_raw_diagnostics_allowed,
            &request.input,
            "timeout_ms",
            error,
        )
    })?;
    let http_request = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
        method,
        url,
        headers,
        body,
        network_policy: staged_policy_placeholder(),
        credential_injections: Vec::new(),
        // Always send a bounded limit, even when caller omits the field, so the
        // host transport stays fail-closed instead of inheriting an unbounded cap.
        response_body_limit: Some(response_body_limit),
        timeout_ms: Some(timeout_ms),
    };
    let response = tokio::task::spawn_blocking(move || egress.execute(http_request))
        .await
        .map_err(|error| {
            if error.is_panic() {
                tracing::error!("first-party HTTP egress worker panicked");
            }
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
        })?
        .map_err(http_error)?;
    let mut output = Map::new();
    output.insert("status".to_string(), json!(response.status));
    output.insert("headers".to_string(), response_headers(response.headers));
    // Response bodies must be valid UTF-8 to appear as body_text. Any invalid
    // byte returns the full response as body_base64 to avoid lossy surprises.
    match String::from_utf8(response.body) {
        Ok(body_text) => {
            output.insert("body_text".to_string(), Value::String(body_text));
        }
        Err(error) => {
            output.insert(
                "body_base64".to_string(),
                Value::String(BASE64_STANDARD.encode(error.into_bytes())),
            );
        }
    }
    output.insert("request_bytes".to_string(), json!(response.request_bytes));
    output.insert("response_bytes".to_string(), json!(response.response_bytes));
    output.insert(
        "redaction_applied".to_string(),
        json!(response.redaction_applied),
    );
    Ok(HttpDispatchOutput {
        output: Value::Object(output),
        network_egress_bytes: response.request_bytes,
    })
}

fn method(input: &Value) -> Result<NetworkMethod, FirstPartyCapabilityError> {
    let method = match input.get("method") {
        Some(value) => value.as_str().ok_or_else(input_error)?,
        None => "get",
    };
    match method.to_ascii_lowercase().as_str() {
        "get" => Ok(NetworkMethod::Get),
        "post" => Ok(NetworkMethod::Post),
        "put" => Ok(NetworkMethod::Put),
        "patch" => Ok(NetworkMethod::Patch),
        "delete" => Ok(NetworkMethod::Delete),
        "head" => Ok(NetworkMethod::Head),
        _ => Err(input_error()),
    }
}

fn required_string<'a>(
    input: &'a Value,
    field: &str,
) -> Result<&'a str, FirstPartyCapabilityError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(input_error)
}

fn headers(input: &Value) -> Result<Vec<(String, String)>, FirstPartyCapabilityError> {
    let Some(headers) = input.get("headers") else {
        return Ok(Vec::new());
    };
    let parsed: Vec<(String, String)> = match headers {
        Value::Object(object) => object
            .iter()
            .map(|(name, value)| {
                let value = value.as_str().ok_or_else(input_error)?;
                header_pair(name, value)
            })
            .collect::<Result<_, _>>()?,
        Value::Array(items) => items
            .iter()
            .map(|item| {
                let name = required_string(item, "name")?;
                let value = required_string(item, "value")?;
                header_pair(name, value)
            })
            .collect::<Result<_, _>>()?,
        _ => return Err(input_error()),
    };
    if parsed.len() > MAX_HTTP_HEADERS {
        return Err(input_error());
    }
    Ok(parsed)
}

fn header_pair(name: &str, value: &str) -> Result<(String, String), FirstPartyCapabilityError> {
    if !valid_http_field_name(name)
        || name.len() > MAX_HTTP_HEADER_NAME_BYTES
        || value.len() > MAX_HTTP_HEADER_VALUE_BYTES
        || value
            .chars()
            .any(|character| matches!(character, '\r' | '\n' | '\0'))
    {
        return Err(input_error());
    }
    Ok((name.to_string(), value.to_string()))
}

fn body(input: &Value) -> Result<Vec<u8>, FirstPartyCapabilityError> {
    if input.get("body").is_some() && input.get("body_base64").is_some() {
        return Err(input_error());
    }
    let body = if let Some(encoded) = input.get("body_base64") {
        let encoded = encoded.as_str().ok_or_else(input_error)?;
        BASE64_STANDARD.decode(encoded).map_err(|_| input_error())?
    } else {
        match input.get("body") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::String(value)) => value.as_bytes().to_vec(),
            Some(value) => serde_json::to_vec(value).map_err(|_| input_error())?,
        }
    };
    if body.len() as u64 > MAX_NETWORK_EGRESS_BYTES {
        return Err(input_error());
    }
    Ok(body)
}

fn json_body_needs_default_content_type(input: &Value) -> bool {
    matches!(
        input.get("body"),
        Some(Value::Array(_))
            | Some(Value::Bool(_))
            | Some(Value::Number(_))
            | Some(Value::Object(_))
    )
}

fn has_header(headers: &[(String, String)], expected: &str) -> bool {
    headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(expected))
}

fn staged_policy_placeholder() -> NetworkPolicy {
    // First-party HTTP policy is staged in HostHttpEgressService from the grant
    // obligation for this scope/capability. This fallback request field is
    // ignored on that path and only exists for request-policy test services.
    NetworkPolicy::default()
}

fn response_body_limit(input: &Value) -> Result<u64, FirstPartyCapabilityError> {
    let limit = ranged_u64(
        input,
        "response_body_limit",
        DEFAULT_RESPONSE_BODY_LIMIT,
        1,
        MAX_RESPONSE_BODY_LIMIT,
    )?;
    Ok(limit.max(DEFAULT_RESPONSE_BODY_LIMIT))
}

fn timeout_ms(input: &Value) -> Result<u32, FirstPartyCapabilityError> {
    let value = ranged_u64(
        input,
        "timeout_ms",
        DEFAULT_HTTP_TIMEOUT_MS.into(),
        1,
        u64::MAX,
    )?;
    let value = value.min(u64::from(MAX_HTTP_TIMEOUT_MS));
    u32::try_from(value).map_err(|_| input_error())
}

fn ranged_u64(
    input: &Value,
    field: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, FirstPartyCapabilityError> {
    let Some(value) = input.get(field) else {
        return Ok(default);
    };
    let value = value.as_u64().ok_or_else(input_error)?;
    if value < min || value > max {
        return Err(input_error());
    }
    Ok(value)
}

fn response_headers(headers: Vec<(String, String)>) -> Value {
    Value::Array(
        headers
            .into_iter()
            .map(|(name, value)| json!({ "name": name, "value": value }))
            .collect(),
    )
}

fn http_error(error: RuntimeHttpEgressError) -> FirstPartyCapabilityError {
    let kind = match error.reason_code() {
        // Host credential injection failures are backend/client integration faults;
        // production maps RuntimeDispatchErrorKind::Client to RuntimeFailureKind::Backend.
        RuntimeHttpEgressReasonCode::CredentialUnavailable => RuntimeDispatchErrorKind::Client,
        RuntimeHttpEgressReasonCode::RequestDenied => RuntimeDispatchErrorKind::InputEncode,
        RuntimeHttpEgressReasonCode::NetworkError => RuntimeDispatchErrorKind::NetworkDenied,
        RuntimeHttpEgressReasonCode::ResponseError => RuntimeDispatchErrorKind::OperationFailed,
        RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
            RuntimeDispatchErrorKind::OutputTooLarge
        }
    };
    tracing::debug!(
        runtime_http_reason = error.stable_runtime_reason(),
        dispatch_error_kind = kind.as_str(),
        "first-party HTTP egress failed"
    );
    let mut usage = ResourceUsage::default();
    if !matches!(error, RuntimeHttpEgressError::Credential { .. }) {
        usage.network_egress_bytes = error.request_bytes();
    }
    FirstPartyCapabilityError::new(kind).with_usage(usage)
}

fn log_raw_http_input_error_for_local_diagnostics(
    unsafe_raw_diagnostics_allowed: bool,
    input: &Value,
    validation_stage: &'static str,
    error: FirstPartyCapabilityError,
) -> FirstPartyCapabilityError {
    if crate::unsafe_raw_http_diagnostics_enabled(unsafe_raw_diagnostics_allowed) {
        tracing::warn!(
            validation_stage,
            raw_http_tool_input = %input,
            unsafe_raw_diagnostics = true,
            "unsafe raw HTTP tool input diagnostic enabled"
        );
    }
    error
}
