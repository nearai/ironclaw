const GITHUB_API_ROOT: &str = "https://api.github.com";
const GITHUB_API_VERSION: &str = "2026-03-10";
const HTTP_TIMEOUT_MS: u32 = 10_000;

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
