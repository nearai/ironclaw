//! Shared OAuth flow infrastructure, callback server, and landing pages.
//!
//! Every OAuth flow in the codebase (WASM tool auth, MCP server auth, NEAR AI login)
//! uses the same callback port, landing page, and listener logic from this module.
//!
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::tools::wasm::{ssrf_safe_client_builder_for_target, validate_and_resolve_http_target};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ironclaw_common::ExtensionName;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use url::Url;

pub use crate::auth::providers::{
    OAuthCredentials, builtin_client_id_override_env, builtin_credentials,
    hosted_proxy_client_secret,
};
use crate::secrets::{CreateSecretParams, SecretsStore};

// ── Shared callback server ──────────────────────────────────────────────

// Core OAuth callback infrastructure is defined in `crate::llm::oauth_helpers`
// and re-exported here for backward compatibility.
pub use crate::llm::oauth_helpers::{
    OAUTH_CALLBACK_PORT, OAuthCallbackError, bind_callback_listener, callback_host, callback_url,
    is_loopback_host, landing_html, wait_for_callback,
};

// ── Shared OAuth flow steps ─────────────────────────────────────────

/// Only allow `https://` URLs for auth/setup links to prevent scheme injection
/// (e.g. `javascript:`, `file://`). Host validation is explicitly out of scope:
/// the URL source is a trusted local extension tool result, and display-side
/// rendering controls where the user is actually navigated.
///
/// Both the v1 dispatcher (`src/agent/dispatcher.rs`) and the v2 effect adapter
/// (`src/bridge/effect_adapter.rs`) call this on every `auth_url` extracted
/// from `tool_activate`/`tool_auth` output before surfacing it to the client.
/// Keeping the helper in one place ensures the v1/v2 invariants stay symmetric.
pub(crate) fn sanitize_auth_url(url: Option<&str>) -> Option<String> {
    url.map(str::trim).and_then(|u| {
        if u.chars().any(char::is_control) {
            return None;
        }
        if urlencoding::decode(u)
            .ok()
            .is_some_and(|decoded| decoded.chars().any(char::is_control))
        {
            return None;
        }
        url::Url::parse(u)
            .ok()
            .filter(|parsed| parsed.scheme().eq_ignore_ascii_case("https"))
            .filter(|parsed| parsed.has_host())
            .map(|parsed| parsed.to_string())
    })
}

#[cfg(test)]
mod sanitize_tests {
    use super::sanitize_auth_url;

    #[test]
    fn rejects_non_https_schemes() {
        assert!(sanitize_auth_url(Some("javascript:alert(1)")).is_none());
        assert!(sanitize_auth_url(Some("file:///etc/passwd")).is_none());
        assert!(sanitize_auth_url(Some("http://example.com")).is_none());
        assert!(sanitize_auth_url(Some("data:text/html,<h1>")).is_none());
        assert!(sanitize_auth_url(Some("")).is_none());
        assert!(sanitize_auth_url(None).is_none());
    }

    #[test]
    fn allows_https() {
        assert_eq!(
            sanitize_auth_url(Some("https://accounts.google.com/o/oauth2/auth")),
            Some("https://accounts.google.com/o/oauth2/auth".to_string())
        );
    }

    #[test]
    fn trims_whitespace_before_validating_scheme() {
        assert_eq!(
            sanitize_auth_url(Some("  https://example.com/auth  ")),
            Some("https://example.com/auth".to_string())
        );
    }

    #[test]
    fn allows_mixed_case_https_scheme() {
        assert_eq!(
            sanitize_auth_url(Some("HTTPS://example.com/auth")),
            Some("https://example.com/auth".to_string())
        );
    }

    #[test]
    fn rejects_invalid_or_control_character_urls() {
        assert!(sanitize_auth_url(Some("https://")).is_none());
        assert!(sanitize_auth_url(Some("https://example.com/\nattack")).is_none());
        assert!(sanitize_auth_url(Some("https://example.com/\rattack")).is_none());
        assert!(sanitize_auth_url(Some("https://example.com/%0d%0aattack")).is_none());
        assert!(
            sanitize_auth_url(Some("https://example.com/?next=%0D%0ALocation:%20evil")).is_none()
        );
    }
}

/// Truncate `body` to at most `max_bytes` UTF-8 bytes, walking back to the
/// nearest char boundary so the result is always a valid `&str`. Appends
/// `"..."` when truncation actually happens. Used to bound any
/// upstream-controlled response text we interpolate into error strings.
fn truncate_at_char_boundary(body: &str, max_bytes: usize) -> String {
    if body.len() <= max_bytes {
        return body.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &body[..end])
}

/// Read an OAuth error response body for inclusion in a log / error
/// message, truncated to `max_bytes` at a UTF-8 char boundary.
///
/// Two hazards rolled into one helper so every `!status.is_success()`
/// site in this module composes an error message the same way:
///
/// 1. **Leak risk.** OAuth error responses can echo request details,
///    partial token material, or unbounded vendor-specific blobs.
///    Surfacing the raw body into an error string leaks that into
///    logs, SSE events, and panic output.
/// 2. **Read failures.** `response.text().await` can fail on network
///    resets, encoding issues, or header/body mismatches. We swallow
///    those and fall back to an empty string — the HTTP status code
///    is already in the caller's outer `format!`, so it's still
///    actionable without the body. Raising the read failure instead
///    would obscure the actual provider error with a secondary I/O
///    error. This is the `// silent-ok` case per
///    `.claude/rules/error-handling.md`.
async fn consume_oauth_error_body(response: reqwest::Response, max_bytes: usize) -> String {
    // silent-ok: upstream error body may be unreadable (network reset,
    // bad encoding); the caller's format! already includes status,
    // which is the actionable part.
    let body = response.text().await.unwrap_or_default();
    truncate_at_char_boundary(&body, max_bytes)
}

/// Response from the OAuth token exchange.
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
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

/// Errors returned while constructing an OAuth authorization URL.
///
/// The only currently-modeled variant is `MalformedConfig`, returned when the
/// provided `authorization_url` cannot be parsed by the `url` crate. That
/// indicates a misconfigured descriptor / capabilities entry; the caller
/// should surface it to the operator rather than attempting to "fix up" the
/// URL through string concatenation, which is what gemini-code-assist flagged
/// on #2746 as a security-posture issue.
#[derive(Debug, Clone, thiserror::Error)]
pub enum OAuthUrlError {
    /// The `authorization_url` could not be parsed as a valid URL.
    #[error("Malformed OAuth authorization URL: {0}")]
    MalformedConfig(String),
}

/// Build an OAuth 2.0 authorization URL with optional PKCE and CSRF state.
///
/// Returns an `OAuthUrlResult` containing the authorization URL, optional PKCE
/// code verifier, and a random `state` parameter for CSRF protection. The caller
/// must validate the `state` value in the callback before exchanging the code.
///
/// Returns `Err(OAuthUrlError::MalformedConfig)` if `authorization_url` cannot
/// be parsed. We deliberately do not try to normalize a malformed URL through
/// manual string concatenation — rejecting a bad config is the only secure
/// response (see gemini-code-assist review on #2746).
pub fn build_oauth_url(
    authorization_url: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    use_pkce: bool,
    extra_params: &HashMap<String, String>,
) -> Result<OAuthUrlResult, OAuthUrlError> {
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

    // Build the authorization URL via the `url` crate so query-string encoding
    // goes through a single well-tested code path. This replaces a hand-rolled
    // `format!` + `urlencoding::encode` loop that had a history of truncating
    // the last character of the final query parameter on some platforms
    // (nearai/ironclaw#2391: `access_type=offline` was being received by
    // Google as `access_type=offlin`). A `Url::parse` failure here means the
    // descriptor/capabilities config is malformed; we reject rather than
    // concat-normalize (gemini-code-assist review on #2746) so a bad config
    // cannot silently produce a half-formed URL.
    let auth_url = build_oauth_authorization_url_string(
        authorization_url,
        client_id,
        redirect_uri,
        &state,
        scopes,
        code_challenge.as_deref(),
        extra_params,
    )?;

    Ok(OAuthUrlResult {
        url: auth_url,
        code_verifier,
        state,
    })
}

/// Append OAuth authorization-request query parameters to `authorization_url`.
///
/// Uses `url::Url::parse_with_params`-style encoding via `query_pairs_mut()`
/// so every value is percent-encoded exactly once with the standard
/// `application/x-www-form-urlencoded` rules. Any non-URL characters in
/// `scopes`, `extra_params`, `state`, etc. are encoded safely.
///
/// Returns `Err(OAuthUrlError::MalformedConfig)` if `authorization_url` cannot
/// be parsed as a URL. We deliberately do not fall back to a manual
/// string-concat path: a malformed authorization URL is a config error that
/// the operator should see, not something the agent should try to paper over
/// (gemini-code-assist review on #2746).
fn build_oauth_authorization_url_string(
    authorization_url: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    scopes: &[String],
    code_challenge: Option<&str>,
    extra_params: &HashMap<String, String>,
) -> Result<String, OAuthUrlError> {
    let mut url = Url::parse(authorization_url).map_err(|e| {
        OAuthUrlError::MalformedConfig(format!(
            "could not parse authorization URL {authorization_url:?}: {e}"
        ))
    })?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("client_id", client_id);
        qp.append_pair("response_type", "code");
        qp.append_pair("redirect_uri", redirect_uri);
        qp.append_pair("state", state);
        if !scopes.is_empty() {
            qp.append_pair("scope", &scopes.join(" "));
        }
        if let Some(challenge) = code_challenge {
            qp.append_pair("code_challenge", challenge);
            qp.append_pair("code_challenge_method", "S256");
        }
        for (key, value) in extra_params {
            qp.append_pair(key, value);
        }
    }
    Ok(url.into())
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
    let extra_token_params = HashMap::new();
    exchange_oauth_code_with_params(
        token_url,
        client_id,
        client_secret,
        code,
        redirect_uri,
        code_verifier,
        access_token_field,
        &extra_token_params,
    )
    .await
}

/// Exchange an OAuth authorization code for tokens with generic extra form parameters.
///
/// **SSRF + redirect hardening.** `token_url` is supply-chain controlled (it
/// originates in tool capabilities JSON), so the URL is validated through
/// [`validate_and_resolve_http_target`] before the request, the client is
/// pinned to the resolved address via [`ssrf_safe_client_builder_for_target`],
/// and `redirect(Policy::none())` is set so an attacker cannot redirect the
/// authorization code, PKCE verifier, or `client_secret` to a different host.
/// Error responses have their bodies truncated before being interpolated.
#[allow(clippy::too_many_arguments)]
pub async fn exchange_oauth_code_with_params(
    token_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
    access_token_field: &str,
    extra_token_params: &HashMap<String, String>,
) -> Result<OAuthTokenResponse, OAuthCallbackError> {
    let resolved_target = validate_and_resolve_http_target(token_url)
        .await
        .map_err(|e| OAuthCallbackError::Io(format!("Token URL rejected: {e}")))?;
    let client = ssrf_safe_client_builder_for_target(&resolved_target)
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| OAuthCallbackError::Io(format!("build HTTP client: {e}")))?;

    let mut token_params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
    ];

    if let Some(verifier) = code_verifier {
        token_params.push(("code_verifier", verifier.to_string()));
    }

    for (key, value) in extra_token_params {
        token_params.push((key.as_str(), value.clone()));
    }

    let mut request = client.post(token_url);

    if let Some(secret) = client_secret {
        request = request.basic_auth(client_id, Some(secret));
    } else {
        token_params.push(("client_id", client_id.to_string()));
    }

    let token_response = request
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&token_params)
        .send()
        .await
        .map_err(|e| OAuthCallbackError::Io(format!("Token exchange request failed: {}", e)))?;

    if !token_response.status().is_success() {
        let status = token_response.status();
        let truncated = consume_oauth_error_body(token_response, 500).await;
        return Err(OAuthCallbackError::Io(format!(
            "Token exchange failed: {} - {}",
            status, truncated
        )));
    }

    let content_type = token_response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let body = token_response
        .text()
        .await
        .map_err(|e| OAuthCallbackError::Io(format!("Failed to read token response: {}", e)))?;

    oauth_token_response_from_body(&body, access_token_field, content_type.as_deref())
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
    let mut extra_token_params = HashMap::new();
    if let Some(resource) = resource {
        extra_token_params.insert("resource".to_string(), resource.to_string());
    }
    exchange_oauth_code_with_params(
        token_url,
        client_id,
        client_secret,
        code,
        redirect_uri,
        code_verifier,
        access_token_field,
        &extra_token_params,
    )
    .await
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
        // Saturate on overflow: a hostile / buggy provider returning
        // `u64::MAX` for `expires_in` would either wrap to a negative
        // i64 (immediately invalidating the token) or panic in older
        // chrono versions. `try_seconds` returns `None` past chrono's
        // internal millisecond limit; saturate to `TimeDelta::MAX` so
        // the token simply lives "effectively forever" rather than
        // poisoning storage. Mirrors the pattern in `auth/mod.rs`.
        let expires_secs = i64::try_from(secs).unwrap_or(i64::MAX);
        let expires_delta =
            chrono::Duration::try_seconds(expires_secs).unwrap_or(chrono::TimeDelta::MAX);
        let expires_at = chrono::Utc::now() + expires_delta;
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
///
/// **SSRF hardening.** `validation.url` is supply-chain controlled (it lives
/// in the tool's capabilities JSON), so without validation a malicious tool
/// author could redirect IronClaw to send the freshly-minted access token to
/// an internal endpoint as `Authorization: Bearer <token>`. We resolve and
/// validate the URL through [`validate_and_resolve_http_target`] and pin
/// reqwest to the validated address via [`ssrf_safe_client_builder_for_target`],
/// plus disable redirects so a 302 from a public-looking host cannot bounce
/// the bearer-bearing request to an internal one.
pub async fn validate_oauth_token(
    token: &str,
    validation: &crate::tools::wasm::ValidationEndpointSchema,
) -> Result<(), OAuthCallbackError> {
    let resolved_target = validate_and_resolve_http_target(&validation.url)
        .await
        .map_err(|e| OAuthCallbackError::Io(format!("Validation URL rejected: {e}")))?;
    let client = ssrf_safe_client_builder_for_target(&resolved_target)
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
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
        let truncated = consume_oauth_error_body(response, 200).await;
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
    pub extension_name: ExtensionName,
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
    /// SSE broadcast manager for notifying the web UI.
    pub sse_manager: Option<Arc<crate::channels::web::sse::SseManager>>,
    /// OAuth proxy auth token for authenticating with the hosted token exchange proxy.
    /// Kept as `gateway_token` for public API compatibility.
    pub gateway_token: Option<String>,
    /// Additional form params for the token exchange request.
    /// Used for provider-specific requirements such as RFC 8707 `resource`.
    pub token_exchange_extra_params: HashMap<String, String>,
    /// Secret name for persisting the client ID (MCP OAuth only).
    /// Needed so token refresh can find the client_id after the session ends.
    pub client_id_secret_name: Option<String>,
    /// Secret name for persisting the client secret (MCP DCR only).
    /// Needed for providers that return a client secret during DCR and expect
    /// it to be replayed during later refreshes.
    pub client_secret_secret_name: Option<String>,
    /// Absolute UNIX timestamp when the DCR client secret expires, if any.
    pub client_secret_expires_at: Option<u64>,
    /// When this flow was created (for expiry).
    pub created_at: std::time::Instant,
    /// Whether successful OAuth should auto-activate `extension_name`.
    pub auto_activate_extension: bool,
}

impl std::fmt::Debug for PendingOAuthFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingOAuthFlow")
            .field("extension_name", &self.extension_name)
            .field("display_name", &self.display_name)
            .field("secret_name", &self.secret_name)
            .field("created_at", &self.created_at)
            .field("auto_activate_extension", &self.auto_activate_extension)
            .finish_non_exhaustive()
    }
}

impl PendingOAuthFlow {
    pub fn oauth_proxy_auth_token(&self) -> Option<&str> {
        self.gateway_token.as_deref()
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
    crate::config::helpers::env_or_override("IRONCLAW_OAUTH_CALLBACK_URL")
        .map(|raw| {
            url::Url::parse(&raw)
                .ok()
                .and_then(|u| u.host_str().map(String::from))
                .map(|host| !is_loopback_host(&host))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// Returns the configured OAuth token-exchange proxy URL, if any.
pub fn exchange_proxy_url() -> Option<String> {
    crate::config::helpers::env_or_override("IRONCLAW_OAUTH_EXCHANGE_URL")
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

/// Returns the configured OAuth proxy auth token, if any.
///
/// New hosted infra can inject a dedicated shared proxy secret via
/// `IRONCLAW_OAUTH_PROXY_AUTH_TOKEN`. Existing hosted instances continue to
/// work by falling back to `GATEWAY_AUTH_TOKEN`.
pub fn oauth_proxy_auth_token() -> Option<String> {
    fn normalized_env_value(key: &str) -> Option<String> {
        crate::config::helpers::env_or_override(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    normalized_env_value("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN")
        .or_else(|| normalized_env_value("GATEWAY_AUTH_TOKEN"))
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

const HOSTED_STATE_PREFIX: &str = "ic2";
const HOSTED_STATE_CHECKSUM_BYTES: usize = 12;

/// Maximum length for a legacy flow ID or instance name.
const LEGACY_STATE_MAX_LEN: usize = 128;
/// Minimum length for a legacy flow ID.
const LEGACY_STATE_MIN_LEN: usize = 8;

/// Validate that a legacy state component (flow_id or instance_name) contains
/// only safe characters: alphanumeric, dash, underscore.
fn is_valid_legacy_state_component(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= LEGACY_STATE_MAX_LEN
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn validate_legacy_flow_id(flow_id: &str) -> Result<(), String> {
    if flow_id.len() < LEGACY_STATE_MIN_LEN {
        return Err(format!(
            "Legacy OAuth flow_id too short ({} chars, minimum {LEGACY_STATE_MIN_LEN})",
            flow_id.len()
        ));
    }
    if flow_id.len() > LEGACY_STATE_MAX_LEN {
        return Err(format!(
            "Legacy OAuth flow_id too long ({} chars, maximum {LEGACY_STATE_MAX_LEN})",
            flow_id.len()
        ));
    }
    if !flow_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Legacy OAuth flow_id contains invalid characters".to_string());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedHostedOAuthState {
    pub flow_id: String,
    pub instance_name: Option<String>,
    pub is_legacy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HostedOAuthStatePayload {
    flow_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instance_name: Option<String>,
    issued_at: u64,
}

fn current_instance_name() -> Option<String> {
    crate::config::helpers::env_or_override("IRONCLAW_INSTANCE_NAME")
        .or_else(|| crate::config::helpers::env_or_override("OPENCLAW_INSTANCE_NAME"))
        .filter(|v| !v.is_empty())
}

fn hosted_state_checksum(payload_bytes: &[u8]) -> String {
    let digest = Sha256::digest(payload_bytes);
    URL_SAFE_NO_PAD.encode(&digest[..HOSTED_STATE_CHECKSUM_BYTES])
}

/// Build a versioned hosted OAuth state envelope.
///
/// The encoded value is opaque to providers and can be decoded by both
/// IronClaw and the external auth proxy for routing and callback lookup.
pub fn encode_hosted_oauth_state(flow_id: &str, instance_name: Option<&str>) -> String {
    let payload = HostedOAuthStatePayload {
        flow_id: flow_id.to_string(),
        instance_name: instance_name
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string),
        issued_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    let payload_json = match serde_json::to_vec(&payload) {
        Ok(payload_json) => payload_json,
        Err(error) => {
            tracing::warn!(%error, flow_id, "Failed to serialize hosted OAuth state payload");
            return payload.flow_id;
        }
    };
    let payload = URL_SAFE_NO_PAD.encode(&payload_json);
    let checksum = hosted_state_checksum(&payload_json);
    format!("{HOSTED_STATE_PREFIX}.{payload}.{checksum}")
}

/// Decode hosted OAuth state in either the new versioned format or the
/// legacy `instance:nonce`/`nonce` forms.
pub fn decode_hosted_oauth_state(state: &str) -> Result<DecodedHostedOAuthState, String> {
    if let Some(rest) = state.strip_prefix(&format!("{HOSTED_STATE_PREFIX}.")) {
        let (payload_b64, checksum) = rest
            .rsplit_once('.')
            .ok_or("Hosted OAuth versioned state missing checksum separator")?;
        let payload_json = URL_SAFE_NO_PAD
            .decode(payload_b64)
            .map_err(|e| format!("Hosted OAuth versioned state base64 decode failed: {e}"))?;
        let expected_checksum = hosted_state_checksum(&payload_json);
        if checksum != expected_checksum {
            return Err("Hosted OAuth state checksum mismatch".to_string());
        }
        let payload: HostedOAuthStatePayload = serde_json::from_slice(&payload_json)
            .map_err(|e| format!("Hosted OAuth versioned state JSON parse failed: {e}"))?;
        if payload.flow_id.trim().is_empty() {
            return Err("Hosted OAuth versioned state has empty flow_id".to_string());
        }
        return Ok(DecodedHostedOAuthState {
            flow_id: payload.flow_id,
            instance_name: payload.instance_name.filter(|v| !v.is_empty()),
            is_legacy: false,
        });
    }

    if let Some((instance_name, flow_id)) = state.split_once(':') {
        if flow_id.is_empty() {
            return Err("Hosted OAuth legacy state is missing flow_id".to_string());
        }
        validate_legacy_flow_id(flow_id)?;
        if !instance_name.is_empty() && !is_valid_legacy_state_component(instance_name) {
            return Err(format!(
                "Legacy OAuth instance name contains invalid characters or exceeds max length ({LEGACY_STATE_MAX_LEN})"
            ));
        }
        tracing::debug!(
            flow_id,
            instance_name,
            "Decoded legacy prefixed OAuth state"
        );
        return Ok(DecodedHostedOAuthState {
            flow_id: flow_id.to_string(),
            instance_name: if instance_name.is_empty() {
                None
            } else {
                Some(instance_name.to_string())
            },
            is_legacy: true,
        });
    }

    if state.is_empty() {
        return Err("Hosted OAuth state is empty".to_string());
    }

    validate_legacy_flow_id(state)?;
    tracing::debug!(flow_id = state, "Decoded legacy raw OAuth state");

    Ok(DecodedHostedOAuthState {
        flow_id: state.to_string(),
        instance_name: None,
        is_legacy: true,
    })
}

/// Build the hosted callback state used by the public OAuth callback endpoint.
///
/// New flows emit a versioned opaque envelope, while callback decoding accepts
/// both the envelope and the legacy `instance:nonce` contract.
pub fn build_platform_state(nonce: &str) -> String {
    encode_hosted_oauth_state(nonce, current_instance_name().as_deref())
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

pub struct ProxyTokenExchangeRequest<'a> {
    pub proxy_url: &'a str,
    /// OAuth proxy auth token.
    /// Kept as `gateway_token` for public API compatibility.
    pub gateway_token: &'a str,
    pub token_url: &'a str,
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
    pub code: &'a str,
    pub redirect_uri: &'a str,
    pub code_verifier: Option<&'a str>,
    pub access_token_field: &'a str,
    pub extra_token_params: &'a HashMap<String, String>,
}

pub struct ProxyRefreshTokenRequest<'a> {
    pub proxy_url: &'a str,
    /// OAuth proxy auth token.
    /// Kept as `gateway_token` for public API compatibility.
    pub gateway_token: &'a str,
    pub token_url: &'a str,
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
    pub refresh_token: &'a str,
    pub resource: Option<&'a str>,
    pub provider: Option<&'a str>,
}

/// Max sane length for an OAuth bearer token. Real-world tokens across
/// Google, GitHub, Notion, Slack, Anthropic, etc. are well under 4 KiB
/// including JWT variants with generous headers/payloads. Anything
/// bigger is almost certainly an HTML page or error blob that the
/// parser extracted as a "token" value — reject it before it gets
/// stored in the secrets store and sent as a `Bearer` header.
const MAX_ACCESS_TOKEN_LEN: usize = 4096;

/// Reject access-token values that look like scraped garbage. A real
/// OAuth access token is a compact opaque string (or JWT) — no
/// whitespace, no HTML/URL brackets, no nulls, bounded length. The
/// form-encoded parser is permissive enough that a random
/// `<input name="access_token" value="<!-- empty -->">` extract
/// would slip through without these checks.
fn validate_access_token(token: &str, access_token_field: &str) -> Result<(), OAuthCallbackError> {
    if token.is_empty() {
        return Err(OAuthCallbackError::Io(format!(
            "Token response '{}' field is empty",
            access_token_field
        )));
    }
    if token.len() > MAX_ACCESS_TOKEN_LEN {
        return Err(OAuthCallbackError::Io(format!(
            "Token response '{}' field is implausibly long ({} bytes > {} byte cap) — likely an error page misparsed as a token",
            access_token_field,
            token.len(),
            MAX_ACCESS_TOKEN_LEN
        )));
    }
    let mut bad_chars: Vec<char> = Vec::new();
    for c in token.chars() {
        // Whitespace, control chars, angle brackets, and NULs have no
        // place in an OAuth bearer token. If any appears, the "token"
        // came from a misparse (HTML / plain-text error page).
        if c.is_whitespace() || c.is_control() || c == '<' || c == '>' {
            bad_chars.push(c);
            if bad_chars.len() >= 3 {
                break;
            }
        }
    }
    if !bad_chars.is_empty() {
        return Err(OAuthCallbackError::Io(format!(
            "Token response '{}' field contains invalid characters — likely an HTML/error-page body misparsed as a token",
            access_token_field
        )));
    }
    Ok(())
}

fn oauth_token_response_from_json(
    token_data: serde_json::Value,
    access_token_field: &str,
) -> Result<OAuthTokenResponse, OAuthCallbackError> {
    let access_token = token_data
        .get(access_token_field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
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
    validate_access_token(&access_token, access_token_field)?;

    let refresh_token = token_data
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(String::from);
    let expires_in = token_data.get("expires_in").and_then(|v| v.as_u64());

    Ok(OAuthTokenResponse {
        access_token,
        refresh_token,
        expires_in,
        token_type: token_data
            .get("token_type")
            .and_then(|v| v.as_str())
            .map(String::from),
        scope: token_data
            .get("scope")
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

fn oauth_token_response_from_form_encoded(
    body: &str,
    access_token_field: &str,
) -> Result<OAuthTokenResponse, OAuthCallbackError> {
    let token_data: HashMap<String, String> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    let access_token = token_data
        .get(access_token_field)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            let fields: Vec<&str> = token_data.keys().map(|k| k.as_str()).collect();
            OAuthCallbackError::Io(format!(
                "No '{}' field in token response (fields present: {:?})",
                access_token_field, fields
            ))
        })?
        .to_string();
    validate_access_token(&access_token, access_token_field)?;

    Ok(OAuthTokenResponse {
        access_token,
        refresh_token: token_data.get("refresh_token").cloned(),
        expires_in: token_data
            .get("expires_in")
            .and_then(|value| value.parse::<u64>().ok()),
        token_type: token_data.get("token_type").cloned(),
        scope: token_data.get("scope").cloned(),
    })
}

/// Classify a response `Content-Type` header value for OAuth token
/// response dispatch. Anything we don't recognise is `Unknown` — the
/// caller falls back to JSON-only, which is the RFC 6749 §5.1 default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenResponseFormat {
    Json,
    FormUrlencoded,
    Unknown,
}

fn classify_token_content_type(content_type: Option<&str>) -> TokenResponseFormat {
    let Some(raw) = content_type else {
        return TokenResponseFormat::Unknown;
    };
    // `Content-Type` can carry parameters like `; charset=UTF-8`; only
    // the media-type prefix matters for dispatch.
    let media = raw
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match media.as_str() {
        "application/json" => TokenResponseFormat::Json,
        "application/x-www-form-urlencoded" => TokenResponseFormat::FormUrlencoded,
        _ => TokenResponseFormat::Unknown,
    }
}

fn oauth_token_response_from_body(
    body: &str,
    access_token_field: &str,
    content_type: Option<&str>,
) -> Result<OAuthTokenResponse, OAuthCallbackError> {
    // Dispatch is content-type-first: the spec (RFC 6749 §5.1) says
    // JSON, GitHub historically sends form-urlencoded, and we should
    // never silently try the form parser on an unknown body — that's
    // the finding reviewer flagged ("HTML error page misparsed as a
    // token"). For unknown / missing content types we default to JSON
    // and surface a clear error on failure, instead of falling through
    // to the permissive form parser.
    match classify_token_content_type(content_type) {
        TokenResponseFormat::FormUrlencoded => {
            oauth_token_response_from_form_encoded(body, access_token_field)
        }
        TokenResponseFormat::Json | TokenResponseFormat::Unknown => {
            let token_data: serde_json::Value =
                serde_json::from_str(body).map_err(|json_error| {
                    OAuthCallbackError::Io(format!(
                        "Failed to parse token response as JSON ({json_error}); \
                         if the provider replies with form-encoded data it MUST set \
                         Content-Type: application/x-www-form-urlencoded."
                    ))
                })?;
            oauth_token_response_from_json(token_data, access_token_field)
        }
    }
}

/// Exchange an OAuth authorization code via the platform's token exchange proxy.
///
/// Authenticated via an OAuth proxy auth token (Bearer header). The caller may
/// either rely on proxy-side secret lookup or forward a `client_secret` when
/// the provider requires it.
///
/// The proxy expects standard OAuth form params plus optional provider-specific
/// token params and returns a standard token response such as
/// `{access_token, refresh_token, expires_in}`.
pub async fn exchange_via_proxy(
    request: ProxyTokenExchangeRequest<'_>,
) -> Result<OAuthTokenResponse, OAuthCallbackError> {
    if request.gateway_token.is_empty() {
        return Err(OAuthCallbackError::Io(
            "OAuth proxy auth token is required for proxy token exchange".to_string(),
        ));
    }
    let exchange_url = format!("{}/oauth/exchange", request.proxy_url.trim_end_matches('/'));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| OAuthCallbackError::Io(format!("Failed to build HTTP client: {}", e)))?;
    let mut params = vec![
        ("code", request.code.to_string()),
        ("redirect_uri", request.redirect_uri.to_string()),
        ("token_url", request.token_url.to_string()),
        ("client_id", request.client_id.to_string()),
        ("access_token_field", request.access_token_field.to_string()),
    ];
    if let Some(verifier) = request.code_verifier {
        params.push(("code_verifier", verifier.to_string()));
    }
    if let Some(secret) = request.client_secret {
        params.push(("client_secret", secret.to_string()));
    }
    for (key, value) in request.extra_token_params {
        params.push((key.as_str(), value.clone()));
    }

    let response = client
        .post(&exchange_url)
        .bearer_auth(request.gateway_token)
        .form(&params)
        .send()
        .await
        .map_err(|e| {
            OAuthCallbackError::Io(format!("Token exchange proxy request failed: {}", e))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let truncated = consume_oauth_error_body(response, 500).await;
        return Err(OAuthCallbackError::Io(format!(
            "Token exchange proxy failed: {} - {}",
            status, truncated
        )));
    }

    let token_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| OAuthCallbackError::Io(format!("Failed to parse proxy response: {}", e)))?;
    oauth_token_response_from_json(token_data, request.access_token_field)
}

/// Refresh an OAuth access token via the platform's token refresh proxy.
///
/// Authenticated via an OAuth proxy auth token (Bearer header). The caller may
/// either rely on proxy-side secret lookup or forward a `client_secret` when
/// the provider requires it.
pub async fn refresh_token_via_proxy(
    request: ProxyRefreshTokenRequest<'_>,
) -> Result<OAuthTokenResponse, OAuthCallbackError> {
    if request.gateway_token.is_empty() {
        return Err(OAuthCallbackError::Io(
            "OAuth proxy auth token is required for proxy token refresh".to_string(),
        ));
    }

    let refresh_url = format!("{}/oauth/refresh", request.proxy_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| OAuthCallbackError::Io(format!("Failed to build HTTP client: {}", e)))?;

    let mut params = vec![
        ("refresh_token", request.refresh_token.to_string()),
        ("token_url", request.token_url.to_string()),
        ("client_id", request.client_id.to_string()),
    ];
    if let Some(secret) = request.client_secret {
        params.push(("client_secret", secret.to_string()));
    }
    if let Some(resource) = request.resource {
        params.push(("resource", resource.to_string()));
    }
    if let Some(provider) = request.provider {
        params.push(("provider", provider.to_string()));
    }

    let response = client
        .post(&refresh_url)
        .bearer_auth(request.gateway_token)
        .form(&params)
        .send()
        .await
        .map_err(|e| {
            OAuthCallbackError::Io(format!("Token refresh proxy request failed: {}", e))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let truncated = consume_oauth_error_body(response, 500).await;
        return Err(OAuthCallbackError::Io(format!(
            "Token refresh proxy failed: {} - {}",
            status, truncated
        )));
    }

    let token_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| OAuthCallbackError::Io(format!("Failed to parse proxy response: {}", e)))?;

    oauth_token_response_from_json(token_data, "access_token")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::sync::Arc;

    use axum::extract::{Form, State};
    use axum::http::HeaderMap;
    use axum::response::Redirect;
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio::sync::{Mutex, oneshot};

    use crate::auth::oauth::{
        builtin_credentials, callback_host, callback_url, is_loopback_host, landing_html,
    };
    use crate::config::helpers::lock_env;
    use crate::testing::credentials::{TEST_OAUTH_CLIENT_ID, TEST_OAUTH_CLIENT_SECRET};

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RecordedProxyRequest {
        authorization: Option<String>,
        form: HashMap<String, String>,
    }

    #[derive(Clone)]
    struct MockProxyState {
        requests: Arc<Mutex<Vec<RecordedProxyRequest>>>,
        exchange_redirect_target: String,
        refresh_redirect_target: String,
    }

    struct MockProxyServer {
        addr: SocketAddr,
        requests: Arc<Mutex<Vec<RecordedProxyRequest>>>,
        shutdown_tx: Option<oneshot::Sender<()>>,
        server_task: Option<tokio::task::JoinHandle<()>>,
    }

    impl MockProxyServer {
        async fn start() -> Self {
            async fn exchange_handler(
                State(state): State<MockProxyState>,
                headers: HeaderMap,
                Form(form): Form<HashMap<String, String>>,
            ) -> Json<serde_json::Value> {
                state.requests.lock().await.push(RecordedProxyRequest {
                    authorization: headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string),
                    form,
                });
                Json(json!({
                    "access_token": "proxy-access-token",
                    "token_type": "Bearer",
                    "refresh_token": "proxy-refresh-token",
                    "expires_in": 7200,
                    "scope": "scope-a scope-b"
                }))
            }

            async fn refresh_handler(
                State(state): State<MockProxyState>,
                headers: HeaderMap,
                Form(form): Form<HashMap<String, String>>,
            ) -> Json<serde_json::Value> {
                state.requests.lock().await.push(RecordedProxyRequest {
                    authorization: headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string),
                    form,
                });
                Json(json!({
                    "access_token": "proxy-access-token",
                    "token_type": "Bearer",
                    "refresh_token": "proxy-refresh-token",
                    "expires_in": 7200,
                    "scope": "scope-a scope-b"
                }))
            }

            async fn exchange_redirect_handler(State(state): State<MockProxyState>) -> Redirect {
                Redirect::temporary(&state.exchange_redirect_target)
            }

            async fn refresh_redirect_handler(State(state): State<MockProxyState>) -> Redirect {
                Redirect::temporary(&state.refresh_redirect_target)
            }

            let requests = Arc::new(Mutex::new(Vec::new()));
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock proxy");
            let addr = listener.local_addr().expect("read mock proxy addr");
            let exchange_redirect_target = format!("http://{addr}/oauth/exchange");
            let refresh_redirect_target = format!("http://{addr}/oauth/refresh");
            let app = Router::new()
                .route("/oauth/exchange", post(exchange_handler))
                .route("/oauth/refresh", post(refresh_handler))
                .route("/redirect/oauth/exchange", post(exchange_redirect_handler))
                .route("/redirect/oauth/refresh", post(refresh_redirect_handler))
                .with_state(MockProxyState {
                    requests: Arc::clone(&requests),
                    exchange_redirect_target,
                    refresh_redirect_target,
                });
            let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
            let server_task = tokio::spawn(async move {
                let _ = axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await;
            });

            Self {
                addr,
                requests,
                shutdown_tx: Some(shutdown_tx),
                server_task: Some(server_task),
            }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }

        fn redirecting_base_url(&self) -> String {
            format!("{}/redirect", self.base_url())
        }

        async fn requests(&self) -> Vec<RecordedProxyRequest> {
            self.requests.lock().await.clone()
        }

        async fn shutdown(mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(task) = self.server_task.take() {
                let _ = task.await;
            }
        }
    }

    impl Drop for MockProxyServer {
        fn drop(&mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(task) = self.server_task.take() {
                task.abort();
            }
        }
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: Under ENV_MUTEX, no concurrent env access.
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
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
        EnvVarGuard { key, original }
    }

    #[test]
    fn test_hosted_proxy_client_secret_suppresses_builtin_secret() {
        let builtin = builtin_credentials("google_oauth_token").expect("google builtin creds");
        let client_secret = Some(builtin.client_secret.to_string());

        let result = super::hosted_proxy_client_secret(&client_secret, Some(&builtin), true);

        assert_eq!(result, None);
    }

    #[test]
    fn test_hosted_proxy_client_secret_preserves_explicit_secret() {
        let builtin = builtin_credentials("google_oauth_token").expect("google builtin creds");
        let client_secret = Some("hosted-server-secret".to_string());

        let result = super::hosted_proxy_client_secret(&client_secret, Some(&builtin), true);

        assert_eq!(result, client_secret);
    }

    #[tokio::test]
    async fn test_exchange_via_proxy_sends_auth_and_form() {
        let server = MockProxyServer::start().await;
        let mut extra_token_params = HashMap::new();
        extra_token_params.insert("resource".to_string(), "https://mcp.notion.com".to_string());

        let response = super::exchange_via_proxy(super::ProxyTokenExchangeRequest {
            proxy_url: &server.base_url(),
            gateway_token: "shared-oauth-proxy-secret",
            code: "auth-code-123",
            redirect_uri: "https://oauth.example.com/oauth/callback",
            token_url: "https://oauth2.googleapis.com/token",
            client_id: TEST_OAUTH_CLIENT_ID,
            client_secret: Some(TEST_OAUTH_CLIENT_SECRET),
            access_token_field: "access_token",
            code_verifier: Some("code-verifier-123"),
            extra_token_params: &extra_token_params,
        })
        .await
        .expect("proxy exchange succeeds");

        assert_eq!(response.access_token, "proxy-access-token");
        assert_eq!(
            response.refresh_token.as_deref(),
            Some("proxy-refresh-token")
        );
        assert_eq!(response.expires_in, Some(7200));
        assert_eq!(response.token_type.as_deref(), Some("Bearer"));
        assert_eq!(response.scope.as_deref(), Some("scope-a scope-b"));

        let requests = server.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer shared-oauth-proxy-secret")
        );
        assert_eq!(
            requests[0].form.get("code").map(String::as_str),
            Some("auth-code-123")
        );
        assert_eq!(
            requests[0].form.get("redirect_uri").map(String::as_str),
            Some("https://oauth.example.com/oauth/callback")
        );
        assert_eq!(
            requests[0].form.get("token_url").map(String::as_str),
            Some("https://oauth2.googleapis.com/token")
        );
        assert_eq!(
            requests[0].form.get("client_id").map(String::as_str),
            Some(TEST_OAUTH_CLIENT_ID)
        );
        assert_eq!(
            requests[0].form.get("client_secret").map(String::as_str),
            Some(TEST_OAUTH_CLIENT_SECRET)
        );
        assert_eq!(
            requests[0]
                .form
                .get("access_token_field")
                .map(String::as_str),
            Some("access_token")
        );
        assert_eq!(
            requests[0].form.get("code_verifier").map(String::as_str),
            Some("code-verifier-123")
        );
        assert_eq!(
            requests[0].form.get("resource").map(String::as_str),
            Some("https://mcp.notion.com")
        );

        server.shutdown().await;
    }

    #[test]
    fn test_github_form_encoded_token_response_parses() {
        let token_data = super::oauth_token_response_from_form_encoded(
            "access_token=github-access-token&token_type=bearer&scope=repo%20workflow",
            "access_token",
        )
        .expect("GitHub-style form-encoded token response should parse");

        assert_eq!(token_data.access_token, "github-access-token");
        assert_eq!(token_data.token_type.as_deref(), Some("bearer"));
        assert_eq!(token_data.scope.as_deref(), Some("repo workflow"));
    }

    /// Regression: the token-response dispatcher must NOT silently fall
    /// through to the permissive form-encoded parser when the upstream
    /// returned an HTML error page (or any non-form Content-Type). The
    /// pre-fix code tried JSON first, then form-encoded — and
    /// `url::form_urlencoded::parse` happily accepts any string, so a
    /// response like `<input name="access_token" value="">` or a plain
    /// error body containing `access_token=...` substring was silently
    /// stored as a token and later sent as a `Bearer` header.
    /// Helper: extract the error message from a token-response result
    /// without requiring `OAuthTokenResponse: Debug` (the type
    /// deliberately doesn't derive `Debug` to avoid leaking token
    /// material into panic output).
    fn expect_token_error(
        result: Result<super::OAuthTokenResponse, super::OAuthCallbackError>,
        what: &str,
    ) -> String {
        match result {
            Ok(_) => panic!("{what}: expected error, got Ok"),
            Err(super::OAuthCallbackError::Io(m)) => m,
            Err(other) => panic!("{what}: unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn test_html_error_page_is_rejected_without_form_content_type() {
        let body = "<html><body><h1>502 Bad Gateway</h1>\
                    <p>Token server is down</p>\
                    <input name=\"access_token\" value=\"abc\"/></body></html>";
        let msg = expect_token_error(
            super::oauth_token_response_from_body(body, "access_token", None),
            "HTML body without form content-type must NOT parse as a token",
        );
        assert!(
            msg.contains("Failed to parse token response as JSON"),
            "must surface the JSON parse error + a form-encoded hint, got: {msg}"
        );
    }

    #[test]
    fn test_plaintext_body_with_token_substring_is_rejected_without_form_content_type() {
        // Looks scrape-able to the form-encoded parser
        // (access_token=... appears inline) but Content-Type is absent,
        // so the dispatcher must not reach the form parser.
        let body = "Rate limit exceeded for access_token=leaked-value.";
        let _ = expect_token_error(
            super::oauth_token_response_from_body(body, "access_token", None),
            "plaintext body w/ substring but no form content-type must error",
        );
    }

    #[test]
    fn test_html_body_with_explicit_form_content_type_still_rejected_by_validator() {
        // A hostile / misconfigured provider could send HTML with a
        // `Content-Type: application/x-www-form-urlencoded` header. The
        // form parser would then happily extract a string with `<` in
        // it — defense-in-depth: `validate_access_token` rejects it.
        let body = "access_token=<html>garbage</html>&token_type=bearer";
        let msg = expect_token_error(
            super::oauth_token_response_from_body(
                body,
                "access_token",
                Some("application/x-www-form-urlencoded"),
            ),
            "HTML-ish value must be rejected by the validator",
        );
        assert!(
            msg.contains("invalid characters"),
            "expected validator rejection, got: {msg}"
        );
    }

    #[test]
    fn test_github_form_response_parses_when_content_type_set() {
        let body = "access_token=gho_github-access-token&token_type=bearer&scope=repo%20workflow";
        let token_data = super::oauth_token_response_from_body(
            body,
            "access_token",
            Some("application/x-www-form-urlencoded; charset=utf-8"),
        )
        .expect("GitHub-style response with correct content-type still parses");
        assert_eq!(token_data.access_token, "gho_github-access-token");
        assert_eq!(token_data.token_type.as_deref(), Some("bearer"));
        assert_eq!(token_data.scope.as_deref(), Some("repo workflow"));
    }

    #[test]
    fn test_json_response_parses_when_content_type_missing() {
        // Most real providers send `Content-Type: application/json`,
        // but some (or network layers) may strip it. JSON is the RFC
        // 6749 §5.1 default, so we still try to parse JSON when the
        // header is absent — just not form-encoded.
        let body = r#"{"access_token":"jwt-token","token_type":"Bearer","expires_in":3600}"#;
        let token_data = super::oauth_token_response_from_body(body, "access_token", None)
            .expect("JSON body with no content-type defaults to JSON parse");
        assert_eq!(token_data.access_token, "jwt-token");
        assert_eq!(token_data.expires_in, Some(3600));
    }

    #[test]
    fn test_oversized_token_value_is_rejected() {
        let mut body = String::from("{\"access_token\":\"");
        body.push_str(&"A".repeat(super::MAX_ACCESS_TOKEN_LEN + 1));
        body.push_str("\",\"token_type\":\"Bearer\"}");
        let msg = expect_token_error(
            super::oauth_token_response_from_body(&body, "access_token", Some("application/json")),
            "implausibly long token must be rejected",
        );
        assert!(msg.contains("implausibly long"), "got: {msg}");
    }

    #[test]
    fn test_whitespace_in_token_is_rejected() {
        let body = r#"{"access_token":"some token with spaces","token_type":"Bearer"}"#;
        let _ = expect_token_error(
            super::oauth_token_response_from_body(body, "access_token", Some("application/json")),
            "tokens must not contain whitespace",
        );
    }

    #[test]
    fn test_classify_content_type_ignores_charset_and_case() {
        use super::{TokenResponseFormat, classify_token_content_type};
        assert_eq!(
            classify_token_content_type(Some("Application/JSON; charset=UTF-8")),
            TokenResponseFormat::Json
        );
        assert_eq!(
            classify_token_content_type(Some("application/x-www-form-urlencoded")),
            TokenResponseFormat::FormUrlencoded
        );
        assert_eq!(
            classify_token_content_type(Some("text/html")),
            TokenResponseFormat::Unknown
        );
        assert_eq!(
            classify_token_content_type(None),
            TokenResponseFormat::Unknown
        );
    }

    #[tokio::test]
    async fn test_refresh_token_via_proxy_sends_auth_and_form() {
        let server = MockProxyServer::start().await;

        let response = super::refresh_token_via_proxy(super::ProxyRefreshTokenRequest {
            proxy_url: &server.base_url(),
            gateway_token: "gateway-test-token",
            token_url: "https://oauth2.googleapis.com/token",
            client_id: TEST_OAUTH_CLIENT_ID,
            client_secret: Some(TEST_OAUTH_CLIENT_SECRET),
            refresh_token: "refresh-token-123",
            resource: Some("https://mcp.notion.com"),
            provider: Some("google"),
        })
        .await
        .expect("proxy refresh succeeds");

        assert_eq!(response.access_token, "proxy-access-token");
        assert_eq!(
            response.refresh_token.as_deref(),
            Some("proxy-refresh-token")
        );
        assert_eq!(response.expires_in, Some(7200));
        assert_eq!(response.token_type.as_deref(), Some("Bearer"));
        assert_eq!(response.scope.as_deref(), Some("scope-a scope-b"));

        let requests = server.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer gateway-test-token")
        );
        assert_eq!(
            requests[0].form.get("token_url").map(String::as_str),
            Some("https://oauth2.googleapis.com/token")
        );
        assert_eq!(
            requests[0].form.get("client_id").map(String::as_str),
            Some(TEST_OAUTH_CLIENT_ID)
        );
        assert_eq!(
            requests[0].form.get("client_secret").map(String::as_str),
            Some(TEST_OAUTH_CLIENT_SECRET)
        );
        assert_eq!(
            requests[0].form.get("refresh_token").map(String::as_str),
            Some("refresh-token-123")
        );
        assert_eq!(
            requests[0].form.get("provider").map(String::as_str),
            Some("google")
        );
        assert_eq!(
            requests[0].form.get("resource").map(String::as_str),
            Some("https://mcp.notion.com")
        );

        server.shutdown().await;
    }

    #[tokio::test]
    async fn test_exchange_via_proxy_does_not_follow_redirects() {
        let server = MockProxyServer::start().await;

        let error = match super::exchange_via_proxy(super::ProxyTokenExchangeRequest {
            proxy_url: &server.redirecting_base_url(),
            gateway_token: "gateway-test-token",
            code: "auth-code-123",
            redirect_uri: "http://localhost:3000/oauth/callback",
            token_url: "https://oauth2.googleapis.com/token",
            client_id: TEST_OAUTH_CLIENT_ID,
            client_secret: Some(TEST_OAUTH_CLIENT_SECRET),
            access_token_field: "access_token",
            code_verifier: Some("code-verifier-123"),
            extra_token_params: &HashMap::new(),
        })
        .await
        {
            Ok(_) => panic!("redirected proxy exchange should fail"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("307"));
        assert!(server.requests().await.is_empty());

        server.shutdown().await;
    }

    #[tokio::test]
    async fn test_refresh_token_via_proxy_does_not_follow_redirects() {
        let server = MockProxyServer::start().await;

        let error = match super::refresh_token_via_proxy(super::ProxyRefreshTokenRequest {
            proxy_url: &server.redirecting_base_url(),
            gateway_token: "gateway-test-token",
            token_url: "https://oauth2.googleapis.com/token",
            client_id: TEST_OAUTH_CLIENT_ID,
            client_secret: Some(TEST_OAUTH_CLIENT_SECRET),
            refresh_token: "refresh-token-123",
            resource: Some("https://mcp.notion.com"),
            provider: Some("google"),
        })
        .await
        {
            Ok(_) => panic!("redirected proxy refresh should fail"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("307"));
        assert!(server.requests().await.is_empty());

        server.shutdown().await;
    }

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
        let _guard = lock_env();
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
        let _guard = lock_env();
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
        let _guard = lock_env();
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
        let _guard = lock_env();
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

        use crate::auth::oauth::build_oauth_url;

        let result = build_oauth_url(
            "https://accounts.google.com/o/oauth2/auth",
            "my-client-id",
            "http://localhost:9876/callback",
            &["openid".to_string(), "email".to_string()],
            false,
            &HashMap::new(),
        )
        .expect("well-formed authorization URL");

        assert!(
            result
                .url
                .starts_with("https://accounts.google.com/o/oauth2/auth?")
        );
        assert!(result.url.contains("client_id=my-client-id"));
        assert!(result.url.contains("response_type=code"));
        assert!(result.url.contains("redirect_uri="));
        // `url` crate uses `application/x-www-form-urlencoded` encoding for
        // query parameters, which encodes spaces as `+`.
        assert!(result.url.contains("scope=openid+email"));
        assert!(result.url.contains("state="));
        assert!(result.code_verifier.is_none());
        assert!(!result.state.is_empty());
    }

    #[test]
    fn test_build_oauth_url_with_pkce() {
        use std::collections::HashMap;

        use crate::auth::oauth::build_oauth_url;

        let result = build_oauth_url(
            "https://auth.example.com/authorize",
            "client-123",
            "http://localhost:9876/callback",
            &[],
            true,
            &HashMap::new(),
        )
        .expect("well-formed authorization URL");

        assert!(result.url.contains("code_challenge="));
        assert!(result.url.contains("code_challenge_method=S256"));
        assert!(result.code_verifier.is_some());
        let verifier = result.code_verifier.unwrap();
        assert!(!verifier.is_empty());
    }

    #[test]
    fn test_build_oauth_url_with_extra_params() {
        use std::collections::HashMap;

        use crate::auth::oauth::build_oauth_url;

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
        )
        .expect("well-formed authorization URL");

        assert!(result.url.contains("access_type=offline"));
        assert!(result.url.contains("prompt=consent"));
    }

    /// Regression test for nearai/ironclaw#2391: Google OAuth was receiving
    /// `access_type=offlin` instead of `access_type=offline`, breaking the
    /// offline-token flow required for any Google Workspace tool (Calendar,
    /// Gmail, Drive, Docs, Sheets, Slides). The bug was reproducibly seen by
    /// end users but not caught by tests that only used `.contains()`, since
    /// `"access_type=offlin"` is a prefix of `"access_type=offline"` when the
    /// URL ended elsewhere. We now parse the URL and compare each query
    /// parameter value *exactly*.
    #[test]
    fn test_build_oauth_url_preserves_access_type_offline_exactly() {
        use std::collections::HashMap;

        use crate::auth::oauth::build_oauth_url;

        let mut extra = HashMap::new();
        extra.insert("access_type".to_string(), "offline".to_string());
        extra.insert("prompt".to_string(), "consent".to_string());

        let result = build_oauth_url(
            "https://accounts.google.com/o/oauth2/v2/auth",
            "test-client-id.apps.googleusercontent.com",
            "http://127.0.0.1:9876/callback",
            &["https://www.googleapis.com/auth/calendar.events".to_string()],
            false,
            &extra,
        )
        .expect("well-formed authorization URL");

        let parsed = url::Url::parse(&result.url).expect("auth url must be valid");
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();

        assert_eq!(
            params.get("access_type").map(String::as_str),
            Some("offline"),
            "access_type must be exactly 'offline' (7 chars), not a truncated value; \
             got {:?} in URL {}",
            params.get("access_type"),
            result.url,
        );
        assert_eq!(
            params.get("prompt").map(String::as_str),
            Some("consent"),
            "prompt must be exactly 'consent', not a truncated value"
        );
        assert_eq!(
            params.get("client_id").map(String::as_str),
            Some("test-client-id.apps.googleusercontent.com")
        );
        assert_eq!(
            params.get("response_type").map(String::as_str),
            Some("code")
        );
        assert_eq!(
            params.get("redirect_uri").map(String::as_str),
            Some("http://127.0.0.1:9876/callback")
        );
        assert_eq!(
            params.get("scope").map(String::as_str),
            Some("https://www.googleapis.com/auth/calendar.events")
        );
    }

    /// Regression test for nearai/ironclaw#2391: exercise the full set of
    /// Google-style extra params (all six Google WASM tools share this
    /// shape) and verify every value survives URL encoding intact.
    ///
    /// A single `HashMap` instance's iteration order is stable — reusing the
    /// same map in a loop would not actually exercise different orderings
    /// (Copilot review on #2746). We therefore rebuild `extra` on every
    /// iteration so the randomized default hasher produces a fresh seed per
    /// map, *and* explicitly drive every key-insertion permutation so each
    /// param lands last at least once regardless of hasher behavior.
    #[test]
    fn test_build_oauth_url_extra_params_preserve_all_chars_across_hash_orderings() {
        use std::collections::HashMap;

        use crate::auth::oauth::build_oauth_url;

        let entries: [(&str, &str); 3] = [
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("include_granted_scopes", "true"),
        ];

        // Every permutation of insertion order (3! = 6), plus a few
        // fresh-map iterations per permutation so the randomized hasher
        // also contributes variation.
        let permutations: [[usize; 3]; 6] = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];

        let mut case = 0usize;
        for perm in &permutations {
            for _ in 0..3 {
                let mut extra: HashMap<String, String> = HashMap::new();
                for &idx in perm {
                    let (k, v) = entries[idx];
                    extra.insert(k.to_string(), v.to_string());
                }

                let result = build_oauth_url(
                    "https://accounts.google.com/o/oauth2/v2/auth",
                    "client-id",
                    "http://127.0.0.1:9876/callback",
                    &["https://www.googleapis.com/auth/gmail.modify".to_string()],
                    false,
                    &extra,
                )
                .expect("well-formed authorization URL");

                let parsed = url::Url::parse(&result.url).expect("auth url must be valid");
                let params: std::collections::HashMap<_, _> =
                    parsed.query_pairs().into_owned().collect();

                for (k, v) in entries {
                    assert_eq!(
                        params.get(k).map(String::as_str),
                        Some(v),
                        "case {case} (perm {perm:?}): {k} truncated to {:?} in {}",
                        params.get(k),
                        result.url,
                    );
                }
                case += 1;
            }
        }
    }

    /// Regression test for nearai/ironclaw#2391: exercise the full CLI
    /// `ironclaw tool auth google-calendar` code path end-to-end. Loads the
    /// actual shipped capabilities JSON, parses it via
    /// `CapabilitiesFile::from_json`, then calls `build_oauth_url` with the
    /// exact `extra_params` the CLI would pass — the same call site as
    /// `cli/tool.rs::auth_tool_oauth`. Per `.claude/rules/testing.md`
    /// "Test Through the Caller, Not Just the Helper": a unit test on
    /// `build_oauth_url` alone can miss a bug in the pipeline from
    /// capabilities-JSON parsing to URL construction.
    #[test]
    fn test_google_calendar_capabilities_produce_correct_oauth_url() {
        use crate::auth::oauth::build_oauth_url;
        use crate::tools::wasm::CapabilitiesFile;

        // Pinned snapshot of the production google-calendar capabilities.
        // Keep this byte-identical to tools-src/google-calendar/
        // google-calendar-tool.capabilities.json for the relevant fields.
        let caps_json = r#"{
            "version": "0.2.0",
            "description": "Google Calendar test fixture",
            "auth": {
                "secret_name": "google_oauth_token",
                "display_name": "Google",
                "oauth": {
                    "authorization_url": "https://accounts.google.com/o/oauth2/v2/auth",
                    "token_url": "https://oauth2.googleapis.com/token",
                    "client_id_env": "GOOGLE_OAUTH_CLIENT_ID",
                    "client_secret_env": "GOOGLE_OAUTH_CLIENT_SECRET",
                    "scopes": [
                        "https://www.googleapis.com/auth/calendar.events"
                    ],
                    "use_pkce": false,
                    "extra_params": {
                        "access_type": "offline",
                        "prompt": "consent"
                    }
                },
                "env_var": "GOOGLE_OAUTH_TOKEN"
            }
        }"#;

        let caps = CapabilitiesFile::from_json(caps_json)
            .expect("google-calendar capabilities parse must succeed");
        let oauth = caps
            .auth
            .as_ref()
            .expect("auth section present")
            .oauth
            .as_ref()
            .expect("oauth section present");

        // Sanity: the parsed extra_params haven't been mutated at load time.
        assert_eq!(
            oauth.extra_params.get("access_type").map(String::as_str),
            Some("offline"),
            "CapabilitiesFile::from_json must preserve access_type=offline \
             intact; got {:?}",
            oauth.extra_params.get("access_type"),
        );

        let result = build_oauth_url(
            &oauth.authorization_url,
            "test-client.apps.googleusercontent.com",
            "http://127.0.0.1:9876/callback",
            &oauth.scopes,
            oauth.use_pkce,
            &oauth.extra_params,
        )
        .expect("well-formed authorization URL");

        let parsed = url::Url::parse(&result.url).expect("auth url must be valid");
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();

        assert_eq!(
            params.get("access_type").map(String::as_str),
            Some("offline"),
            "end-to-end: google-calendar must send access_type=offline to \
             Google, not a truncated value. Full URL: {}",
            result.url
        );
        assert_eq!(params.get("prompt").map(String::as_str), Some("consent"));
        assert_eq!(
            params.get("scope").map(String::as_str),
            Some("https://www.googleapis.com/auth/calendar.events")
        );
    }

    #[test]
    fn test_build_oauth_url_state_is_unique() {
        use std::collections::HashMap;

        use crate::auth::oauth::build_oauth_url;

        let result1 = build_oauth_url(
            "https://auth.example.com/authorize",
            "client",
            "http://localhost:9876/callback",
            &[],
            false,
            &HashMap::new(),
        )
        .expect("well-formed authorization URL");
        let result2 = build_oauth_url(
            "https://auth.example.com/authorize",
            "client",
            "http://localhost:9876/callback",
            &[],
            false,
            &HashMap::new(),
        )
        .expect("well-formed authorization URL");

        // State should be different each time (random)
        assert_ne!(result1.state, result2.state);
    }

    /// Malformed `authorization_url` values must be rejected with
    /// `OAuthUrlError::MalformedConfig`, not silently normalized through
    /// string concatenation (gemini-code-assist review on #2746).
    #[test]
    fn test_build_oauth_url_rejects_malformed_authorization_url() {
        use std::collections::HashMap;

        use crate::auth::oauth::{OAuthUrlError, build_oauth_url};

        let err = build_oauth_url(
            "not a url",
            "client",
            "http://localhost:9876/callback",
            &[],
            false,
            &HashMap::new(),
        )
        .err()
        .expect("malformed authorization URL must be rejected");

        assert!(
            matches!(err, OAuthUrlError::MalformedConfig(_)),
            "expected MalformedConfig, got {err:?}",
        );
    }

    #[test]
    fn test_use_gateway_callback_false_by_default() {
        let _guard = lock_env();
        let original = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("IRONCLAW_OAUTH_CALLBACK_URL");
        }
        assert!(!crate::auth::oauth::use_gateway_callback());
        unsafe {
            if let Some(val) = original {
                std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", val);
            }
        }
    }

    #[test]
    fn test_use_gateway_callback_true_for_hosted() {
        let _guard = lock_env();
        let original = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var(
                "IRONCLAW_OAUTH_CALLBACK_URL",
                "https://kind-deer.agent1.near.ai",
            );
        }
        assert!(crate::auth::oauth::use_gateway_callback());
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
        let _guard = lock_env();
        let original = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", "http://127.0.0.1:3001");
        }
        assert!(!crate::auth::oauth::use_gateway_callback());
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
        let _guard = lock_env();
        let original = std::env::var("IRONCLAW_OAUTH_CALLBACK_URL").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("IRONCLAW_OAUTH_CALLBACK_URL", "");
        }
        assert!(!crate::auth::oauth::use_gateway_callback());
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
        use crate::auth::oauth::{build_platform_state, decode_hosted_oauth_state};

        let _guard = lock_env();
        let original = std::env::var("IRONCLAW_INSTANCE_NAME").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("IRONCLAW_INSTANCE_NAME", "kind-deer");
        }
        let encoded = build_platform_state("abc123");
        let decoded = decode_hosted_oauth_state(&encoded).expect("decode hosted state");
        assert_eq!(decoded.flow_id, "abc123");
        assert_eq!(decoded.instance_name.as_deref(), Some("kind-deer"));
        assert!(!decoded.is_legacy);
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
        use crate::auth::oauth::{build_platform_state, decode_hosted_oauth_state};

        let _guard = lock_env();
        let original = std::env::var("IRONCLAW_INSTANCE_NAME").ok();
        let original_oc = std::env::var("OPENCLAW_INSTANCE_NAME").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("IRONCLAW_INSTANCE_NAME");
            std::env::remove_var("OPENCLAW_INSTANCE_NAME");
        }
        let encoded = build_platform_state("abc123");
        let decoded = decode_hosted_oauth_state(&encoded).expect("decode hosted state");
        assert_eq!(decoded.flow_id, "abc123");
        assert_eq!(decoded.instance_name, None);
        assert!(!decoded.is_legacy);
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
        use crate::auth::oauth::{build_platform_state, decode_hosted_oauth_state};

        let _guard = lock_env();
        let original_ic = std::env::var("IRONCLAW_INSTANCE_NAME").ok();
        let original_oc = std::env::var("OPENCLAW_INSTANCE_NAME").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("IRONCLAW_INSTANCE_NAME");
            std::env::set_var("OPENCLAW_INSTANCE_NAME", "quiet-lion");
        }
        let encoded = build_platform_state("xyz789");
        let decoded = decode_hosted_oauth_state(&encoded).expect("decode hosted state");
        assert_eq!(decoded.flow_id, "xyz789");
        assert_eq!(decoded.instance_name.as_deref(), Some("quiet-lion"));
        assert!(!decoded.is_legacy);
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
    fn test_oauth_proxy_auth_token_prefers_dedicated_env() {
        let _guard = lock_env();
        let _proxy_guard = set_env_var(
            "IRONCLAW_OAUTH_PROXY_AUTH_TOKEN",
            Some("shared-proxy-secret"),
        );
        let _gateway_guard = set_env_var("GATEWAY_AUTH_TOKEN", Some("gateway-token"));

        assert_eq!(
            crate::auth::oauth::oauth_proxy_auth_token().as_deref(),
            Some("shared-proxy-secret")
        );
    }

    #[test]
    fn test_oauth_proxy_auth_token_falls_back_to_gateway_token() {
        let _guard = lock_env();
        let _proxy_guard = set_env_var("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN", None);
        let _gateway_guard = set_env_var("GATEWAY_AUTH_TOKEN", Some("gateway-token"));

        assert_eq!(
            crate::auth::oauth::oauth_proxy_auth_token().as_deref(),
            Some("gateway-token")
        );
    }

    #[test]
    fn test_oauth_proxy_auth_token_whitespace_dedicated_env_falls_back_to_gateway_token() {
        let _guard = lock_env();
        let _proxy_guard = set_env_var("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN", Some("   "));
        let _gateway_guard = set_env_var("GATEWAY_AUTH_TOKEN", Some("gateway-token"));

        assert_eq!(
            crate::auth::oauth::oauth_proxy_auth_token().as_deref(),
            Some("gateway-token")
        );
    }

    #[test]
    fn test_oauth_proxy_auth_token_returns_none_when_unset() {
        let _guard = lock_env();
        let _proxy_guard = set_env_var("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN", None);
        let _gateway_guard = set_env_var("GATEWAY_AUTH_TOKEN", None);

        assert_eq!(crate::auth::oauth::oauth_proxy_auth_token(), None);
    }

    #[test]
    fn test_strip_instance_prefix_with_colon() {
        use crate::auth::oauth::strip_instance_prefix;

        assert_eq!(strip_instance_prefix("kind-deer:abc123"), "abc123");
        assert_eq!(strip_instance_prefix("my-instance:xyz"), "xyz");
    }

    #[test]
    fn test_strip_instance_prefix_without_colon() {
        use crate::auth::oauth::strip_instance_prefix;

        assert_eq!(strip_instance_prefix("abc123"), "abc123");
        assert_eq!(strip_instance_prefix(""), "");
    }

    #[test]
    fn test_decode_hosted_oauth_state_accepts_legacy_formats() {
        use crate::auth::oauth::decode_hosted_oauth_state;

        let decoded = decode_hosted_oauth_state("kind-deer:abc12345").expect("legacy prefixed");
        assert_eq!(decoded.flow_id, "abc12345");
        assert_eq!(decoded.instance_name.as_deref(), Some("kind-deer"));
        assert!(decoded.is_legacy);

        let decoded = decode_hosted_oauth_state("abc12345").expect("legacy raw");
        assert_eq!(decoded.flow_id, "abc12345");
        assert_eq!(decoded.instance_name, None);
        assert!(decoded.is_legacy);
    }

    #[test]
    fn test_decode_hosted_oauth_state_rejects_non_envelope_ic2_prefix() {
        use crate::auth::oauth::decode_hosted_oauth_state;

        // "ic2." prefix must parse as a valid versioned envelope — never fall
        // through to legacy handling, which would use the full malformed
        // envelope as the flow_id and break OAuth callback lookup (#1441).
        decode_hosted_oauth_state("ic2.provider-owned-state")
            .expect_err("ic2-prefixed non-envelope state should fail");
    }

    #[test]
    fn test_decode_hosted_oauth_state_rejects_tampered_checksum() {
        use crate::auth::oauth::{decode_hosted_oauth_state, encode_hosted_oauth_state};

        let encoded = encode_hosted_oauth_state("abc123", Some("kind-deer"));
        let tampered = format!("{encoded}broken");
        let err = decode_hosted_oauth_state(&tampered).expect_err("tampered state should fail");
        assert!(err.contains("checksum"), "unexpected error: {err}");
    }

    /// Verify that `build_oauth_url` includes the RFC 8707 `resource` parameter
    /// when passed through `extra_params`, which is how MCP OAuth gateway mode
    /// scopes tokens to a specific MCP server.
    #[test]
    fn test_build_oauth_url_includes_resource_via_extra_params() {
        use std::collections::HashMap;

        use crate::auth::oauth::build_oauth_url;

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
        )
        .expect("well-formed authorization URL");

        // The resource parameter should be URL-encoded in the auth URL
        assert!(
            result
                .url
                .contains("resource=https%3A%2F%2Fmcp.example.com"),
            "Expected resource param in URL: {}",
            result.url
        );
        // State and PKCE should be present
        assert!(result.url.contains("state="));
        assert!(result.url.contains("code_challenge="));
        assert!(result.code_verifier.is_some());
    }

    /// Malformed `ic2.*` states must return Err, never fall through to legacy
    /// handling where the full envelope would be used as the flow_id (#1441).
    #[test]
    fn test_decode_versioned_state_rejects_malformed_envelopes() {
        use crate::auth::oauth::decode_hosted_oauth_state;

        // Missing checksum separator (no second dot after prefix)
        let err =
            decode_hosted_oauth_state("ic2.nodots").expect_err("missing separator should fail");
        assert!(
            err.contains("checksum separator"),
            "unexpected error: {err}"
        );

        // Bad base64 payload
        let err = decode_hosted_oauth_state("ic2.!!!badbase64!!!.fakechecksum")
            .expect_err("bad base64 should fail");
        assert!(err.contains("base64"), "unexpected error: {err}");

        // Valid base64 but not JSON: use correct checksum so we exercise JSON parsing
        use base64::Engine;
        use sha2::Digest;
        let not_json_bytes = b"not json";
        let not_json_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(not_json_bytes);
        let digest = sha2::Sha256::digest(not_json_bytes);
        let checksum = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(&digest[..super::HOSTED_STATE_CHECKSUM_BYTES]);
        let err = decode_hosted_oauth_state(&format!("ic2.{not_json_b64}.{checksum}"))
            .expect_err("non-JSON payload should fail with JSON parse error");
        assert!(
            err.contains("JSON"),
            "unexpected error (expected JSON parse failure): {err}"
        );
    }

    /// Round-trip: encode_hosted_oauth_state(nonce) → decode → flow_id == nonce.
    /// Ensures the registration key and lookup key are always identical (#1441).
    #[test]
    fn test_oauth_flow_key_round_trip_consistency() {
        use crate::auth::oauth::{decode_hosted_oauth_state, encode_hosted_oauth_state};

        let nonce = "test-nonce-abc123";
        let encoded = encode_hosted_oauth_state(nonce, Some("my-instance"));
        let decoded = decode_hosted_oauth_state(&encoded).expect("round-trip decode");

        assert_eq!(
            decoded.flow_id, nonce,
            "flow_id must match the original nonce"
        );
        assert_eq!(decoded.instance_name.as_deref(), Some("my-instance"));
        assert!(!decoded.is_legacy);

        // Also test without instance name
        let encoded_no_instance = encode_hosted_oauth_state(nonce, None);
        let decoded_no_instance =
            decode_hosted_oauth_state(&encoded_no_instance).expect("round-trip without instance");
        assert_eq!(decoded_no_instance.flow_id, nonce);
        assert_eq!(decoded_no_instance.instance_name, None);
        assert!(!decoded_no_instance.is_legacy);
    }

    /// Legacy flow IDs that are too short must be rejected (#1443).
    #[test]
    fn test_legacy_state_rejects_short_flow_id() {
        use crate::auth::oauth::decode_hosted_oauth_state;

        let err = decode_hosted_oauth_state("abc").expect_err("short raw flow_id");
        assert!(err.contains("too short"), "unexpected error: {err}");

        let err = decode_hosted_oauth_state("inst:abc").expect_err("short prefixed flow_id");
        assert!(err.contains("too short"), "unexpected error: {err}");
    }

    /// Legacy flow IDs with invalid characters must be rejected (#1443).
    #[test]
    fn test_legacy_state_rejects_invalid_characters() {
        use crate::auth::oauth::decode_hosted_oauth_state;

        let err = decode_hosted_oauth_state("flow id with spaces!").expect_err("spaces in flow_id");
        assert!(
            err.contains("invalid characters"),
            "unexpected error: {err}"
        );

        let err = decode_hosted_oauth_state("inst:flow/id?bad=yes")
            .expect_err("special chars in prefixed flow_id");
        assert!(
            err.contains("invalid characters"),
            "unexpected error: {err}"
        );
    }

    /// Legacy instance names with invalid characters must be rejected (#1444).
    #[test]
    fn test_legacy_state_rejects_invalid_instance_name() {
        use crate::auth::oauth::decode_hosted_oauth_state;

        let err = decode_hosted_oauth_state("bad instance!:valid-flow-id-12345")
            .expect_err("invalid instance name");
        assert!(err.contains("instance name"), "unexpected error: {err}");
    }

    /// Excessively long legacy flow IDs must be rejected (#1443).
    #[test]
    fn test_legacy_state_rejects_oversized_flow_id() {
        use crate::auth::oauth::decode_hosted_oauth_state;

        let long_id = "a".repeat(200);
        let err = decode_hosted_oauth_state(&long_id).expect_err("oversized flow_id");
        assert!(err.contains("too long"), "unexpected error: {err}");
    }

    /// Valid legacy flow IDs at boundary lengths are accepted.
    #[test]
    fn test_legacy_state_accepts_boundary_lengths() {
        use crate::auth::oauth::decode_hosted_oauth_state;

        // Exactly 8 chars (minimum)
        let decoded = decode_hosted_oauth_state("abcd1234").expect("8-char flow_id");
        assert_eq!(decoded.flow_id, "abcd1234");
        assert!(decoded.is_legacy);

        // Exactly 128 chars (maximum)
        let max_id = "a".repeat(128);
        let decoded = decode_hosted_oauth_state(&max_id).expect("128-char flow_id");
        assert_eq!(decoded.flow_id, max_id);
        assert!(decoded.is_legacy);
    }
}
