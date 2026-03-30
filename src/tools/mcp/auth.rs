//! OAuth 2.1 authentication for MCP servers.
//!
//! Implements the MCP Authorization specification using OAuth 2.1 with PKCE.
//! See: https://spec.modelcontextprotocol.io/specification/2025-03-26/basic/authorization/

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Weak};
use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

use crate::cli::oauth_defaults::{self, OAUTH_CALLBACK_PORT};
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::tools::mcp::config::McpServerConfig;

/// Shared HTTP client for all OAuth/discovery requests.
///
/// Redirects are disabled for security (prevents redirect-based SSRF).
/// Per-request timeouts can override the default via `.timeout()` on
/// the request builder.
fn oauth_http_client() -> Result<&'static reqwest::Client, AuthError> {
    static CLIENT: std::sync::OnceLock<Result<reqwest::Client, AuthError>> =
        std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| AuthError::Http(e.to_string()))
        })
        .as_ref()
        .map_err(Clone::clone)
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RefreshLockKey {
    server_name: String,
    user_id: String,
}

fn refresh_lock_key(server_name: &str, user_id: &str) -> RefreshLockKey {
    RefreshLockKey {
        server_name: server_name.to_string(),
        user_id: user_id.to_string(),
    }
}

async fn refresh_lock(server_name: &str, user_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        tokio::sync::Mutex<HashMap<RefreshLockKey, Weak<tokio::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();

    let registry = LOCKS.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()));
    let mut locks = registry.lock().await;
    locks.retain(|_, lock| lock.strong_count() > 0);

    let key = refresh_lock_key(server_name, user_id);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }

    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

/// Log a debug message when a discovery/auth response is a redirect.
/// Helps users diagnose configuration issues when legitimate servers
/// redirect and our no-redirect policy causes a failure.
fn log_redirect_if_applicable(url: &str, response: &reqwest::Response) {
    if response.status().is_redirection() {
        let location = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok());
        tracing::debug!(
            "OAuth request to '{}' returned redirect {} -> {:?} (redirects disabled for security)",
            url,
            response.status(),
            location
        );
    }
}

/// OAuth authorization error.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AuthError {
    #[error("Server does not support OAuth authorization")]
    NotSupported,

    #[error("Failed to discover authorization endpoints: {0}")]
    DiscoveryFailed(String),

    #[error("Authorization denied by user")]
    AuthorizationDenied,

    #[error("Token exchange failed: {0}")]
    TokenExchangeFailed(String),

    #[error("Token expired and refresh failed: {0}")]
    RefreshFailed(String),

    #[error("No access token available")]
    NoToken,

    #[error("Timeout waiting for authorization callback")]
    Timeout,

    #[error("Could not bind to callback port")]
    PortUnavailable,

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Secrets error: {0}")]
    Secrets(String),
}

/// OAuth protected resource metadata.
/// Discovered from /.well-known/oauth-protected-resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtectedResourceMetadata {
    /// The protected resource identifier.
    pub resource: String,

    /// Authorization servers that can issue tokens for this resource.
    #[serde(default)]
    pub authorization_servers: Vec<String>,

    /// Scopes supported by this resource.
    #[serde(default)]
    pub scopes_supported: Vec<String>,
}

/// OAuth authorization server metadata.
/// Discovered from /.well-known/oauth-authorization-server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationServerMetadata {
    /// Authorization server issuer.
    pub issuer: String,

    /// Authorization endpoint URL.
    pub authorization_endpoint: String,

    /// Token endpoint URL.
    pub token_endpoint: String,

    /// Dynamic client registration endpoint (if DCR is supported).
    #[serde(default)]
    pub registration_endpoint: Option<String>,

    /// Supported response types.
    #[serde(default)]
    pub response_types_supported: Vec<String>,

    /// Supported grant types.
    #[serde(default)]
    pub grant_types_supported: Vec<String>,

    /// Supported code challenge methods.
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,

    /// Scopes supported by this server.
    #[serde(default)]
    pub scopes_supported: Vec<String>,
}

/// Dynamic Client Registration request.
#[derive(Debug, Clone, Serialize)]
pub struct ClientRegistrationRequest {
    /// Human-readable client name.
    pub client_name: String,

    /// Redirect URIs for OAuth callbacks.
    pub redirect_uris: Vec<String>,

    /// Grant types the client will use.
    pub grant_types: Vec<String>,

    /// Response types the client will use.
    pub response_types: Vec<String>,

    /// Token endpoint authentication method.
    pub token_endpoint_auth_method: String,
}

/// Dynamic Client Registration response.
#[derive(Debug, Clone, Deserialize)]
pub struct ClientRegistrationResponse {
    /// The assigned client ID.
    pub client_id: String,

    /// Client secret (if issued).
    #[serde(default)]
    pub client_secret: Option<String>,

    /// When the client secret expires (if applicable).
    #[serde(default)]
    pub client_secret_expires_at: Option<u64>,

    /// Registration access token for managing the registration.
    #[serde(default)]
    pub registration_access_token: Option<String>,

    /// Registration client URI for managing the registration.
    #[serde(default)]
    pub registration_client_uri: Option<String>,
}

/// Access token with optional refresh token and expiry.
#[derive(Debug, Clone)]
pub struct AccessToken {
    /// The access token value.
    pub access_token: String,

    /// Token type (usually "Bearer").
    pub token_type: String,

    /// Seconds until expiration (if provided).
    pub expires_in: Option<u64>,

    /// Refresh token for obtaining new access tokens.
    pub refresh_token: Option<String>,

    /// Scopes granted.
    pub scope: Option<String>,
}

#[derive(Debug, Clone)]
struct ClientCredentials {
    client_id: String,
    client_secret: Option<String>,
}

/// Token response from the authorization server.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    scope: Option<String>,
}

/// PKCE verifier and challenge pair.
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    /// Code verifier (high-entropy random string).
    pub verifier: String,
    /// Code challenge (S256 hash of verifier).
    pub challenge: String,
}

impl PkceChallenge {
    /// Generate a new PKCE challenge pair.
    pub fn generate() -> Self {
        let mut verifier_bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut verifier_bytes);
        let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

        Self {
            verifier,
            challenge,
        }
    }
}

// ---------------------------------------------------------------------------
// Well-known URI construction (RFC 8414 / RFC 9728)
// ---------------------------------------------------------------------------

/// Build a well-known URI according to RFC 8414 / RFC 9728.
///
/// The path component of the base URL is placed *after* the well-known suffix:
/// ```text
/// https://example.com/path + oauth-authorization-server
///   -> https://example.com/.well-known/oauth-authorization-server/path
/// ```
pub fn build_well_known_uri(base_url: &str, suffix: &str) -> Result<String, AuthError> {
    let parsed = reqwest::Url::parse(base_url)
        .map_err(|e| AuthError::DiscoveryFailed(format!("Invalid URL: {}", e)))?;
    let origin = parsed.origin().ascii_serialization();
    let path = parsed.path().trim_end_matches('/');
    Ok(format!("{}/.well-known/{}{}", origin, suffix, path))
}

// ---------------------------------------------------------------------------
// RFC 8707 resource parameter
// ---------------------------------------------------------------------------

/// Compute the canonical resource URI for RFC 8707.
///
/// Strips fragments and trailing slashes from the server URL.
pub fn canonical_resource_uri(server_url: &str) -> String {
    match reqwest::Url::parse(server_url) {
        Ok(mut parsed) => {
            parsed.set_fragment(None);
            let s = parsed.to_string();
            s.trim_end_matches('/').to_string()
        }
        Err(_) => server_url.trim_end_matches('/').to_string(),
    }
}

// ---------------------------------------------------------------------------
// SSRF protection
// ---------------------------------------------------------------------------

/// Check if an IP address is dangerous (loopback, link-local, private, etc.)
fn is_dangerous_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || (v4.octets()[0] == 169 && v4.octets()[1] == 254) // link-local
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64) // CGNAT 100.64/10
        }
        IpAddr::V6(v6) => {
            let segs = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                // Link-local (fe80::/10)
                || (segs[0] & 0xffc0) == 0xfe80
                // Site-local / deprecated (fec0::/10)
                || (segs[0] & 0xffc0) == 0xfec0
                // Unique local (fc00::/7)
                || (segs[0] & 0xfe00) == 0xfc00
                // Documentation (2001:db8::/32)
                || (segs[0] == 0x2001 && segs[1] == 0x0db8)
                // Check for IPv4-mapped IPv6 (::ffff:x.x.x.x)
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_dangerous_ip(IpAddr::V4(v4)))
        }
    }
}

/// Validate that a URL is safe for server-side requests (SSRF protection).
async fn validate_url_safe(url: &str) -> Result<(), AuthError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| AuthError::DiscoveryFailed(format!("Invalid URL: {}", e)))?;

    // Must be HTTPS. HTTP is only allowed for localhost/loopback (dev scenarios).
    let scheme = parsed.scheme();
    if scheme != "https" && scheme != "http" {
        return Err(AuthError::DiscoveryFailed(format!(
            "Unsupported scheme: {}",
            scheme
        )));
    }
    if scheme == "http" {
        if !crate::tools::mcp::config::is_localhost_url(url) {
            let host = parsed.host_str().unwrap_or("");
            return Err(AuthError::DiscoveryFailed(format!(
                "HTTP is only allowed for localhost; use HTTPS for '{}'",
                host
            )));
        }
        // Localhost HTTP is allowed for dev — skip SSRF checks since we've
        // already validated the host is localhost/loopback.
        return Ok(());
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| AuthError::DiscoveryFailed("URL has no host".to_string()))?;

    // For IP literals, parse directly and check.
    if let Ok(ip) = host.parse::<IpAddr>()
        && is_dangerous_ip(ip)
    {
        return Err(AuthError::DiscoveryFailed(format!(
            "URL points to a restricted IP address: {}",
            host
        )));
    }

    // For hostnames, resolve DNS and check each resolved address.
    // This prevents DNS-based SSRF where a hostname resolves to an internal IP
    // (e.g., 169.254.169.254 for cloud metadata endpoints).
    if host.parse::<IpAddr>().is_err() {
        let addr = format!("{}:{}", host, parsed.port_or_known_default().unwrap_or(443));
        match tokio::net::lookup_host(&addr).await {
            Ok(addrs) => {
                for socket_addr in addrs {
                    if is_dangerous_ip(socket_addr.ip()) {
                        return Err(AuthError::DiscoveryFailed(format!(
                            "URL hostname '{}' resolves to restricted IP address: {}",
                            host,
                            socket_addr.ip()
                        )));
                    }
                }
            }
            Err(e) => {
                // DNS failure = fail closed (do not allow the request)
                return Err(AuthError::DiscoveryFailed(format!(
                    "DNS resolution failed for '{}': {}",
                    host, e
                )));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Multi-strategy OAuth discovery helpers
// ---------------------------------------------------------------------------

/// Parse the resource_metadata URL from a WWW-Authenticate header value.
fn parse_resource_metadata_url(www_authenticate: &str) -> Option<String> {
    // Try comma-separated parameters first
    for part in www_authenticate.split(',') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("resource_metadata=\"") {
            return rest.strip_suffix('"').map(|s| s.to_string());
        }
        if let Some(rest) = part.strip_prefix("resource_metadata=") {
            let val = rest.trim_matches('"');
            return Some(val.to_string());
        }
    }
    // Also try whitespace-separated tokens (e.g. Bearer resource_metadata="url")
    for part in www_authenticate.split_whitespace() {
        if let Some(rest) = part.strip_prefix("resource_metadata=\"") {
            return rest
                .trim_end_matches(',')
                .strip_suffix('"')
                .map(|s| s.to_string());
        }
        if let Some(rest) = part.strip_prefix("resource_metadata=") {
            let val = rest.trim_matches('"').trim_end_matches(',');
            return Some(val.to_string());
        }
    }
    None
}

/// Fetch protected resource metadata from a URL.
async fn fetch_resource_metadata(url: &str) -> Result<ProtectedResourceMetadata, AuthError> {
    validate_url_safe(url).await?;

    let client = oauth_http_client()?;

    let response = client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(e.to_string()))?;

    log_redirect_if_applicable(url, &response);

    if !response.status().is_success() {
        return Err(AuthError::DiscoveryFailed(format!(
            "HTTP {}",
            response.status()
        )));
    }

    response
        .json()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(format!("Invalid metadata: {}", e)))
}

/// Try to discover OAuth metadata via 401 challenge response.
///
/// Also accepts 400 responses, since some servers return 400 for
/// unauthenticated requests.  In practice the 400 path rarely yields a
/// `WWW-Authenticate` header (GitHub's MCP does not), so discovery
/// typically falls through to strategy 2 (RFC 9728) or 3 (direct).
async fn discover_via_401(server_url: &str) -> Result<AuthorizationServerMetadata, AuthError> {
    validate_url_safe(server_url).await?;

    let client = oauth_http_client()?;

    let response = client
        .post(server_url)
        .timeout(Duration::from_secs(10))
        .header("Content-Type", "application/json")
        .body("{}")
        .send()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(e.to_string()))?;

    log_redirect_if_applicable(server_url, &response);

    let status = response.status().as_u16();

    // Accept 401 (standard) and 400 (some servers like GitHub MCP use this).
    // In both cases, look for WWW-Authenticate header with discovery metadata.
    if status != 401 && status != 400 {
        return Err(AuthError::DiscoveryFailed(format!(
            "Expected 401 or 400, got {}",
            response.status()
        )));
    }

    let www_auth = response
        .headers()
        .get("WWW-Authenticate")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AuthError::DiscoveryFailed(format!("No WWW-Authenticate header in {} response", status))
        })?;

    let resource_metadata_url = parse_resource_metadata_url(www_auth).ok_or_else(|| {
        AuthError::DiscoveryFailed(
            "No resource_metadata URL in WWW-Authenticate header".to_string(),
        )
    })?;

    let resource_meta = fetch_resource_metadata(&resource_metadata_url).await?;
    try_discover_from_auth_servers(&resource_meta).await
}

/// Try to discover auth server metadata from resource metadata's authorization_servers list.
async fn try_discover_from_auth_servers(
    resource_meta: &ProtectedResourceMetadata,
) -> Result<AuthorizationServerMetadata, AuthError> {
    let auth_server_url = resource_meta
        .authorization_servers
        .first()
        .ok_or_else(|| AuthError::DiscoveryFailed("No authorization servers listed".to_string()))?;

    discover_authorization_server(auth_server_url).await
}

// ---------------------------------------------------------------------------
// Discovery functions
// ---------------------------------------------------------------------------

/// Discover protected resource metadata from an MCP server.
pub async fn discover_protected_resource(
    server_url: &str,
) -> Result<ProtectedResourceMetadata, AuthError> {
    validate_url_safe(server_url).await?;

    let client = oauth_http_client()?;

    let well_known_url = build_well_known_uri(server_url, "oauth-protected-resource")?;

    let response = client
        .get(&well_known_url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(e.to_string()))?;

    log_redirect_if_applicable(&well_known_url, &response);

    if !response.status().is_success() {
        return Err(AuthError::NotSupported);
    }

    response
        .json()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(format!("Invalid metadata: {}", e)))
}

/// Discover authorization server metadata.
pub async fn discover_authorization_server(
    auth_server_url: &str,
) -> Result<AuthorizationServerMetadata, AuthError> {
    validate_url_safe(auth_server_url).await?;

    let client = oauth_http_client()?;

    let well_known_url = build_well_known_uri(auth_server_url, "oauth-authorization-server")?;

    let response = client
        .get(&well_known_url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(e.to_string()))?;

    log_redirect_if_applicable(&well_known_url, &response);

    if !response.status().is_success() {
        return Err(AuthError::DiscoveryFailed(format!(
            "HTTP {}",
            response.status()
        )));
    }

    response
        .json()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(format!("Invalid metadata: {}", e)))
}

/// Discover OAuth endpoints for an MCP server.
///
/// First checks if endpoints are explicitly configured, then falls back to discovery.
pub async fn discover_oauth_endpoints(
    server_config: &McpServerConfig,
) -> Result<(String, String), AuthError> {
    let oauth = server_config
        .oauth
        .as_ref()
        .ok_or(AuthError::NotSupported)?;

    // If endpoints are explicitly configured, use them
    if let (Some(auth_url), Some(token_url)) = (&oauth.authorization_url, &oauth.token_url) {
        return Ok((auth_url.clone(), token_url.clone()));
    }

    // Try to discover from the server
    let resource_meta = discover_protected_resource(&server_config.url).await?;

    // Get the first authorization server
    let auth_server_url = resource_meta
        .authorization_servers
        .first()
        .ok_or_else(|| AuthError::DiscoveryFailed("No authorization servers listed".to_string()))?;

    // Discover the authorization server metadata
    let auth_meta = discover_authorization_server(auth_server_url).await?;

    Ok((auth_meta.authorization_endpoint, auth_meta.token_endpoint))
}

/// Discover full OAuth metadata including DCR support.
///
/// Returns authorization server metadata which includes registration_endpoint if DCR is supported.
/// Uses a 3-strategy discovery chain:
/// 1. **401-based**: POST to MCP server, parse WWW-Authenticate header for resource_metadata URL
/// 2. **RFC 9728**: Discover protected resource metadata, then authorization server from it
/// 3. **Direct**: Treat MCP server as its own auth server
pub async fn discover_full_oauth_metadata(
    server_url: &str,
) -> Result<AuthorizationServerMetadata, AuthError> {
    // Strategy 1: 401-based discovery
    if let Ok(meta) = discover_via_401(server_url).await {
        return Ok(meta);
    }

    // Strategy 2: RFC 9728 protected resource discovery
    if let Ok(resource_meta) = discover_protected_resource(server_url).await
        && let Ok(meta) = try_discover_from_auth_servers(&resource_meta).await
    {
        return Ok(meta);
    }

    // Strategy 3: Direct - treat MCP server as its own auth server
    discover_authorization_server(server_url).await
}

/// Perform Dynamic Client Registration with an authorization server.
///
/// This allows clients to register themselves at runtime without pre-configured credentials.
pub async fn register_client(
    registration_endpoint: &str,
    redirect_uri: &str,
) -> Result<ClientRegistrationResponse, AuthError> {
    validate_url_safe(registration_endpoint).await?;

    let client = oauth_http_client()?;

    let request = ClientRegistrationRequest {
        client_name: "IronClaw".to_string(),
        redirect_uris: vec![redirect_uri.to_string()],
        grant_types: vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ],
        response_types: vec!["code".to_string()],
        token_endpoint_auth_method: "none".to_string(), // Public client (no secret)
    };

    let response = client
        .post(registration_endpoint)
        .json(&request)
        .send()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(format!("DCR request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AuthError::DiscoveryFailed(format!(
            "DCR failed: HTTP {} - {}",
            status, body
        )));
    }

    response
        .json()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(format!("Invalid DCR response: {}", e)))
}

/// Perform the OAuth 2.1 authorization flow for an MCP server.
///
/// Supports two modes:
/// 1. Pre-configured OAuth: Uses the client_id from server config
/// 2. Dynamic Client Registration: Discovers and registers with the server automatically
///
/// Flow:
/// 1. Discovers authorization endpoints from the server
/// 2. If no client_id configured, attempts Dynamic Client Registration (DCR)
/// 3. Generates PKCE challenge
/// 4. Opens browser for user authorization
/// 5. Receives callback with authorization code
/// 6. Exchanges code for access token
/// 7. Stores token securely
pub async fn authorize_mcp_server(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
) -> Result<AccessToken, AuthError> {
    // Find an available port for the callback first (needed for DCR)
    let (listener, port) = find_available_port().await?;
    let host = oauth_defaults::callback_host();
    let redirect_uri = format!("http://{}:{}/callback", host, port);

    // Warn when the callback is served over plain HTTP to a remote host.
    // Authorization codes travel unencrypted; SSH port forwarding is safer:
    //   ssh -L <port>:127.0.0.1:<port> user@your-server
    if !oauth_defaults::is_loopback_host(&host) {
        println!("Warning: MCP OAuth callback is using plain HTTP to a remote host ({host}).");
        println!("         Authorization codes will be transmitted unencrypted.");
        println!("         Consider SSH port forwarding instead:");
        println!("           ssh -L {port}:127.0.0.1:{port} user@{host}");
    }

    // Determine client_id and endpoints
    let (
        client_id,
        client_secret,
        client_secret_expires_at,
        authorization_url,
        token_url,
        use_pkce,
        scopes,
        mut extra_params,
    ) = if let Some(oauth) = &server_config.oauth {
        // Pre-configured OAuth
        let (auth_url, tok_url) = discover_oauth_endpoints(server_config).await?;
        (
            oauth.client_id.clone(),
            None,
            None,
            auth_url,
            tok_url,
            oauth.use_pkce,
            oauth.scopes.clone(),
            oauth.extra_params.clone(),
        )
    } else {
        // Try Dynamic Client Registration
        println!("  Discovering OAuth endpoints...");
        let auth_meta = discover_full_oauth_metadata(&server_config.url).await?;

        let registration_endpoint = auth_meta
            .registration_endpoint
            .ok_or(AuthError::NotSupported)?;

        println!("  Registering client dynamically...");
        let registration = register_client(&registration_endpoint, &redirect_uri).await?;
        println!("  Client registered: {}", registration.client_id);

        (
            registration.client_id,
            registration.client_secret,
            registration.client_secret_expires_at,
            auth_meta.authorization_endpoint,
            auth_meta.token_endpoint,
            true, // Always use PKCE for DCR clients
            auth_meta.scopes_supported,
            HashMap::new(),
        )
    };

    // Generate PKCE challenge
    let pkce = if use_pkce {
        Some(PkceChallenge::generate())
    } else {
        None
    };

    // Generate OAuth state parameter. While optional in OAuth 2.1 with PKCE,
    // some MCP servers (e.g. Attio) require it.
    let mut state_bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut state_bytes);
    let state = URL_SAFE_NO_PAD.encode(state_bytes);
    extra_params.insert("state".to_string(), state);

    // Compute canonical resource URI for RFC 8707
    let resource = canonical_resource_uri(&server_config.url);

    // Validate the discovered authorization URL to prevent a malicious MCP server
    // from redirecting the user to a phishing page or non-HTTPS endpoint.
    validate_url_safe(&authorization_url)
        .await
        .map_err(|e| AuthError::DiscoveryFailed(format!("Unsafe authorization endpoint: {}", e)))?;

    // Build authorization URL
    let auth_url = build_authorization_url(
        &authorization_url,
        &client_id,
        &redirect_uri,
        &scopes,
        pkce.as_ref(),
        &extra_params,
        Some(&resource),
    );

    // Open browser
    println!("  Opening browser for {} login...", server_config.name);
    if let Err(e) = open::that(&auth_url) {
        println!("  Could not open browser: {}", e);
        println!("  Please open this URL manually:");
        println!("  {}", auth_url);
    }

    println!("  Waiting for authorization...");

    // Wait for callback. State is sent in the URL for servers that require it
    // (e.g. Attio), but we don't enforce validation on the callback because MCP
    // servers use PKCE which already binds the request to the token exchange,
    // and some servers may not echo state back.
    let code = wait_for_authorization_callback(listener, &server_config.name).await?;

    println!("  Exchanging code for token...");

    // Exchange code for token
    let token = exchange_code_for_token(
        &token_url,
        &client_id,
        client_secret.as_deref(),
        &code,
        &redirect_uri,
        pkce.as_ref(),
        Some(&resource),
    )
    .await?;

    // Store the tokens
    store_tokens(secrets, user_id, server_config, &token).await?;

    // Store the client_id for DCR (needed for token refresh)
    if server_config.oauth.is_none() {
        store_client_id(secrets, user_id, server_config, &client_id).await?;
        if let Some(ref client_secret) = client_secret {
            store_client_secret(
                secrets,
                user_id,
                server_config,
                client_secret,
                client_secret_expires_at,
            )
            .await?;
        }
    }

    Ok(token)
}

/// Bind the OAuth callback listener on the shared fixed port.
pub async fn find_available_port() -> Result<(TcpListener, u16), AuthError> {
    let listener = oauth_defaults::bind_callback_listener()
        .await
        .map_err(|_| AuthError::PortUnavailable)?;
    Ok((listener, OAUTH_CALLBACK_PORT))
}

/// Build the authorization URL with all required parameters.
pub fn build_authorization_url(
    base_url: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    pkce: Option<&PkceChallenge>,
    extra_params: &HashMap<String, String>,
    resource: Option<&str>,
) -> String {
    let mut url = format!(
        "{}?client_id={}&response_type=code&redirect_uri={}",
        base_url,
        urlencoding::encode(client_id),
        urlencoding::encode(redirect_uri)
    );

    if !scopes.is_empty() {
        url.push_str(&format!(
            "&scope={}",
            urlencoding::encode(&scopes.join(" "))
        ));
    }

    if let Some(pkce) = pkce {
        url.push_str(&format!(
            "&code_challenge={}&code_challenge_method=S256",
            urlencoding::encode(&pkce.challenge)
        ));
    }

    for (key, value) in extra_params {
        url.push_str(&format!(
            "&{}={}",
            urlencoding::encode(key),
            urlencoding::encode(value)
        ));
    }

    if let Some(resource) = resource {
        url.push_str(&format!("&resource={}", urlencoding::encode(resource)));
    }

    url
}

/// Wait for the authorization callback and extract the code.
pub async fn wait_for_authorization_callback(
    listener: TcpListener,
    server_name: &str,
) -> Result<String, AuthError> {
    oauth_defaults::wait_for_callback(listener, "/callback", "code", server_name, None)
        .await
        .map_err(|e| match e {
            oauth_defaults::OAuthCallbackError::Denied => AuthError::AuthorizationDenied,
            oauth_defaults::OAuthCallbackError::Timeout => AuthError::Timeout,
            oauth_defaults::OAuthCallbackError::PortInUse(_, msg) => {
                AuthError::Http(format!("Port error: {}", msg))
            }
            oauth_defaults::OAuthCallbackError::StateMismatch { .. } => {
                AuthError::Http("CSRF state mismatch in OAuth callback".to_string())
            }
            oauth_defaults::OAuthCallbackError::Io(msg) => AuthError::Http(msg),
        })
}

/// Exchange the authorization code for an access token.
pub async fn exchange_code_for_token(
    token_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    redirect_uri: &str,
    pkce: Option<&PkceChallenge>,
    resource: Option<&str>,
) -> Result<AccessToken, AuthError> {
    validate_url_safe(token_url).await?;

    let client = oauth_http_client()?;

    let mut params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("client_id", client_id.to_string()),
    ];

    if let Some(secret) = client_secret {
        params.push(("client_secret", secret.to_string()));
    }

    if let Some(pkce) = pkce {
        params.push(("code_verifier", pkce.verifier.clone()));
    }

    if let Some(resource) = resource {
        params.push(("resource", resource.to_string()));
    }

    let response = client
        .post(token_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| AuthError::TokenExchangeFailed(e.to_string()))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AuthError::TokenExchangeFailed(format!(
            "HTTP {} - {}",
            status, body
        )));
    }

    let token_response: TokenResponse = response
        .json()
        .await
        .map_err(|e| AuthError::TokenExchangeFailed(format!("Invalid response: {}", e)))?;

    Ok(AccessToken {
        access_token: token_response.access_token,
        token_type: token_response.token_type,
        expires_in: token_response.expires_in,
        refresh_token: token_response.refresh_token,
        scope: token_response.scope,
    })
}

/// Store access and refresh tokens securely.
pub async fn store_tokens(
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
    server_config: &McpServerConfig,
    token: &AccessToken,
) -> Result<(), AuthError> {
    // Store access token (with expiry if provided)
    let mut params =
        CreateSecretParams::new(server_config.token_secret_name(), &token.access_token)
            .with_provider(format!("mcp:{}", server_config.name));

    if let Some(secs) = token.expires_in {
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(secs as i64);
        params = params.with_expiry(expires_at);
    }

    secrets
        .create(user_id, params)
        .await
        .map_err(|e| AuthError::Secrets(e.to_string()))?;

    // Store refresh token if present (no expiry — long-lived)
    if let Some(ref refresh_token) = token.refresh_token {
        let params =
            CreateSecretParams::new(server_config.refresh_token_secret_name(), refresh_token)
                .with_provider(format!("mcp:{}", server_config.name));

        secrets
            .create(user_id, params)
            .await
            .map_err(|e| AuthError::Secrets(e.to_string()))?;
    }

    Ok(())
}

/// Store the DCR client ID for future token refresh.
pub async fn store_client_id(
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
    server_config: &McpServerConfig,
    client_id: &str,
) -> Result<(), AuthError> {
    let params = CreateSecretParams::new(server_config.client_id_secret_name(), client_id)
        .with_provider(format!("mcp:{}", server_config.name));

    secrets
        .create(user_id, params)
        .await
        .map(|_| ())
        .map_err(|e| AuthError::Secrets(e.to_string()))
}

/// Store the DCR client secret for future token refresh.
pub async fn store_client_secret(
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
    server_config: &McpServerConfig,
    client_secret: &str,
    client_secret_expires_at: Option<u64>,
) -> Result<(), AuthError> {
    let mut params =
        CreateSecretParams::new(server_config.client_secret_secret_name(), client_secret)
            .with_provider(format!("mcp:{}", server_config.name));

    if let Some(expires_at) = client_secret_expires_at
        && let Some(dt) = chrono::DateTime::<chrono::Utc>::from_timestamp(expires_at as i64, 0)
    {
        params = params.with_expiry(dt);
    }

    secrets
        .create(user_id, params)
        .await
        .map(|_| ())
        .map_err(|e| AuthError::Secrets(e.to_string()))
}

/// Get the client ID for a server (from config or stored DCR).
async fn get_client_id(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
) -> Result<String, AuthError> {
    // First check if OAuth is configured with a client_id
    if let Some(ref oauth) = server_config.oauth {
        return Ok(oauth.client_id.clone());
    }

    // Otherwise try to get the DCR client_id from secrets
    match secrets
        .get_decrypted(user_id, &server_config.client_id_secret_name())
        .await
    {
        Ok(client_id) => Ok(client_id.expose().to_string()),
        Err(crate::secrets::SecretError::NotFound(_)) => Err(AuthError::RefreshFailed(
            "No client ID found. Please re-authenticate.".to_string(),
        )),
        Err(e) => Err(AuthError::Secrets(e.to_string())),
    }
}

/// Get client credentials for a server (from config or stored DCR registration).
async fn get_client_credentials(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
) -> Result<ClientCredentials, AuthError> {
    let client_id = get_client_id(server_config, secrets, user_id).await?;
    let client_secret = match secrets
        .get_decrypted(user_id, &server_config.client_secret_secret_name())
        .await
    {
        Ok(secret) => Some(secret.expose().to_string()),
        Err(crate::secrets::SecretError::NotFound(_) | crate::secrets::SecretError::Expired) => {
            None
        }
        Err(e) => return Err(AuthError::Secrets(e.to_string())),
    };

    Ok(ClientCredentials {
        client_id,
        client_secret,
    })
}

/// Get the stored access token for an MCP server.
pub async fn get_access_token(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
) -> Result<Option<String>, AuthError> {
    match secrets
        .get_decrypted(user_id, &server_config.token_secret_name())
        .await
    {
        Ok(token) => Ok(Some(token.expose().to_string())),
        Err(crate::secrets::SecretError::NotFound(_)) => Ok(None),
        Err(e) => Err(AuthError::Secrets(e.to_string())),
    }
}

/// Check if a server has valid authentication.
///
/// Returns true if:
/// - A valid access token is stored (regardless of how it was obtained)
/// - The server doesn't require authentication at all
pub async fn is_authenticated(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
) -> bool {
    // Check if we have a stored token (from either pre-configured OAuth or DCR)
    secrets
        .exists(user_id, &server_config.token_secret_name())
        .await
        .unwrap_or(false)
}

/// Refresh an access token using the refresh token.
///
/// Works with both pre-configured OAuth and Dynamic Client Registration (DCR).
/// For DCR, retrieves the client_id from stored secrets.
pub async fn refresh_access_token(
    server_config: &McpServerConfig,
    secrets: &Arc<dyn SecretsStore + Send + Sync>,
    user_id: &str,
) -> Result<AccessToken, AuthError> {
    let lock = refresh_lock(&server_config.name, user_id).await;
    let _guard = lock.lock().await;

    match secrets
        .get_decrypted(user_id, &server_config.token_secret_name())
        .await
    {
        Ok(token) => {
            return Ok(AccessToken {
                access_token: token.expose().to_string(),
                token_type: "Bearer".to_string(),
                expires_in: None,
                refresh_token: None,
                scope: None,
            });
        }
        Err(crate::secrets::SecretError::Expired | crate::secrets::SecretError::NotFound(_)) => {}
        Err(e) => return Err(AuthError::Secrets(e.to_string())),
    }

    // Get client_id (from config or stored DCR)
    let credentials = get_client_credentials(server_config, secrets, user_id).await?;

    // Get the refresh token (try current name, fall back to legacy name for
    // users who authenticated before the naming convention was fixed).
    // Only fall back on NotFound/Expired — propagate real errors (DB, decryption).
    let refresh_token = match secrets
        .get_decrypted(user_id, &server_config.refresh_token_secret_name())
        .await
    {
        Ok(token) => token,
        Err(crate::secrets::SecretError::NotFound(_) | crate::secrets::SecretError::Expired) => {
            secrets
                .get_decrypted(user_id, &server_config.legacy_refresh_token_secret_name())
                .await
                .map_err(|e| AuthError::RefreshFailed(format!("No refresh token: {}", e)))?
        }
        Err(e) => {
            return Err(AuthError::RefreshFailed(format!(
                "Failed to read refresh token: {e}"
            )));
        }
    };

    // Discover the token endpoint
    let token_url = if let Some(ref oauth) = server_config.oauth {
        if let Some(ref url) = oauth.token_url {
            url.clone()
        } else {
            // Discover from server
            let auth_meta = discover_full_oauth_metadata(&server_config.url).await?;
            auth_meta.token_endpoint
        }
    } else {
        // DCR - always discover
        let auth_meta = discover_full_oauth_metadata(&server_config.url).await?;
        auth_meta.token_endpoint
    };

    validate_url_safe(&token_url).await?;

    let token = if let Some(proxy_url) = oauth_defaults::exchange_proxy_url() {
        let resource = canonical_resource_uri(&server_config.url);
        let gateway_token = oauth_defaults::oauth_proxy_auth_token().ok_or_else(|| {
            AuthError::RefreshFailed(
                "OAuth refresh proxy is configured but no proxy auth token is available"
                    .to_string(),
            )
        })?;
        let token_response =
            oauth_defaults::refresh_token_via_proxy(oauth_defaults::ProxyRefreshTokenRequest {
                proxy_url: &proxy_url,
                gateway_token: &gateway_token,
                token_url: &token_url,
                client_id: &credentials.client_id,
                client_secret: credentials.client_secret.as_deref(),
                refresh_token: refresh_token.expose(),
                resource: Some(&resource),
                provider: Some(&format!("mcp:{}", server_config.name)),
            })
            .await
            .map_err(|e| AuthError::RefreshFailed(e.to_string()))?;

        AccessToken {
            access_token: token_response.access_token,
            token_type: token_response
                .token_type
                .unwrap_or_else(|| "Bearer".to_string()),
            expires_in: token_response.expires_in,
            refresh_token: token_response.refresh_token,
            scope: token_response.scope,
        }
    } else {
        let client = oauth_http_client()?;

        // Compute canonical resource URI for RFC 8707
        let resource = canonical_resource_uri(&server_config.url);

        let mut params = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.expose().to_string()),
            ("client_id", credentials.client_id.clone()),
            ("resource", resource),
        ];
        if let Some(client_secret) = credentials.client_secret.as_deref() {
            params.push(("client_secret", client_secret.to_string()));
        }

        let response = client
            .post(&token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| AuthError::RefreshFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AuthError::RefreshFailed(format!(
                "HTTP {} - {}",
                status, body
            )));
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .map_err(|e| AuthError::RefreshFailed(format!("Invalid response: {}", e)))?;

        AccessToken {
            access_token: token_response.access_token,
            token_type: token_response.token_type,
            expires_in: token_response.expires_in,
            refresh_token: token_response.refresh_token,
            scope: token_response.scope,
        }
    };

    // Store the new tokens
    store_tokens(secrets, user_id, server_config, &token).await?;

    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use axum::{
        Router,
        extract::{Form, State},
        routing::post,
    };
    use secrecy::SecretString;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    use crate::config::helpers::lock_env;
    use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
    use crate::testing::credentials::TEST_GATEWAY_CRYPTO_KEY;

    #[derive(Clone, Debug, Default)]
    struct RecordedRefreshRequest {
        authorization: Option<String>,
        form: HashMap<String, String>,
    }

    #[derive(Clone, Default)]
    struct MockRefreshState {
        requests: Arc<Mutex<Vec<RecordedRefreshRequest>>>,
    }

    impl MockRefreshState {
        async fn requests(&self) -> Vec<RecordedRefreshRequest> {
            self.requests.lock().await.clone()
        }
    }

    fn test_secrets_store() -> Arc<dyn SecretsStore + Send + Sync> {
        Arc::new(InMemorySecretsStore::new(Arc::new(
            SecretsCrypto::new(SecretString::from(TEST_GATEWAY_CRYPTO_KEY.to_string()))
                .expect("test crypto"),
        )))
    }

    async fn start_refresh_server() -> Option<(String, MockRefreshState)> {
        async fn token_handler(
            State(state): State<MockRefreshState>,
            headers: axum::http::HeaderMap,
            Form(form): Form<HashMap<String, String>>,
        ) -> axum::Json<serde_json::Value> {
            state.requests.lock().await.push(RecordedRefreshRequest {
                authorization: headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string),
                form,
            });
            axum::Json(serde_json::json!({
                "access_token": "refreshed-access-token",
                "token_type": "Bearer",
                "refresh_token": "rotated-refresh-token",
                "expires_in": 3600
            }))
        }

        let state = MockRefreshState::default();
        let app = Router::new()
            .route("/token", post(token_handler))
            .route("/oauth/refresh", post(token_handler))
            .with_state(state.clone());
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                eprintln!("Skipping refresh server test: loopback bind denied by sandbox");
                return None;
            }
            Err(error) => panic!("failed to bind refresh test server: {error}"),
        };
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Some((format!("http://127.0.0.1:{}", addr.port()), state))
    }

    #[test]
    fn test_pkce_challenge_generation() {
        let pkce = PkceChallenge::generate();

        // Verifier should be base64url encoded
        assert!(!pkce.verifier.is_empty());
        assert!(!pkce.verifier.contains('+'));
        assert!(!pkce.verifier.contains('/'));
        assert!(!pkce.verifier.contains('='));

        // Challenge should be different from verifier
        assert_ne!(pkce.verifier, pkce.challenge);

        // Two challenges should be different
        let pkce2 = PkceChallenge::generate();
        assert_ne!(pkce.verifier, pkce2.verifier);
    }

    #[test]
    fn test_build_authorization_url() {
        let url = build_authorization_url(
            "https://auth.example.com/authorize",
            "client-123",
            "http://localhost:9876/callback",
            &["read".to_string(), "write".to_string()],
            None,
            &HashMap::new(),
            None,
        );

        assert!(url.starts_with("https://auth.example.com/authorize?"));
        assert!(url.contains("client_id=client-123"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("scope=read%20write"));
    }

    #[test]
    fn test_build_authorization_url_with_pkce() {
        let pkce = PkceChallenge::generate();
        let url = build_authorization_url(
            "https://auth.example.com/authorize",
            "client-123",
            "http://localhost:9876/callback",
            &[],
            Some(&pkce),
            &HashMap::new(),
            None,
        );

        assert!(url.contains(&format!("code_challenge={}", pkce.challenge)));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn test_build_authorization_url_with_extra_params() {
        let mut extra = HashMap::new();
        extra.insert("owner".to_string(), "user".to_string());
        extra.insert("state".to_string(), "abc123".to_string());

        let url = build_authorization_url(
            "https://auth.example.com/authorize",
            "client-123",
            "http://localhost:9876/callback",
            &[],
            None,
            &extra,
            None,
        );

        assert!(url.contains("owner=user"));
        assert!(url.contains("state=abc123"));
    }

    #[test]
    fn test_pkce_challenge_s256_is_correct_sha256() {
        let pkce = PkceChallenge::generate();

        // Recompute the S256 challenge from scratch and compare.
        let mut hasher = Sha256::new();
        hasher.update(pkce.verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(hasher.finalize());

        assert_eq!(pkce.challenge, expected);
    }

    #[test]
    fn test_build_authorization_url_empty_scopes_no_scope_param() {
        let url = build_authorization_url(
            "https://auth.example.com/authorize",
            "client-123",
            "http://localhost:9876/callback",
            &[],
            None,
            &HashMap::new(),
            None,
        );

        // With no scopes, the URL must not contain a scope parameter at all.
        assert!(!url.contains("scope="));
    }

    #[test]
    fn test_build_authorization_url_special_characters_are_encoded() {
        let url = build_authorization_url(
            "https://auth.example.com/authorize",
            "client id&evil=true",
            "http://localhost:9876/call back?x=1",
            &[],
            None,
            &HashMap::new(),
            None,
        );

        // Spaces and ampersands in client_id must be percent-encoded.
        assert!(url.contains("client_id=client%20id%26evil%3Dtrue"));
        // Spaces and question marks in redirect_uri must be percent-encoded.
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A9876%2Fcall%20back%3Fx%3D1"));
    }

    #[test]
    fn test_protected_resource_metadata_serde_roundtrip_full() {
        let meta = ProtectedResourceMetadata {
            resource: "https://mcp.example.com".to_string(),
            authorization_servers: vec![
                "https://auth1.example.com".to_string(),
                "https://auth2.example.com".to_string(),
            ],
            scopes_supported: vec!["read".to_string(), "write".to_string()],
        };

        let json = serde_json::to_string(&meta).unwrap();
        let deserialized: ProtectedResourceMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.resource, meta.resource);
        assert_eq!(
            deserialized.authorization_servers,
            meta.authorization_servers
        );
        assert_eq!(deserialized.scopes_supported, meta.scopes_supported);
    }

    #[test]
    fn test_protected_resource_metadata_serde_roundtrip_minimal() {
        // Only required field, optional vecs should default to empty.
        let json = r#"{"resource": "https://mcp.example.com"}"#;
        let meta: ProtectedResourceMetadata = serde_json::from_str(json).unwrap();

        assert_eq!(meta.resource, "https://mcp.example.com");
        assert!(meta.authorization_servers.is_empty());
        assert!(meta.scopes_supported.is_empty());
    }

    #[test]
    fn test_authorization_server_metadata_serde_roundtrip_all_fields() {
        let meta = AuthorizationServerMetadata {
            issuer: "https://auth.example.com".to_string(),
            authorization_endpoint: "https://auth.example.com/authorize".to_string(),
            token_endpoint: "https://auth.example.com/token".to_string(),
            registration_endpoint: Some("https://auth.example.com/register".to_string()),
            response_types_supported: vec!["code".to_string()],
            grant_types_supported: vec![
                "authorization_code".to_string(),
                "refresh_token".to_string(),
            ],
            code_challenge_methods_supported: vec!["S256".to_string()],
            scopes_supported: vec!["openid".to_string(), "profile".to_string()],
        };

        let json = serde_json::to_string(&meta).unwrap();
        let rt: AuthorizationServerMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(rt.issuer, meta.issuer);
        assert_eq!(rt.authorization_endpoint, meta.authorization_endpoint);
        assert_eq!(rt.token_endpoint, meta.token_endpoint);
        assert_eq!(rt.registration_endpoint, meta.registration_endpoint);
        assert_eq!(rt.response_types_supported, meta.response_types_supported);
        assert_eq!(rt.grant_types_supported, meta.grant_types_supported);
        assert_eq!(
            rt.code_challenge_methods_supported,
            meta.code_challenge_methods_supported
        );
        assert_eq!(rt.scopes_supported, meta.scopes_supported);
    }

    #[test]
    fn test_authorization_server_metadata_serde_without_registration() {
        let json = r#"{
            "issuer": "https://auth.example.com",
            "authorization_endpoint": "https://auth.example.com/authorize",
            "token_endpoint": "https://auth.example.com/token"
        }"#;

        let meta: AuthorizationServerMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(meta.issuer, "https://auth.example.com");
        assert!(meta.registration_endpoint.is_none());
        assert!(meta.response_types_supported.is_empty());
        assert!(meta.grant_types_supported.is_empty());
    }

    #[test]
    fn test_client_registration_request_serialization() {
        let req = ClientRegistrationRequest {
            client_name: "IronClaw".to_string(),
            redirect_uris: vec!["http://localhost:9876/callback".to_string()],
            grant_types: vec![
                "authorization_code".to_string(),
                "refresh_token".to_string(),
            ],
            response_types: vec!["code".to_string()],
            token_endpoint_auth_method: "none".to_string(),
        };

        let value: serde_json::Value = serde_json::to_value(&req).unwrap();

        assert_eq!(value["client_name"], "IronClaw");
        assert_eq!(value["redirect_uris"][0], "http://localhost:9876/callback");
        assert_eq!(value["grant_types"][0], "authorization_code");
        assert_eq!(value["grant_types"][1], "refresh_token");
        assert_eq!(value["response_types"][0], "code");
        assert_eq!(value["token_endpoint_auth_method"], "none");
    }

    #[test]
    fn test_client_registration_response_deserialization_full() {
        let json = r#"{
            "client_id": "abc-123",
            "client_secret": "s3cret",
            "client_secret_expires_at": 1700000000,
            "registration_access_token": "reg-tok",
            "registration_client_uri": "https://auth.example.com/register/abc-123"
        }"#;

        let resp: ClientRegistrationResponse = serde_json::from_str(json).unwrap();

        assert_eq!(resp.client_id, "abc-123");
        assert_eq!(resp.client_secret.as_deref(), Some("s3cret"));
        assert_eq!(resp.client_secret_expires_at, Some(1700000000));
        assert_eq!(resp.registration_access_token.as_deref(), Some("reg-tok"));
        assert_eq!(
            resp.registration_client_uri.as_deref(),
            Some("https://auth.example.com/register/abc-123")
        );
    }

    #[test]
    fn test_client_registration_response_deserialization_minimal() {
        let json = r#"{"client_id": "xyz-789"}"#;

        let resp: ClientRegistrationResponse = serde_json::from_str(json).unwrap();

        assert_eq!(resp.client_id, "xyz-789");
        assert!(resp.client_secret.is_none());
        assert!(resp.client_secret_expires_at.is_none());
        assert!(resp.registration_access_token.is_none());
        assert!(resp.registration_client_uri.is_none());
    }

    #[test]
    fn test_access_token_construction() {
        let token = AccessToken {
            access_token: "at-abc".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: Some(3600),
            refresh_token: Some("rt-xyz".to_string()),
            scope: Some("read write".to_string()),
        };

        assert_eq!(token.access_token, "at-abc");
        assert_eq!(token.token_type, "Bearer");
        assert_eq!(token.expires_in, Some(3600));
        assert_eq!(token.refresh_token.as_deref(), Some("rt-xyz"));
        assert_eq!(token.scope.as_deref(), Some("read write"));

        // Also test with no optional fields.
        let minimal = AccessToken {
            access_token: "tok".to_string(),
            token_type: "bearer".to_string(),
            expires_in: None,
            refresh_token: None,
            scope: None,
        };
        assert!(minimal.expires_in.is_none());
        assert!(minimal.refresh_token.is_none());
        assert!(minimal.scope.is_none());
    }

    #[test]
    fn test_token_response_to_access_token_pattern() {
        // TokenResponse is private, but we can test the conversion pattern
        // by deserializing JSON the same way exchange_code_for_token does.
        let json = r#"{
            "access_token": "eyJ-token",
            "token_type": "Bearer",
            "expires_in": 7200,
            "refresh_token": "refresh-me",
            "scope": "openid profile"
        }"#;

        // Deserialize via the same struct path the production code uses.
        let resp: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = AccessToken {
            access_token: resp["access_token"].as_str().unwrap().to_string(),
            token_type: resp["token_type"].as_str().unwrap().to_string(),
            expires_in: resp["expires_in"].as_u64(),
            refresh_token: resp["refresh_token"].as_str().map(String::from),
            scope: resp["scope"].as_str().map(String::from),
        };

        assert_eq!(token.access_token, "eyJ-token");
        assert_eq!(token.token_type, "Bearer");
        assert_eq!(token.expires_in, Some(7200));
        assert_eq!(token.refresh_token.as_deref(), Some("refresh-me"));
        assert_eq!(token.scope.as_deref(), Some("openid profile"));

        // Without optional fields.
        let minimal_json = r#"{"access_token": "tok", "token_type": "bearer"}"#;
        let resp: serde_json::Value = serde_json::from_str(minimal_json).unwrap();
        let token = AccessToken {
            access_token: resp["access_token"].as_str().unwrap().to_string(),
            token_type: resp["token_type"].as_str().unwrap().to_string(),
            expires_in: resp["expires_in"].as_u64(),
            refresh_token: resp["refresh_token"].as_str().map(String::from),
            scope: resp["scope"].as_str().map(String::from),
        };
        assert!(token.expires_in.is_none());
        assert!(token.refresh_token.is_none());
        assert!(token.scope.is_none());
    }

    #[test]
    fn test_auth_error_display_strings() {
        let cases: Vec<(AuthError, &str)> = vec![
            (
                AuthError::NotSupported,
                "Server does not support OAuth authorization",
            ),
            (
                AuthError::DiscoveryFailed("timeout".to_string()),
                "Failed to discover authorization endpoints: timeout",
            ),
            (
                AuthError::AuthorizationDenied,
                "Authorization denied by user",
            ),
            (
                AuthError::TokenExchangeFailed("bad code".to_string()),
                "Token exchange failed: bad code",
            ),
            (
                AuthError::RefreshFailed("expired".to_string()),
                "Token expired and refresh failed: expired",
            ),
            (AuthError::NoToken, "No access token available"),
            (
                AuthError::Timeout,
                "Timeout waiting for authorization callback",
            ),
            (
                AuthError::PortUnavailable,
                "Could not bind to callback port",
            ),
            (
                AuthError::Http("connection refused".to_string()),
                "HTTP error: connection refused",
            ),
            (
                AuthError::Secrets("decrypt failed".to_string()),
                "Secrets error: decrypt failed",
            ),
        ];

        for (error, expected) in cases {
            let display = error.to_string();
            assert_eq!(
                display, expected,
                "AuthError display mismatch for {:?}",
                error
            );
        }
    }

    #[test]
    fn test_auth_error_clone_preserves_http_variant_and_payload() {
        let original = AuthError::Http("builder failed".to_string());
        let cloned = original.clone();

        match cloned {
            AuthError::Http(message) => assert_eq!(message, "builder failed"), // safety: test assertion in #[cfg(test)] module; not production panic path
            other => panic!("expected AuthError::Http variant, got {other:?}"),
        }
    }

    // --- New tests for well-known URI construction ---

    #[test]
    fn test_build_well_known_uri_no_path() {
        let uri =
            build_well_known_uri("https://example.com", "oauth-authorization-server").unwrap();
        assert_eq!(
            uri,
            "https://example.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_build_well_known_uri_with_path() {
        let uri =
            build_well_known_uri("https://example.com/path", "oauth-authorization-server").unwrap();
        assert_eq!(
            uri,
            "https://example.com/.well-known/oauth-authorization-server/path"
        );
    }

    #[test]
    fn test_build_well_known_uri_with_trailing_slash() {
        let uri =
            build_well_known_uri("https://example.com/path/", "oauth-protected-resource").unwrap();
        assert_eq!(
            uri,
            "https://example.com/.well-known/oauth-protected-resource/path"
        );
    }

    #[test]
    fn test_build_well_known_uri_root_trailing_slash() {
        let uri =
            build_well_known_uri("https://example.com/", "oauth-authorization-server").unwrap();
        assert_eq!(
            uri,
            "https://example.com/.well-known/oauth-authorization-server"
        );
    }

    // --- New tests for canonical_resource_uri ---

    #[test]
    fn test_canonical_resource_uri_strips_fragment() {
        assert_eq!(
            canonical_resource_uri("https://mcp.example.com/v1#section"),
            "https://mcp.example.com/v1"
        );
    }

    #[test]
    fn test_canonical_resource_uri_strips_trailing_slash() {
        assert_eq!(
            canonical_resource_uri("https://mcp.example.com/v1/"),
            "https://mcp.example.com/v1"
        );
    }

    #[test]
    fn test_canonical_resource_uri_no_changes_needed() {
        assert_eq!(
            canonical_resource_uri("https://mcp.example.com/v1"),
            "https://mcp.example.com/v1"
        );
    }

    // --- New tests for SSRF protection ---

    #[test]
    fn test_is_dangerous_ip_loopback_v4() {
        assert!(is_dangerous_ip("127.0.0.1".parse().unwrap()));
        assert!(is_dangerous_ip("127.0.0.2".parse().unwrap()));
    }

    #[test]
    fn test_is_dangerous_ip_private_v4() {
        assert!(is_dangerous_ip("10.0.0.1".parse().unwrap()));
        assert!(is_dangerous_ip("172.16.0.1".parse().unwrap()));
        assert!(is_dangerous_ip("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn test_is_dangerous_ip_link_local_v4() {
        assert!(is_dangerous_ip("169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn test_is_dangerous_ip_cgnat() {
        assert!(is_dangerous_ip("100.64.0.1".parse().unwrap()));
        assert!(is_dangerous_ip("100.127.255.254".parse().unwrap()));
    }

    #[test]
    fn test_is_dangerous_ip_safe_v4() {
        assert!(!is_dangerous_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_dangerous_ip("1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn test_is_dangerous_ip_ipv4_mapped_v6_loopback() {
        // ::ffff:127.0.0.1 must be blocked
        let ip: IpAddr = "::ffff:127.0.0.1".parse().unwrap();
        assert!(is_dangerous_ip(ip));
    }

    #[test]
    fn test_is_dangerous_ip_ipv4_mapped_v6_link_local() {
        // ::ffff:169.254.169.254 must be blocked
        let ip: IpAddr = "::ffff:169.254.169.254".parse().unwrap();
        assert!(is_dangerous_ip(ip));
    }

    #[test]
    fn test_is_dangerous_ip_unspecified() {
        assert!(is_dangerous_ip("0.0.0.0".parse().unwrap()));
        assert!(is_dangerous_ip("::".parse().unwrap()));
    }

    #[test]
    fn test_is_dangerous_ip_v6_loopback() {
        assert!(is_dangerous_ip("::1".parse().unwrap()));
    }

    #[tokio::test]
    async fn test_validate_url_safe_https() {
        assert!(validate_url_safe("https://example.com/path").await.is_ok());
    }

    #[tokio::test]
    async fn test_validate_url_safe_http_localhost_allowed() {
        // HTTP is only allowed for localhost dev scenarios
        assert!(validate_url_safe("http://localhost/path").await.is_ok());
        assert!(
            validate_url_safe("http://localhost:8080/path")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_validate_url_safe_http_non_localhost_rejected() {
        // HTTP to non-localhost hosts must be rejected (plaintext credential risk)
        assert!(validate_url_safe("http://example.com/path").await.is_err());
    }

    #[tokio::test]
    async fn test_validate_url_safe_bad_scheme() {
        assert!(validate_url_safe("ftp://example.com/path").await.is_err());
        assert!(validate_url_safe("file:///etc/passwd").await.is_err());
    }

    #[tokio::test]
    async fn test_validate_url_safe_private_ip() {
        // 127.0.0.1 over HTTP is allowed (localhost dev scenario)
        assert!(validate_url_safe("http://127.0.0.1/path").await.is_ok());
        // Private/link-local IPs over HTTPS are blocked (SSRF protection)
        assert!(validate_url_safe("https://10.0.0.1/path").await.is_err());
        assert!(
            validate_url_safe("https://169.254.169.254/latest/meta-data")
                .await
                .is_err()
        );
        // Private IPs over HTTP (non-localhost) are blocked
        assert!(validate_url_safe("http://10.0.0.1/path").await.is_err());
    }

    #[tokio::test]
    async fn test_validate_url_safe_public_ip() {
        assert!(validate_url_safe("https://8.8.8.8/dns").await.is_ok());
    }

    // --- New tests for parse_resource_metadata_url ---

    #[test]
    fn test_parse_resource_metadata_url_bearer() {
        let header = r#"Bearer resource_metadata="https://res.example.com/.well-known/oauth-protected-resource""#;
        let url = parse_resource_metadata_url(header);
        assert_eq!(
            url.as_deref(),
            Some("https://res.example.com/.well-known/oauth-protected-resource")
        );
    }

    #[test]
    fn test_parse_resource_metadata_url_with_other_params() {
        let header = r#"Bearer realm="example", resource_metadata="https://res.example.com/meta""#;
        let url = parse_resource_metadata_url(header);
        assert_eq!(url.as_deref(), Some("https://res.example.com/meta"));
    }

    #[test]
    fn test_parse_resource_metadata_url_missing() {
        let header = r#"Bearer realm="example""#;
        let url = parse_resource_metadata_url(header);
        assert!(url.is_none());
    }

    // --- New tests for resource parameter in authorization URL ---

    #[test]
    fn test_build_authorization_url_with_resource() {
        let url = build_authorization_url(
            "https://auth.example.com/authorize",
            "client-123",
            "http://localhost:9876/callback",
            &[],
            None,
            &HashMap::new(),
            Some("https://mcp.example.com/v1"),
        );

        assert!(url.contains("resource=https%3A%2F%2Fmcp.example.com%2Fv1"));
    }

    #[test]
    fn test_build_authorization_url_without_resource() {
        let url = build_authorization_url(
            "https://auth.example.com/authorize",
            "client-123",
            "http://localhost:9876/callback",
            &[],
            None,
            &HashMap::new(),
            None,
        );

        assert!(!url.contains("resource="));
    }

    /// Regression test: MCP OAuth authorization URLs must include a `state`
    /// parameter. While OAuth 2.1 makes `state` optional when PKCE is used,
    /// some MCP servers (e.g. Attio) require it and reject requests without it:
    /// {"error":"invalid_request","error_description":"Invalid value provided
    /// for: state"}
    ///
    /// Including `state` is harmless for servers that don't require it, since
    /// it is a standard OAuth parameter that compliant servers will echo back
    /// or ignore.
    ///
    /// The state is generated in `authorize_mcp_server` and injected into
    /// `extra_params` before `build_authorization_url` is called. This test
    /// verifies that `build_authorization_url` correctly propagates state from
    /// extra_params into the URL, and that each generated state is unique.
    #[test]
    fn test_authorization_url_includes_state_parameter() {
        // Simulate what authorize_mcp_server does: generate state and
        // insert it into extra_params.
        let mut extra_params = HashMap::new();
        let mut state_bytes = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut state_bytes);
        let state = URL_SAFE_NO_PAD.encode(state_bytes);
        extra_params.insert("state".to_string(), state.clone());

        let pkce = PkceChallenge::generate();
        let url = build_authorization_url(
            "https://app.attio.com/oidc/authorize",
            "test-client",
            "http://127.0.0.1:9876/callback",
            &[
                "mcp".to_string(),
                "offline_access".to_string(),
                "openid".to_string(),
            ],
            Some(&pkce),
            &extra_params,
            Some("https://mcp.attio.com/mcp"),
        );

        // State must be present in the URL
        assert!(
            url.contains(&format!("state={}", state)),
            "Authorization URL must include the state parameter, got: {}",
            url,
        );

        // State must be base64url-encoded (no padding, no +/)
        assert!(!state.contains('+'), "State must be base64url-safe");
        assert!(!state.contains('/'), "State must be base64url-safe");
        assert!(!state.contains('='), "State must not have padding");

        // State must have sufficient entropy (16 bytes -> 22 base64url chars)
        assert!(
            state.len() >= 22,
            "State must have at least 128 bits of entropy, got {} chars",
            state.len(),
        );

        // Two generated states must differ
        let mut state_bytes_2 = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut state_bytes_2);
        let state_2 = URL_SAFE_NO_PAD.encode(state_bytes_2);
        assert_ne!(state, state_2, "State must be unique per request");
    }

    #[tokio::test]
    async fn test_refresh_access_token_direct_includes_stored_client_secret() {
        let secrets = test_secrets_store();
        let user_id = "test-user";
        let Some((base_url, state)) = start_refresh_server().await else {
            return;
        };
        let server = McpServerConfig::new("notion", "https://mcp.notion.com/mcp").with_oauth(
            crate::tools::mcp::config::OAuthConfig::new("configured-client")
                .with_endpoints("http://127.0.0.1/authorize", format!("{base_url}/token")),
        );

        secrets
            .create(
                user_id,
                CreateSecretParams::new(server.refresh_token_secret_name(), "refresh-token-123"),
            )
            .await
            .unwrap();
        store_client_secret(&secrets, user_id, &server, "stored-client-secret", None)
            .await
            .unwrap();

        let token = refresh_access_token(&server, &secrets, user_id)
            .await
            .expect("refresh succeeds");
        assert_eq!(token.access_token, "refreshed-access-token");
        assert_eq!(
            token.refresh_token.as_deref(),
            Some("rotated-refresh-token")
        );

        let requests = state.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].form.get("client_id").map(String::as_str),
            Some("configured-client")
        );
        assert_eq!(
            requests[0].form.get("client_secret").map(String::as_str),
            Some("stored-client-secret")
        );
        assert_eq!(
            requests[0].form.get("refresh_token").map(String::as_str),
            Some("refresh-token-123")
        );
        assert_eq!(
            requests[0].form.get("resource").map(String::as_str),
            Some("https://mcp.notion.com/mcp")
        );

        let stored_refresh = secrets
            .get_decrypted(user_id, &server.refresh_token_secret_name())
            .await
            .unwrap();
        assert_eq!(stored_refresh.expose(), "rotated-refresh-token");
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_refresh_access_token_uses_proxy_when_configured() {
        let _env_guard = lock_env();
        let Some((base_url, state)) = start_refresh_server().await else {
            return;
        };
        let _proxy_url_guard = set_env_var("IRONCLAW_OAUTH_EXCHANGE_URL", Some(&base_url));
        let _proxy_token_guard = set_env_var(
            "IRONCLAW_OAUTH_PROXY_AUTH_TOKEN",
            Some("gateway-test-token"),
        );

        let secrets = test_secrets_store();
        let user_id = "test-user";
        let server = McpServerConfig::new("notion", "https://mcp.notion.com/mcp").with_oauth(
            crate::tools::mcp::config::OAuthConfig::new("configured-client").with_endpoints(
                "http://127.0.0.1/authorize",
                "https://auth.example.com/token",
            ),
        );

        secrets
            .create(
                user_id,
                CreateSecretParams::new(server.refresh_token_secret_name(), "refresh-token-123"),
            )
            .await
            .unwrap();
        store_client_secret(&secrets, user_id, &server, "stored-client-secret", None)
            .await
            .unwrap();

        refresh_access_token(&server, &secrets, user_id)
            .await
            .expect("proxy refresh succeeds");

        let requests = state.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer gateway-test-token")
        );
        assert_eq!(
            requests[0].form.get("token_url").map(String::as_str),
            Some("https://auth.example.com/token")
        );
        assert_eq!(
            requests[0].form.get("provider").map(String::as_str),
            Some("mcp:notion")
        );
        assert_eq!(
            requests[0].form.get("client_secret").map(String::as_str),
            Some("stored-client-secret")
        );
        assert_eq!(
            requests[0].form.get("resource").map(String::as_str),
            Some("https://mcp.notion.com/mcp")
        );
    }

    #[tokio::test]
    async fn test_refresh_access_token_serializes_concurrent_refreshes() {
        let secrets = test_secrets_store();
        let user_id = "test-user";
        let Some((base_url, state)) = start_refresh_server().await else {
            return;
        };
        let server = McpServerConfig::new("notion", "https://mcp.notion.com/mcp").with_oauth(
            crate::tools::mcp::config::OAuthConfig::new("configured-client")
                .with_endpoints("http://127.0.0.1/authorize", format!("{base_url}/token")),
        );

        secrets
            .create(
                user_id,
                CreateSecretParams::new(server.refresh_token_secret_name(), "refresh-token-123"),
            )
            .await
            .unwrap();

        let (first, second) = tokio::join!(
            refresh_access_token(&server, &secrets, user_id),
            refresh_access_token(&server, &secrets, user_id),
        );
        assert!(first.is_ok(), "first refresh should succeed: {first:?}");
        assert!(second.is_ok(), "second refresh should succeed: {second:?}");

        let requests = state.requests().await;
        assert_eq!(
            requests.len(),
            1,
            "only one outbound refresh should run for concurrent callers"
        );
    }

    #[tokio::test]
    async fn test_refresh_lock_reuses_same_key() {
        let first = refresh_lock("notion", "user-a").await;
        let second = refresh_lock("notion", "user-a").await;
        let other_user = refresh_lock("notion", "user-b").await;

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &other_user));
    }

    #[tokio::test]
    async fn test_refresh_lock_recreates_dropped_entry() {
        let first = refresh_lock("notion", "user-a").await;
        let first_ptr = Arc::as_ptr(&first);
        drop(first);

        let second = refresh_lock("notion", "user-a").await;

        assert_ne!(Arc::as_ptr(&second), first_ptr);
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: Tests use lock_env() to serialize environment access.
            unsafe {
                if let Some(ref value) = self.original {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
        let original = std::env::var(key).ok();
        // SAFETY: Tests use lock_env() to serialize environment access.
        unsafe {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
        EnvVarGuard { key, original }
    }
}
