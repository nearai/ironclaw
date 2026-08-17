#[cfg(not(test))]
const GITHUB_API_ROOT: &str = "https://api.github.com";
#[cfg(not(test))]
const GITHUB_API_VERSION: &str = "2026-03-10";
#[cfg(not(test))]
const HTTP_TIMEOUT_MS: u32 = 10_000;

#[cfg(not(test))]
pub(crate) fn github_request(
    method: &str,
    path: &str,
    body: Option<String>,
) -> Result<String, String> {
    let url = format!("{GITHUB_API_ROOT}{path}");
    let headers = serde_json::json!({
        "Accept": "application/vnd.github+json",
        "Content-Type": "application/json",
        "X-GitHub-Api-Version": GITHUB_API_VERSION,
        "User-Agent": "IronClaw-GitHub-Reborn-WASM"
    });

    let body_bytes = body.map(String::into_bytes);
    let response = crate::near::agent::host::http_request(
        method,
        &url,
        &headers.to_string(),
        body_bytes.as_deref(),
        Some(HTTP_TIMEOUT_MS),
    )
    .map_err(|error| sanitize_host_error(&error))?;

    if (200..300).contains(&response.status) {
        if response.body.is_empty() {
            return Ok(serde_json::json!({ "status": response.status }).to_string());
        }
        let body =
            String::from_utf8(response.body).map_err(|_| "github_api_invalid_utf8".to_string())?;
        return Ok(body);
    }

    if response.status == 401 {
        return Err(unauthorized_error_payload(&response.body));
    }

    if response.status == 422 && is_github_validation_error_body(&response.body) {
        return Err("github_api_error_status_422_validation".to_string());
    }

    Err(format!("github_api_error_status_{}", response.status))
}

/// Bound applied to a captured provider message before it is embedded in the
/// guest error envelope -- keeps the payload small independent of the
/// host's own `MAX_WASM_GUEST_MESSAGE_BYTES` backstop.
const MAX_PROVIDER_MESSAGE_CHARS: usize = 512;

/// Build the structured guest error envelope for a 401 response, carrying the
/// provider's rejection `message` (when the body has one) alongside the
/// stable `code`/`kind` pair. This is a pre-built envelope -- unlike other
/// error codes in this module, which are plain stable strings wrapped later
/// by `guest_error_payload` -- so `guest_error_payload` recognizes and passes
/// it through unchanged instead of re-wrapping it.
pub(crate) fn unauthorized_error_payload(body: &[u8]) -> String {
    let mut payload = serde_json::json!({
        "code": "github_api_error_status_401",
        "kind": "auth_required",
    });
    if let Some(message) = provider_error_message(body) {
        payload["message"] = serde_json::Value::String(message);
    }
    payload.to_string()
}

fn provider_error_message(body: &[u8]) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_slice(body).ok()?;
    let message = parsed.get("message")?.as_str()?.trim();
    if message.is_empty() {
        return None;
    }
    Some(bounded_message(message))
}

fn bounded_message(message: &str) -> String {
    message.chars().take(MAX_PROVIDER_MESSAGE_CHARS).collect()
}

#[cfg(test)]
pub(crate) fn github_request(
    method: &str,
    path: &str,
    body: Option<String>,
) -> Result<String, String> {
    test_support::record_request(method, path, body);
    test_support::take_response()
        .unwrap_or_else(|| Err("github_test_missing_mock_response".to_string()))
}

pub(crate) fn sanitize_host_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("auth")
        || lower.contains("credential")
        || lower.contains("secret")
        || lower.contains("token")
    {
        return "AuthRequired".to_string();
    }
    if lower.contains("timeout") || lower.contains("deadline") {
        return "github_api_timeout".to_string();
    }
    if lower.contains("redirect") {
        return "github_api_redirect_denied".to_string();
    }
    if lower.contains("body") || lower.contains("size") || lower.contains("large") {
        return "github_api_body_limit".to_string();
    }
    if lower.contains("deny") || lower.contains("allow") || lower.contains("host") {
        return "github_api_egress_denied".to_string();
    }
    "github_api_request_failed".to_string()
}

fn is_github_validation_error_body(body: &[u8]) -> bool {
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    let message_is_validation = parsed
        .get("message")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|message| message.eq_ignore_ascii_case("Validation Failed"));
    let has_validation_errors = parsed
        .get("errors")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|errors| !errors.is_empty());

    message_is_validation && has_validation_errors
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct CapturedRequest {
        pub(crate) method: String,
        pub(crate) path: String,
        pub(crate) body: Option<String>,
    }

    thread_local! {
        static REQUESTS: RefCell<Vec<CapturedRequest>> = const { RefCell::new(Vec::new()) };
        static RESPONSES: RefCell<VecDeque<Result<String, String>>> = const { RefCell::new(VecDeque::new()) };
    }

    pub(crate) fn set_response(response: Result<String, String>) {
        set_responses([response]);
    }

    pub(crate) fn set_responses<const N: usize>(responses: [Result<String, String>; N]) {
        REQUESTS.with(|requests| requests.borrow_mut().clear());
        RESPONSES.with(|next_responses| {
            *next_responses.borrow_mut() = responses.into();
        });
    }

    pub(crate) fn requests() -> Vec<CapturedRequest> {
        REQUESTS.with(|requests| requests.borrow().clone())
    }

    pub(super) fn record_request(method: &str, path: &str, body: Option<String>) {
        REQUESTS.with(|requests| {
            requests.borrow_mut().push(CapturedRequest {
                method: method.to_string(),
                path: path.to_string(),
                body,
            });
        });
    }

    pub(super) fn take_response() -> Option<Result<String, String>> {
        RESPONSES.with(|responses| responses.borrow_mut().pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::{is_github_validation_error_body, unauthorized_error_payload};

    #[test]
    fn unauthorized_error_payload_carries_the_provider_message() {
        let payload = unauthorized_error_payload(br#"{"message":"Bad credentials"}"#);
        let value: serde_json::Value = serde_json::from_str(&payload).expect("valid json");

        assert_eq!(value["code"], "github_api_error_status_401");
        assert_eq!(value["kind"], "auth_required");
        assert_eq!(value["message"], "Bad credentials");
    }

    #[test]
    fn unauthorized_error_payload_degrades_gracefully_without_a_message() {
        for body in [
            &b"{}"[..],
            b"not json",
            br#"{"message":""}"#,
            br#"{"message":123}"#,
        ] {
            let payload = unauthorized_error_payload(body);
            let value: serde_json::Value = serde_json::from_str(&payload).expect("valid json");

            assert_eq!(value["code"], "github_api_error_status_401");
            assert_eq!(value["kind"], "auth_required");
            assert!(
                value.get("message").is_none(),
                "no message field expected for body {body:?}, got {value:?}"
            );
        }
    }

    #[test]
    fn unauthorized_error_payload_bounds_the_message_to_512_chars() {
        let long_message = "a".repeat(1000);
        let body = serde_json::json!({ "message": long_message }).to_string();

        let payload = unauthorized_error_payload(body.as_bytes());
        let value: serde_json::Value = serde_json::from_str(&payload).expect("valid json");

        assert_eq!(
            value["message"].as_str().expect("message string").len(),
            512
        );
    }

    #[test]
    fn github_validation_422_body_requires_validation_error_details() {
        assert!(is_github_validation_error_body(
            br#"{"message":"Validation Failed","errors":[{"resource":"Search","field":"q","code":"invalid"}],"status":"422"}"#
        ));

        assert!(!is_github_validation_error_body(
            br#"{"message":"Validation failed, or the endpoint has been spammed.","status":"422"}"#
        ));

        assert!(!is_github_validation_error_body(
            br#"{"message":"You have triggered an abuse detection mechanism.","status":"422"}"#
        ));
    }
}
