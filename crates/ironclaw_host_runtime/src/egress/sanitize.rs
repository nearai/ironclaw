use ironclaw_host_api::{
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, is_sensitive_runtime_request_header,
    is_sensitive_runtime_response_header,
};
use ironclaw_network::{NetworkHttpResponse, percent_decode_url_component_lossy};
use ironclaw_safety::{LeakDetector, http_parts_contain_manual_credentials, redact_exact_values};

pub(super) fn validate_runtime_request(
    request: &RuntimeHttpEgressRequest,
    leak_detector: &LeakDetector,
) -> Result<(), RuntimeHttpEgressError> {
    // Outbound credentials must flow through the staged credential-injection
    // path (`credential_injections`), never as raw runtime/model-supplied
    // headers or `user:pass@` URLs — for ALL runtimes, including FirstParty.
    // `builtin.http` is FirstParty but takes model-supplied headers, so
    // exempting FirstParty here would let the model smuggle Authorization /
    // Cookie / x-api-key headers to allowlisted hosts. Host-minted credentials
    // (e.g. the Trace Commons bearer) are injected AFTER this guard via the
    // stager + `apply_credential_injections`, so they are not present here.
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

    scan_runtime_url_for_leaks(leak_detector, &request.url)?;
    scan_runtime_headers_and_body_for_leaks(leak_detector, request)?;
    Ok(())
}

fn runtime_request_contains_manual_credentials(request: &RuntimeHttpEgressRequest) -> bool {
    http_parts_contain_manual_credentials(&request.url, &request.headers)
}

fn scan_runtime_url_for_leaks(
    detector: &LeakDetector,
    raw_url: &str,
) -> Result<(), RuntimeHttpEgressError> {
    detector
        .scan_and_clean(raw_url)
        .map_err(|_| runtime_request_leak_error())?;
    scan_decoded_url_for_leaks(detector, raw_url)
}

fn scan_runtime_headers_and_body_for_leaks(
    detector: &LeakDetector,
    request: &RuntimeHttpEgressRequest,
) -> Result<(), RuntimeHttpEgressError> {
    for (_name, value) in &request.headers {
        detector
            .scan_and_clean(value)
            .map_err(|_| runtime_request_leak_error())?;
    }

    let body = String::from_utf8_lossy(&request.body);
    detector
        .scan_and_clean(&body)
        .map_err(|_| runtime_request_leak_error())?;
    Ok(())
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
    if let Some(fragment) = parsed.fragment() {
        scan_decoded_component_for_leaks(detector, fragment)?;
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
    let decoded = percent_decode_url_component_lossy(component);
    if decoded.as_ref() != component {
        detector
            .scan_and_clean(decoded.as_ref())
            .map_err(|_| runtime_request_leak_error())?;
    }
    Ok(())
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

    let (replacement_body, body_redacted) = {
        let body_text = String::from_utf8_lossy(&body);
        if redaction_values.is_empty() {
            let cleaned = leak_detector
                .scan_and_clean(body_text.as_ref())
                .map_err(|_| RuntimeHttpEgressError::Response {
                    reason: "response_leak_blocked".to_string(),
                    request_bytes: usage.request_bytes,
                    response_bytes: usage.response_bytes,
                })?;
            let leak_detector_redacted = cleaned != body_text.as_ref();
            (
                leak_detector_redacted.then(|| cleaned.into_bytes()),
                leak_detector_redacted,
            )
        } else {
            let exact_redacted = redact_exact_values(body_text.into_owned(), redaction_values);
            let exact_body_redacted = exact_redacted.contains("[REDACTED]");
            let cleaned = leak_detector.scan_and_clean(&exact_redacted).map_err(|_| {
                RuntimeHttpEgressError::Response {
                    reason: "response_leak_blocked".to_string(),
                    request_bytes: usage.request_bytes,
                    response_bytes: usage.response_bytes,
                }
            })?;
            let leak_detector_redacted = cleaned != exact_redacted;
            (
                (exact_body_redacted || leak_detector_redacted).then(|| cleaned.into_bytes()),
                exact_body_redacted || leak_detector_redacted,
            )
        }
    };
    if body_redacted {
        redaction_applied = true;
    }
    let body = replacement_body.unwrap_or(body);

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

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        CapabilityId, InvocationId, NetworkMethod, NetworkPolicy, ResourceScope, RuntimeKind,
        UserId,
    };

    fn request_with_header(runtime: RuntimeKind, header: (&str, &str)) -> RuntimeHttpEgressRequest {
        RuntimeHttpEgressRequest {
            runtime,
            scope: ResourceScope::local_default(UserId::new("user1").unwrap(), InvocationId::new())
                .unwrap(),
            capability_id: CapabilityId::new("builtin.http").unwrap(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![(header.0.to_string(), header.1.to_string())],
            body: Vec::new(),
            network_policy: NetworkPolicy {
                allowed_targets: vec![],
                deny_private_ip_ranges: true,
                max_egress_bytes: Some(4096),
            },
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        }
    }

    #[test]
    fn validate_runtime_request_denies_sensitive_header_for_first_party_runtime() {
        // Regression: FirstParty must NOT be exempt from the sensitive-header
        // guard. `builtin.http` is FirstParty but takes model-supplied headers,
        // so exempting it would let the model smuggle Authorization to an
        // allowlisted host. Host-minted credentials flow through the staged
        // credential-injection path instead, after this guard.
        let detector = LeakDetector::new();
        let request = request_with_header(
            RuntimeKind::FirstParty,
            ("authorization", "Bearer attacker"),
        );

        let error = validate_runtime_request(&request, &detector)
            .expect_err("first-party sensitive header must be denied");

        assert!(matches!(
            error,
            RuntimeHttpEgressError::Request { ref reason, .. }
                if reason == "sensitive_header_denied"
        ));
    }

    #[test]
    fn validate_runtime_request_denies_manual_url_credentials_for_first_party_runtime() {
        let detector = LeakDetector::new();
        let mut request =
            request_with_header(RuntimeKind::FirstParty, ("accept", "application/json"));
        request.url = "https://user:pass@api.example.test/v1/run".to_string();

        let error = validate_runtime_request(&request, &detector)
            .expect_err("first-party manual url credentials must be denied");

        assert!(matches!(
            error,
            RuntimeHttpEgressError::Request { ref reason, .. }
                if reason == "manual_credentials_denied"
        ));
    }

    #[test]
    fn scan_decoded_url_for_leaks_allows_unparseable_encoded_url() {
        let detector = LeakDetector::new();

        scan_decoded_url_for_leaks(
            &detector,
            "://%73%6b%2d%70%72%6f%6a%2dtest1234567890abcdefghij",
        )
        .expect("decoded scan is skipped when URL parsing fails");
    }

    #[test]
    fn scan_runtime_url_for_leaks_blocks_raw_secret_when_url_parse_fails() {
        let detector = LeakDetector::new();

        let error = scan_runtime_url_for_leaks(&detector, "://sk-proj-test1234567890abcdefghij")
            .expect_err("raw scan should run before decoded URL parsing");

        assert!(matches!(
            error,
            RuntimeHttpEgressError::Request { ref reason, .. }
                if reason == "credential_leak_blocked"
        ));
    }

    #[test]
    fn scan_decoded_url_for_leaks_blocks_percent_encoded_fragment() {
        let detector = LeakDetector::new();

        let error = scan_decoded_url_for_leaks(
            &detector,
            "https://api.example.test/v1/run#%73%6b%2d%70%72%6f%6a%2dtest1234567890abcdefghij",
        )
        .expect_err("decoded fragment leak should be blocked");

        assert!(matches!(
            error,
            RuntimeHttpEgressError::Request { ref reason, .. }
                if reason == "credential_leak_blocked"
        ));
    }
}
