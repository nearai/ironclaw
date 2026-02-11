//! HTTP request tool.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;

use crate::context::JobContext;
use crate::safety::LeakDetector;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

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

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_disallowed_ip(&ip) {
            return Err(ToolError::NotAuthorized(
                "private or local IPs are not allowed".to_string(),
            ));
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
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "HTTP headers to include"
                },
                "body": {
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

        let method = params
            .get("method")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("missing 'method' parameter".to_string())
            })?;

        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("missing 'url' parameter".to_string()))?;
        let parsed_url = validate_url(url)?;

        // Parse headers
        let headers: HashMap<String, String> = params
            .get("headers")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let headers_vec: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

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
        for (key, value) in headers {
            request = request.header(&key, &value);
        }

        // Add body if present
        let body_bytes = if let Some(body) = params.get("body") {
            let bytes = serde_json::to_vec(body)
                .map_err(|e| ToolError::InvalidParameters(format!("invalid body JSON: {}", e)))?;
            request = request.json(body);
            Some(bytes)
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

        // Get response body
        let body_text = response.text().await.map_err(|e| {
            ToolError::ExternalService(format!("failed to read response body: {}", e))
        })?;

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
    use super::validate_url;

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
}
