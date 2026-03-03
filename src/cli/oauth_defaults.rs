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
use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

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

/// Fixed port for all OAuth callbacks.
///
/// Every redirect URI registered with providers must use this port:
/// `http://localhost:9876/callback` (or `/auth/callback` for NEAR AI).
pub const OAUTH_CALLBACK_PORT: u16 = 9876;

/// Returns the OAuth callback base URL.
///
/// Checks `IRONCLAW_OAUTH_CALLBACK_URL` env var first (useful for remote/VPS
/// deployments where `127.0.0.1` is unreachable from the user's browser),
/// then falls back to `http://{callback_host()}:{OAUTH_CALLBACK_PORT}`.
pub fn callback_url() -> String {
    std::env::var("IRONCLAW_OAUTH_CALLBACK_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("http://{}:{}", callback_host(), OAUTH_CALLBACK_PORT))
}

/// Returns the hostname used in OAuth callback URLs.
///
/// Reads `OAUTH_CALLBACK_HOST` from the environment (default: `127.0.0.1`).
///
/// **Remote server usage:** set `OAUTH_CALLBACK_HOST` to the network interface
/// address you want to listen on (e.g. the server's LAN IP or `0.0.0.0`).
/// The callback listener will bind to that specific address instead of the
/// loopback interface, so the OAuth redirect can reach an external browser.
/// Note: this transmits the session token over plain HTTP — prefer SSH port
/// forwarding (`ssh -L 9876:127.0.0.1:9876 user@host`) when possible.
///
/// # Example
///
/// ```bash
/// export OAUTH_CALLBACK_HOST=203.0.113.10
/// ironclaw login
/// # Opens: http://203.0.113.10:9876/auth/callback
/// ```
pub fn callback_host() -> String {
    std::env::var("OAUTH_CALLBACK_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
}

/// Returns `true` if `host` is a loopback address that only accepts local connections.
///
/// Covers `localhost` (case-insensitive), the full `127.0.0.0/8` IPv4 loopback
/// range, and `::1` for IPv6.
pub fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Error from the OAuth callback listener.
#[derive(Debug, thiserror::Error)]
pub enum OAuthCallbackError {
    #[error("Port {0} is in use (another auth flow running?): {1}")]
    PortInUse(u16, String),

    #[error("Authorization denied by user")]
    Denied,

    #[error("Timed out waiting for authorization")]
    Timeout,

    #[error("CSRF state mismatch: expected {expected}, got {actual}")]
    StateMismatch { expected: String, actual: String },

    #[error("IO error: {0}")]
    Io(String),
}

/// Map a `std::io::Error` from a bind attempt to an `OAuthCallbackError`.
fn bind_error(e: std::io::Error) -> OAuthCallbackError {
    if e.kind() == std::io::ErrorKind::AddrInUse {
        OAuthCallbackError::PortInUse(OAUTH_CALLBACK_PORT, e.to_string())
    } else {
        OAuthCallbackError::Io(e.to_string())
    }
}

/// Bind the OAuth callback listener on the fixed port.
///
/// When `OAUTH_CALLBACK_HOST` is a loopback address (the default `127.0.0.1`),
/// binds to `127.0.0.1` first and falls back to `[::1]` so local-only auth
/// flows remain restricted to the local machine.
///
/// When `OAUTH_CALLBACK_HOST` is set to a remote address, binds to that
/// specific address so only connections directed to it are accepted.
pub async fn bind_callback_listener() -> Result<TcpListener, OAuthCallbackError> {
    let host = callback_host();

    if is_loopback_host(&host) {
        // Local mode: prefer IPv4 loopback, fall back to IPv6.
        let ipv4_addr = format!("127.0.0.1:{}", OAUTH_CALLBACK_PORT);
        match TcpListener::bind(&ipv4_addr).await {
            Ok(listener) => return Ok(listener),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                return Err(OAuthCallbackError::PortInUse(
                    OAUTH_CALLBACK_PORT,
                    e.to_string(),
                ));
            }
            Err(_) => {
                // IPv4 not available, fall back to IPv6
            }
        }
        TcpListener::bind(format!("[::1]:{}", OAUTH_CALLBACK_PORT))
            .await
            .map_err(bind_error)
    } else {
        // Remote mode: bind to the specific configured host address only,
        // not 0.0.0.0, to limit exposure to the intended interface.
        let addr = format!("{}:{}", host, OAUTH_CALLBACK_PORT);
        TcpListener::bind(&addr).await.map_err(bind_error)
    }
}

/// Wait for an OAuth callback and extract a query parameter value.
///
/// Listens for a GET request matching `path_prefix` (e.g., "/callback" or "/auth/callback"),
/// extracts the value of `param_name` (e.g., "code" or "token"), and shows a branded
/// landing page using `display_name` (e.g., "Google", "Notion", "NEAR AI").
///
/// When `expected_state` is `Some`, the callback's `state` query parameter is validated
/// against it to prevent CSRF attacks. If the state doesn't match, the callback is
/// rejected with an error page.
///
/// Times out after 5 minutes.
pub async fn wait_for_callback(
    listener: TcpListener,
    path_prefix: &str,
    param_name: &str,
    display_name: &str,
    expected_state: Option<&str>,
) -> Result<String, OAuthCallbackError> {
    let path_prefix = path_prefix.to_string();
    let param_name = param_name.to_string();
    let display_name = display_name.to_string();
    let expected_state = expected_state.map(String::from);

    tokio::time::timeout(Duration::from_secs(300), async move {
        loop {
            let (mut socket, _) = listener
                .accept()
                .await
                .map_err(|e| OAuthCallbackError::Io(e.to_string()))?;

            let mut reader = BufReader::new(&mut socket);
            let mut request_line = String::new();
            reader
                .read_line(&mut request_line)
                .await
                .map_err(|e| OAuthCallbackError::Io(e.to_string()))?;

            if let Some(path) = request_line.split_whitespace().nth(1)
                && path.starts_with(&path_prefix)
                && let Some(query) = path.split('?').nth(1)
            {
                // Check for error first
                if query.contains("error=") {
                    let html = landing_html(&display_name, false);
                    let response = format!(
                        "HTTP/1.1 400 Bad Request\r\n\
                         Content-Type: text/html; charset=utf-8\r\n\
                         Connection: close\r\n\
                         \r\n\
                         {}",
                        html
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    return Err(OAuthCallbackError::Denied);
                }

                // Parse all query params into a map for validation
                let params: HashMap<&str, String> = query
                    .split('&')
                    .filter_map(|p| {
                        let mut parts = p.splitn(2, '=');
                        let key = parts.next()?;
                        let val = parts.next().unwrap_or("");
                        Some((
                            key,
                            urlencoding::decode(val)
                                .unwrap_or_else(|_| val.into())
                                .into_owned(),
                        ))
                    })
                    .collect();

                // Validate CSRF state parameter
                if let Some(ref expected) = expected_state {
                    let actual = params.get("state").cloned().unwrap_or_default();
                    if actual != *expected {
                        let html = landing_html(&display_name, false);
                        let response = format!(
                            "HTTP/1.1 403 Forbidden\r\n\
                             Content-Type: text/html; charset=utf-8\r\n\
                             Connection: close\r\n\
                             \r\n\
                             {}",
                            html
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        return Err(OAuthCallbackError::StateMismatch {
                            expected: expected.clone(),
                            actual,
                        });
                    }
                }

                // Look for the target parameter
                if let Some(value) = params.get(param_name.as_str()) {
                    let html = landing_html(&display_name, true);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\n\
                         Content-Type: text/html; charset=utf-8\r\n\
                         Connection: close\r\n\
                         \r\n\
                         {}",
                        html
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;

                    return Ok(value.clone());
                }
            }

            // Not the callback we're looking for
            let response = "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(response.as_bytes()).await;
        }
    })
    .await
    .map_err(|_| OAuthCallbackError::Timeout)?
}

/// Escape a string for safe interpolation into HTML content.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

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
        rand::thread_rng().fill_bytes(&mut verifier_bytes);
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
    rand::thread_rng().fill_bytes(&mut state_bytes);
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
    let client = reqwest::Client::new();
    let mut token_params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
    ];

    if let Some(verifier) = code_verifier {
        token_params.push(("code_verifier", verifier.to_string()));
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

// ── Landing pages ───────────────────────────────────────────────────

pub fn landing_html(provider_name: &str, success: bool) -> String {
    let safe_name = html_escape(provider_name);
    let (icon, heading, subtitle, accent) = if success {
        (
            r##"<div style="width:64px;height:64px;border-radius:50%;background:#22c55e;display:flex;align-items:center;justify-content:center;margin:0 auto 24px">
                <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
              </div>"##,
            format!("{} Connected", safe_name),
            "You can close this window and return to your terminal.",
            "#22c55e",
        )
    } else {
        (
            r##"<div style="width:64px;height:64px;border-radius:50%;background:#ef4444;display:flex;align-items:center;justify-content:center;margin:0 auto 24px">
                <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
              </div>"##,
            "Authorization Failed".to_string(),
            "The request was denied. You can close this window and try again.",
            "#ef4444",
        )
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>IronClaw - {heading}</title>
<style>
  * {{ margin:0; padding:0; box-sizing:border-box }}
  body {{
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    background: #0a0a0a;
    color: #e5e5e5;
    display: flex;
    justify-content: center;
    align-items: center;
    min-height: 100vh;
  }}
  .card {{
    text-align: center;
    padding: 48px 40px;
    max-width: 420px;
    border: 1px solid #262626;
    border-radius: 16px;
    background: #141414;
  }}
  h1 {{
    font-size: 22px;
    font-weight: 600;
    margin-bottom: 8px;
    color: #fafafa;
  }}
  p {{
    font-size: 14px;
    color: #a3a3a3;
    line-height: 1.5;
  }}
  .accent {{ color: {accent}; }}
  .brand {{
    margin-top: 32px;
    font-size: 12px;
    color: #525252;
    letter-spacing: 0.5px;
    text-transform: uppercase;
  }}
</style>
</head>
<body>
  <div class="card">
    {icon}
    <h1>{heading}</h1>
    <p>{subtitle}</p>
    <div class="brand">IronClaw</div>
  </div>
</body>
</html>"#,
        heading = heading,
        icon = icon,
        subtitle = subtitle,
        accent = accent,
    )
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
}
