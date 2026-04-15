//! OAuth callback infrastructure used by the NEAR AI session login flow.
//!
//! These utilities (callback server, landing pages, hostname detection) were
//! originally in the old CLI OAuth module and are moved here so the `llm`
//! module is self-contained.

use std::collections::HashMap;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// Fixed port for the OAuth callback listener.
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

    #[error("CSRF state mismatch: expected {expected}, got {actual}")]
    StateMismatch { expected: String, actual: String },

    #[error("IO error: {0}")]
    Io(String),
}

/// Returns the OAuth callback base URL.
///
/// Checks `IRONCLAW_OAUTH_CALLBACK_URL` env var first (useful for remote/VPS
/// deployments where `127.0.0.1` is unreachable from the user's browser),
/// then falls back to `http://{callback_host()}:{OAUTH_CALLBACK_PORT}`.
pub fn callback_url() -> String {
    crate::config::helpers::env_or_override("IRONCLAW_OAUTH_CALLBACK_URL")
        .unwrap_or_else(|| format!("http://{}:{}", callback_host(), OAUTH_CALLBACK_PORT))
}

/// Returns the hostname used in OAuth callback URLs.
///
/// Reads `OAUTH_CALLBACK_HOST` from the environment (default: `127.0.0.1`).
///
/// **Remote server usage:** set `OAUTH_CALLBACK_HOST` to the specific network
/// interface address you want to listen on (e.g. the server's LAN IP).
/// Wildcard addresses (`0.0.0.0`, `::`) are rejected — use a specific interface
/// IP to limit exposure. The callback listener will bind to that address so the
/// OAuth redirect can reach an external browser.
/// Note: this transmits the session token over plain HTTP — prefer SSH port
/// forwarding (`ssh -L 9876:127.0.0.1:9876 user@host`) when possible.
pub fn callback_host() -> String {
    crate::config::helpers::env_or_override("OAUTH_CALLBACK_HOST")
        .unwrap_or_else(|| "127.0.0.1".to_string())
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

/// Returns `true` if `host` is a wildcard/unspecified address (`0.0.0.0` or `::`).
///
/// Wildcard binds accept connections on all interfaces, which is a security risk
/// for OAuth callbacks that carry session tokens over plain HTTP.
fn is_wildcard_host(host: &str) -> bool {
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_unspecified())
        .unwrap_or(false)
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

    if is_wildcard_host(&host) {
        return Err(OAuthCallbackError::Io(format!(
            "OAUTH_CALLBACK_HOST={host} is a wildcard address — this would accept \
             connections on all interfaces, exposing the session token. \
             Use a specific interface IP (e.g. 192.168.1.x) or SSH port forwarding instead."
        )));
    }

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

/// Generate a branded HTML landing page for the OAuth callback result.
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
<title>{brand} - {heading}</title>
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
    <div class="brand">{brand}</div>
  </div>
</body>
</html>"#,
        heading = heading,
        icon = icon,
        subtitle = subtitle,
        accent = accent,
        brand = crate::config::platform_display_name(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    #[test]
    fn loopback_detection() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("127.0.0.2")); // full 127.0.0.0/8 range
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("LOCALHOST"));
        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("192.168.1.1"));
        assert!(!is_loopback_host("::"));
        assert!(!is_loopback_host("example.com"));
    }

    #[test]
    fn wildcard_detection() {
        assert!(is_wildcard_host("0.0.0.0"));
        assert!(is_wildcard_host("::"));
        assert!(!is_wildcard_host("127.0.0.1"));
        assert!(!is_wildcard_host("192.168.1.1"));
        assert!(!is_wildcard_host("::1"));
        assert!(!is_wildcard_host("localhost"));
    }

    // Lock held across await to serialize env-var mutation; the awaited op is a quick local TCP bind.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn bind_rejects_wildcard_ipv4() {
        let _guard = lock_env();
        let original = std::env::var("OAUTH_CALLBACK_HOST").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("OAUTH_CALLBACK_HOST", "0.0.0.0") };
        let result = bind_callback_listener().await;
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            match &original {
                Some(v) => std::env::set_var("OAUTH_CALLBACK_HOST", v),
                None => std::env::remove_var("OAUTH_CALLBACK_HOST"),
            }
        }
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("wildcard"),
            "error should mention wildcard: {err}"
        );
    }

    // Lock held across await to serialize env-var mutation; the awaited op is a quick local TCP bind.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn bind_rejects_wildcard_ipv6() {
        let _guard = lock_env();
        let original = std::env::var("OAUTH_CALLBACK_HOST").ok();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("OAUTH_CALLBACK_HOST", "::") };
        let result = bind_callback_listener().await;
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            match &original {
                Some(v) => std::env::set_var("OAUTH_CALLBACK_HOST", v),
                None => std::env::remove_var("OAUTH_CALLBACK_HOST"),
            }
        }
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("wildcard"),
            "error should mention wildcard: {err}"
        );
    }
}
