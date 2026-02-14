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

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

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

/// Error from the OAuth callback listener.
#[derive(Debug, thiserror::Error)]
pub enum OAuthCallbackError {
    #[error("Port {0} is in use (another auth flow running?): {1}")]
    PortInUse(u16, String),

    #[error("Authorization denied by user")]
    Denied,

    #[error("Timed out waiting for authorization")]
    Timeout,

    #[error("IO error: {0}")]
    Io(String),
}

/// Bind the OAuth callback listener on the fixed port.
///
/// Tries IPv6 loopback (`[::1]`) first so that `http://localhost:…` redirects
/// work on systems where `localhost` resolves to `::1`. Falls back to IPv4
/// (`127.0.0.1`) only if IPv6 fails for a reason other than `AddrInUse`
/// (e.g., IPv6 not supported on the host). If the port is already occupied
/// on IPv6, the port is occupied period, so we fail immediately.
pub async fn bind_callback_listener() -> Result<TcpListener, OAuthCallbackError> {
    let ipv6_addr = format!("[::1]:{}", OAUTH_CALLBACK_PORT);
    match TcpListener::bind(&ipv6_addr).await {
        Ok(listener) => return Ok(listener),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            return Err(OAuthCallbackError::PortInUse(
                OAUTH_CALLBACK_PORT,
                e.to_string(),
            ));
        }
        Err(_) => {
            // IPv6 not available on this host, fall back to IPv4
        }
    }
    TcpListener::bind(format!("127.0.0.1:{}", OAUTH_CALLBACK_PORT))
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                OAuthCallbackError::PortInUse(OAUTH_CALLBACK_PORT, e.to_string())
            } else {
                OAuthCallbackError::Io(e.to_string())
            }
        })
}

/// Wait for an OAuth callback and extract a query parameter value.
///
/// Listens for a GET request matching `path_prefix` (e.g., "/callback" or "/auth/callback"),
/// extracts the value of `param_name` (e.g., "code" or "token"), and shows a branded
/// landing page using `display_name` (e.g., "Google", "Notion", "NEAR AI").
///
/// Times out after 5 minutes.
pub async fn wait_for_callback(
    listener: TcpListener,
    path_prefix: &str,
    param_name: &str,
    display_name: &str,
) -> Result<String, OAuthCallbackError> {
    let path_prefix = path_prefix.to_string();
    let param_name = param_name.to_string();
    let display_name = display_name.to_string();

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

                // Look for the target parameter
                for param in query.split('&') {
                    let parts: Vec<&str> = param.splitn(2, '=').collect();
                    if parts.len() == 2 && parts[0] == param_name {
                        let value = urlencoding::decode(parts[1])
                            .unwrap_or_else(|_| parts[1].into())
                            .into_owned();

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

                        return Ok(value);
                    }
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

/// HTML landing page shown in the browser after an OAuth redirect.
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
    use crate::cli::oauth_defaults::{builtin_credentials, landing_html};

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
}
