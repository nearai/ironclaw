//! Shared OAuth infrastructure: built-in credentials, callback server, landing pages.
//!
//! Every OAuth flow in the codebase (WASM tool auth, MCP server auth, NEAR AI login)
//! uses the same callback port, landing page, and listener logic from this module.
//!
//! # Built-in Credentials
//!
//! Many CLI tools (gcloud, rclone, gdrive) ship with default OAuth credentials
//! so users don't need to register their own OAuth app. Google explicitly
//! documents that client_secret for "Desktop App" / "Installed App" types
//! is NOT actually secret.
//!
//! Default credentials are hardcoded below. They can be overridden at:
//!
//! - **Compile time**: Set IRONCLAW_GOOGLE_CLIENT_ID / IRONCLAW_GOOGLE_CLIENT_SECRET
//!   env vars before building to replace the hardcoded defaults.
//! - **Runtime**: Users can set GOOGLE_OAUTH_CLIENT_ID / GOOGLE_OAUTH_CLIENT_SECRET
//!   env vars, which take priority over built-in defaults.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

use crate::secrets::{CreateSecretParams, SecretsStore};

// ── Built-in credentials ────────────────────────────────────────────────

pub struct OAuthCredentials {
    pub client_id: &'static str,
    pub client_secret: &'static str,
}

/// Google OAuth "Desktop App" credentials, shared across all Google tools.
/// Compile-time env vars override the hardcoded defaults below.
const GOOGLE_CLIENT_ID: &str = match option_env!("IRONCLAW_GOOGLE_CLIENT_ID") {
    Some(v) => v,
    None => "564604149681-efo25d43rs85v0tibdepsmdv5dsrhhr0.apps.googleusercontent.com",
};
const GOOGLE_CLIENT_SECRET: &str = match option_env!("IRONCLAW_GOOGLE_CLIENT_SECRET") {
    Some(v) => v,
    None => "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",
};

/// Returns built-in OAuth credentials for a provider, keyed by secret_name.
///
/// The secret_name comes from the tool's capabilities.json `auth.secret_name` field.
/// Returns `None` if no built-in credentials are configured for that provider.
pub fn builtin_credentials(secret_name: &str) -> Option<OAuthCredentials> {
    match secret_name {
        "google_oauth_token" => Some(OAuthCredentials {
            client_id: GOOGLE_CLIENT_ID,
            client_secret: GOOGLE_CLIENT_SECRET,
        }),
        _ => None,
    }
}

// ── Shared callback server ──────────────────────────────────────────────

// Core OAuth callback infrastructure is defined in `crate::llm::oauth_helpers`
// and re-exported here for backward compatibility.
pub use crate::llm::oauth_helpers::{
    OAUTH_CALLBACK_PORT, OAuthCallbackError, bind_callback_listener, callback_host, callback_url,
    is_loopback_host, landing_html, wait_for_callback,
};

// ── Shared OAuth flow steps ─────────────────────────────────────────

/// Response from the OAuth token exchange.
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
}

/// Result of building an OAuth 2.0 authorization URL.
pub struct OAuthUrlResult {
    /// The full authorization URL to redirect the user to.
    pub url: String,
    /// PKCE code verifier (must be sent with the token exchange request).
    pub code_verifier: Option<String>,
    /// Random state parameter for CSRF protection (must be validated in callback).
    pub state: String,
}

/// Build an OAuth 2.0 authorization URL with optional PKCE and CSRF state.
///
/// Returns an `OAuthUrlResult` containing the authorization URL, optional PKCE
/// code verifier, and a random `state` parameter for CSRF protection. The caller
/// must validate the `state` value in the callback before exchanging the code.
pub fn build_oauth_url(
    authorization_url: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    use_pkce: bool,
    extra_params: &HashMap<String, String>,
) -> OAuthUrlResult {
    // Generate PKCE verifier and challenge
    let (code_verifier, code_challenge) = if use_pkce {
        let mut verifier_bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut verifier_bytes);
        let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

        (Some(verifier), Some(challenge))
    } else {
        (None, None)
    };

    // Generate random state for CSRF protection
    let mut state_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut state_bytes);
    let state = URL_SAFE_NO_PAD.encode(state_bytes);

    // Build authorization URL
    let mut auth_url = format!(
        "{}?client_id={}&response_type=code&redirect_uri={}&state={}",
        authorization_url,
        urlencoding::encode(client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(&state),
    );

    if !scopes.is_empty() {
        auth_url.push_str(&format!(
            "&scope={}",
            urlencoding::encode(&scopes.join(" "))
        ));
    }

    if let Some(ref challenge) = code_challenge {
        auth_url.push_str(&format!(
            "&code_challenge={}&code_challenge_method=S256",
            challenge
        ));
    }

    for (key, value) in extra_params {
        auth_url.push_str(&format!(
            "&{}={}",
            urlencoding::encode(key),
            urlencoding::encode(value)
        ));
    }

    OAuthUrlResult {
        url: auth_url,
        code_verifier,
        state,
    }
}

/// Exchange an OAuth authorization code for tokens.
///
/// POSTs to `token_url` with the authorization code and optional PKCE verifier.
/// If `client_secret` is provided, uses HTTP Basic auth; otherwise includes
/// `client_id` in the form body (for public clients).
pub async fn exchange_oauth_code(
    token_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
    access_token_field: &str,
) -> Result<OAuthTokenResponse, OAuthCallbackError> {
    exchange_oauth_code_with_resource(
        token_url,
        client_id,
        client_secret,
        code,
        redirect_uri,
        code_verifier,
        access_token_field,
        None,
    )
    .await
}

/// Exchange an OAuth authorization code for tokens, with optional RFC 8707 `resource` parameter.
///
/// The `resource` parameter scopes the issued token to a specific server (used by MCP OAuth).
#[allow(clippy::too_many_arguments)]
pub async fn exchange_oauth_code_with_resource(
    token_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
    access_token_field: &str,
    resource: Option<&str>,
) -> Result<OAuthTokenResponse, OAuthCallbackError> {
    let client = reqwest::Client::new();
    let mut token_params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
    ];

    if let Some(verifier) = code_verifier {
        token_params.push(("code_verifier", verifier.to_string()));
    }

    if let Some(resource) = resource {
        token_params.push(("resource", resource.to_string()));
    }

    let mut request = client.post(token_url);

    if let Some(secret) = client_secret {
        request = request.basic_auth(client_id, Some(secret));
    } else {
        token_params.push(("client_id", client_id.to_string()));
    }

    let token_response = request
        .form(&token_params)
        .send()
        .await
        .map_err(|e| OAuthCallbackError::Io(format!("Token exchange request failed: {}", e)))?;

    if !token_response.status().is_success() {
        let status = token_response.status();
        let body = token_response.text().await.unwrap_or_default();
        return Err(OAuthCallbackError::Io(format!(
            "Token exchange failed: {} - {}",
            status, body
        )));
    }

    let token_data: serde_json::Value = token_response
        .json()
        .await
        .map_err(|e| OAuthCallbackError::Io(format!("Failed to parse token response: {}", e)))?;

    let access_token = token_data
        .get(access_token_field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            // Log only the field names present, not values (which may contain tokens)
            let fields: Vec<&str> = token_data
                .as_object()
                .map(|o| o.keys().map(|k| k.as_str()).collect())
                .unwrap_or_default();
            OAuthCallbackError::Io(format!(
                "No '{}' field in token response (fields present: {:?})",
                access_token_field, fields
            ))
        })?
        .to_string();

    let refresh_token = token_data
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(String::from);
    let expires_in = token_data.get("expires_in").and_then(|v| v.as_u64());

    Ok(OAuthTokenResponse {
        access_token,
        refresh_token,
        expires_in,
    })
}

/// Store OAuth tokens (access + refresh) in the secrets store.
///
/// Also stores the granted scopes as `{secret_name}_scopes` so that scope
/// expansion can be detected on subsequent activations.
#[allow(clippy::too_many_arguments)]
pub async fn store_oauth_tokens(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    secret_name: &str,
    provider: Option<&str>,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_in: Option<u64>,
    scopes: &[String],
) -> Result<(), OAuthCallbackError> {
    let mut params = CreateSecretParams::new(secret_name, access_token);

    if let Some(prov) = provider {
        params = params.with_provider(prov);
    }

    if let Some(secs) = expires_in {
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(secs as i64);
        params = params.with_expiry(expires_at);
    }

    store
        .create(user_id, params)
        .await
        .map_err(|e| OAuthCallbackError::Io(format!("Failed to save token: {}", e)))?;

    // Store refresh token separately (no expiry, it's long-lived)
    if let Some(rt) = refresh_token {
        let refresh_name = format!("{}_refresh_token", secret_name);
        let mut refresh_params = CreateSecretParams::new(&refresh_name, rt);
        if let Some(prov) = provider {
            refresh_params = refresh_params.with_provider(prov);
        }
        store
            .create(user_id, refresh_params)
            .await
            .map_err(|e| OAuthCallbackError::Io(format!("Failed to save refresh token: {}", e)))?;
    }

    // Store granted scopes for scope expansion detection
    if !scopes.is_empty() {
        let scopes_name = format!("{}_scopes", secret_name);
        let scopes_value = scopes.join(" ");
        let scopes_params = CreateSecretParams::new(&scopes_name, &scopes_value);
        // Best-effort: scope tracking failure shouldn't block auth
        let _ = store.create(user_id, scopes_params).await;
    }

    Ok(())
}

/// Validate an OAuth token against a tool's validation endpoint.
///
/// Sends a request to the configured endpoint with the token as a Bearer header.
/// Returns `Ok(())` if the response status matches the expected success status,
/// or an error with details if validation fails (wrong account, expired token, etc.).
pub async fn validate_oauth_token(
    token: &str,
    validation: &crate::tools::wasm::ValidationEndpointSchema,
) -> Result<(), OAuthCallbackError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| OAuthCallbackError::Io(format!("Failed to build HTTP client: {}", e)))?;

    let request = match validation.method.to_uppercase().as_str() {
        "POST" => client.post(&validation.url),
        _ => client.get(&validation.url),
    };

    let mut request = request.header("Authorization", format!("Bearer {}", token));

    // Add custom headers from the validation schema (e.g., Notion-Version)
    for (key, value) in &validation.headers {
        request = request.header(key, value);
    }

    let response = request
        .send()
        .await
        .map_err(|e| OAuthCallbackError::Io(format!("Validation request failed: {}", e)))?;

    if response.status().as_u16() == validation.success_status {
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let truncated: String = if body.len() > 200 {
            let mut end = 200;
            while end > 0 && !body.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &body[..end])
        } else {
            body
        };
        Err(OAuthCallbackError::Io(format!(
            "Token validation failed: HTTP {} (expected {}): {}",
            status, validation.success_status, truncated
        )))
    }
}

// ── Gateway callback support ─────────────────────────────────────────

/// State for an in-progress OAuth flow, keyed by CSRF `state` parameter.
///
/// Created by `start_wasm_oauth()` and consumed by the web gateway's
/// `/oauth/callback` handler when running in hosted mode.
pub struct PendingOAuthFlow {
    /// Extension name (e.g., "google_calendar").
    pub extension_name: String,
    /// Human-readable display name (e.g., "Google Calendar").
    pub display_name: String,
    /// OAuth token exchange URL.
    pub token_url: String,
    /// OAuth client ID.
    pub client_id: String,
    /// OAuth client secret (optional for PKCE-only flows).
    pub client_secret: Option<String>,
    /// The redirect_uri used in the authorization request.
    pub redirect_uri: String,
    /// PKCE code verifier (must match the code_challenge sent in the auth URL).
    pub code_verifier: Option<String>,
    /// Field name in token response containing the access token.
    pub access_token_field: String,
    /// Secret name for storage (e.g., "google_oauth_token").
    pub secret_name: String,
    /// Provider hint (e.g., "google").
    pub provider: Option<String>,
    /// Token validation endpoint (optional).
    pub validation_endpoint: Option<crate::tools::wasm::ValidationEndpointSchema>,
    /// Scopes that were requested.
    pub scopes: Vec<String>,
    /// User ID for secret storage.
    pub user_id: String,
    /// Secrets store reference for token persistence.
    pub secrets: Arc<dyn SecretsStore + Send + Sync>,
    /// SSE broadcast sender for notifying the web UI.
    pub sse_sender: Option<tokio::sync::broadcast::Sender<crate::channels::web::types::SseEvent>>,
    /// Gateway auth token for authenticating with the platform token exchange proxy.
    pub gateway_token: Option<String>,
    /// RFC 8707 resource parameter (MCP OAuth only).
    /// Sent during token exchange to scope the token to a specific MCP server.
    pub resource: Option<String>,
    /// When this flow was created (for expiry).
    pub created_at: std::time::Instant,
}

impl std::fmt::Debug for PendingOAuthFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingOAuthFlow")
            .field("extension_name", &self.extension_name)
            .field("display_name", &self.display_name)
            .field("secret_name", &self.secret_name)
            .field("created_at", &self.created_at)
            .finish_non_exhaustive()
    }
}

/// Thread-safe registry of pending OAuth flows, keyed by CSRF `state` parameter.
pub type PendingOAuthRegistry = Arc<RwLock<HashMap<String, PendingOAuthFlow>>>;

/// Create a new empty pending OAuth flow registry.
pub fn new_pending_oauth_registry() -> PendingOAuthRegistry {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Returns `true` if OAuth callbacks should be routed through the web gateway
/// instead of the local TCP listener.
///
/// This is the case when `IRONCLAW_OAUTH_CALLBACK_URL` is set to a non-loopback
/// URL, meaning the user's browser will redirect to a hosted gateway rather than
/// localhost.
pub fn use_gateway_callback() -> bool {
    std::env::var("IRONCLAW_OAUTH_CALLBACK_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|raw| {
            url::Url::parse(&raw)
                .ok()
                .and_then(|u| u.host_str().map(String::from))
                .map(|host| !is_loopback_host(&host))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// Maximum age for pending OAuth flows (5 minutes, matching TCP listener timeout).
pub const OAUTH_FLOW_EXPIRY: Duration = Duration::from_secs(300);

/// Remove expired flows from the registry.
///
/// Called when inserting new flows to prevent accumulation from abandoned
/// OAuth attempts.
pub async fn sweep_expired_flows(registry: &PendingOAuthRegistry) {
    let mut flows = registry.write().await;
    flows.retain(|_, flow| flow.created_at.elapsed() < OAUTH_FLOW_EXPIRY);
}

// ── Platform routing helpers ────────────────────────────────────────

/// Prepend instance name to CSRF state for platform routing.
///
/// The NEAR AI platform nginx proxy at `auth.DOMAIN` parses the instance name
/// from the `state` query parameter (format: `instance:nonce`) to route the
/// OAuth callback to the correct container.
///
/// Returns the nonce unchanged when `IRONCLAW_INSTANCE_NAME` is not set
/// (local/non-platform mode).
pub fn build_platform_state(nonce: &str) -> String {
    let instance = std::env::var("IRONCLAW_INSTANCE_NAME")
        .or_else(|_| std::env::var("OPENCLAW_INSTANCE_NAME"))
        .ok()
        .filter(|v| !v.is_empty());
    match instance {
        Some(name) => format!("{}:{}", name, nonce),
        None => nonce.to_string(),
    }
}

/// Strip the instance prefix from a state parameter to recover the lookup nonce.
///
/// `"myinstance:abc123"` → `"abc123"`, `"abc123"` → `"abc123"` (no prefix).
///
/// Safe because nonces are base64url-encoded (`[A-Za-z0-9_-]`, no colons).
pub fn strip_instance_prefix(state: &str) -> &str {
    state
        .split_once(':')
        .map(|(_, nonce)| nonce)
        .unwrap_or(state)
}

/// Exchange an OAuth authorization code via the platform's token exchange proxy.
///
/// The proxy holds `client_secret` server-side so the container never sees it.
/// Authenticated via the gateway auth token (Bearer header).
///
/// The proxy expects form params `{code, redirect_uri, code_verifier}` and
/// returns a standard Google token response `{access_token, refresh_token, expires_in}`.
pub async fn exchange_via_proxy(
    proxy_url: &str,
    gateway_token: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
    access_token_field: &str,
) -> Result<OAuthTokenResponse, OAuthCallbackError> {
    if gateway_token.is_empty() {
        return Err(OAuthCallbackError::Io(
            "Gateway auth token is required for proxy token exchange".to_string(),
        ));
    }
    let exchange_url = format!("{}/oauth/exchange", proxy_url.trim_end_matches('/'));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| OAuthCallbackError::Io(format!("Failed to build HTTP client: {}", e)))?;
    let mut params = vec![
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
    ];
    if let Some(verifier) = code_verifier {
        params.push(("code_verifier", verifier.to_string()));
    }

    let response = client
        .post(&exchange_url)
        .bearer_auth(gateway_token)
        .form(&params)
        .send()
        .await
        .map_err(|e| {
            OAuthCallbackError::Io(format!("Token exchange proxy request failed: {}", e))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(OAuthCallbackError::Io(format!(
            "Token exchange proxy failed: {} - {}",
            status, body
        )));
    }

    let token_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| OAuthCallbackError::Io(format!("Failed to parse proxy response: {}", e)))?;

    let access_token = token_data
        .get(access_token_field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            let fields: Vec<&str> = token_data
                .as_object()
                .map(|o| o.keys().map(|k| k.as_str()).collect())
                .unwrap_or_default();
            OAuthCallbackError::Io(format!(
                "No '{}' field in proxy response (fields present: {:?})",
                access_token_field, fields
            ))
        })?
        .to_string();

    let refresh_token = token_data
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(String::from);
    let expires_in = token_data.get("expires_in").and_then(|v| v.as_u64());

    Ok(OAuthTokenResponse {
        access_token,
        refresh_token,
        expires_in,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::cli::oauth_defaults::{
        builtin_credentials, callback_host, callback_url, is_loopback_host, landing_html,
    };

    /// Serializes env-mutating tests to prevent parallel races.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_is_loopback_host() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("127.0.0.2")); // full 127.0.0.0/8 range
        assert!(is_loopback_host("127.255.255.254"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("LOCALHOST"));
        assert!(!is_loopback_host("203.0.113.10"));
        assert!(!is_loopback_host("my-server.example.com"));
        assert!(!is_loopback_host("0.0.0.0"));
    }

    #[test]
    fn test_callback_host_default() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let original = std::env::var("OAUTH_CALLBACK_HOST").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("OAUTH_CALLBACK_HOST");
        }
        assert_eq!(callback_host(), "127.0.0.1");
        // Restore
        unsafe {
            if let Some(val) = original {
                std::env::set_var("OAUTH_CALLBACK_HOST", val);
            }
        }
    }

    #[test]
    fn test_callback_host_env_override() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let original_host = std::env::var("OAUTH_CALLBACK_HOST").ok();
        let original_url = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("OAUTH_CALLBACK_HOST", "203.0.113.10");
            std::env::remove_var("IRONCLAW_OAUTH_CALLBACK_URL");
        }
        assert_eq!(callback_host(), "203.0.113.10");
        // callback_url() fallback should incorporate the custom host
        let url = callback_url();
        assert!(url.contains("203.0.113.10"), "url was: {url}");
        // Restore
        unsafe {
            if let Some(val) = original_host {
                std::env::set_var("OAUTH_CALLBACK_HOST", val);
            } else {
                std::env::remove_var("OAUTH_CALLBACK_HOST");
            }
            if let Some(val) = original_url {
                std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", val);
            }
        }
    }

    #[test]
    fn test_callback_url_default() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        // Clear both env vars to test default behavior
        let original_url = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        let original_host = std::env::var("OAUTH_CALLBACK_HOST").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("IRONCLAW_OAUTH_CALLBACK_URL");
            std::env::remove_var("OAUTH_CALLBACK_HOST");
        }
        let url = callback_url();
        assert_eq!(url, "http://127.0.0.1:9876");
        // Restore
        unsafe {
            if let Some(val) = original_url {
                std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", val);
            }
            if let Some(val) = original_host {
                std::env::set_var("OAUTH_CALLBACK_HOST", val);
            }
        }
    }

    #[test]
    fn test_callback_url_env_override() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let original = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var(
                "IRONCLAW_OAUTH_CALLBACK_URL",
                "https://myserver.example.com:9876",
            );
        }
        let url = callback_url();
        assert_eq!(url, "https://myserver.example.com:9876");
        // Restore
        unsafe {
            if let Some(val) = original {
                std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", val);
            } else {
                std::env::remove_var("IRONCLAW_OAUTH_CALLBACK_URL");
            }
        }
    }

    #[test]
    fn test_unknown_provider_returns_none() {
        assert!(builtin_credentials("unknown_token").is_none());
    }

    #[test]
    fn test_google_returns_based_on_compile_env() {
        let creds = builtin_credentials("google_oauth_token");
        assert!(creds.is_some());
        let creds = creds.unwrap();
        assert!(!creds.client_id.is_empty());
        assert!(!creds.client_secret.is_empty());
    }

    #[test]
    fn test_landing_html_success_contains_key_elements() {
        let html = landing_html("Google", true);
        assert!(html.contains("Google Connected"));
        assert!(html.contains("charset"));
        assert!(html.contains("IronClaw"));
        assert!(html.contains("#22c55e")); // green accent
        assert!(!html.contains("Failed"));
    }

    #[test]
    fn test_landing_html_escapes_provider_name() {
        let html = landing_html("<script>alert(1)</script>", true);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_landing_html_error_contains_key_elements() {
        let html = landing_html("Notion", false);
        assert!(html.contains("Authorization Failed"));
        assert!(html.contains("charset"));
        assert!(html.contains("IronClaw"));
        assert!(html.contains("#ef4444")); // red accent
        assert!(!html.contains("Connected"));
    }

    #[test]
    fn test_build_oauth_url_basic() {
        use std::collections::HashMap;

        use crate::cli::oauth_defaults::build_oauth_url;

        let result = build_oauth_url(
            "https://accounts.google.com/o/oauth2/auth",
            "my-client-id",
            "http://localhost:9876/callback",
            &["openid".to_string(), "email".to_string()],
            false,
            &HashMap::new(),
        );

        assert!(
            result
                .url
                .starts_with("https://accounts.google.com/o/oauth2/auth?")
        );
        assert!(result.url.contains("client_id=my-client-id"));
        assert!(result.url.contains("response_type=code"));
        assert!(result.url.contains("redirect_uri="));
        assert!(result.url.contains("scope=openid%20email"));
        assert!(result.url.contains("state="));
        assert!(result.code_verifier.is_none());
        assert!(!result.state.is_empty());
    }

    #[test]
    fn test_build_oauth_url_with_pkce() {
        use std::collections::HashMap;

        use crate::cli::oauth_defaults::build_oauth_url;

        let result = build_oauth_url(
            "https://auth.example.com/authorize",
            "client-123",
            "http://localhost:9876/callback",
            &[],
            true,
            &HashMap::new(),
        );

        assert!(result.url.contains("code_challenge="));
        assert!(result.url.contains("code_challenge_method=S256"));
        assert!(result.code_verifier.is_some());
        let verifier = result.code_verifier.unwrap();
        assert!(!verifier.is_empty());
    }

    #[test]
    fn test_build_oauth_url_with_extra_params() {
        use std::collections::HashMap;

        use crate::cli::oauth_defaults::build_oauth_url;

        let mut extra = HashMap::new();
        extra.insert("access_type".to_string(), "offline".to_string());
        extra.insert("prompt".to_string(), "consent".to_string());

        let result = build_oauth_url(
            "https://auth.example.com/authorize",
            "client-123",
            "http://localhost:9876/callback",
            &["read".to_string()],
            false,
            &extra,
        );

        assert!(result.url.contains("access_type=offline"));
        assert!(result.url.contains("prompt=consent"));
    }

    #[test]
    fn test_build_oauth_url_state_is_unique() {
        use std::collections::HashMap;

        use crate::cli::oauth_defaults::build_oauth_url;

        let result1 = build_oauth_url(
            "https://auth.example.com/authorize",
            "client",
            "http://localhost:9876/callback",
            &[],
            false,
            &HashMap::new(),
        );
        let result2 = build_oauth_url(
            "https://auth.example.com/authorize",
            "client",
            "http://localhost:9876/callback",
            &[],
            false,
            &HashMap::new(),
        );

        // State should be different each time (random)
        assert_ne!(result1.state, result2.state);
    }

    #[test]
    fn test_use_gateway_callback_false_by_default() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let original = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("IRONCLAW_OAUTH_CALLBACK_URL");
        }
        assert!(!crate::cli::oauth_defaults::use_gateway_callback());
        unsafe {
            if let Some(val) = original {
                std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", val);
            }
        }
    }

    #[test]
    fn test_use_gateway_callback_true_for_hosted() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let original = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var(
                "IRONCLAW_OAUTH_CALLBACK_URL",
                "https://kind-deer.agent1.near.ai",
            );
        }
        assert!(crate::cli::oauth_defaults::use_gateway_callback());
        unsafe {
            if let Some(val) = original {
                std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", val);
            } else {
                std::env::remove_var("IRONCLAW_OAUTH_CALLBACK_URL");
            }
        }
    }

    #[test]
    fn test_use_gateway_callback_false_for_localhost() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let original = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", "http://127.0.0.1:3001");
        }
        assert!(!crate::cli::oauth_defaults::use_gateway_callback());
        unsafe {
            if let Some(val) = original {
                std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", val);
            } else {
                std::env::remove_var("IRONCLAW_OAUTH_CALLBACK_URL");
            }
        }
    }

    #[test]
    fn test_use_gateway_callback_false_for_empty() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let original = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", "");
        }
        assert!(!crate::cli::oauth_defaults::use_gateway_callback());
        unsafe {
            if let Some(val) = original {
                std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", val);
            } else {
                std::env::remove_var("IRONCLAW_OAUTH_CALLBACK_URL");
            }
        }
    }

    #[test]
    fn test_build_platform_state_with_instance() {
        use crate::cli::oauth_defaults::build_platform_state;

        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let original = std::env::var("IRONCLAW_INSTANCE_NAME").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("IRONCLAW_INSTANCE_NAME", "kind-deer");
        }
        assert_eq!(build_platform_state("abc123"), "kind-deer:abc123");
        unsafe {
            if let Some(val) = original {
                std::env::set_var("IRONCLAW_INSTANCE_NAME", val);
            } else {
                std::env::remove_var("IRONCLAW_INSTANCE_NAME");
            }
        }
    }

    #[test]
    fn test_build_platform_state_without_instance() {
        use crate::cli::oauth_defaults::build_platform_state;

        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let original = std::env::var("IRONCLAW_INSTANCE_NAME").ok();
        let original_oc = std::env::var("OPENCLAW_INSTANCE_NAME").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("IRONCLAW_INSTANCE_NAME");
            std::env::remove_var("OPENCLAW_INSTANCE_NAME");
        }
        assert_eq!(build_platform_state("abc123"), "abc123");
        unsafe {
            if let Some(val) = original {
                std::env::set_var("IRONCLAW_INSTANCE_NAME", val);
            }
            if let Some(val) = original_oc {
                std::env::set_var("OPENCLAW_INSTANCE_NAME", val);
            }
        }
    }

    #[test]
    fn test_build_platform_state_with_openclaw_instance() {
        use crate::cli::oauth_defaults::build_platform_state;

        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let original_ic = std::env::var("IRONCLAW_INSTANCE_NAME").ok();
        let original_oc = std::env::var("OPENCLAW_INSTANCE_NAME").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("IRONCLAW_INSTANCE_NAME");
            std::env::set_var("OPENCLAW_INSTANCE_NAME", "quiet-lion");
        }
        assert_eq!(build_platform_state("xyz789"), "quiet-lion:xyz789");
        unsafe {
            if let Some(val) = original_ic {
                std::env::set_var("IRONCLAW_INSTANCE_NAME", val);
            }
            if let Some(val) = original_oc {
                std::env::set_var("OPENCLAW_INSTANCE_NAME", val);
            } else {
                std::env::remove_var("OPENCLAW_INSTANCE_NAME");
            }
        }
    }

    #[test]
    fn test_strip_instance_prefix_with_colon() {
        use crate::cli::oauth_defaults::strip_instance_prefix;

        assert_eq!(strip_instance_prefix("kind-deer:abc123"), "abc123");
        assert_eq!(strip_instance_prefix("my-instance:xyz"), "xyz");
    }

    #[test]
    fn test_strip_instance_prefix_without_colon() {
        use crate::cli::oauth_defaults::strip_instance_prefix;

        assert_eq!(strip_instance_prefix("abc123"), "abc123");
        assert_eq!(strip_instance_prefix(""), "");
    }

    /// Verify that `build_oauth_url` includes the RFC 8707 `resource` parameter
    /// when passed through `extra_params`, which is how MCP OAuth gateway mode
    /// scopes tokens to a specific MCP server.
    #[test]
    fn test_build_oauth_url_includes_resource_via_extra_params() {
        use std::collections::HashMap;

        use crate::cli::oauth_defaults::build_oauth_url;

        let mut extra = HashMap::new();
        extra.insert(
            "resource".to_string(),
            "https://mcp.example.com".to_string(),
        );

        let result = build_oauth_url(
            "https://auth.example.com/authorize",
            "client-123",
            "https://gateway.example.com/oauth/callback",
            &["read".to_string()],
            true,
            &extra,
        );

        // The resource parameter should be URL-encoded in the auth URL
        assert!(
            result.url.contains("resource=https%3A%2F%2Fmcp.example.com"),
            "Expected resource param in URL: {}",
            result.url
        );
        // State and PKCE should be present
        assert!(result.url.contains("state="));
        assert!(result.url.contains("code_challenge="));
        assert!(result.code_verifier.is_some());
    }
}
