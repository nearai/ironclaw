use ironclaw_host_api::{
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, is_sensitive_runtime_request_header,
    is_sensitive_runtime_response_header,
};
use ironclaw_network::NetworkHttpResponse;
use ironclaw_safety::{LeakDetector, params_contain_manual_credentials, redact_exact_values};

pub(super) fn validate_runtime_request(
    request: &RuntimeHttpEgressRequest,
) -> Result<(), RuntimeHttpEgressError> {
    if let Some((name, _)) = request
        .headers
        .iter()
        .find(|(name, _)| is_sensitive_runtime_request_header(name))
    {
        return Err(RuntimeHttpEgressError::Request {
            reason: format!("sensitive_header_denied:{name}"),
            request_bytes: 0,
            response_bytes: 0,
        });
    }

    if runtime_request_contains_manual_credentials(request) {
        return Err(RuntimeHttpEgressError::Request {
            reason: "manual_credentials_denied".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        });
    }

    let detector = LeakDetector::new();
    detector
        .scan_http_request(&request.url, &request.headers, Some(&request.body))
        .map_err(|_| runtime_request_leak_error())?;
    scan_decoded_url_for_leaks(&detector, &request.url)?;
    Ok(())
}

fn runtime_request_contains_manual_credentials(request: &RuntimeHttpEgressRequest) -> bool {
    let headers = request
        .headers
        .iter()
        .map(|(name, value)| serde_json::json!({ "name": name, "value": value }))
        .collect::<Vec<_>>();
    let params = serde_json::json!({
        "url": request.url,
        "headers": headers,
    });
    params_contain_manual_credentials(&params)
}

fn scan_decoded_url_for_leaks(
    detector: &LeakDetector,
    raw_url: &str,
) -> Result<(), RuntimeHttpEgressError> {
    let Ok(parsed) = url::Url::parse(raw_url) else {
        return Ok(());
    };

    scan_component_for_leaks(detector, parsed.path())?;
    if let Some(query) = parsed.query() {
        scan_component_for_leaks(detector, query)?;
    }
    if !parsed.username().is_empty() {
        scan_component_for_leaks(detector, parsed.username())?;
    }
    if let Some(password) = parsed.password() {
        scan_component_for_leaks(detector, password)?;
    }
    for (name, value) in parsed.query_pairs() {
        detector
            .scan_and_clean(name.as_ref())
            .map_err(|_| runtime_request_leak_error())?;
        detector
            .scan_and_clean(value.as_ref())
            .map_err(|_| runtime_request_leak_error())?;
    }
    Ok(())
}

fn scan_component_for_leaks(
    detector: &LeakDetector,
    component: &str,
) -> Result<(), RuntimeHttpEgressError> {
    let decoded = percent_decode_lossy(component);
    if decoded != component {
        detector
            .scan_and_clean(&decoded)
            .map_err(|_| runtime_request_leak_error())?;
    }
    Ok(())
}

fn percent_decode_lossy(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            decoded.push((high << 4) | low);
            index += 3;
            continue;
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn runtime_request_leak_error() -> RuntimeHttpEgressError {
    RuntimeHttpEgressError::Request {
        reason: "credential_leak_blocked".to_string(),
        request_bytes: 0,
        response_bytes: 0,
    }
}

pub(super) fn sanitize_runtime_response(
    response: NetworkHttpResponse,
    redaction_values: &[String],
) -> Result<(NetworkHttpResponse, bool), RuntimeHttpEgressError> {
    let NetworkHttpResponse {
        status,
        headers,
        body,
        usage,
    } = response;
    let mut redaction_applied = false;
    let mut sanitized_headers = Vec::new();
    let detector = LeakDetector::new();

    for (name, value) in headers {
        if is_sensitive_runtime_response_header(&name) {
            redaction_applied = true;
            continue;
        }
        let exact_redacted = redact_exact_values(value, redaction_values);
        if exact_redacted.contains("[REDACTED]") {
            redaction_applied = true;
        }
        let cleaned = detector.scan_and_clean(&exact_redacted).map_err(|_| {
            RuntimeHttpEgressError::Response {
                reason: "response_leak_blocked".to_string(),
                request_bytes: usage.request_bytes,
                response_bytes: usage.response_bytes,
            }
        })?;
        if cleaned != exact_redacted {
            redaction_applied = true;
        }
        sanitized_headers.push((name, cleaned));
    }

    let body_text = String::from_utf8_lossy(&body).into_owned();
    let exact_redacted = redact_exact_values(body_text, redaction_values);
    let exact_body_redacted = exact_redacted.contains("[REDACTED]");
    if exact_body_redacted {
        redaction_applied = true;
    }
    let cleaned =
        detector
            .scan_and_clean(&exact_redacted)
            .map_err(|_| RuntimeHttpEgressError::Response {
                reason: "response_leak_blocked".to_string(),
                request_bytes: usage.request_bytes,
                response_bytes: usage.response_bytes,
            })?;
    let leak_detector_redacted = cleaned != exact_redacted;
    if leak_detector_redacted {
        redaction_applied = true;
    }
    let body = if exact_body_redacted || leak_detector_redacted {
        cleaned.into_bytes()
    } else {
        body
    };

    Ok((
        NetworkHttpResponse {
            status,
            headers: sanitized_headers,
            body,
            usage,
        },
        redaction_applied,
    ))
}
