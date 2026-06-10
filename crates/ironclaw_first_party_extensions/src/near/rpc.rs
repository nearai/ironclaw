//! NEAR RPC + 1Click HTTP plumbing, decoding, and error helpers.

use super::*;

pub(crate) struct CallFunctionResponse {
    pub(crate) value: Value,
    pub(crate) egress_bytes: u64,
}

pub(crate) fn require_egress(
    request: &NearDispatchRequest<'_>,
) -> Result<Arc<dyn RuntimeHttpEgress>, NearDispatchError> {
    request
        .runtime_http_egress
        .as_ref()
        .ok_or_else(|| NearDispatchError::new(RuntimeDispatchErrorKind::NetworkDenied))
        .cloned()
}

/// POST an arbitrary JSON payload, returning the parsed response body and the
/// total bytes spent. HTTP-level errors map through `map_egress_error`.
pub(crate) async fn http_post_json(
    request: &NearDispatchRequest<'_>,
    egress: Arc<dyn RuntimeHttpEgress>,
    network_policy: NetworkPolicy,
    url: &str,
    payload: &Value,
) -> Result<(Value, u64), NearDispatchError> {
    let body = serde_json::to_vec(payload).map_err(|_| input_error())?;
    let http = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
        method: NetworkMethod::Post,
        url: url.to_string(),
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body,
        network_policy,
        credential_injections: Vec::new(),
        response_body_limit: Some(RESPONSE_BODY_LIMIT),
        save_body_to: None,
        timeout_ms: Some(DEFAULT_TIMEOUT_MS),
    };
    let resp = execute_runtime_http(http, egress)
        .await
        .map_err(map_egress_error)?;
    let egress_bytes = resp.request_bytes.saturating_add(resp.response_bytes);
    let parsed: Value =
        serde_json::from_slice(&resp.body).map_err(|_| output_decode_error(egress_bytes))?;
    Ok((parsed, egress_bytes))
}

/// Issue a `call_function` view query against `account_id`, base64-encoding the
/// JSON args. Checks the RPC `error` branch before returning.
pub(crate) async fn call_function(
    request: &NearDispatchRequest<'_>,
    egress: Arc<dyn RuntimeHttpEgress>,
    account_id: &str,
    method_name: &str,
    args: Option<&Value>,
) -> Result<CallFunctionResponse, NearDispatchError> {
    let args_base64 = encode_args(args)?;
    let rpc = json!({
        "jsonrpc": "2.0",
        "id": "1",
        "method": "query",
        "params": {
            "request_type": "call_function",
            "finality": "final",
            "account_id": account_id,
            "method_name": method_name,
            "args_base64": args_base64,
        }
    });
    let (body, egress_bytes) = http_post_json(
        request,
        egress,
        near_rpc_network_policy(),
        FASTNEAR_RPC_URL,
        &rpc,
    )
    .await?;
    if body.get("error").is_some() {
        return Err(operation_error(egress_bytes));
    }
    Ok(CallFunctionResponse {
        value: body,
        egress_bytes,
    })
}

/// base64-encode the JSON-serialized args object. `None`/null → empty string.
pub(crate) fn encode_args(args: Option<&Value>) -> Result<String, NearDispatchError> {
    match args {
        None => Ok(String::new()),
        Some(Value::Null) => Ok(String::new()),
        Some(value) => {
            let bytes = serde_json::to_vec(value).map_err(|_| input_error())?;
            Ok(STANDARD.encode(bytes))
        }
    }
}

/// Decode a `call_function` RPC body: `result.result` is a raw byte array (NOT
/// base64) that we convert directly to UTF-8 and JSON-parse.
/// Construct an `OutputDecode` error annotated with the bytes already spent on
/// the round-trip that produced the undecodable body.
pub(crate) fn output_decode_error(egress_bytes: u64) -> NearDispatchError {
    NearDispatchError::new(RuntimeDispatchErrorKind::OutputDecode).with_usage(ResourceUsage {
        network_egress_bytes: egress_bytes,
        ..ResourceUsage::default()
    })
}

pub(crate) fn decode_view_result(
    body: &Value,
    egress_bytes: u64,
) -> Result<Value, NearDispatchError> {
    if body.get("error").is_some() {
        return Err(operation_error(egress_bytes));
    }
    let raw = body
        .pointer("/result/result")
        .ok_or_else(|| output_decode_error(egress_bytes))?;
    let bytes: Vec<u8> =
        serde_json::from_value(raw.clone()).map_err(|_| output_decode_error(egress_bytes))?;
    // A view method returning nothing yields an empty byte array; surface it as
    // JSON null instead of failing to parse an empty slice.
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|_| output_decode_error(egress_bytes))
}

/// Extract the `result` object from a top-level query RPC body, mapping an RPC
/// `error` to `OperationFailed`.
pub(crate) fn rpc_result(body: &Value, egress_bytes: u64) -> Result<Value, NearDispatchError> {
    if body.get("error").is_some() {
        return Err(operation_error(egress_bytes));
    }
    body.get("result")
        .cloned()
        .ok_or_else(|| output_decode_error(egress_bytes))
}

pub(crate) fn success(output: Value, egress_bytes: u64) -> NearDispatchResult {
    let output_bytes = serde_json::to_vec(&output)
        .map(|bytes| bytes.len() as u64)
        .unwrap_or(0);
    NearDispatchResult {
        output,
        usage: ResourceUsage {
            output_bytes,
            network_egress_bytes: egress_bytes,
            ..ResourceUsage::default()
        },
    }
}

pub(crate) async fn execute_runtime_http(
    request: RuntimeHttpEgressRequest,
    egress: Arc<dyn RuntimeHttpEgress>,
) -> Result<ironclaw_host_api::RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
    std::panic::AssertUnwindSafe(egress.execute(request))
        .catch_unwind()
        .await
        .map_err(|_| RuntimeHttpEgressError::Network {
            reason: "worker_join".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        })?
}

pub(crate) fn near_rpc_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: FASTNEAR_RPC_HOST.to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(NETWORK_EGRESS_LIMIT),
    }
}

pub(crate) fn intents_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: INTENTS_HOST.to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(NETWORK_EGRESS_LIMIT),
    }
}

pub(crate) fn map_egress_error(error: RuntimeHttpEgressError) -> NearDispatchError {
    let kind = match error.reason_code() {
        RuntimeHttpEgressReasonCode::CredentialUnavailable => RuntimeDispatchErrorKind::Client,
        RuntimeHttpEgressReasonCode::RequestDenied => RuntimeDispatchErrorKind::InputEncode,
        RuntimeHttpEgressReasonCode::PolicyDenied => RuntimeDispatchErrorKind::PolicyDenied,
        RuntimeHttpEgressReasonCode::NetworkError => RuntimeDispatchErrorKind::NetworkDenied,
        RuntimeHttpEgressReasonCode::ResponseError => RuntimeDispatchErrorKind::OutputDecode,
        RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
            RuntimeDispatchErrorKind::OutputTooLarge
        }
    };
    NearDispatchError::new(kind).with_usage(ResourceUsage {
        network_egress_bytes: error.request_bytes(),
        ..ResourceUsage::default()
    })
}

pub(crate) fn input_error() -> NearDispatchError {
    NearDispatchError::new(RuntimeDispatchErrorKind::InputEncode)
}

pub(crate) fn operation_error(egress_bytes: u64) -> NearDispatchError {
    NearDispatchError::new(RuntimeDispatchErrorKind::OperationFailed).with_usage(ResourceUsage {
        network_egress_bytes: egress_bytes,
        ..ResourceUsage::default()
    })
}
