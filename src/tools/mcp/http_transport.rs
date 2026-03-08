//! HTTP transport for MCP servers.
//!
//! Implements the Streamable HTTP transport, communicating with MCP servers
//! over HTTP POST with JSON and SSE response support.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::tools::mcp::protocol::{McpRequest, McpResponse};
use crate::tools::mcp::session::McpSessionManager;
use crate::tools::mcp::transport::McpTransport;
use crate::tools::tool::ToolError;

/// MCP transport that communicates with a server over HTTP.
///
/// Sends JSON-RPC requests as HTTP POST with `Content-Type: application/json`
/// and accepts either JSON or SSE (`text/event-stream`) responses. Optionally
/// manages session IDs via [`McpSessionManager`] and supports custom headers.
pub struct HttpMcpTransport {
    server_url: String,
    server_name: String,
    http_client: reqwest::Client,
    session_manager: Option<Arc<McpSessionManager>>,
    custom_headers: HashMap<String, String>,
}

impl HttpMcpTransport {
    /// Create a new HTTP transport for the given server URL.
    pub fn new(server_url: impl Into<String>, server_name: impl Into<String>) -> Self {
        Self {
            server_url: server_url.into(),
            server_name: server_name.into(),
            // reqwest::Client::builder().build() only fails if the TLS backend
            // cannot initialize, which does not happen with the default rustls
            // feature set. Panic is acceptable here (same as reqwest's own
            // `Client::new()`).
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
            session_manager: None,
            custom_headers: HashMap::new(),
        }
    }

    /// Attach a session manager for Mcp-Session-Id tracking.
    pub fn with_session_manager(mut self, session_manager: Arc<McpSessionManager>) -> Self {
        self.session_manager = Some(session_manager);
        self
    }

    /// Set custom headers that will be sent with every request.
    #[cfg(test)]
    pub fn with_custom_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.custom_headers = headers;
        self
    }

    /// Get the server URL.
    #[cfg(test)]
    pub(crate) fn server_url(&self) -> &str {
        &self.server_url
    }

    /// Get the session manager, if one is configured.
    #[cfg(test)]
    pub(crate) fn session_manager(&self) -> Option<&Arc<McpSessionManager>> {
        self.session_manager.as_ref()
    }
}

#[async_trait]
impl McpTransport for HttpMcpTransport {
    async fn send(
        &self,
        request: &McpRequest,
        headers: &HashMap<String, String>,
    ) -> Result<McpResponse, ToolError> {
        // Build the HTTP request.
        let mut req_builder = self
            .http_client
            .post(&self.server_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(request);

        // Apply custom headers configured on the transport.
        for (key, value) in &self.custom_headers {
            req_builder = req_builder.header(key.as_str(), value.as_str());
        }

        // Apply per-request headers (e.g. Authorization, Mcp-Session-Id).
        for (key, value) in headers {
            req_builder = req_builder.header(key.as_str(), value.as_str());
        }

        // Send the request.
        let response = req_builder.send().await.map_err(|e| {
            let mut chain = format!("[{}] MCP HTTP request failed: {}", self.server_name, e);
            let mut source = std::error::Error::source(&e);
            while let Some(cause) = source {
                chain.push_str(&format!(" -> {}", cause));
                source = cause.source();
            }
            ToolError::ExternalService(chain)
        })?;

        // Extract session ID from response headers before consuming the body.
        if let Some(ref session_manager) = self.session_manager
            && let Some(session_id) = response
                .headers()
                .get("Mcp-Session-Id")
                .and_then(|v| v.to_str().ok())
        {
            session_manager
                .update_session_id(&self.server_name, Some(session_id.to_string()))
                .await;
        }

        // Handle error status codes.
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let sanitized = sanitize_error_body(&body);
            return Err(ToolError::ExternalService(format!(
                "[{}] MCP server returned status: {} - {}",
                self.server_name, status, sanitized
            )));
        }

        // Determine response format from Content-Type.
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("text/event-stream") {
            self.parse_sse_response(response).await
        } else {
            response.json().await.map_err(|e| {
                ToolError::ExternalService(format!(
                    "[{}] Failed to parse MCP response: {}",
                    self.server_name, e
                ))
            })
        }
    }

    async fn shutdown(&self) -> Result<(), ToolError> {
        // HTTP transport is stateless; nothing to shut down.
        Ok(())
    }

    fn supports_http_features(&self) -> bool {
        true
    }
}

impl HttpMcpTransport {
    /// Parse a Server-Sent Events response, returning the first valid JSON-RPC
    /// `data:` line as an [`McpResponse`].
    async fn parse_sse_response(
        &self,
        response: reqwest::Response,
    ) -> Result<McpResponse, ToolError> {
        use futures::StreamExt;

        const MAX_SSE_BUFFER: usize = 10 * 1024 * 1024; // 10 MB

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                ToolError::ExternalService(format!(
                    "[{}] Failed to read SSE chunk: {}",
                    self.server_name, e
                ))
            })?;

            buffer.push_str(&String::from_utf8_lossy(&chunk));

            if buffer.len() > MAX_SSE_BUFFER {
                return Err(ToolError::ExternalService(format!(
                    "[{}] SSE response exceeded {} byte limit",
                    self.server_name, MAX_SSE_BUFFER
                )));
            }

            // Process only complete lines (terminated by \n). The last
            // element of split('\n') may be an incomplete line; keep it
            // in the buffer for the next chunk.
            let mut remaining_start = 0;
            let bytes = buffer.as_bytes();
            for (i, &b) in bytes.iter().enumerate() {
                if b == b'\n' {
                    let line = &buffer[remaining_start..i];
                    remaining_start = i + 1;

                    if let Some(json_str) = line.strip_prefix("data: ")
                        && let Ok(response) = serde_json::from_str::<McpResponse>(json_str)
                    {
                        return Ok(response);
                    }
                }
            }
            // Keep only the unprocessed trailing fragment.
            if remaining_start > 0 {
                buffer = buffer[remaining_start..].to_string();
            }
        }

        // Process any remaining data without a trailing newline.
        if let Some(json_str) = buffer.strip_prefix("data: ")
            && let Ok(response) = serde_json::from_str::<McpResponse>(json_str.trim())
        {
            return Ok(response);
        }

        Err(ToolError::ExternalService(format!(
            "[{}] No valid data in SSE response: {}",
            self.server_name, buffer
        )))
    }
}

/// Sanitize an HTTP error body for safe inclusion in error messages.
///
/// - Replaces HTML bodies with a placeholder
/// - Strips control characters (except `\n` and `\t`)
/// - Truncates to 500 characters
pub(crate) fn sanitize_error_body(body: &str) -> String {
    let trimmed = body.trim();

    // Detect HTML: starts with `<` or contains `<html` (case-insensitive).
    if trimmed.starts_with('<') || trimmed.to_ascii_lowercase().contains("<html") {
        return "(HTML error page)".to_string();
    }

    // Strip control characters (anything < 0x20 except \n and \t).
    let cleaned: String = body
        .chars()
        .filter(|&c| c >= '\x20' || c == '\n' || c == '\t')
        .collect();

    // Truncate to 500 characters (char-based to avoid splitting multi-byte).
    if cleaned.chars().count() > 500 {
        cleaned.chars().take(500).collect()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_error_body_html_body() {
        let body = "<html><body>500 Internal Error</body></html>";
        assert_eq!(sanitize_error_body(body), "(HTML error page)");
    }

    #[test]
    fn test_sanitize_error_body_doctype_prefix() {
        let body = "<!DOCTYPE html>\n<html><body>Bad Gateway</body></html>";
        assert_eq!(sanitize_error_body(body), "(HTML error page)");
    }

    #[test]
    fn test_sanitize_error_body_html_case_insensitive() {
        let body = "Error: <HTML><BODY>Server Error</BODY></HTML>";
        assert_eq!(sanitize_error_body(body), "(HTML error page)");
    }

    #[test]
    fn test_sanitize_error_body_plain_text_kept() {
        let body = "Something went wrong";
        assert_eq!(sanitize_error_body(body), "Something went wrong");
    }

    #[test]
    fn test_sanitize_error_body_truncates_long_text() {
        let body = "a".repeat(600);
        let result = sanitize_error_body(&body);
        assert_eq!(result.len(), 500);
    }

    #[test]
    fn test_sanitize_error_body_truncates_multibyte_safely() {
        // Each emoji is 4 bytes. 200 emojis = 800 bytes but 200 chars.
        // Pad to >500 chars with multi-byte content.
        let body = "é".repeat(600); // 'é' is 2 bytes
        let result = sanitize_error_body(&body);
        assert_eq!(result.chars().count(), 500);
        // Ensure the result is valid UTF-8 (String guarantees this, but verify
        // we didn't accidentally slice mid-character).
        assert!(result.is_char_boundary(result.len()));
    }

    #[test]
    fn test_sanitize_error_body_strips_control_chars() {
        let body = "error\x00message\x01with\x02controls\nbut\tkeep these";
        let result = sanitize_error_body(body);
        assert_eq!(result, "errormessagewithcontrols\nbut\tkeep these");
    }

    #[test]
    fn test_sanitize_error_body_empty_string() {
        assert_eq!(sanitize_error_body(""), "");
    }

    #[test]
    fn test_sanitize_error_body_whitespace_only() {
        assert_eq!(sanitize_error_body("   "), "   ");
    }

    #[test]
    fn test_new_creates_transport() {
        let transport = HttpMcpTransport::new("http://localhost:8080", "test");
        assert_eq!(transport.server_url(), "http://localhost:8080");
        assert!(transport.session_manager().is_none());
        assert!(transport.custom_headers.is_empty());
    }

    #[test]
    fn test_supports_http_features() {
        let http_transport = HttpMcpTransport::new("http://localhost:8080", "test");
        assert!(http_transport.supports_http_features());
    }

    #[test]
    fn test_with_session_manager() {
        let session_manager = Arc::new(McpSessionManager::new());
        let transport = HttpMcpTransport::new("http://localhost:8080", "test")
            .with_session_manager(session_manager.clone());
        assert!(transport.session_manager().is_some());
    }

    #[test]
    fn test_with_custom_headers() {
        let mut headers = HashMap::new();
        headers.insert("X-Custom".to_string(), "value".to_string());
        let transport =
            HttpMcpTransport::new("http://localhost:8080", "test").with_custom_headers(headers);
        assert_eq!(transport.custom_headers.get("X-Custom").unwrap(), "value");
    }
}
