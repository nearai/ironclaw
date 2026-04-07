//! Shared HTTP SSRF defenses for WASM tool and channel runtimes.

use std::net::{IpAddr, ToSocketAddrs};

/// Build a reqwest client builder with the WASM SSRF redirect policy applied.
///
/// Redirects are disabled so callers must explicitly validate each hop instead
/// of letting reqwest follow `Location` to an unvalidated target.
pub(crate) fn ssrf_safe_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
}

/// Resolve the URL's hostname and reject connections to private/internal IP addresses.
///
/// This prevents DNS rebinding attacks where an attacker-controlled hostname
/// passes the allowlist check, then resolves to an internal address.
pub(crate) fn reject_private_ip(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("Failed to parse URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("Unsupported URL scheme: {}", parsed.scheme()));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URL contains userinfo (@) which is not allowed".to_string());
    }

    let host = parsed
        .host_str()
        .map(|h| {
            h.strip_prefix('[')
                .and_then(|v| v.strip_suffix(']'))
                .unwrap_or(h)
        })
        .ok_or_else(|| "Failed to parse host from URL".to_string())?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_private_ip(ip) {
            Err(format!(
                "HTTP request to private/internal IP {} is not allowed",
                ip
            ))
        } else {
            Ok(())
        };
    }

    let addrs: Vec<_> = format!("{}:0", host)
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed for {}: {}", host, e))?
        .collect();

    if addrs.is_empty() {
        return Err(format!("DNS resolution returned no addresses for {}", host));
    }

    for addr in &addrs {
        if is_private_ip(addr.ip()) {
            return Err(format!(
                "DNS rebinding detected: {} resolved to private IP {}",
                host,
                addr.ip()
            ));
        }
    }

    Ok(())
}

pub(crate) fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xFE00) == 0xFC00
                || (v6.segments()[0] & 0xFFC0) == 0xFE80
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::extract::State;
    use axum::response::Redirect;
    use axum::routing::get;
    use axum::{Router, serve};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_ssrf_safe_client_builder_disables_redirects() {
        async fn final_handler(State(hits): State<Arc<AtomicUsize>>) -> &'static str {
            hits.fetch_add(1, Ordering::SeqCst);
            "ok"
        }

        let final_hits = Arc::new(AtomicUsize::new(0));
        let final_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let final_addr = final_listener.local_addr().unwrap();
        let final_app = Router::new()
            .route("/final", get(final_handler))
            .with_state(Arc::clone(&final_hits));
        let final_handle = tokio::spawn(async move {
            serve(final_listener, final_app).await.unwrap();
        });

        let redirect_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let redirect_addr = redirect_listener.local_addr().unwrap();
        let location = format!("http://{final_addr}/final");
        let redirect_app = Router::new().route(
            "/start",
            get(move || async move { Redirect::temporary(&location) }),
        );
        let redirect_handle = tokio::spawn(async move {
            serve(redirect_listener, redirect_app).await.unwrap();
        });

        let client = super::ssrf_safe_client_builder().build().unwrap();
        let response: reqwest::Response = client
            .get(format!("http://{redirect_addr}/start"))
            .send()
            .await
            .unwrap();

        assert!(response.status().is_redirection());
        assert_eq!(final_hits.load(Ordering::SeqCst), 0);

        redirect_handle.abort();
        final_handle.abort();
    }
}
