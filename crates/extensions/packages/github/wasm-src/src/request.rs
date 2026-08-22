#[cfg(not(test))]
const GITHUB_API_ROOT: &str = "https://api.github.com";
#[cfg(not(test))]
const GITHUB_API_VERSION: &str = "2026-03-10";
#[cfg(not(test))]
const HTTP_TIMEOUT_MS: u32 = 10_000;

thread_local! {
    /// The provider's bounded `message` field from the most recent `401`
    /// GitHub API response body, if any. `github_request` stashes it here
    /// immediately before returning its `Err(code)` so `lib.rs::execute`
    /// can attach it to the typed `guest-failure.message` alongside the
    /// stable `code` for the auth-required diagnostic the host carries onto
    /// the auth gate — the host scrubs and bounds it before it becomes
    /// guest-visible, so this only needs to carry the raw text out. Scoped
    /// to `401` only: other provider error bodies are not validated end to
    /// end onto a model-visible surface and echoing them verbatim would
    /// widen what a guest-authored response body can put in front of the
    /// model without the same scrutiny.
    static LAST_ERROR_MESSAGE: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Take (and clear) the provider message captured alongside the most recent
/// `github_request` error, if any.
pub(crate) fn take_last_error_message() -> Option<String> {
    LAST_ERROR_MESSAGE.with(|cell| cell.borrow_mut().take())
}

fn set_last_error_message(body: &[u8]) {
    let message = provider_error_message(body);
    LAST_ERROR_MESSAGE.with(|cell| *cell.borrow_mut() = message);
}

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
    .map_err(|failure| host_failure_code(&failure))?;

    if (200..300).contains(&response.status) {
        if response.body.is_empty() {
            return Ok(serde_json::json!({ "status": response.status }).to_string());
        }
        let body =
            String::from_utf8(response.body).map_err(|_| "github_api_invalid_utf8".to_string())?;
        return Ok(body);
    }

    if response.status == 422 && is_github_validation_error_body(&response.body) {
        return Err("github_api_error_status_422_validation".to_string());
    }

    if response.status == 401 {
        set_last_error_message(&response.body);
    }

    Err(format!("github_api_error_status_{}", response.status))
}

/// Bound applied to a captured provider message before it is embedded in the
/// guest error envelope -- keeps the payload small independent of the
/// host's own `MAX_WASM_GUEST_MESSAGE_BYTES` backstop.
const MAX_PROVIDER_MESSAGE_CHARS: usize = 512;

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

fn host_failure_code(failure: &crate::near::agent::host::HttpFailure) -> String {
    use crate::near::agent::host::HttpErrorKind;

    match failure.kind {
        HttpErrorKind::AuthRequired => "AuthRequired",
        HttpErrorKind::Input => "invalid_parameters",
        HttpErrorKind::OutputTooLarge => "github_api_body_limit",
        HttpErrorKind::Executor => "github_api_executor_failed",
        HttpErrorKind::NetworkDenied => "github_api_egress_denied",
        HttpErrorKind::Client => "github_api_request_failed",
        HttpErrorKind::OperationFailed => failure
            .code
            .as_deref()
            .unwrap_or("github_api_request_failed"),
    }
    .to_string()
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
    use super::{
        is_github_validation_error_body, provider_error_message, set_last_error_message,
        take_last_error_message,
    };

    #[test]
    fn provider_error_message_carries_the_rejection_text() {
        let message = provider_error_message(br#"{"message":"Bad credentials"}"#);

        assert_eq!(message.as_deref(), Some("Bad credentials"));
    }

    #[test]
    fn provider_error_message_degrades_gracefully_without_usable_text() {
        for body in [
            &b"{}"[..],
            b"not json",
            br#"{"message":""}"#,
            br#"{"message":"   "}"#,
            br#"{"message":123}"#,
        ] {
            assert!(
                provider_error_message(body).is_none(),
                "no message expected for body {body:?}"
            );
        }
    }

    #[test]
    fn provider_error_message_bounds_the_message_to_512_chars() {
        let body = serde_json::json!({ "message": "a".repeat(1000) }).to_string();

        let message = provider_error_message(body.as_bytes()).expect("message present");

        assert_eq!(message.chars().count(), 512);
    }

    #[test]
    fn production_capture_uses_the_validated_provider_message_parser() {
        set_last_error_message(br#"{"message":"   "}"#);

        assert!(take_last_error_message().is_none());
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
