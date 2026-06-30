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

    Err(format!("github_api_error_status_{}", response.status))
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
