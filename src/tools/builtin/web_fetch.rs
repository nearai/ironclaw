//! Web fetch tool — GET a URL and return its content as clean Markdown.
//!
//! Distinct from the generic `http` tool (which handles API calls with full
//! method/header/body control). `web_fetch` is purpose-built for reading web
//! pages, articles, and documentation:
//!
//! - GET-only, no custom headers or body
//! - Always attempts HTML → Markdown conversion via Readability
//! - Returns structured output: `{url, title, content, word_count}`
//! - Auto-approved (no confirmation prompt)
//!
//! All the same security infrastructure as `http`:
//! HTTPS-only, SSRF protection, DNS rebinding defence, outbound/inbound leak
//! scanning, 5 MB response cap, no redirect following.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use crate::context::JobContext;
use crate::safety::LeakDetector;
use crate::tools::builtin::http::validate_url;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, ToolRateLimitConfig};

#[cfg(feature = "html-to-markdown")]
use crate::tools::builtin::convert_html_to_markdown;

/// Maximum response body size — matches the `http` tool limit.
const MAX_RESPONSE_SIZE: usize = 5 * 1024 * 1024;

/// Extract the `<title>` text from raw HTML without a full DOM parser.
///
/// Uses `to_ascii_lowercase()` (not `to_lowercase()`) so that byte offsets
/// remain valid across both strings.  HTML tag names are ASCII-only, so
/// ASCII-only case folding is sufficient.  Unicode `to_lowercase()` can
/// change byte lengths (e.g. `İ` → `i\u{307}`), making offsets derived
/// from the lowercased string invalid when used to index into the original.
fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let tag_start = lower.find("<title")?;
    let tag_end = html[tag_start..].find('>')? + tag_start + 1;
    let close = lower[tag_end..].find("</title>")? + tag_end;
    let title = html[tag_end..close].trim().to_string();
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

/// Web fetch tool — retrieve a URL and return clean Markdown content.
pub struct WebFetchTool {
    client: Client,
}

impl WebFetchTool {
    /// Create a new `WebFetchTool` with the same client settings as `HttpTool`.
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("Failed to create HTTP client for web_fetch");

        Self { client }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL and extract its content as clean Markdown. \
         Use for reading articles, documentation, and web pages. \
         For API calls (POST, custom headers, authentication), use the `http` tool instead."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "HTTPS URL to fetch. Must be a public URL (no localhost or private IPs)."
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let url_str = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("'url' is required".to_string()))?;

        // SSRF defence: HTTPS-only, no localhost, no private IPs, DNS rebinding check.
        let parsed_url = validate_url(url_str)?;

        // Outbound leak scan — reject if URL contains secrets.
        let detector = LeakDetector::new();
        detector
            .scan_http_request(parsed_url.as_str(), &[], None)
            .map_err(|e| ToolError::NotAuthorized(e.to_string()))?;

        // Send GET request.
        let response = self
            .client
            .get(parsed_url.clone())
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ToolError::Timeout(Duration::from_secs(30))
                } else {
                    ToolError::ExternalService(e.to_string())
                }
            })?;

        let status = response.status().as_u16();

        // Block redirects — same SSRF defence as `http` tool.
        if (300..400).contains(&status) {
            return Err(ToolError::NotAuthorized(format!(
                "request returned redirect (HTTP {}), which is blocked to prevent SSRF",
                status
            )));
        }

        // Detect content type before consuming the response.
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        // Pre-check Content-Length to reject obviously oversized responses.
        if let Some(content_length) = response.headers().get(reqwest::header::CONTENT_LENGTH)
            && let Ok(s) = content_length.to_str()
            && let Ok(len) = s.parse::<usize>()
            && len > MAX_RESPONSE_SIZE
        {
            return Err(ToolError::ExecutionFailed(format!(
                "Response Content-Length ({} bytes) exceeds maximum allowed size ({} bytes)",
                len, MAX_RESPONSE_SIZE
            )));
        }

        // Stream body with a hard 5 MB cap.
        let mut body: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = StreamExt::next(&mut stream).await {
            let chunk = chunk.map_err(|e| {
                ToolError::ExternalService(format!("failed to read response body: {}", e))
            })?;
            if body.len() + chunk.len() > MAX_RESPONSE_SIZE {
                return Err(ToolError::ExecutionFailed(format!(
                    "Response body exceeds maximum allowed size ({} bytes)",
                    MAX_RESPONSE_SIZE
                )));
            }
            body.extend_from_slice(&chunk);
        }

        let raw_text = String::from_utf8_lossy(&body).into_owned();

        // HTML → Markdown conversion (always attempted for HTML responses).
        let is_html = content_type.contains("text/html");

        #[cfg(feature = "html-to-markdown")]
        let (content, title) = if is_html {
            let title = extract_title(&raw_text);
            match convert_html_to_markdown(&raw_text, parsed_url.as_str()) {
                Ok(md) => (md, title),
                Err(e) => {
                    tracing::warn!(
                        url = %parsed_url,
                        error = %e,
                        "HTML-to-markdown conversion failed, returning raw text"
                    );
                    (raw_text.clone(), title)
                }
            }
        } else {
            (raw_text.clone(), None)
        };

        #[cfg(not(feature = "html-to-markdown"))]
        let (content, title) = if is_html {
            (raw_text.clone(), extract_title(&raw_text))
        } else {
            (raw_text.clone(), None)
        };

        let word_count = content.split_whitespace().count();

        let result = serde_json::json!({
            "url": url_str,
            "title": title,
            "content": content,
            "word_count": word_count,
        });

        Ok(ToolOutput::success(result, start.elapsed()).with_raw(raw_text))
    }

    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        Some(Duration::from_secs(5))
    }

    fn requires_sanitization(&self) -> bool {
        true // External data always needs sanitization
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Web fetch is always auto-approved — the SSRF/leak protections are
        // unconditional, and reading public web pages doesn't require confirmation.
        ApprovalRequirement::Never
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(30, 500)) // same as http tool
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_title_finds_basic_title() {
        let html = "<html><head><title>Hello World</title></head><body></body></html>";
        assert_eq!(extract_title(html), Some("Hello World".to_string()));
    }

    #[test]
    fn extract_title_trims_whitespace() {
        let html = "<html><head><title>  Spaced Title  </title></head></html>";
        assert_eq!(extract_title(html), Some("Spaced Title".to_string()));
    }

    #[test]
    fn extract_title_returns_none_when_absent() {
        let html = "<html><head></head><body>No title</body></html>";
        assert_eq!(extract_title(html), None);
    }

    #[test]
    fn extract_title_handles_case_insensitive_tag() {
        let html = "<html><head><TITLE>Case Test</TITLE></head></html>";
        assert_eq!(extract_title(html), Some("Case Test".to_string()));
    }

    #[test]
    fn extract_title_with_non_ascii_before_tag() {
        // Turkish dotless-ı (U+0131) is 2 bytes in UTF-8 and lowercases to
        // ASCII 'i' (1 byte).  Using to_lowercase() would shift the byte offset
        // of '<title>' so that html[tag_start..] panics at a non-char boundary.
        // to_ascii_lowercase() preserves byte lengths and must not panic.
        let html = "<html><head><meta charset=\"utf-8\"/><title>ıTitle</title></head></html>";
        let result = extract_title(html);
        assert!(result.is_some(), "should extract title with non-ASCII content");
        assert!(result.unwrap().contains("Title"));
    }

    #[test]
    fn extract_title_with_tag_attributes() {
        // <title lang="en"> has attributes — ensure the '>' scan still lands correctly.
        let html = "<html><head><title lang=\"en\">Attributed</title></head></html>";
        assert_eq!(extract_title(html), Some("Attributed".to_string()));
    }

    #[test]
    fn web_fetch_tool_name_and_schema() {
        let tool = WebFetchTool::new();
        assert_eq!(tool.name(), "web_fetch");
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "url");
        assert_eq!(schema["properties"]["url"]["type"], "string");
    }

    #[test]
    fn web_fetch_never_requires_approval() {
        let tool = WebFetchTool::new();
        let params = serde_json::json!({"url": "https://example.com"});
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::Never
        );
    }
}
