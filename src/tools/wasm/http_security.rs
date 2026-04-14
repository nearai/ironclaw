//! WASM-specific SSRF helpers.
//!
//! The general SSRF helpers live in `crate::tools::http_security`. This
//! file only contains the blocking-DNS path used by WASM host functions.

#[cfg(feature = "wasm-sandbox")]
use std::net::ToSocketAddrs;

#[cfg(feature = "wasm-sandbox")]
use crate::tools::http_security::is_private_ip;

/// Resolve the URL's hostname (blocking) and reject connections to private
/// or internal IP addresses.
///
/// This prevents DNS rebinding attacks where an attacker-controlled hostname
/// passes the allowlist check, then resolves to an internal address.
#[cfg(feature = "wasm-sandbox")]
pub(crate) fn reject_private_ip(url: &str) -> Result<(), String> {
    use std::net::IpAddr;

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
