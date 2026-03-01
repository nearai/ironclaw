//! HTTP request tool.

use std::collections::HashMap;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use crate::context::JobContext;
use crate::safety::LeakDetector;
use crate::secrets::SecretsStore;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};
use crate::tools::wasm::{InjectedCredentials, SharedCredentialRegistry, inject_credential};

#[cfg(feature = "html-to-markdown")]
use crate::tools::builtin::convert_html_to_markdown;

/// Maximum response body size for text responses (5 MB).
///
/// 5 MB is large enough for typical JSON API responses and moderate HTML pages,
/// but small enough to prevent OOM from malicious or runaway servers.  The WASM
/// HTTP wrapper uses the same limit for consistency.
const MAX_RESPONSE_SIZE: usize = 5 * 1024 * 1024;

/// Maximum response body size when saving to disk via `save_to` (50 MB).
///
/// Larger limit for file downloads since the body is written to disk, not held
/// in memory for LLM context. Matches the WASM attachment size cap.
const MAX_SAVE_TO_SIZE: usize = 50 * 1024 * 1024;

/// Tool for making HTTP requests.
pub struct HttpTool {
    client: Client,
    credential_registry: Option<Arc<SharedCredentialRegistry>>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
}

impl HttpTool {
    /// Create a new HTTP tool.
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 10 {
                    return attempt.error("too many redirects");
                }
                // Reject scheme downgrades (https → http)
                if attempt.url().scheme() != "https" {
                    return attempt.error("redirect to non-HTTPS URL is not allowed");
                }
                // Extract host info before consuming attempt
                let host_owned = attempt.url().host_str().map(|h| h.to_owned());
                let port = attempt.url().port_or_known_default().unwrap_or(443);

                if let Some(host) = host_owned {
                    let host_lower = host.to_lowercase();
                    if host_lower == "localhost" || host_lower.ends_with(".localhost") {
                        return attempt.error("redirect to localhost is not allowed");
                    }
                    if let Ok(ip) = host.parse::<IpAddr>()
                        && is_disallowed_ip(&ip)
                    {
                        return attempt
                            .error("redirect to private/local IP is not allowed");
                    }
                    // Resolve hostname and check all IPs
                    let socket_addr = format!("{}:{}", host, port);
                    if let Ok(addrs) = socket_addr.to_socket_addrs() {
                        for addr in addrs {
                            if is_disallowed_ip(&addr.ip()) {
                                let msg = format!(
                                    "redirect target '{}' resolves to disallowed IP {}",
                                    host,
                                    addr.ip()
                                );
                                return attempt.error(msg);
                            }
                        }
                    }
                }
                attempt.follow()
            }))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            credential_registry: None,
            secrets_store: None,
        }
    }

    /// Attach a credential registry and secrets store for auto-injection.
    pub fn with_credentials(
        mut self,
        registry: Arc<SharedCredentialRegistry>,
        secrets_store: Arc<dyn SecretsStore + Send + Sync>,
    ) -> Self {
        self.credential_registry = Some(registry);
        self.secrets_store = Some(secrets_store);
        self
    }
}

/// Validate and resolve a `save_to` path, ensuring it stays under `/tmp/`.
///
/// Uses `path_utils::validate_path` with `/tmp` as the base directory to catch
/// traversal attacks like `/tmp/../../etc/passwd` and symlink escapes.
/// Creates parent directories as needed before validation.
fn validate_save_to_path(save_to: &str) -> Result<std::path::PathBuf, ToolError> {
    // Quick prefix check before doing any fs work
    if !save_to.starts_with("/tmp/") {
        return Err(ToolError::InvalidParameters(
            "save_to path must be under /tmp/".to_string(),
        ));
    }
    let save_path = std::path::Path::new(save_to);
    if let Some(parent) = save_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            ToolError::ExecutionFailed(format!("failed to create directory: {}", e))
        })?;
    }
    // Delegate to the shared path validator with /tmp as sandbox base
    let tmp_base = std::path::Path::new("/tmp");
    crate::tools::builtin::path_utils::validate_path(save_to, Some(tmp_base))
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

#[cfg(feature = "html-to-markdown")]
/// Heuristic: treat as HTML if the `Content-Type` header contains `text/html`.
fn is_html_response(headers: &HashMap<String, String>) -> bool {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.to_lowercase().contains("text/html"))
        .unwrap_or(false)
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

/// Extract host from URL in params (for approval checks).
fn extract_host_from_params(params: &serde_json::Value) -> Option<String> {
    params
        .get("url")
        .and_then(|u| u.as_str())
        .and_then(|u| reqwest::Url::parse(u).ok())
        .and_then(|u| u.host_str().map(|h| h.to_string()))
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
        "Make HTTP requests to external APIs. Supports GET, POST, PUT, DELETE methods. \
         Use save_to to download binary files (images, PDFs, etc.) to a local path, \
         e.g. {\"method\":\"GET\",\"url\":\"https://picsum.photos/800/600\",\"save_to\":\"/tmp/photo.jpg\"}."
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
                    "description": "Request body (for POST/PUT/PATCH). Can be a JSON object, array, string, or other value."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Request timeout in seconds (default: 30)"
                },
                "save_to": {
                    "type": "string",
                    "description": "Save response body as raw bytes to this file path instead of returning it. Use for binary downloads (images, PDFs, etc.). The path must be under /tmp/."
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
        let mut parsed_url = validate_url(url)?;

        // Parse headers
        let mut headers_vec = parse_headers_param(params.get("headers"))?;

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

        // Credential injection from shared registry
        if let (Some(registry), Some(store)) = (
            self.credential_registry.as_ref(),
            self.secrets_store.as_ref(),
        ) {
            let host = parsed_url.host_str().unwrap_or("");
            let matched: Vec<crate::secrets::CredentialMapping> = registry.find_for_host(host);
            for mapping in &matched {
                match store
                    .get_decrypted(&_ctx.user_id, &mapping.secret_name)
                    .await
                {
                    Ok(secret) => {
                        let mut injected = InjectedCredentials::empty();
                        inject_credential(&mut injected, &mapping.location, &secret);
                        for (name, value) in &injected.headers {
                            request = request.header(name.as_str(), value.as_str());
                            headers_vec.push((name.clone(), value.clone()));
                        }
                        for (name, value) in &injected.query_params {
                            parsed_url.query_pairs_mut().append_pair(name, value);
                            request = request.query(&[(name.as_str(), value.as_str())]);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            secret = %mapping.secret_name,
                            error = %e,
                            "Failed to inject credential for HTTP tool"
                        );
                    }
                }
            }
        }

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

        // Redirects are followed automatically (up to 10 hops).
        // If we still see a 3xx here, the chain was too long.

        let headers: HashMap<String, String> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
            .collect();

        // Use a larger size limit when saving to disk (file downloads)
        let saving_to_disk = params.get("save_to").is_some();
        let max_size = if saving_to_disk {
            MAX_SAVE_TO_SIZE
        } else {
            MAX_RESPONSE_SIZE
        };

        // Pre-check Content-Length header to reject obviously oversized responses
        // before downloading anything, preventing OOM from malicious servers.
        if let Some(content_length) = response.headers().get(reqwest::header::CONTENT_LENGTH)
            && let Ok(s) = content_length.to_str()
            && let Ok(len) = s.parse::<usize>()
            && len > max_size
        {
            tracing::warn!(
                url = %parsed_url,
                content_length = len,
                max = max_size,
                "Rejected HTTP response: Content-Length exceeds limit"
            );
            return Err(ToolError::ExecutionFailed(format!(
                "Response Content-Length ({} bytes) exceeds maximum allowed size ({} bytes)",
                len, max_size
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
            if body.len() + chunk.len() > max_size {
                return Err(ToolError::ExecutionFailed(format!(
                    "Response body exceeds maximum allowed size ({} bytes)",
                    max_size
                )));
            }
            body.extend_from_slice(&chunk);
        }
        let body_bytes = bytes::Bytes::from(body);

        // If save_to is specified, write raw bytes to file and return metadata.
        if let Some(save_to) = params.get("save_to").and_then(|v| v.as_str()) {
            let save_to_owned = save_to.to_string();
            let bytes_clone = body_bytes.clone();
            tokio::task::spawn_blocking(move || {
                let canonical = validate_save_to_path(&save_to_owned)?;
                std::fs::write(&canonical, &bytes_clone).map_err(|e| {
                    ToolError::ExecutionFailed(format!("failed to write file: {}", e))
                })?;
                Ok::<_, ToolError>(canonical)
            })
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("spawn_blocking failed: {}", e)))?
            .map_err(|e: ToolError| e)?;
            let result = serde_json::json!({
                "status": status,
                "saved_to": save_to,
                "size_bytes": body_bytes.len(),
                "headers": headers,
            });
            return Ok(ToolOutput::success(result, start.elapsed()));
        }

        let body_text = String::from_utf8_lossy(&body_bytes).into_owned();

        #[cfg(feature = "html-to-markdown")]
        let body_text = if is_html_response(&headers) {
            match convert_html_to_markdown(&body_text, parsed_url.as_str()) {
                Ok(md) => md,
                Err(e) => {
                    tracing::warn!(url = %parsed_url, error = %e, "HTML-to-markdown conversion failed, returning raw HTML");
                    body_text
                }
            }
        } else {
            body_text
        };

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

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        // 1. Manual auth headers/query params in LLM params
        if crate::safety::params_contain_manual_credentials(params) {
            return ApprovalRequirement::Always;
        }
        // 2. Target host has credential mappings (will be auto-injected)
        if let Some(ref registry) = self.credential_registry
            && let Some(host) = extract_host_from_params(params)
            && registry.has_credentials_for_host(&host)
        {
            return ApprovalRequirement::Always;
        }
        // Default: outbound HTTP still needs approval unless auto-approved
        ApprovalRequirement::UnlessAutoApproved
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(30, 500))
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
    fn test_http_tool_schema_body_is_freeform() {
        let schema = HttpTool::new().parameters_schema();
        let body = schema
            .get("properties")
            .and_then(|p| p.get("body"))
            .expect("body schema missing");

        // Body is intentionally freeform (no "type" constraint) for OpenAI
        // compatibility. OpenAI rejects union types containing "array" unless
        // "items" is also specified, and body accepts any JSON value.
        assert!(
            body.get("type").is_none(),
            "body schema should not have a 'type' to be freeform for OpenAI compatibility"
        );
    }

    // ── Approval requirement tests ──────────────────────────────────────

    #[test]
    fn test_no_auth_headers_returns_unless_auto_approved() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data"
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_auth_header_object_format_returns_always() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data",
            "headers": {"Authorization": "Bearer token123"}
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_auth_header_array_format_returns_always() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data",
            "headers": [{"name": "Authorization", "value": "Bearer token123"}]
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_auth_header_case_insensitive() {
        let tool = HttpTool::new();

        // Object format with mixed case
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"AUTHORIZATION": "Bearer x"}
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);

        // Array format with mixed case
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": [{"name": "X-Api-Key", "value": "key123"}]
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_all_auth_header_names_detected() {
        let tool = HttpTool::new();
        for header_name in [
            "authorization",
            "x-api-key",
            "cookie",
            "proxy-authorization",
            "x-auth-token",
            "api-key",
            "x-token",
            "x-access-token",
            "x-session-token",
            "x-csrf-token",
            "x-secret",
            "x-api-secret",
        ] {
            let mut headers = serde_json::Map::new();
            headers.insert(header_name.to_string(), serde_json::json!("value"));
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://example.com",
                "headers": headers
            });
            assert_eq!(
                tool.requires_approval(&params),
                ApprovalRequirement::Always,
                "Header '{}' should trigger Always approval",
                header_name
            );
        }
    }

    #[test]
    fn test_non_auth_headers_return_unless_auto_approved() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"Content-Type": "application/json", "Accept": "text/html"}
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_empty_headers_return_unless_auto_approved() {
        let tool = HttpTool::new();

        // Empty object
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {}
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );

        // Empty array
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": []
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    // ── Credential registry approval tests ─────────────────────────────

    #[test]
    fn test_host_with_credential_mapping_returns_always() {
        use crate::secrets::CredentialMapping;
        use crate::tools::wasm::SharedCredentialRegistry;

        let registry = Arc::new(SharedCredentialRegistry::new());
        registry.add_mappings(vec![CredentialMapping::bearer(
            "openai_key",
            "api.openai.com",
        )]);

        let tool = HttpTool::new().with_credentials(
            registry,
            // secrets_store is not used in requires_approval, just needs to be present
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    "0123456789abcdef0123456789abcdef".to_string(),
                ))
                .unwrap(),
            ))),
        );

        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.openai.com/v1/models"
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_host_without_credential_mapping_returns_unless_auto_approved() {
        use crate::tools::wasm::SharedCredentialRegistry;

        let registry = Arc::new(SharedCredentialRegistry::new());
        // Empty registry - no credential mappings

        let tool = HttpTool::new().with_credentials(
            registry,
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    "0123456789abcdef0123456789abcdef".to_string(),
                ))
                .unwrap(),
            ))),
        );

        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data"
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_url_query_param_credential_returns_always() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data?api_key=secret123"
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_bearer_value_in_custom_header_returns_always() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"X-Custom": "Bearer sk-test123"}
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Always);
    }

    #[test]
    fn test_extract_host_from_params_valid() {
        let params = serde_json::json!({
            "url": "https://api.example.com/path"
        });
        assert_eq!(
            extract_host_from_params(&params),
            Some("api.example.com".to_string())
        );
    }

    #[test]
    fn test_extract_host_from_params_missing_url() {
        let params = serde_json::json!({"method": "GET"});
        assert_eq!(extract_host_from_params(&params), None);
    }

    // ── save_to path validation tests ─────────────────────────────────────

    #[test]
    fn test_save_to_rejects_path_outside_tmp() {
        let err = validate_save_to_path("/etc/passwd").unwrap_err();
        assert!(err.to_string().contains("must be under /tmp/"));
    }

    #[test]
    fn test_save_to_rejects_home_dir() {
        let err = validate_save_to_path("/home/user/file.txt").unwrap_err();
        assert!(err.to_string().contains("must be under /tmp/"));
    }

    #[test]
    fn test_save_to_rejects_traversal_via_dotdot() {
        // Starts with /tmp/ but traverses out
        let err = validate_save_to_path("/tmp/../../etc/passwd").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("escapes") || msg.contains("resolves outside"),
            "expected path traversal rejection, got: {}",
            msg
        );
    }

    #[test]
    fn test_save_to_rejects_deep_traversal() {
        let err = validate_save_to_path("/tmp/a/b/../../../../etc/shadow").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("escapes") || msg.contains("resolves outside"),
            "expected path traversal rejection, got: {}",
            msg
        );
    }

    #[test]
    fn test_save_to_accepts_simple_tmp_path() {
        let path = validate_save_to_path("/tmp/test_ironclaw_photo.jpg").unwrap();
        assert!(path.starts_with("/tmp"));
        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_save_to_accepts_nested_tmp_path() {
        let path =
            validate_save_to_path("/tmp/ironclaw_test_subdir/nested/file.png").unwrap();
        assert!(path.starts_with("/tmp"));
        // Cleanup
        let _ = std::fs::remove_dir_all("/tmp/ironclaw_test_subdir");
    }

    #[test]
    fn test_save_to_rejects_bare_tmp() {
        // /tmp without trailing slash should be rejected (not a file under /tmp/)
        let err = validate_save_to_path("/tmp").unwrap_err();
        assert!(err.to_string().contains("must be under /tmp/"));
    }
}
