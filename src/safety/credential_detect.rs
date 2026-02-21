//! Broad detection of manually-provided credentials in HTTP request parameters.
//!
//! Used by the built-in HTTP tool to decide whether approval is needed when
//! the LLM provides auth data directly in headers or URL query parameters.

/// Check whether HTTP request parameters contain manually-provided credentials.
///
/// Inspects both headers (name/value) and URL query parameters for patterns
/// that indicate authentication data.
pub fn params_contain_manual_credentials(params: &serde_json::Value) -> bool {
    headers_contain_credentials(params) || url_contains_credential_params(params)
}

/// Header names that are exact matches for credential-carrying headers (case-insensitive).
const AUTH_HEADER_EXACT: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "x-api-key",
    "api-key",
    "x-auth-token",
    "x-token",
    "x-access-token",
    "x-session-token",
    "x-csrf-token",
    "x-secret",
    "x-api-secret",
];

/// Substrings in header names that suggest credentials (case-insensitive).
/// Note: "key" is excluded to avoid false positives like "X-Idempotency-Key".
const AUTH_HEADER_SUBSTRINGS: &[&str] = &["auth", "token", "secret", "credential", "password"];

/// Value prefixes that indicate auth schemes (case-insensitive).
const AUTH_VALUE_PREFIXES: &[&str] = &[
    "bearer ",
    "basic ",
    "token ",
    "digest ",
    "hoba ",
    "mutual ",
    "aws4-hmac-sha256 ",
];

/// URL query parameter names that are exact matches for credentials (case-insensitive).
const AUTH_QUERY_EXACT: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "access_token",
    "token",
    "key",
    "secret",
    "password",
    "auth",
    "auth_token",
    "session_token",
    "client_secret",
    "client_id",
    "app_key",
    "app_secret",
    "sig",
    "signature",
];

/// Substrings in query parameter names that suggest credentials (case-insensitive).
const AUTH_QUERY_SUBSTRINGS: &[&str] = &["token", "secret", "auth", "password", "credential"];

fn header_name_is_credential(name: &str) -> bool {
    let lower = name.to_lowercase();

    if AUTH_HEADER_EXACT.contains(&lower.as_str()) {
        return true;
    }

    AUTH_HEADER_SUBSTRINGS.iter().any(|sub| lower.contains(sub))
}

fn header_value_is_credential(value: &str) -> bool {
    let lower = value.to_lowercase();
    AUTH_VALUE_PREFIXES.iter().any(|pfx| lower.starts_with(pfx))
}

fn headers_contain_credentials(params: &serde_json::Value) -> bool {
    match params.get("headers") {
        Some(serde_json::Value::Object(map)) => map.iter().any(|(k, v)| {
            header_name_is_credential(k)
                || v.as_str().is_some_and(header_value_is_credential)
        }),
        Some(serde_json::Value::Array(items)) => items.iter().any(|item| {
            let name_match = item
                .get("name")
                .and_then(|n| n.as_str())
                .is_some_and(header_name_is_credential);
            let value_match = item
                .get("value")
                .and_then(|v| v.as_str())
                .is_some_and(header_value_is_credential);
            name_match || value_match
        }),
        _ => false,
    }
}

fn query_param_is_credential(name: &str) -> bool {
    let lower = name.to_lowercase();

    if AUTH_QUERY_EXACT.contains(&lower.as_str()) {
        return true;
    }

    AUTH_QUERY_SUBSTRINGS.iter().any(|sub| lower.contains(sub))
}

fn url_contains_credential_params(params: &serde_json::Value) -> bool {
    let url_str = match params.get("url").and_then(|u| u.as_str()) {
        Some(u) => u,
        None => return false,
    };

    let parsed = match url::Url::parse(url_str) {
        Ok(u) => u,
        Err(_) => return false,
    };

    parsed
        .query_pairs()
        .any(|(name, _)| query_param_is_credential(&name))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Header name exact match ────────────────────────────────────────

    #[test]
    fn test_authorization_header_detected() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com",
            "headers": {"Authorization": "Bearer token123"}
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_all_exact_header_names() {
        for name in AUTH_HEADER_EXACT {
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://example.com",
                "headers": {name.to_string(): "some_value"}
            });
            assert!(
                params_contain_manual_credentials(&params),
                "Header '{}' should be detected",
                name
            );
        }
    }

    #[test]
    fn test_header_name_case_insensitive() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"AUTHORIZATION": "value"}
        });
        assert!(params_contain_manual_credentials(&params));
    }

    // ── Header name substring match ────────────────────────────────────

    #[test]
    fn test_header_substring_auth() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"X-Custom-Auth-Header": "value"}
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_header_substring_token() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"X-My-Token": "value"}
        });
        assert!(params_contain_manual_credentials(&params));
    }

    // ── Header value prefix match ──────────────────────────────────────

    #[test]
    fn test_bearer_value_detected() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"X-Custom": "Bearer sk-abc123"}
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_basic_value_detected() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"X-Custom": "Basic dXNlcjpwYXNz"}
        });
        assert!(params_contain_manual_credentials(&params));
    }

    // ── Array-format headers ───────────────────────────────────────────

    #[test]
    fn test_array_format_header_name() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": [{"name": "Authorization", "value": "Bearer token"}]
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_array_format_header_value_prefix() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": [{"name": "X-Custom", "value": "Token abc123"}]
        });
        assert!(params_contain_manual_credentials(&params));
    }

    // ── URL query parameter detection ──────────────────────────────────

    #[test]
    fn test_url_api_key_param() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data?api_key=abc123"
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_url_access_token_param() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data?access_token=xyz"
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_url_query_substring_match() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data?my_auth_code=xyz"
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_url_query_case_insensitive() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data?API_KEY=abc"
        });
        assert!(params_contain_manual_credentials(&params));
    }

    // ── False positive checks ──────────────────────────────────────────

    #[test]
    fn test_idempotency_key_not_detected() {
        let params = serde_json::json!({
            "method": "POST",
            "url": "https://api.example.com",
            "headers": {"X-Idempotency-Key": "uuid-1234"}
        });
        assert!(!params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_content_type_not_detected() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"Content-Type": "application/json", "Accept": "text/html"}
        });
        assert!(!params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_no_headers_no_query() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com/path"
        });
        assert!(!params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_safe_query_params() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/search?q=hello&page=1&limit=10"
        });
        assert!(!params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_empty_headers() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {}
        });
        assert!(!params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_invalid_url_returns_false() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "not a url"
        });
        assert!(!params_contain_manual_credentials(&params));
    }
}
