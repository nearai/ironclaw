use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::error::ConfigError;

use crate::config::INJECTED_VARS;

/// Crate-wide mutex for tests that mutate process environment variables.
///
/// The process environment is global state shared across all threads.
/// Per-module mutexes do NOT prevent races between modules running in
/// parallel.  Every `unsafe { set_var / remove_var }` call in tests
/// MUST hold this single lock.
#[cfg(test)]
pub(crate) static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Acquire the env-var mutex, recovering from poison.
///
/// A poisoned mutex means a previous test panicked while holding the lock.
/// The env state might be slightly stale, but cascading every subsequent
/// test into a `PoisonError` panic is far worse. Recover and carry on.
#[cfg(test)]
pub(crate) fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner())
}

/// Thread-safe mutable overlay for env vars set at runtime.
///
/// Unlike `INJECTED_VARS` (which is set once at startup from the secrets
/// store), this map supports writes at any point during the process
/// lifetime. It replaces unsafe `std::env::set_var` calls that would
/// otherwise be UB in multi-threaded programs (Rust 1.82+).
///
/// Priority: real env vars > `RUNTIME_ENV_OVERRIDES` > `INJECTED_VARS`.
static RUNTIME_ENV_OVERRIDES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn runtime_overrides() -> &'static Mutex<HashMap<String, String>> {
    RUNTIME_ENV_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Set a runtime environment override (thread-safe alternative to `std::env::set_var`).
///
/// Values set here are visible to `optional_env()`, `env_or_override()`, and
/// all config resolution that goes through those helpers. This avoids the UB
/// of `std::env::set_var` in multi-threaded programs.
pub fn set_runtime_env(key: &str, value: &str) {
    runtime_overrides()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_string(), value.to_string());
}

/// Read an env var, checking the real environment first, then runtime overrides.
///
/// Priority: real env vars > runtime overrides > `INJECTED_VARS`.
/// Empty values are treated as unset at every layer for consistency with
/// `optional_env()`.
///
/// Use this instead of `std::env::var()` when the value might have been set
/// via `set_runtime_env()` (e.g., `NEARAI_API_KEY` during interactive login).
pub fn env_or_override(key: &str) -> Option<String> {
    // Real env vars always win
    if let Ok(val) = std::env::var(key)
        && !val.is_empty()
    {
        return Some(val);
    }

    // Check runtime overrides (skip empty values for consistency with optional_env)
    if let Some(val) = runtime_overrides()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .filter(|v| !v.is_empty())
        .cloned()
    {
        return Some(val);
    }

    // Check INJECTED_VARS (secrets from DB, set once at startup)
    if let Some(val) = INJECTED_VARS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .filter(|v| !v.is_empty())
        .cloned()
    {
        return Some(val);
    }

    None
}

pub(crate) fn optional_env(key: &str) -> Result<Option<String>, ConfigError> {
    // Check real env vars first (always win over injected secrets)
    match std::env::var(key) {
        Ok(val) if val.is_empty() => {}
        Ok(val) => return Ok(Some(val)),
        Err(std::env::VarError::NotPresent) => {}
        Err(e) => {
            return Err(ConfigError::ParseError(format!(
                "failed to read {key}: {e}"
            )));
        }
    }

    // Fall back to runtime overrides (set via set_runtime_env)
    if let Some(val) = runtime_overrides()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .filter(|v| !v.is_empty())
        .cloned()
    {
        return Ok(Some(val));
    }

    // Fall back to thread-safe overlay (secrets injected from DB)
    if let Some(val) = INJECTED_VARS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(key)
        .cloned()
    {
        return Ok(Some(val));
    }

    Ok(None)
}

pub(crate) fn parse_optional_env<T>(key: &str, default: T) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    optional_env(key)?
        .map(|s| {
            s.parse().map_err(|e| ConfigError::InvalidValue {
                key: key.to_string(),
                message: format!("{e}"),
            })
        })
        .transpose()
        .map(|opt| opt.unwrap_or(default))
}

/// Parse a boolean from an env var with a default.
///
/// Accepts "true"/"1" as true, "false"/"0" as false.
pub(crate) fn parse_bool_env(key: &str, default: bool) -> Result<bool, ConfigError> {
    match optional_env(key)? {
        Some(s) => match s.to_lowercase().as_str() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(ConfigError::InvalidValue {
                key: key.to_string(),
                message: format!("must be 'true' or 'false', got '{s}'"),
            }),
        },
        None => Ok(default),
    }
}

/// Parse an env var into `Option<T>` — returns `None` when unset,
/// `Some(parsed)` when set to a valid value.
pub(crate) fn parse_option_env<T>(key: &str) -> Result<Option<T>, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    optional_env(key)?
        .map(|s| {
            s.parse().map_err(|e| ConfigError::InvalidValue {
                key: key.to_string(),
                message: format!("{e}"),
            })
        })
        .transpose()
}

/// Parse a string from an env var with a default.
pub(crate) fn parse_string_env(
    key: &str,
    default: impl Into<String>,
) -> Result<String, ConfigError> {
    Ok(optional_env(key)?.unwrap_or_else(|| default.into()))
}

/// Validate a user-configurable base URL to prevent SSRF attacks (#1103).
///
/// Rejects:
/// - Non-HTTP(S) schemes (file://, ftp://, etc.)
/// - HTTPS URLs pointing at private/loopback/link-local IPs
/// - HTTP URLs pointing at anything other than localhost/127.0.0.1/::1
///
/// This is intended for config-time validation of base URLs like
/// `OLLAMA_BASE_URL`, `EMBEDDING_BASE_URL`, `NEARAI_BASE_URL`, etc.
pub(crate) fn validate_base_url(url: &str, field_name: &str) -> Result<(), ConfigError> {
    use std::net::{IpAddr, Ipv4Addr};

    let parsed = reqwest::Url::parse(url).map_err(|e| ConfigError::InvalidValue {
        key: field_name.to_string(),
        message: format!("invalid URL '{}': {}", url, e),
    })?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(ConfigError::InvalidValue {
            key: field_name.to_string(),
            message: format!("only http/https URLs are allowed, got '{}'", scheme),
        });
    }

    let host = parsed.host_str().ok_or_else(|| ConfigError::InvalidValue {
        key: field_name.to_string(),
        message: "URL is missing a host".to_string(),
    })?;

    let host_lower = host.to_lowercase();

    // For HTTP (non-TLS), only allow localhost — remote HTTP endpoints
    // risk credential leakage (e.g. NEAR AI bearer tokens sent over plaintext).
    if scheme == "http" {
        let is_localhost = host_lower == "localhost"
            || host_lower == "127.0.0.1"
            || host_lower == "::1"
            || host_lower == "[::1]"
            || host_lower.ends_with(".localhost");
        if !is_localhost {
            return Err(ConfigError::InvalidValue {
                key: field_name.to_string(),
                message: format!(
                    "HTTP (non-TLS) is only allowed for localhost, got '{}'. \
                     Use HTTPS for remote endpoints.",
                    host
                ),
            });
        }
        return Ok(());
    }

    // Check whether an IP is in a blocked range (private, loopback,
    // link-local, multicast, metadata, CGN, ULA).
    let is_dangerous_ip = |ip: &IpAddr| -> bool {
        match ip {
            IpAddr::V4(v4) => {
                v4.is_private()
                    || v4.is_loopback()
                    || v4.is_link_local()
                    || v4.is_multicast()
                    || v4.is_unspecified()
                    || *v4 == Ipv4Addr::new(169, 254, 169, 254)
                    || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64) // CGN
            }
            IpAddr::V6(v6) => {
                if let Some(v4) = v6.to_ipv4_mapped() {
                    v4.is_private()
                        || v4.is_loopback()
                        || v4.is_link_local()
                        || v4.is_multicast()
                        || v4.is_unspecified()
                        || v4 == Ipv4Addr::new(169, 254, 169, 254)
                        || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64) // CGN
                } else {
                    v6.is_loopback()
                        || v6.is_unspecified()
                        || (v6.octets()[0] & 0xfe) == 0xfc // ULA (fc00::/7)
                        || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local (fe80::/10)
                        || v6.octets()[0] == 0xff // multicast (ff00::/8)
                }
            }
        }
    };

    // For HTTPS, reject private/loopback/link-local/metadata IPs.
    // Check both IP literals and resolved hostnames to prevent DNS-based SSRF.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_dangerous_ip(&ip) {
            return Err(ConfigError::InvalidValue {
                key: field_name.to_string(),
                message: format!(
                    "URL points to a private/internal IP '{}'. \
                     This is blocked to prevent SSRF attacks.",
                    ip
                ),
            });
        }
    } else {
        // Hostname — resolve and check all resulting IPs as defense-in-depth.
        // NOTE: This does NOT fully prevent DNS rebinding attacks (the hostname
        // could resolve to a different IP at request time). Full protection
        // would require pinning the resolved IP in the HTTP client's connector.
        // This validation catches the common case of misconfigured or malicious URLs.
        //
        // NOTE: `to_socket_addrs()` performs blocking DNS resolution. This is
        // acceptable because `validate_base_url` runs at config-load time only,
        // before the async runtime is fully driving I/O. If this ever moves to
        // a hot path, wrap in `tokio::task::spawn_blocking` or use
        // `tokio::net::lookup_host`.
        use std::net::ToSocketAddrs;
        let port = parsed.port().unwrap_or(443);
        match (host, port).to_socket_addrs() {
            Ok(addrs) => {
                for addr in addrs {
                    if is_dangerous_ip(&addr.ip()) {
                        return Err(ConfigError::InvalidValue {
                            key: field_name.to_string(),
                            message: format!(
                                "hostname '{}' resolves to private/internal IP '{}'. \
                                 This is blocked to prevent SSRF attacks.",
                                host,
                                addr.ip()
                            ),
                        });
                    }
                }
            }
            Err(e) => {
                return Err(ConfigError::InvalidValue {
                    key: field_name.to_string(),
                    message: format!(
                        "failed to resolve hostname '{}': {}. \
                         Base URLs must be resolvable at config time.",
                        host, e
                    ),
                });
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_env_override_is_visible_to_env_or_override() {
        // Use a unique key that won't collide with real env vars.
        let key = "IRONCLAW_TEST_RUNTIME_OVERRIDE_42";

        // Not set initially
        assert!(env_or_override(key).is_none());

        // Set via the thread-safe overlay
        set_runtime_env(key, "test_value");

        // Now visible
        assert_eq!(env_or_override(key), Some("test_value".to_string()));
    }

    #[test]
    fn runtime_env_override_is_visible_to_optional_env() {
        let key = "IRONCLAW_TEST_OPTIONAL_ENV_OVERRIDE_42";

        assert_eq!(optional_env(key).unwrap(), None);

        set_runtime_env(key, "hello");

        assert_eq!(optional_env(key).unwrap(), Some("hello".to_string()));
    }

    #[test]
    fn real_env_var_takes_priority_over_runtime_override() {
        let _guard = lock_env();
        let key = "IRONCLAW_TEST_ENV_PRIORITY_42";

        // Set runtime override
        set_runtime_env(key, "override_value");

        // Set real env var (should win)
        // SAFETY: test runs under ENV_MUTEX
        unsafe { std::env::set_var(key, "real_value") };

        assert_eq!(env_or_override(key), Some("real_value".to_string()));

        // Clean up
        unsafe { std::env::remove_var(key) };

        // Now the runtime override is visible again
        assert_eq!(env_or_override(key), Some("override_value".to_string()));
    }

    // --- lock_env poison recovery (regression for env mutex cascade) ---

    #[test]
    fn lock_env_recovers_from_poisoned_mutex() {
        // Simulate a poisoned mutex: spawn a thread that panics while holding the lock.
        let _ = std::thread::spawn(|| {
            let _guard = ENV_MUTEX.lock().unwrap();
            panic!("intentional poison");
        })
        .join();

        // The mutex is now poisoned. lock_env() should recover, not cascade.
        assert!(ENV_MUTEX.lock().is_err(), "mutex should be poisoned");
        let _guard = lock_env(); // must not panic
    }

    // --- validate_base_url tests (regression for #1103) ---

    #[test]
    fn validate_base_url_allows_https() {
        // Use IP literals to avoid DNS resolution in sandboxed test environments.
        assert!(validate_base_url("https://8.8.8.8", "TEST").is_ok());
        assert!(validate_base_url("https://8.8.8.8/v1", "TEST").is_ok());
    }

    #[test]
    fn validate_base_url_allows_http_localhost() {
        assert!(validate_base_url("http://localhost:11434", "TEST").is_ok());
        assert!(validate_base_url("http://127.0.0.1:11434", "TEST").is_ok());
        assert!(validate_base_url("http://[::1]:11434", "TEST").is_ok());
    }

    #[test]
    fn validate_base_url_rejects_http_remote() {
        assert!(validate_base_url("http://evil.example.com", "TEST").is_err());
        assert!(validate_base_url("http://192.168.1.1", "TEST").is_err());
    }

    #[test]
    fn validate_base_url_rejects_non_http_schemes() {
        assert!(validate_base_url("file:///etc/passwd", "TEST").is_err());
        assert!(validate_base_url("ftp://evil.com", "TEST").is_err());
    }

    #[test]
    fn validate_base_url_rejects_cloud_metadata() {
        assert!(validate_base_url("https://169.254.169.254", "TEST").is_err());
    }

    #[test]
    fn validate_base_url_rejects_private_ips() {
        assert!(validate_base_url("https://10.0.0.1", "TEST").is_err());
        assert!(validate_base_url("https://192.168.1.1", "TEST").is_err());
        assert!(validate_base_url("https://172.16.0.1", "TEST").is_err());
    }

    #[test]
    fn validate_base_url_rejects_cgn_range() {
        // Carrier-grade NAT: 100.64.0.0/10
        assert!(validate_base_url("https://100.64.0.1", "TEST").is_err());
        assert!(validate_base_url("https://100.127.255.254", "TEST").is_err());
    }

    #[test]
    fn validate_base_url_rejects_ipv4_mapped_ipv6() {
        // ::ffff:10.0.0.1 is an IPv4-mapped IPv6 address pointing to private IP
        assert!(validate_base_url("https://[::ffff:10.0.0.1]", "TEST").is_err());
        assert!(validate_base_url("https://[::ffff:169.254.169.254]", "TEST").is_err());
    }

    #[test]
    fn validate_base_url_rejects_ula_ipv6() {
        // fc00::/7 — unique local addresses
        assert!(validate_base_url("https://[fc00::1]", "TEST").is_err());
        assert!(validate_base_url("https://[fd12:3456:789a::1]", "TEST").is_err());
    }

    #[test]
    fn validate_base_url_handles_url_with_credentials() {
        // URLs with embedded credentials — validate_base_url checks the host,
        // not the credentials. Use IP literal to avoid DNS in sandboxed envs.
        let result = validate_base_url("https://user:pass@8.8.8.8", "TEST");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_base_url_rejects_empty_and_invalid() {
        assert!(validate_base_url("", "TEST").is_err());
        assert!(validate_base_url("not-a-url", "TEST").is_err());
        assert!(validate_base_url("://missing-scheme", "TEST").is_err());
    }

    #[test]
    fn validate_base_url_rejects_unspecified_ipv4() {
        assert!(validate_base_url("https://0.0.0.0", "TEST").is_err());
    }

    #[test]
    fn validate_base_url_rejects_ipv6_loopback_https() {
        // IPv6 loopback is allowed over HTTP (localhost equivalent),
        // but must be rejected over HTTPS as a dangerous IP.
        assert!(validate_base_url("https://[::1]", "TEST").is_err());
    }

    #[test]
    fn validate_base_url_rejects_ipv6_link_local() {
        // fe80::/10 — link-local addresses
        assert!(validate_base_url("https://[fe80::1]", "TEST").is_err());
    }

    #[test]
    fn validate_base_url_rejects_ipv6_multicast() {
        // ff00::/8 — multicast addresses
        assert!(validate_base_url("https://[ff02::1]", "TEST").is_err());
    }

    #[test]
    fn validate_base_url_rejects_ipv6_unspecified() {
        // :: — unspecified address
        assert!(validate_base_url("https://[::]", "TEST").is_err());
    }

    #[test]
    fn validate_base_url_rejects_dns_failure() {
        // .invalid TLD is guaranteed to never resolve (RFC 6761)
        let result = validate_base_url("https://ssrf-test.invalid", "TEST");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("failed to resolve"),
            "Expected DNS resolution failure, got: {err}"
        );
    }
}
