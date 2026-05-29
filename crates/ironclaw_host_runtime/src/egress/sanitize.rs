use ironclaw_host_api::{
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, is_sensitive_runtime_request_header,
    is_sensitive_runtime_response_header,
};
use ironclaw_network::NetworkHttpResponse;
use ironclaw_safety::{LeakDetector, http_parts_contain_manual_credentials, redact_exact_values};
use std::borrow::Cow;

pub(super) fn validate_runtime_request(
    request: &RuntimeHttpEgressRequest,
    leak_detector: &LeakDetector,
) -> Result<(), RuntimeHttpEgressError> {
    if let Some((_name, _)) = request
        .headers
        .iter()
        .find(|(name, _)| is_sensitive_runtime_request_header(name))
    {
        return Err(RuntimeHttpEgressError::Request {
            reason: "sensitive_header_denied".to_string(),
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

    leak_detector
        .scan_http_request(&request.url, &request.headers, Some(&request.body))
        .map_err(|_| runtime_request_leak_error())?;
    scan_decoded_url_for_leaks(leak_detector, &request.url)?;
    Ok(())
}

fn runtime_request_contains_manual_credentials(request: &RuntimeHttpEgressRequest) -> bool {
    http_parts_contain_manual_credentials(&request.url, &request.headers)
}

fn scan_decoded_url_for_leaks(
    detector: &LeakDetector,
    raw_url: &str,
) -> Result<(), RuntimeHttpEgressError> {
    let Ok(parsed) = url::Url::parse(raw_url) else {
        return Ok(());
    };

    scan_decoded_component_for_leaks(detector, parsed.path())?;
    if !parsed.username().is_empty() {
        scan_decoded_component_for_leaks(detector, parsed.username())?;
    }
    if let Some(password) = parsed.password() {
        scan_decoded_component_for_leaks(detector, password)?;
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

/// Scan percent-decoded URL components for leak matches.
///
/// The raw URL string is scanned earlier, so this helper only needs to catch
/// decoded-delta forms that appear after parsing path and userinfo segments.
fn scan_decoded_component_for_leaks(
    detector: &LeakDetector,
    component: &str,
) -> Result<(), RuntimeHttpEgressError> {
    let decoded = percent_decode(component);
    if decoded.as_ref() != component {
        detector
            .scan_and_clean(decoded.as_ref())
            .map_err(|_| runtime_request_leak_error())?;
    }
    Ok(())
}

fn percent_decode(input: &str) -> Cow<'_, str> {
    if !input.as_bytes().contains(&b'%') {
        Cow::Borrowed(input)
    } else {
        percent_encoding::percent_decode_str(input).decode_utf8_lossy()
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
    leak_detector: &LeakDetector,
) -> Result<(NetworkHttpResponse, bool), RuntimeHttpEgressError> {
    let NetworkHttpResponse {
        status,
        headers,
        body,
        usage,
    } = response;
    let mut redaction_applied = false;
    let mut sanitized_headers = Vec::new();

    for (name, value) in headers {
        if is_sensitive_runtime_response_header(&name) {
            redaction_applied = true;
            continue;
        }
        let exact_redacted = redact_exact_values(value, redaction_values);
        if exact_redacted.contains("[REDACTED]") {
            redaction_applied = true;
        }
        let cleaned = leak_detector.scan_and_clean(&exact_redacted).map_err(|_| {
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
    let cleaned = leak_detector.scan_and_clean(&exact_redacted).map_err(|_| {
        RuntimeHttpEgressError::Response {
            reason: "response_leak_blocked".to_string(),
            request_bytes: usage.request_bytes,
            response_bytes: usage.response_bytes,
        }
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
