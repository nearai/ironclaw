//! Request tracing middleware for the Axum gateway.
//!
//! Wraps each incoming HTTP request in a tracing span containing a
//! `request_id` (from `X-Request-Id` header or auto-generated UUID),
//! `channel`, HTTP method, and matched route path. Logs request
//! completion with status code and latency.

use axum::{extract::MatchedPath, http::Request, middleware::Next, response::Response};
use std::time::Instant;
use tracing::Instrument;

use crate::observability::runtime_log::{components, fields, phases};

/// Headers that must never appear in log output.
const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
    "proxy-authorization",
];

/// Axum `from_fn` middleware that creates a per-request tracing span and
/// logs request lifecycle events using the unified runtime log field names.
pub async fn request_tracing_middleware(
    matched_path: Option<MatchedPath>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = matched_path
        .as_ref()
        .map(|mp| mp.as_str().to_owned())
        .unwrap_or_else(|| request.uri().path().to_owned());

    // Use the caller-supplied request ID or generate one.
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let span = tracing::info_span!(
        "http_request",
        { fields::REQUEST_ID } = %request_id,
        { fields::CHANNEL } = components::GATEWAY,
        { fields::COMPONENT } = components::GATEWAY,
        http.method = %method,
        http.route = %path,
    );

    let start = Instant::now();

    let response = next.run(request).instrument(span.clone()).await;

    let latency_ms = start.elapsed().as_millis();
    let status = response.status().as_u16();

    let _guard = span.enter();
    if status >= 400 && status < 500 {
        tracing::warn!(
            { fields::PHASE } = phases::REJECT,
            http.status = status,
            latency_ms = latency_ms,
            "Request rejected: {} {} -> {} ({}ms)",
            method,
            path,
            status,
            latency_ms,
        );
    } else if status >= 500 {
        tracing::warn!(
            { fields::PHASE } = phases::FAIL,
            http.status = status,
            latency_ms = latency_ms,
            "Request failed: {} {} -> {} ({}ms)",
            method,
            path,
            status,
            latency_ms,
        );
    } else {
        tracing::info!(
            { fields::PHASE } = phases::COMPLETE,
            http.status = status,
            latency_ms = latency_ms,
            "Request complete: {} {} -> {} ({}ms)",
            method,
            path,
            status,
            latency_ms,
        );
    }

    response
}

/// Returns `true` if the given header name is considered sensitive
/// and must not be logged.
#[allow(dead_code)]
pub fn is_sensitive_header(name: &str) -> bool {
    SENSITIVE_HEADERS.contains(&name.to_ascii_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_headers_detected() {
        assert!(is_sensitive_header("Authorization"));
        assert!(is_sensitive_header("COOKIE"));
        assert!(is_sensitive_header("x-api-key"));
        assert!(is_sensitive_header("X-Auth-Token"));
        assert!(!is_sensitive_header("Content-Type"));
        assert!(!is_sensitive_header("X-Request-Id"));
    }

    #[test]
    fn sensitive_header_list_not_empty() {
        assert!(!SENSITIVE_HEADERS.is_empty());
    }
}
