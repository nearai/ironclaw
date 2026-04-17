//! HTTP request tool.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use crate::auth::resolve_secret_for_runtime;
use crate::context::JobContext;
use crate::db::UserStore;
use crate::secrets::SecretsStore;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};
use crate::tools::wasm::{InjectedCredentials, SharedCredentialRegistry, inject_credential};
use ironclaw_safety::LeakDetector;

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

/// Default request timeout when the caller does not provide one.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Maximum allowed request timeout to bound resource usage from LLM-controlled inputs.
const MAX_TIMEOUT_SECS: u64 = 300;

/// Maximum number of redirects to follow for simple GET requests.
const MAX_REDIRECTS: usize = 3;

/// Descriptive User-Agent so public APIs don't reject bare requests.
const USER_AGENT: &str = concat!(
    "IronClaw-Agent/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/nearai/ironclaw)"
);

/// Tool for making HTTP requests.
///
/// Each request builds a per-request [`Client`] with DNS pinning to prevent
/// TOCTOU DNS rebinding attacks.  The hostname is resolved once, validated
/// against the SSRF blocklist, and then pinned via
/// [`reqwest::ClientBuilder::resolve_to_addrs`] so that reqwest connects
/// directly to the pre-validated IPs without a second DNS lookup.
pub struct HttpTool {
    credential_registry: Option<Arc<SharedCredentialRegistry>>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    role_lookup: Option<Arc<dyn UserStore>>,
}

impl HttpTool {
    /// Create a new HTTP tool.
    pub fn new() -> Self {
        Self {
            credential_registry: None,
            secrets_store: None,
            role_lookup: None,
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

    pub fn with_role_lookup(mut self, role_lookup: Arc<dyn UserStore>) -> Self {
        self.role_lookup = Some(role_lookup);
        self
    }
}

/// Validate and resolve a `save_to` path, ensuring it stays under `/tmp/`.
///
/// Uses `path_utils::validate_path` with `/tmp` as the base directory to catch
/// traversal attacks like `/tmp/../../etc/passwd` and symlink escapes.
/// Creates parent directories only after validation succeeds.
fn validate_save_to_path(save_to: &str) -> Result<std::path::PathBuf, ToolError> {
    // Quick prefix check before doing any fs work
    if !save_to.starts_with("/tmp/") {
        return Err(ToolError::InvalidParameters(
            "save_to path must be under /tmp/".to_string(),
        ));
    }
    // Validate path BEFORE creating directories to prevent traversal-based
    // directory creation outside /tmp (e.g. `/tmp/../../etc/passwd`).
    let tmp_base = std::path::Path::new("/tmp");
    let validated = crate::tools::builtin::path_utils::validate_path(save_to, Some(tmp_base))?;
    // Only create parent directories for the validated (safe) path
    if let Some(parent) = validated.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            ToolError::ExecutionFailed(format!("failed to create directory: {}", e))
        })?;
    }
    Ok(validated)
}

/// Whether the HTTP tool allows localhost/HTTP URLs (for E2E testing).
///
/// Set `HTTP_ALLOW_LOCALHOST=true` to bypass HTTPS-only and SSRF checks for
/// `http://127.0.0.1` targets.  **Never enable in production.**
fn allow_localhost() -> bool {
    static ALLOW: OnceLock<bool> = OnceLock::new();
    *ALLOW.get_or_init(|| {
        std::env::var("HTTP_ALLOW_LOCALHOST")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false)
    })
}

/// Parse and validate a URL without DNS resolution.
///
/// Checks scheme (HTTPS only), rejects localhost and private/link-local IP
/// literals.  Does **not** resolve hostnames -- use [`validate_and_resolve_url`]
/// for the full DNS-pinning flow that eliminates the TOCTOU rebinding window.
pub(crate) fn validate_url(url: &str) -> Result<reqwest::Url, ToolError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| ToolError::InvalidParameters(format!("invalid URL: {}", e)))?;

    // In test mode, allow http:// and localhost/127.0.0.1 targets.
    if allow_localhost() {
        if parsed.scheme() != "https" && parsed.scheme() != "http" {
            return Err(ToolError::NotAuthorized(
                "only http(s) URLs are allowed".to_string(),
            ));
        }
        return Ok(parsed);
    }

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

    Ok(parsed)
}

/// Resolve DNS for a validated URL and check every resolved address against
/// the SSRF blocklist.
///
/// Returns the resolved [`SocketAddr`]s so that callers can pin the hostname
/// via [`reqwest::ClientBuilder::resolve_to_addrs`], preventing a DNS rebinding
/// attack where a second, independent resolution (inside reqwest) returns a
/// different -- potentially private -- IP after our validation pass.
pub(crate) async fn validate_and_resolve_url(
    url: &reqwest::Url,
) -> Result<Vec<SocketAddr>, ToolError> {
    let host = url
        .host_str()
        .ok_or_else(|| ToolError::InvalidParameters("URL missing host".to_string()))?;

    let port = url.port_or_known_default().unwrap_or(443);

    let addrs: Vec<SocketAddr> = tokio::net::lookup_host(format!("{}:{}", host, port))
        .await
        .map_err(|e| {
            ToolError::ExternalService(format!("DNS resolution failed for '{}': {}", host, e))
        })?
        .collect();

    if addrs.is_empty() {
        return Err(ToolError::ExternalService(format!(
            "DNS resolution for '{}' returned no addresses",
            host
        )));
    }

    if !allow_localhost() {
        for addr in &addrs {
            if is_disallowed_ip(&addr.ip()) {
                return Err(ToolError::NotAuthorized(format!(
                    "hostname '{}' resolves to disallowed IP {}",
                    host,
                    addr.ip()
                )));
            }
        }
    }

    Ok(addrs)
}

/// Build a reqwest [`Client`] that pins the given hostname to the
/// pre-validated resolved addresses, preventing any second DNS lookup.
pub(crate) fn build_pinned_client(
    host: &str,
    resolved_addrs: &[SocketAddr],
    timeout: Duration,
    redirect_policy: reqwest::redirect::Policy,
) -> Result<Client, ToolError> {
    let builder = Client::builder()
        .timeout(timeout)
        .redirect(redirect_policy)
        .user_agent(USER_AGENT)
        .resolve_to_addrs(host, resolved_addrs);

    builder
        .build()
        .map_err(|e| ToolError::ExternalService(format!("failed to build HTTP client: {}", e)))
}

/// Check whether an IPv4 address falls in a disallowed range (private,
/// loopback, link-local, multicast, unspecified, or cloud metadata).
fn is_disallowed_ipv4(v4: &Ipv4Addr) -> bool {
    v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_unspecified()
        || *v4 == Ipv4Addr::new(169, 254, 169, 254)
        || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
}

fn is_disallowed_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_disallowed_ipv4(v4),
        IpAddr::V6(v6) => {
            // Catch IPv4-mapped IPv6 addresses (e.g. ::ffff:169.254.169.254)
            // that would bypass IPv4-only checks.
            if let Some(v4) = v6.to_ipv4_mapped()
                && is_disallowed_ipv4(&v4)
            {
                return true;
            }

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
    fn parse_header_object(
        map: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Vec<(String, String)>, ToolError> {
        let mut out = Vec::with_capacity(map.len());
        for (k, v) in map {
            let value = v.as_str().ok_or_else(|| {
                ToolError::InvalidParameters(format!("header '{}' must have a string value", k))
            })?;
            out.push((k.clone(), value.to_string()));
        }
        Ok(out)
    }

    fn parse_header_array(items: &[serde_json::Value]) -> Result<Vec<(String, String)>, ToolError> {
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

    match headers {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(serde_json::Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(Vec::new());
            }
            let parsed = serde_json::from_str::<serde_json::Value>(trimmed).map_err(|e| {
                ToolError::InvalidParameters(format!(
                    "headers string must contain valid JSON object/array: {}",
                    e
                ))
            })?;
            match parsed {
                serde_json::Value::Object(map) => parse_header_object(&map),
                serde_json::Value::Array(items) => parse_header_array(&items),
                _ => Err(ToolError::InvalidParameters(
                    "headers string must decode to a JSON object or array".to_string(),
                )),
            }
        }
        Some(serde_json::Value::Object(map)) => parse_header_object(map),
        Some(serde_json::Value::Array(items)) => parse_header_array(items),
        Some(_) => Err(ToolError::InvalidParameters(
            "'headers' must be an object or an array of {name, value}".to_string(),
        )),
    }
}

fn parse_timeout_secs_param(timeout: Option<&serde_json::Value>) -> Result<Option<u64>, ToolError> {
    let parsed = match timeout {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => n.as_u64().map(Some).ok_or_else(|| {
            ToolError::InvalidParameters("timeout_secs must be a non-negative integer".to_string())
        }),
        Some(serde_json::Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let secs = trimmed.parse::<u64>().map_err(|_| {
                ToolError::InvalidParameters(
                    "timeout_secs string must contain a non-negative integer".to_string(),
                )
            })?;
            Ok(Some(secs))
        }
        Some(_) => Err(ToolError::InvalidParameters(
            "timeout_secs must be an integer".to_string(),
        )),
    }?;

    if let Some(secs) = parsed
        && secs > MAX_TIMEOUT_SECS
    {
        return Err(ToolError::InvalidParameters(format!(
            "timeout_secs must be <= {}",
            MAX_TIMEOUT_SECS
        )));
    }

    Ok(parsed)
}

fn parse_save_to_param(save_to: Option<&serde_json::Value>) -> Result<Option<String>, ToolError> {
    match save_to {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(path)) => {
            let trimmed = path.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Some(_) => Err(ToolError::InvalidParameters(
            "save_to must be a string".to_string(),
        )),
    }
}

/// Extract host from URL in params (for approval checks).
/// Extract the host from an HTTP tool's params (for credential registry lookup).
pub fn extract_host_from_params(params: &serde_json::Value) -> Option<String> {
    params
        .get("url")
        .and_then(|u| u.as_str())
        .and_then(|u| reqwest::Url::parse(u).ok())
        .and_then(|u| u.host_str().map(|h| h.to_string()))
}

/// Deduplicate credential mappings by `(secret_name, location)`.
///
/// The same secret can be declared by both a WASM tool's capabilities
/// and a skill's `credentials` block, producing duplicates. Without
/// dedup the HTTP client sends multiple identical `Authorization`
/// headers which some servers (e.g. GitHub) reject as 401 Bad
/// credentials.
pub(crate) fn dedup_credential_mappings(
    mappings: Vec<crate::secrets::CredentialMapping>,
) -> Vec<crate::secrets::CredentialMapping> {
    let mut seen: std::collections::HashSet<(String, crate::secrets::CredentialLocation)> =
        std::collections::HashSet::new();
    mappings
        .into_iter()
        .filter(|m| seen.insert((m.secret_name.clone(), m.location.clone())))
        .collect()
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
                    "description": "HTTP method (default: GET)"
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
            "required": ["url"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let method = params["method"].as_str().unwrap_or("GET");
        let method_upper = method.to_uppercase();

        let url = require_str(&params, "url")?;
        let mut parsed_url = validate_url(url)?;

        // Resolve DNS once, validate against SSRF blocklist, then pin the
        // resolved addresses into the reqwest client so it cannot re-resolve
        // to a different (potentially private) IP.
        let resolved_addrs = validate_and_resolve_url(&parsed_url).await?;
        let host = parsed_url
            .host_str()
            .ok_or_else(|| ToolError::InvalidParameters("URL missing host".into()))?
            .to_string();
        let client = build_pinned_client(
            &host,
            &resolved_addrs,
            Duration::from_secs(30),
            reqwest::redirect::Policy::none(),
        )?;

        // Parse headers. `headers_vec` collects both the caller-supplied
        // headers AND any credential-injection results appended later.
        // We keep a separate snapshot of just the caller-supplied set so
        // the trace recorder never sees injected `Authorization` / API-key
        // values. Recording must happen at the pre-injection boundary —
        // see the `intercept_req` construction below.
        let mut headers_vec = parse_headers_param(params.get("headers"))?;
        let caller_headers: Vec<(String, String)> = headers_vec.clone();

        // Block LLM-provided authorization headers when the host has registered
        // credential mappings. Credentials must come from the registry, not from
        // LLM-generated arguments — prevents prompt-injection exfiltration.
        if let Some(registry) = self.credential_registry.as_ref() {
            let cred_host = parsed_url.host_str().unwrap_or("");
            if registry.has_credentials_for_host(cred_host) {
                let forbidden: &[&str] = &["authorization", "x-api-key", "api-key", "x-auth-token"];
                for (name, _) in &headers_vec {
                    if forbidden.iter().any(|f| name.eq_ignore_ascii_case(f)) {
                        return Err(ToolError::NotAuthorized(format!(
                            "Manual '{}' header blocked for host '{}': \
                             credentials are auto-injected by the credential system",
                            name, cred_host
                        )));
                    }
                }
            }
        }

        let timeout_secs = parse_timeout_secs_param(params.get("timeout_secs"))?;
        let save_to = parse_save_to_param(params.get("save_to"))?;
        let effective_timeout = Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS));

        // Build request
        let mut request = match method.to_uppercase().as_str() {
            "GET" => client.get(parsed_url.clone()),
            "POST" => client.post(parsed_url.clone()),
            "PUT" => client.put(parsed_url.clone()),
            "DELETE" => client.delete(parsed_url.clone()),
            "PATCH" => client.patch(parsed_url.clone()),
            _ => {
                return Err(ToolError::InvalidParameters(format!(
                    "unsupported method: {}",
                    method
                )));
            }
        };

        request = request.timeout(effective_timeout);

        // Add headers
        for (key, value) in &headers_vec {
            request = request.header(key.as_str(), value.as_str());
        }

        // Add body if present (skip null — Python's None becomes JSON null)
        let body_bytes = if let Some(body) = params.get("body")
            && !body.is_null()
        {
            if let Some(body_str) = body.as_str() {
                if body_str.is_empty() {
                    None
                } else if let Ok(json_body) = serde_json::from_str::<serde_json::Value>(body_str) {
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

        // Leak detection on outbound request BEFORE credential injection.
        // Credentials are injected by the system, not the LLM — they are not
        // exfiltration attempts. Scanning after injection would false-positive
        // on the injected Authorization header values.
        let detector = LeakDetector::new();
        detector
            .scan_http_request(parsed_url.as_str(), &headers_vec, body_bytes.as_deref())
            .map_err(|e| ToolError::NotAuthorized(format!("{}", e)))?;

        // Credential injection from shared registry (after leak check).
        // If a credential is registered but not yet configured, we proceed
        // without auth and check the response status — many endpoints (e.g.
        // GitHub public repo search) work without authentication.  Only if
        // the server returns 401/403 do we raise `authentication_required`.
        //
        // We track *why* the credential is unavailable so the 401/403 handler
        // can send the user down the right remediation path: a `Missing`
        // credential needs to be configured, while a `RefreshFailed` credential
        // already exists but its refresh token is dead and the user must
        // re-authenticate.
        #[derive(Clone, Copy, Debug)]
        enum MissingReason {
            NotConfigured,
            RefreshFailed,
        }
        let mut missing_credential: Option<(String, MissingReason)> = None;
        let mut injected_any_credential = false;
        if let (Some(registry), Some(store)) = (
            self.credential_registry.as_ref(),
            self.secrets_store.as_ref(),
        ) {
            let cred_host = parsed_url.host_str().unwrap_or("").to_string();
            let matched: Vec<crate::secrets::CredentialMapping> =
                registry.find_for_host(&cred_host);
            tracing::debug!(
                host = %cred_host,
                matched_count = matched.len(),
                url = %parsed_url,
                "HTTP tool credential lookup"
            );
            // Dedupe mappings by (secret_name, location) — see
            // `dedup_credential_mappings` doc comment for rationale.
            let dedup_matched = dedup_credential_mappings(matched);
            for mapping in &dedup_matched {
                let oauth_refresh = registry.oauth_refresh_for_secret(&mapping.secret_name);
                match resolve_secret_for_runtime(
                    store.as_ref(),
                    &ctx.user_id,
                    &mapping.secret_name,
                    self.role_lookup.as_deref(),
                    oauth_refresh.as_ref(),
                    crate::auth::DefaultFallback::AdminOnly,
                )
                .await
                {
                    Ok(secret) => {
                        injected_any_credential = true;
                        missing_credential = None;
                        // Redacted preview for triage: first and last 4 chars
                        // only, never the middle. Lets an operator tell at a
                        // glance whether the decrypted value even looks like
                        // the expected token (e.g. `ghp_…`, `github_pat_…`)
                        // without leaking it to logs.
                        let secret_str = secret.expose();
                        let char_count = secret_str.chars().count();
                        let preview = if char_count <= 8 {
                            "<short>".to_string()
                        } else {
                            let head: String = secret_str.chars().take(4).collect();
                            let tail: String = secret_str.chars().skip(char_count - 4).collect();
                            format!("{head}…{tail}")
                        };
                        tracing::debug!(
                            user_id = %ctx.user_id,
                            secret_name = %mapping.secret_name,
                            secret_len = secret.len(),
                            secret_preview = %preview,
                            "HTTP tool: credential found and injecting"
                        );
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
                    Err(error) if error.requires_authentication() && !injected_any_credential => {
                        tracing::debug!(
                            secret = %mapping.secret_name,
                            host = %cred_host,
                            error = ?error,
                            "Credential unavailable — proceeding without auth"
                        );
                        let reason = match error {
                            crate::auth::CredentialResolutionError::RefreshFailed => {
                                MissingReason::RefreshFailed
                            }
                            _ => MissingReason::NotConfigured,
                        };
                        missing_credential = Some((mapping.secret_name.clone(), reason));
                    }
                    Err(e) => {
                        tracing::warn!(
                            secret = %mapping.secret_name,
                            error = ?e,
                            "Failed to inject credential for HTTP tool"
                        );
                    }
                }
            }
        }

        // Build the interceptor request descriptor for recording/replay.
        // Use `caller_headers` (snapshot taken before credential injection)
        // so injected `Authorization: Bearer ...` / API keys never reach
        // the recorder. Replay matching uses `method` + `url` only, so
        // omitting the injected headers is safe for determinism.
        let intercept_req = crate::llm::recording::HttpExchangeRequest {
            method: method_upper,
            url: parsed_url.to_string(),
            headers: caller_headers,
            body: body_bytes
                .as_ref()
                .map(|b| crate::llm::recording::redact_body(&String::from_utf8_lossy(b))),
        };

        // Check HTTP interceptor (replay mode returns pre-recorded response)
        if let Some(ref interceptor) = ctx.http_interceptor
            && let Some(recorded) = interceptor.before_request(&intercept_req).await
        {
            let headers: HashMap<String, String> = recorded.headers.iter().cloned().collect();
            let body: serde_json::Value = serde_json::from_str(&recorded.body)
                .unwrap_or_else(|_| serde_json::Value::String(recorded.body.clone()));
            let result = serde_json::json!({
                "status": recorded.status,
                "headers": headers,
                "body": body
            });
            return Ok(ToolOutput::success(result, start.elapsed()).with_raw(recorded.body));
        }

        // Determine if this is a simple GET (eligible for redirect following).
        let is_simple_get =
            method.eq_ignore_ascii_case("GET") && headers_vec.is_empty() && body_bytes.is_none();

        // Execute request, optionally following redirects for simple GETs.
        // Each redirect hop gets its own DNS resolution + SSRF validation +
        // pinned client to prevent rebinding attacks across hops.
        let response = if is_simple_get {
            let mut redirects_remaining = MAX_REDIRECTS;
            loop {
                // Build a per-hop pinned client for the current URL.
                let hop_addrs = validate_and_resolve_url(&parsed_url).await?;
                let hop_host = parsed_url
                    .host_str()
                    .ok_or_else(|| ToolError::InvalidParameters("URL missing host".into()))?
                    .to_string();
                let hop_client = build_pinned_client(
                    &hop_host,
                    &hop_addrs,
                    effective_timeout,
                    reqwest::redirect::Policy::none(),
                )?;

                let resp = hop_client
                    .get(parsed_url.clone())
                    .header(
                        reqwest::header::ACCEPT,
                        "text/markdown, text/html;q=0.9, application/json;q=0.9, */*;q=0.8",
                    )
                    .send()
                    .await
                    .map_err(|e| {
                        if e.is_timeout() {
                            ToolError::Timeout(effective_timeout)
                        } else {
                            ToolError::ExternalService(e.to_string())
                        }
                    })?;

                let status = resp.status().as_u16();
                if (300..400).contains(&status) {
                    if redirects_remaining == 0 {
                        return Err(ToolError::ExecutionFailed(format!(
                            "too many redirects (max {})",
                            MAX_REDIRECTS
                        )));
                    }

                    let location = resp
                        .headers()
                        .get(reqwest::header::LOCATION)
                        .and_then(|v| v.to_str().ok())
                        .ok_or_else(|| {
                            ToolError::ExecutionFailed(format!(
                                "redirect (HTTP {}) has no Location header",
                                status
                            ))
                        })?;

                    let next_url_str =
                        if location.starts_with("http://") || location.starts_with("https://") {
                            location.to_string()
                        } else {
                            parsed_url
                                .join(location)
                                .map(|u| u.to_string())
                                .map_err(|e| {
                                    ToolError::ExecutionFailed(format!(
                                        "could not resolve relative redirect '{}': {}",
                                        location, e
                                    ))
                                })?
                        };

                    // SSRF re-validation on every hop (URL structure checks).
                    // DNS resolution + IP validation happens at the top of the
                    // next loop iteration via validate_and_resolve_url.
                    parsed_url = validate_url(&next_url_str)?;
                    let hop_detector = LeakDetector::new();
                    hop_detector
                        .scan_http_request(parsed_url.as_str(), &[], None)
                        .map_err(|e| ToolError::NotAuthorized(e.to_string()))?;

                    redirects_remaining -= 1;
                    tracing::debug!(
                        to = %parsed_url,
                        hops_left = redirects_remaining,
                        "http tool following redirect"
                    );
                    continue;
                }

                break resp;
            }
        } else {
            let resp = request.send().await.map_err(|e| {
                if e.is_timeout() {
                    ToolError::Timeout(effective_timeout)
                } else {
                    ToolError::ExternalService(e.to_string())
                }
            })?;

            let status = resp.status().as_u16();

            // Block redirects for non-simple requests (potential SSRF)
            if (300..400).contains(&status) {
                return Err(ToolError::NotAuthorized(format!(
                    "request returned redirect (HTTP {}), which is blocked to prevent SSRF",
                    status
                )));
            }

            resp
        };

        let status = response.status().as_u16();

        // If the server returned 401/403 and we had a missing credential,
        // surface the authentication_required error so the auth flow triggers.
        // Distinguish "never configured" from "refresh failed" so the user is
        // sent to the right remediation path (configure vs re-authenticate).
        if matches!(status, 401 | 403)
            && let Some((cred_name, reason)) = missing_credential.as_ref()
        {
            let (error_kind, message) = match reason {
                MissingReason::NotConfigured => (
                    "authentication_required",
                    format!(
                        "Credential '{}' is not configured. \
                         The server returned HTTP {}. Set up credentials to access this endpoint.",
                        cred_name, status
                    ),
                ),
                MissingReason::RefreshFailed => (
                    "authentication_refresh_failed",
                    format!(
                        "Credential '{}' exists but its OAuth refresh failed. \
                         The server returned HTTP {}. Re-authenticate this credential to repair the stored tokens.",
                        cred_name, status
                    ),
                ),
            };
            return Err(ToolError::ExecutionFailed(
                serde_json::json!({
                    "error": error_kind,
                    "credential_name": cred_name,
                    "message": message,
                })
                .to_string(),
            ));
        }

        // Strip sensitive response headers before they reach the LLM context.
        // These headers may contain tokens, session cookies, or auth challenges
        // that the LLM should never see (Pica pattern: auth header stripping).
        const REDACTED_RESPONSE_HEADERS: &[&str] = &[
            "authorization",
            "www-authenticate",
            "set-cookie",
            "x-api-key",
            "x-auth-token",
            "proxy-authenticate",
            "proxy-authorization",
        ];

        let headers: HashMap<String, String> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| {
                let key = k.to_string();
                if REDACTED_RESPONSE_HEADERS
                    .iter()
                    .any(|r| key.eq_ignore_ascii_case(r))
                {
                    None
                } else {
                    v.to_str().ok().map(|v| (key, v.to_string()))
                }
            })
            .collect();

        // Use a larger size limit when saving to disk (file downloads)
        let saving_to_disk = save_to.is_some();
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
        if let Some(save_to) = save_to {
            let saved_to = save_to.clone();
            let bytes_clone = body_bytes.clone();
            tokio::task::spawn_blocking(move || {
                let canonical = validate_save_to_path(&save_to)?;
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
                "saved_to": saved_to,
                "size_bytes": body_bytes.len(),
                "headers": headers,
            });
            return Ok(ToolOutput::success(result, start.elapsed()));
        }

        let body_text = String::from_utf8_lossy(&body_bytes).into_owned();

        // Scan response body for leaked credentials before it reaches the LLM.
        let response_detector = LeakDetector::new();
        let scan_result = response_detector.scan(&body_text);
        if scan_result.should_block {
            tracing::warn!(
                url = %parsed_url,
                matches = scan_result.matches.len(),
                "Response body contains leaked credential pattern, blocking"
            );
            return Err(ToolError::NotAuthorized(
                "Response blocked: contains credential patterns that must not reach the LLM"
                    .to_string(),
            ));
        }

        // Record the HTTP exchange if interceptor is present (recording mode)
        if let Some(ref interceptor) = ctx.http_interceptor {
            let resp_headers: Vec<(String, String)> = headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            interceptor
                .after_response(
                    &intercept_req,
                    &crate::llm::recording::HttpExchangeResponse {
                        status,
                        headers: resp_headers,
                        body: body_text.clone(),
                    },
                )
                .await;
        }

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
        let has_credentials = ironclaw_safety::params_contain_manual_credentials(params)
            || (self.credential_registry.as_ref().is_some_and(|registry| {
                extract_host_from_params(params)
                    .is_some_and(|host| registry.has_credentials_for_host(&host))
            }));

        if has_credentials {
            return ApprovalRequirement::UnlessAutoApproved;
        }

        // GET requests (or missing method, since GET is the default) are low-risk
        let method = params["method"].as_str().unwrap_or("GET");
        if method.eq_ignore_ascii_case("GET") {
            return ApprovalRequirement::Never;
        }

        ApprovalRequirement::UnlessAutoApproved
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(30, 500))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::credentials::{TEST_OPENAI_API_KEY, test_secrets_store};

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
        // Carrier-grade NAT
        assert!(is_disallowed_ip(&IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
        // Public
        assert!(!is_disallowed_ip(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn test_is_disallowed_ip_catches_ipv4_mapped_ipv6() {
        use std::net::Ipv6Addr;

        // ::ffff:127.0.0.1 (IPv4-mapped loopback)
        let mapped_loopback = IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x7f00, 0x0001));
        assert!(
            is_disallowed_ip(&mapped_loopback),
            "IPv4-mapped ::ffff:127.0.0.1 should be disallowed"
        );

        // ::ffff:169.254.169.254 (IPv4-mapped cloud metadata)
        let mapped_metadata = IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0xa9fe, 0xa9fe));
        assert!(
            is_disallowed_ip(&mapped_metadata),
            "IPv4-mapped ::ffff:169.254.169.254 should be disallowed"
        );

        // ::ffff:10.0.0.1 (IPv4-mapped private)
        let mapped_private = IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x0a00, 0x0001));
        assert!(
            is_disallowed_ip(&mapped_private),
            "IPv4-mapped ::ffff:10.0.0.1 should be disallowed"
        );

        // ::ffff:8.8.8.8 (IPv4-mapped public -- should be allowed)
        let mapped_public = IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x0808, 0x0808));
        assert!(
            !is_disallowed_ip(&mapped_public),
            "IPv4-mapped ::ffff:8.8.8.8 should be allowed"
        );
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
    fn test_parse_headers_param_accepts_stringified_array() {
        let headers =
            serde_json::json!("[{\"name\":\"Authorization\",\"value\":\"Bearer token\"}]");
        let parsed = parse_headers_param(Some(&headers)).unwrap();
        assert_eq!(
            parsed,
            vec![("Authorization".to_string(), "Bearer token".to_string())]
        );
    }

    #[test]
    fn test_parse_headers_param_rejects_double_string_encoding() {
        let headers = serde_json::json!("\"hello\"");
        let err = parse_headers_param(Some(&headers)).unwrap_err();
        assert!(
            err.to_string()
                .contains("headers string must decode to a JSON object or array"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn test_parse_timeout_secs_param_accepts_string_integer() {
        let timeout = serde_json::json!("30");
        assert_eq!(parse_timeout_secs_param(Some(&timeout)).unwrap(), Some(30));
    }

    #[test]
    fn test_parse_timeout_secs_param_treats_empty_string_as_none() {
        let timeout = serde_json::json!("");
        assert_eq!(parse_timeout_secs_param(Some(&timeout)).unwrap(), None);
    }

    #[test]
    fn test_parse_timeout_secs_param_rejects_value_above_cap() {
        let timeout = serde_json::json!(MAX_TIMEOUT_SECS + 1);
        let err = parse_timeout_secs_param(Some(&timeout)).unwrap_err();
        assert!(
            err.to_string()
                .contains(&format!("timeout_secs must be <= {}", MAX_TIMEOUT_SECS)),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn test_parse_timeout_secs_param_rejects_string_value_above_cap() {
        let timeout = serde_json::json!((MAX_TIMEOUT_SECS + 1).to_string());
        let err = parse_timeout_secs_param(Some(&timeout)).unwrap_err();
        assert!(
            err.to_string()
                .contains(&format!("timeout_secs must be <= {}", MAX_TIMEOUT_SECS)),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn test_parse_save_to_param_treats_empty_string_as_none() {
        let save_to = serde_json::json!("");
        assert_eq!(parse_save_to_param(Some(&save_to)).unwrap(), None);
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
    fn test_get_no_auth_headers_returns_never() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data"
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Never);
    }

    #[test]
    fn test_post_no_auth_headers_returns_unless_auto_approved() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "POST",
            "url": "https://api.example.com/data"
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_auth_header_object_format_returns_unless_auto_approved() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data",
            "headers": {"Authorization": "Bearer token123"}
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_auth_header_array_format_returns_unless_auto_approved() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data",
            "headers": [{"name": "Authorization", "value": "Bearer token123"}]
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
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
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );

        // Array format with mixed case
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": [{"name": "X-Api-Key", "value": "key123"}]
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
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
                ApprovalRequirement::UnlessAutoApproved,
                "Header '{}' should trigger UnlessAutoApproved approval",
                header_name
            );
        }
    }

    #[test]
    fn test_get_non_auth_headers_return_never() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"Content-Type": "application/json", "Accept": "text/html"}
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Never);
    }

    #[test]
    fn test_get_empty_headers_return_never() {
        let tool = HttpTool::new();

        // Empty object
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {}
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Never);

        // Empty array
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": []
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Never);
    }

    // ── Credential registry approval tests ─────────────────────────────

    #[test]
    fn test_host_with_credential_mapping_returns_unless_auto_approved() {
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
            Arc::new(test_secrets_store()),
        );

        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.openai.com/v1/models"
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_get_host_without_credential_mapping_returns_never() {
        use crate::tools::wasm::SharedCredentialRegistry;

        let registry = Arc::new(SharedCredentialRegistry::new());
        // Empty registry - no credential mappings

        let tool = HttpTool::new().with_credentials(registry, Arc::new(test_secrets_store()));

        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data"
        });
        assert_eq!(tool.requires_approval(&params), ApprovalRequirement::Never);
    }

    #[test]
    fn test_url_query_param_credential_returns_unless_auto_approved() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data?api_key=secret123"
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    #[test]
    fn test_bearer_value_in_custom_header_returns_unless_auto_approved() {
        let tool = HttpTool::new();
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"X-Custom": format!("Bearer {TEST_OPENAI_API_KEY}")}
        });
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved
        );
    }

    /// Regression test: credentialed HTTP requests must return
    /// `UnlessAutoApproved` (not `Always`) so that the session auto-approve
    /// set is respected when the user says "always".
    #[test]
    fn test_credentialed_requests_respect_auto_approve() {
        let tool = HttpTool::new();

        // Manual credentials (Authorization header)
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.github.com/orgs/Casa",
            "headers": {"Authorization": "Bearer ghp_abc123"}
        });
        // Must NOT be Always — Always ignores the session auto-approve set
        assert_ne!(
            tool.requires_approval(&params),
            ApprovalRequirement::Always,
            "Credentialed HTTP requests must not return Always; use UnlessAutoApproved"
        );
        assert_eq!(
            tool.requires_approval(&params),
            ApprovalRequirement::UnlessAutoApproved,
        );
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

    #[test]
    fn test_requires_approval_with_stringified_http_params() {
        use crate::tools::wasm::SharedCredentialRegistry;

        let tool = HttpTool::new().with_credentials(
            Arc::new(SharedCredentialRegistry::new()),
            Arc::new(test_secrets_store()),
        );
        let req = serde_json::json!({
            "body": "",
            "headers": "[]",
            "method": "GET",
            "save_to": "",
            "timeout_secs": "30",
            "url": "https://r.jina.ai/http://news.baidu.com/"
        });
        let _ = tool.requires_approval(&req);
    }

    // ── DNS pinning tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_validate_and_resolve_rejects_loopback_hostname() {
        // "localhost" is blocked at the URL validation level, but verify
        // that validate_and_resolve_url also catches loopback IPs returned
        // by DNS for any hostname that resolves to 127.0.0.1.
        let url = reqwest::Url::parse("https://127.0.0.1/test").unwrap();
        // 127.0.0.1 is an IP literal -- validate_url blocks it before
        // we ever reach validate_and_resolve_url, but the function should
        // still reject if called directly.
        let err = validate_and_resolve_url(&url).await.unwrap_err();
        assert!(
            err.to_string().contains("disallowed"),
            "expected disallowed IP error, got: {}",
            err
        );
    }

    // Requires network access -- run with: cargo test -- --ignored
    #[ignore]
    #[tokio::test]
    async fn test_validate_and_resolve_accepts_public_host() {
        // example.com resolves to public IPs.
        let url = reqwest::Url::parse("https://example.com").unwrap();
        let addrs = validate_and_resolve_url(&url).await.unwrap();
        assert!(!addrs.is_empty(), "should resolve to at least one address");
        for addr in &addrs {
            assert!(
                !is_disallowed_ip(&addr.ip()),
                "example.com resolved to disallowed IP: {}",
                addr.ip()
            );
        }
    }

    #[test]
    fn test_build_pinned_client_succeeds() {
        let addrs = vec![SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
            443,
        )];
        let client = build_pinned_client(
            "example.com",
            &addrs,
            Duration::from_secs(10),
            reqwest::redirect::Policy::none(),
        );
        assert!(client.is_ok(), "should build client successfully");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn requires_approval_multi_thread_no_panic() {
        use crate::secrets::CredentialMapping;
        use crate::tools::wasm::SharedCredentialRegistry;

        // Test with credential registry (uses std::sync::RwLock - should be safe)
        let registry = Arc::new(SharedCredentialRegistry::new());
        registry.add_mappings(vec![CredentialMapping::bearer("test_key", "api.test.com")]);

        let tool = HttpTool::new().with_credentials(registry, Arc::new(test_secrets_store()));

        // These calls should not panic in multi-thread runtime
        let params_no_auth = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data"
        });
        let _ = tool.requires_approval(&params_no_auth);

        let params_with_cred = serde_json::json!({
            "method": "GET",
            "url": "https://api.test.com/v1/models"
        });
        let _ = tool.requires_approval(&params_with_cred);

        let params_with_auth = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com",
            "headers": {"Authorization": "Bearer token"}
        });
        let _ = tool.requires_approval(&params_with_auth);
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
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_save_to_accepts_nested_tmp_path() {
        let path = validate_save_to_path("/tmp/ironclaw_test_subdir/nested/file.png").unwrap();
        assert!(path.starts_with("/tmp"));
        let _ = std::fs::remove_dir_all("/tmp/ironclaw_test_subdir");
    }

    #[test]
    fn test_save_to_rejects_bare_tmp() {
        let err = validate_save_to_path("/tmp").unwrap_err();
        assert!(err.to_string().contains("must be under /tmp/"));
    }

    // ── Forbidden auth header blocking tests ───────────────────────────

    #[test]
    fn test_forbidden_auth_header_blocked_for_registered_host() {
        // parse_headers_param is called before the execute() block check,
        // so we test the blocking logic directly by simulating what execute does.
        use crate::secrets::CredentialMapping;
        use crate::tools::wasm::SharedCredentialRegistry;

        let registry = Arc::new(SharedCredentialRegistry::new());
        registry.add_mappings(vec![CredentialMapping::bearer(
            "github_token",
            "api.github.com",
        )]);

        // Simulate: host has registered credentials, LLM provides Authorization header
        let cred_host = "api.github.com";
        assert!(registry.has_credentials_for_host(cred_host));

        let forbidden: &[&str] = &["authorization", "x-api-key", "api-key", "x-auth-token"];
        let llm_headers = [(
            "Authorization".to_string(),
            "Bearer stolen_token".to_string(),
        )];

        let blocked = llm_headers
            .iter()
            .any(|(name, _)| forbidden.iter().any(|f| name.eq_ignore_ascii_case(f)));
        assert!(
            blocked,
            "LLM-provided Authorization header should be blocked"
        );
    }

    #[test]
    fn test_non_auth_header_allowed_for_registered_host() {
        use crate::secrets::CredentialMapping;
        use crate::tools::wasm::SharedCredentialRegistry;

        let registry = Arc::new(SharedCredentialRegistry::new());
        registry.add_mappings(vec![CredentialMapping::bearer(
            "github_token",
            "api.github.com",
        )]);

        let forbidden: &[&str] = &["authorization", "x-api-key", "api-key", "x-auth-token"];
        let llm_headers = [
            ("Accept".to_string(), "application/json".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];

        let blocked = llm_headers
            .iter()
            .any(|(name, _)| forbidden.iter().any(|f| name.eq_ignore_ascii_case(f)));
        assert!(!blocked, "Non-auth headers should not be blocked");
    }

    #[test]
    fn test_auth_header_allowed_for_unregistered_host() {
        use crate::tools::wasm::SharedCredentialRegistry;

        // Empty registry — no credential mappings registered
        let registry = Arc::new(SharedCredentialRegistry::new());

        // Host has NO registered credentials, so LLM-provided auth headers are fine
        let cred_host = "api.example.com";
        assert!(!registry.has_credentials_for_host(cred_host));
    }

    // ── Caller-level recording hygiene ────────────────────────────────

    /// Spy interceptor that captures the request descriptor passed by
    /// `HttpTool` to `before_request` and returns a canned response so
    /// no real HTTP call is made.
    #[derive(Debug)]
    struct SpyInterceptor {
        captured: tokio::sync::Mutex<Option<crate::llm::recording::HttpExchangeRequest>>,
    }

    impl SpyInterceptor {
        fn new() -> Self {
            Self {
                captured: tokio::sync::Mutex::new(None),
            }
        }

        async fn captured_request(&self) -> Option<crate::llm::recording::HttpExchangeRequest> {
            self.captured.lock().await.clone()
        }
    }

    #[async_trait::async_trait]
    impl crate::llm::recording::HttpInterceptor for SpyInterceptor {
        async fn before_request(
            &self,
            request: &crate::llm::recording::HttpExchangeRequest,
        ) -> Option<crate::llm::recording::HttpExchangeResponse> {
            *self.captured.lock().await = Some(request.clone());
            Some(crate::llm::recording::HttpExchangeResponse {
                status: 200,
                headers: vec![],
                body: r#"{"ok":true}"#.to_string(),
            })
        }

        async fn after_response(
            &self,
            _request: &crate::llm::recording::HttpExchangeRequest,
            _response: &crate::llm::recording::HttpExchangeResponse,
        ) {
        }
    }

    /// Regression: the request descriptor passed to the HTTP interceptor
    /// must use `caller_headers` (pre-injection snapshot), NOT `headers_vec`
    /// which includes injected `Authorization` / API-key headers.
    #[tokio::test]
    async fn http_tool_interceptor_sees_caller_headers_not_injected() {
        use crate::secrets::CredentialMapping;
        use crate::tools::wasm::SharedCredentialRegistry;

        let registry = Arc::new(SharedCredentialRegistry::new());
        registry.add_mappings(vec![CredentialMapping::bearer(
            "github_token",
            "api.github.com",
        )]);

        // Store a secret so credential injection actually fires
        let store = Arc::new(test_secrets_store());
        store
            .create(
                "default",
                crate::secrets::CreateSecretParams::new(
                    "github_token",
                    "ghp_supersecretvalue1234567890",
                ),
            )
            .await
            .unwrap();

        let tool = HttpTool::new().with_credentials(registry, store);

        let spy = Arc::new(SpyInterceptor::new());
        let mut ctx = crate::context::JobContext::new("test", "test");
        ctx.http_interceptor = Some(spy.clone() as Arc<dyn crate::llm::recording::HttpInterceptor>);

        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.github.com/repos/test/test"
        });

        let result = tool.execute(params, &ctx).await;
        assert!(result.is_ok(), "tool should succeed via spy interceptor");

        let captured = spy.captured_request().await;
        assert!(captured.is_some(), "spy should have captured a request");

        let req = captured.unwrap();
        // The captured headers must NOT contain injected Authorization
        let has_auth = req
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization"));
        assert!(
            !has_auth,
            "interceptor request must not contain injected Authorization header; got: {:?}",
            req.headers
        );
        // The raw token must not appear anywhere in the serialized request
        let serialized = serde_json::to_string(&req).unwrap();
        assert!(
            !serialized.contains("ghp_supersecretvalue1234567890"),
            "raw token leaked into interceptor request: {serialized}"
        );
    }

    // ── Credential dedup regression ───────────────────────────────────

    /// Regression: duplicate `CredentialMapping` entries with the same
    /// `(secret_name, location)` must be deduped so only one
    /// `Authorization` header is injected. The original bug caused
    /// GitHub 401 "Bad credentials" when both a WASM tool's capabilities
    /// and a skill's `credentials` block declared the same secret.
    ///
    /// Calls the production `dedup_credential_mappings` function — if the
    /// function is removed, this test fails to compile.
    #[test]
    fn credential_mapping_dedup_removes_duplicates() {
        use crate::secrets::CredentialMapping;

        let mappings = vec![
            CredentialMapping::bearer("github_token", "api.github.com"),
            CredentialMapping::bearer("github_token", "api.github.com"),
            CredentialMapping::bearer("github_token", "*.github.com"), // same secret, same location type
        ];

        let dedup = super::dedup_credential_mappings(mappings);

        assert_eq!(
            dedup.len(),
            1,
            "duplicate (secret_name, location) pairs should be deduped to one entry; got {}",
            dedup.len()
        );
        assert_eq!(dedup[0].secret_name, "github_token");
    }

    /// Dedup must preserve entries with different locations for the same secret.
    #[test]
    fn credential_mapping_dedup_preserves_different_locations() {
        use crate::secrets::CredentialMapping;

        let mappings = vec![
            CredentialMapping::bearer("my_token", "api.example.com"),
            CredentialMapping::header("my_token", "X-Api-Key", "api.example.com"),
        ];

        let dedup = super::dedup_credential_mappings(mappings);

        assert_eq!(
            dedup.len(),
            2,
            "different locations for the same secret must be preserved; got {}",
            dedup.len()
        );
    }
}
