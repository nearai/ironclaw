//! HTTP request tool.

use std::collections::HashMap;
use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use crate::context::JobContext;
use crate::safety::LeakDetector;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

/// Maximum response body size (5 MB).
///
/// 5 MB is large enough for typical JSON API responses and moderate HTML pages,
/// but small enough to prevent OOM from malicious or runaway servers.  The WASM
/// HTTP wrapper uses the same limit for consistency.
const MAX_RESPONSE_SIZE: usize = 5 * 1024 * 1024;

/// Tool for making HTTP requests.
pub struct HttpTool {
    client: Client,
}

impl HttpTool {
    /// Create a new HTTP tool.
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("Failed to create HTTP client");

        Self { client }
    }
}

fn validate_url(url: &str) -> Result<reqwest::Url, ToolError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| ToolError::InvalidParameters(format!("invalid URL: {}", e)))?;

    if parsed.scheme() != "https" {
        return Err(ToolError::NotAuthorized(
            "only https URLs are allowed".to_string(),
        ));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| ToolError::InvalidParameters("URL missing host".to_string()))?;

    let host_lower = host.to_lowercase();
    if host_lower == "localhost" || host_lower.ends_with(".localhost") {
        return Err(ToolError::NotAuthorized(
            "localhost is not allowed".to_string(),
        ));
    }

    // Check literal IP addresses
    if let Ok(ip) = host.parse::<IpAddr>()
        && is_disallowed_ip(&ip)
    {
        return Err(ToolError::NotAuthorized(
            "private or local IPs are not allowed".to_string(),
        ));
    }

    // Resolve hostname and check all resolved IPs against the blocklist.
    // This prevents DNS rebinding where a hostname resolves to a private IP.
    let port = parsed.port_or_known_default().unwrap_or(443);
    let socket_addr = format!("{}:{}", host, port);
    if let Ok(addrs) = socket_addr.to_socket_addrs() {
        for addr in addrs {
            if is_disallowed_ip(&addr.ip()) {
                return Err(ToolError::NotAuthorized(format!(
                    "hostname '{}' resolves to disallowed IP {}",
                    host,
                    addr.ip()
                )));
            }
        }
    }

    Ok(parsed)
}

fn is_disallowed_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified()
                || *v4 == std::net::Ipv4Addr::new(169, 254, 169, 254)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_multicast()
                || v6.is_unspecified()
        }
    }
}

fn parse_headers_param(
    headers: Option<&serde_json::Value>,
) -> Result<Vec<(String, String)>, ToolError> {
    match headers {
        None => Ok(Vec::new()),
        Some(serde_json::Value::Object(map)) => {
            let mut out = Vec::with_capacity(map.len());
            for (k, v) in map {
                let value = v.as_str().ok_or_else(|| {
                    ToolError::InvalidParameters(format!("header '{}' must have a string value", k))
                })?;
                out.push((k.clone(), value.to_string()));
            }
            Ok(out)
        }
        Some(serde_json::Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                let obj = item.as_object().ok_or_else(|| {
                    ToolError::InvalidParameters(format!(
                        "headers[{}] must be an object with 'name' and 'value'",
                        idx
                    ))
                })?;
                let name = obj.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters(format!("headers[{}].name must be a string", idx))
                })?;
                let value = obj.get("value").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters(format!("headers[{}].value must be a string", idx))
                })?;
                out.push((name.to_string(), value.to_string()));
            }
            Ok(out)
        }
        Some(_) => Err(ToolError::InvalidParameters(
            "'headers' must be an object or an array of {name, value}".to_string(),
        )),
    }
}

impl Default for HttpTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for HttpTool {
    fn name(&self) -> &str {
        "http"
    }

    fn description(&self) -> &str {
        "Make HTTP requests to external APIs. Supports GET, POST, PUT, DELETE methods."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"],
                    "description": "HTTP method"
                },
                "url": {
                    "type": "string",
                    "description": "The URL to request"
                },
                "headers": {
                    "type": "array",
                    "description": "Optional headers as a list of {name, value} objects",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "value": { "type": "string" }
                        },
                        "required": ["name", "value"],
                        "additionalProperties": false
                    }
                },
                "body": {
                    "type": ["object", "array", "string", "number", "boolean", "null"],
                    "description": "Request body (for POST/PUT/PATCH)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Request timeout in seconds (default: 30)"
                }
            },
            "required": ["method", "url"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let method = require_str(&params, "method")?;

        let url = require_str(&params, "url")?;
        let parsed_url = validate_url(url)?;

        // Parse headers
        let headers_vec = parse_headers_param(params.get("headers"))?;

        // Build request
        let mut request = match method.to_uppercase().as_str() {
            "GET" => self.client.get(parsed_url.clone()),
            "POST" => self.client.post(parsed_url.clone()),
            "PUT" => self.client.put(parsed_url.clone()),
            "DELETE" => self.client.delete(parsed_url.clone()),
            "PATCH" => self.client.patch(parsed_url.clone()),
            _ => {
                return Err(ToolError::InvalidParameters(format!(
                    "unsupported method: {}",
                    method
                )));
            }
        };

        // Add headers
        for (key, value) in &headers_vec {
            request = request.header(key.as_str(), value.as_str());
        }

        // Add body if present
        let body_bytes = if let Some(body) = params.get("body") {
            if let Some(body_str) = body.as_str() {
                if let Ok(json_body) = serde_json::from_str::<serde_json::Value>(body_str) {
                    let bytes = serde_json::to_vec(&json_body).map_err(|e| {
                        ToolError::InvalidParameters(format!("invalid body JSON: {}", e))
                    })?;
                    request = request.json(&json_body);
                    Some(bytes)
                } else {
                    let bytes = body_str.as_bytes().to_vec();
                    request = request.body(body_str.to_string());
                    Some(bytes)
                }
            } else {
                let bytes = serde_json::to_vec(body).map_err(|e| {
                    ToolError::InvalidParameters(format!("invalid body JSON: {}", e))
                })?;
                request = request.json(body);
                Some(bytes)
            }
        } else {
            None
        };

        // Leak detection on outbound request (url/headers/body)
        let detector = LeakDetector::new();
        detector
            .scan_http_request(parsed_url.as_str(), &headers_vec, body_bytes.as_deref())
            .map_err(|e| ToolError::NotAuthorized(format!("{}", e)))?;

        // Execute request
        let response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                ToolError::Timeout(Duration::from_secs(30))
            } else {
                ToolError::ExternalService(e.to_string())
            }
        })?;

        let status = response.status().as_u16();

        // Block redirects: the server tried to send us elsewhere (potential SSRF)
        if (300..400).contains(&status) {
            return Err(ToolError::NotAuthorized(format!(
                "request returned redirect (HTTP {}), which is blocked to prevent SSRF",
                status
            )));
        }

        let headers: HashMap<String, String> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
            .collect();

        // Pre-check Content-Length header to reject obviously oversized responses
        // before downloading anything, preventing OOM from malicious servers.
        if let Some(content_length) = response.headers().get(reqwest::header::CONTENT_LENGTH)
            && let Ok(s) = content_length.to_str()
            && let Ok(len) = s.parse::<usize>()
            && len > MAX_RESPONSE_SIZE
        {
            tracing::warn!(
                url = %parsed_url,
                content_length = len,
                max = MAX_RESPONSE_SIZE,
                "Rejected HTTP response: Content-Length exceeds limit"
            );
            return Err(ToolError::ExecutionFailed(format!(
                "Response Content-Length ({} bytes) exceeds maximum allowed size ({} bytes)",
                len, MAX_RESPONSE_SIZE
            )));
        }

        // Stream the response body with a hard size cap. Even if Content-Length was
        // absent or lied about the size, we stop reading once we exceed the limit.
        let mut body = Vec::new();
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
        let body_bytes = bytes::Bytes::from(body);

        let body_text = String::from_utf8_lossy(&body_bytes).into_owned();

        // Try to parse as JSON, fall back to string
        let body: serde_json::Value = serde_json::from_str(&body_text)
            .unwrap_or_else(|_| serde_json::Value::String(body_text.clone()));

        let result = serde_json::json!({
            "status": status,
            "headers": headers,
            "body": body
        });

        Ok(ToolOutput::success(result, start.elapsed()).with_raw(body_text))
    }

    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        Some(Duration::from_secs(5)) // Average HTTP request time
    }

    fn requires_sanitization(&self) -> bool {
        true // External data always needs sanitization
    }

    fn requires_approval(&self) -> bool {
        true // HTTP requests go to external services, require user approval
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_tool_schema_headers_is_array() {
        let tool = HttpTool::new();
        let schema = tool.parameters_schema();
        assert_eq!(schema["properties"]["headers"]["type"], "array");
    }

    #[test]
    fn test_validate_url_rejects_http() {
        let err = validate_url("http://example.com").unwrap_err();
        assert!(err.to_string().contains("https"));
    }

    #[test]
    fn test_validate_url_rejects_localhost() {
        let err = validate_url("https://localhost:8080").unwrap_err();
        assert!(err.to_string().contains("localhost"));
    }

    #[test]
    fn test_validate_url_accepts_https_public() {
        let url = validate_url("https://example.com").unwrap();
        assert_eq!(url.host_str(), Some("example.com"));
    }

    #[test]
    fn test_validate_url_rejects_private_ip_literal() {
        let err = validate_url("https://192.168.1.1/api").unwrap_err();
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn test_validate_url_rejects_loopback_ip() {
        let err = validate_url("https://127.0.0.1/api").unwrap_err();
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn test_validate_url_rejects_link_local() {
        let err = validate_url("https://169.254.169.254/latest/meta-data/").unwrap_err();
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn test_is_disallowed_ip_covers_ranges() {
        use std::net::Ipv4Addr;

        // Private ranges
        assert!(is_disallowed_ip(&IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_disallowed_ip(&IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_disallowed_ip(&IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1))));
        // Loopback
        assert!(is_disallowed_ip(&IpAddr::V4(Ipv4Addr::LOCALHOST)));
        // Cloud metadata
        assert!(is_disallowed_ip(&IpAddr::V4(Ipv4Addr::new(
            169, 254, 169, 254
        ))));
        // Public
        assert!(!is_disallowed_ip(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn test_max_response_size_is_reasonable() {
        // MAX_RESPONSE_SIZE should be 5 MB to prevent OOM while allowing typical API responses.
        assert_eq!(MAX_RESPONSE_SIZE, 5 * 1024 * 1024);
    }

    #[test]
    fn test_parse_headers_param_accepts_object_legacy_shape() {
        let headers = serde_json::json!({"Authorization": "Bearer token"});
        let parsed = parse_headers_param(Some(&headers)).unwrap();
        assert_eq!(
            parsed,
            vec![("Authorization".to_string(), "Bearer token".to_string())]
        );
    }

    #[test]
    fn test_parse_headers_param_accepts_array_shape() {
        let headers = serde_json::json!([
            {"name": "Authorization", "value": "Bearer token"},
            {"name": "X-Test", "value": "1"}
        ]);
        let parsed = parse_headers_param(Some(&headers)).unwrap();
        assert_eq!(
            parsed,
            vec![
                ("Authorization".to_string(), "Bearer token".to_string()),
                ("X-Test".to_string(), "1".to_string())
            ]
        );
    }

    #[test]
    fn test_http_tool_schema_body_has_type() {
        let schema = HttpTool::new().parameters_schema();
        let body = schema
            .get("properties")
            .and_then(|p| p.get("body"))
            .expect("body schema missing");

        assert!(
            body.get("type").is_some(),
            "body schema must include a type for OpenAI-compatible tool validation"
        );
    }
}
