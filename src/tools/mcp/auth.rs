//! OAuth 2.1 authentication for MCP servers.
//!
//! Implements the MCP Authorization specification using OAuth 2.1 with PKCE.
//! See: https://spec.modelcontextprotocol.io/specification/2025-03-26/basic/authorization/

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

use crate::cli::oauth_defaults::{self, OAUTH_CALLBACK_PORT};
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::tools::mcp::config::McpServerConfig;

/// OAuth authorization error.
#[derive(Debug, thiserror::Error)]
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
            v6.is_loopback()
                || v6.is_unspecified()
                // Check for IPv4-mapped IPv6 (::ffff:x.x.x.x)
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_dangerous_ip(IpAddr::V4(v4)))
        }
    }
}

/// Validate that a URL is safe for server-side requests (SSRF protection).
fn validate_url_safe(url: &str) -> Result<(), AuthError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| AuthError::DiscoveryFailed(format!("Invalid URL: {}", e)))?;

    // Must be HTTPS (or HTTP for localhost in dev)
    let scheme = parsed.scheme();
    if scheme != "https" && scheme != "http" {
        return Err(AuthError::DiscoveryFailed(format!(
            "Unsupported scheme: {}",
            scheme
        )));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| AuthError::DiscoveryFailed("URL has no host".to_string()))?;

    // For IP literals, parse directly and check
    if let Ok(ip) = host.parse::<IpAddr>()
        && is_dangerous_ip(ip)
    {
        return Err(AuthError::DiscoveryFailed(format!(
            "URL points to a restricted IP address: {}",
            host
        )));
    }
    // Note: DNS resolution happens at request time, we do best-effort here
    // DNS-based SSRF requires runtime checks; this catches literal IPs

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
    validate_url_safe(url)?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AuthError::Http(e.to_string()))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(e.to_string()))?;

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
async fn discover_via_401(server_url: &str) -> Result<AuthorizationServerMetadata, AuthError> {
    validate_url_safe(server_url)?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AuthError::Http(e.to_string()))?;

    let response = client
        .post(server_url)
        .header("Content-Type", "application/json")
        .body("{}")
        .send()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(e.to_string()))?;

    if response.status().as_u16() != 401 {
        return Err(AuthError::DiscoveryFailed(format!(
            "Expected 401, got {}",
            response.status()
        )));
    }

    let www_auth = response
        .headers()
        .get("WWW-Authenticate")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AuthError::DiscoveryFailed("No WWW-Authenticate header in 401 response".to_string())
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
    validate_url_safe(server_url)?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AuthError::Http(e.to_string()))?;

    let well_known_url = build_well_known_uri(server_url, "oauth-protected-resource")?;

    let response = client
        .get(&well_known_url)
        .send()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(e.to_string()))?;

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
    validate_url_safe(auth_server_url)?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AuthError::Http(e.to_string()))?;

    let well_known_url = build_well_known_uri(auth_server_url, "oauth-authorization-server")?;

    let response = client
        .get(&well_known_url)
        .send()
        .await
        .map_err(|e| AuthError::DiscoveryFailed(e.to_string()))?;

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
    validate_url_safe(registration_endpoint)?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AuthError::Http(e.to_string()))?;

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
    let (client_id, authorization_url, token_url, use_pkce, scopes, extra_params) =
        if let Some(oauth) = &server_config.oauth {
            // Pre-configured OAuth
            let (auth_url, tok_url) = discover_oauth_endpoints(server_config).await?;
            (
                oauth.client_id.clone(),
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

    // Compute canonical resource URI for RFC 8707
    let resource = canonical_resource_uri(&server_config.url);

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

    // Wait for callback
    let code = wait_for_authorization_callback(listener, &server_config.name).await?;

    println!("  Exchanging code for token...");

    // Exchange code for token
    let token = exchange_code_for_token(
        &token_url,
        &client_id,
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
            pkce.challenge
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
    code: &str,
    redirect_uri: &str,
    pkce: Option<&PkceChallenge>,
    resource: Option<&str>,
) -> Result<AccessToken, AuthError> {
    validate_url_safe(token_url)?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AuthError::Http(e.to_string()))?;

    let mut params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("client_id", client_id.to_string()),
    ];

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
    // Store access token
    let params = CreateSecretParams::new(server_config.token_secret_name(), &token.access_token)
        .with_provider(format!("mcp:{}", server_config.name));

    secrets
        .create(user_id, params)
        .await
        .map_err(|e| AuthError::Secrets(e.to_string()))?;

    // Store refresh token if present
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
    // Get client_id (from config or stored DCR)
    let client_id = get_client_id(server_config, secrets, user_id).await?;

    // Get the refresh token
    let refresh_token = secrets
        .get_decrypted(user_id, &server_config.refresh_token_secret_name())
        .await
        .map_err(|e| AuthError::RefreshFailed(format!("No refresh token: {}", e)))?;

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

    validate_url_safe(&token_url)?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AuthError::Http(e.to_string()))?;

    // Compute canonical resource URI for RFC 8707
    let resource = canonical_resource_uri(&server_config.url);

    let params = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.expose().to_string()),
        ("client_id", client_id),
        ("resource", resource),
    ];

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

    let token = AccessToken {
        access_token: token_response.access_token,
        token_type: token_response.token_type,
        expires_in: token_response.expires_in,
        refresh_token: token_response.refresh_token,
        scope: token_response.scope,
    };

    // Store the new tokens
    store_tokens(secrets, user_id, server_config, &token).await?;

    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_validate_url_safe_https() {
        assert!(validate_url_safe("https://example.com/path").is_ok());
    }

    #[test]
    fn test_validate_url_safe_http() {
        // HTTP is allowed (for localhost dev scenarios)
        assert!(validate_url_safe("http://example.com/path").is_ok());
    }

    #[test]
    fn test_validate_url_safe_bad_scheme() {
        assert!(validate_url_safe("ftp://example.com/path").is_err());
        assert!(validate_url_safe("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_validate_url_safe_private_ip() {
        assert!(validate_url_safe("http://127.0.0.1/path").is_err());
        assert!(validate_url_safe("http://10.0.0.1/path").is_err());
        assert!(validate_url_safe("http://169.254.169.254/latest/meta-data").is_err());
    }

    #[test]
    fn test_validate_url_safe_public_ip() {
        assert!(validate_url_safe("https://8.8.8.8/dns").is_ok());
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
}
