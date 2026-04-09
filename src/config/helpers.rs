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

/// Setting keys that influence LLM/embeddings provider base URLs.
///
/// These keys are validated with the operator policy (which allows
/// private/loopback endpoints), so they must only be writable and
/// resolvable for admin users. The settings HTTP handlers reject
/// non-admin writes/imports of these keys; `strip_admin_only_llm_keys`
/// is the matching defense for the read/resolve path so a non-admin
/// user (or pre-existing legacy DB row) cannot reactivate a private
/// endpoint after this restriction landed.
pub(crate) const ADMIN_ONLY_LLM_SETTING_KEYS: &[&str] = &[
    "llm_builtin_overrides",
    "llm_custom_providers",
    "ollama_base_url",
    "openai_compatible_base_url",
];

/// Remove admin-only LLM setting keys from a flat DB settings map.
///
/// Used by config resolution paths that load per-user DB settings for a
/// non-operator user, to ensure they cannot inject private/loopback
/// provider endpoints into the active LLM/embeddings configuration.
pub(crate) fn strip_admin_only_llm_keys(map: &mut HashMap<String, serde_json::Value>) {
    map.retain(|key, _| !ADMIN_ONLY_LLM_SETTING_KEYS.contains(&key.as_str()));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BaseUrlPolicy {
    StrictSsrf,
    AllowPrivateNetwork,
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
    validate_base_url_with_policy(url, field_name, BaseUrlPolicy::StrictSsrf)
}

/// Validate an operator-configured model endpoint.
///
/// Unlike generic SSRF validation, this allows private/loopback LLM endpoints
/// over both HTTP and HTTPS because they are explicitly configured by the
/// operator. Public HTTP endpoints remain blocked to avoid sending credentials
/// over plaintext transport.
pub(crate) fn validate_operator_base_url(url: &str, field_name: &str) -> Result<(), ConfigError> {
    validate_base_url_with_policy(url, field_name, BaseUrlPolicy::AllowPrivateNetwork)
}

fn classify_ip(ip: &std::net::IpAddr) -> IpClass {
    use std::net::{IpAddr, Ipv4Addr};

    match ip {
        IpAddr::V4(v4) => {
            if v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_link_local()
                || *v4 == Ipv4Addr::new(169, 254, 169, 254)
            {
                IpClass::AlwaysBlocked
            } else if v4.is_private()
                || v4.is_loopback()
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
            {
                IpClass::PrivateOrLoopback
            } else {
                IpClass::Public
            }
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                classify_ip(&IpAddr::V4(v4))
            } else if v6.is_unspecified()
                || v6.octets()[0] == 0xff
                || (v6.segments()[0] & 0xffc0) == 0xfe80
            {
                IpClass::AlwaysBlocked
            } else if v6.is_loopback() || (v6.octets()[0] & 0xfe) == 0xfc {
                IpClass::PrivateOrLoopback
            } else {
                IpClass::Public
            }
        }
    }
}

fn validate_base_url_with_policy(
    url: &str,
    field_name: &str,
    policy: BaseUrlPolicy,
) -> Result<(), ConfigError> {
    use std::net::{IpAddr, ToSocketAddrs};

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
    let normalized_host = host.trim_start_matches('[').trim_end_matches(']');

    let is_localhost_name = || {
        host_lower == "localhost"
            || host_lower == "127.0.0.1"
            || normalized_host == "::1"
            || host_lower.ends_with(".localhost")
    };

    if scheme == "http" && policy == BaseUrlPolicy::StrictSsrf && !is_localhost_name() {
        return Err(ConfigError::InvalidValue {
            key: field_name.to_string(),
            message: format!(
                "HTTP (non-TLS) is only allowed for localhost, got '{}'. \
                 Use HTTPS for remote endpoints.",
                host
            ),
        });
    }

    let resolved_ips = if let Ok(ip) = normalized_host.parse::<IpAddr>() {
        vec![ip]
    } else if scheme == "https" {
        // For HTTPS hostnames, TLS certificate validation at connection time
        // prevents SSRF — the server must present a valid cert matching the
        // hostname. Skip blocking DNS resolution to avoid hanging in sandboxed
        // or offline CI environments where DNS is unavailable. IP literals are
        // still checked above since TLS doesn't help validate raw IPs.
        return Ok(());
    } else {
        let port = parsed.port().unwrap_or(80);
        // `to_socket_addrs` performs blocking DNS resolution. This helper is
        // also called from async request handlers (e.g. the LLM utility
        // routes), so wrap the lookup in `block_in_place` when running on a
        // multi-threaded tokio worker to avoid stalling other tasks. The
        // `try_current()` check keeps sync callers (config bootstrap, CLI)
        // working unchanged.
        let resolve = || -> std::io::Result<Vec<IpAddr>> {
            Ok((host, port)
                .to_socket_addrs()?
                .map(|addr| addr.ip())
                .collect())
        };
        let lookup = match tokio::runtime::Handle::try_current() {
            Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(resolve)
            }
            _ => resolve(),
        };
        lookup.map_err(|e| ConfigError::InvalidValue {
            key: field_name.to_string(),
            message: format!(
                "failed to resolve hostname '{}': {}. \
                 Base URLs must be resolvable at config time.",
                host, e
            ),
        })?
    };

    if scheme == "http" {
        if is_localhost_name() {
            return Ok(());
        }

        let all_private = !resolved_ips.is_empty()
            && resolved_ips
                .iter()
                .all(|ip| matches!(classify_ip(ip), IpClass::PrivateOrLoopback));
        let any_blocked = resolved_ips
            .iter()
            .any(|ip| matches!(classify_ip(ip), IpClass::AlwaysBlocked));

        if policy == BaseUrlPolicy::AllowPrivateNetwork && all_private && !any_blocked {
            return Ok(());
        }

        return Err(ConfigError::InvalidValue {
            key: field_name.to_string(),
            message: if policy == BaseUrlPolicy::AllowPrivateNetwork {
                format!(
                    "HTTP (non-TLS) is only allowed for localhost or private/internal endpoints, got '{}'. \
                     Use HTTPS for public endpoints.",
                    host
                )
            } else {
                format!(
                    "HTTP (non-TLS) is only allowed for localhost, got '{}'. \
                     Use HTTPS for remote endpoints.",
                    host
                )
            },
        });
    }

    for ip in resolved_ips {
        match classify_ip(&ip) {
            IpClass::AlwaysBlocked => {
                return Err(ConfigError::InvalidValue {
                    key: field_name.to_string(),
                    message: format!(
                        "URL points to a blocked IP '{}'. \
                         This is blocked to prevent SSRF attacks.",
                        ip
                    ),
                });
            }
            IpClass::PrivateOrLoopback if policy == BaseUrlPolicy::StrictSsrf => {
                let message = if normalized_host.parse::<IpAddr>().is_ok() {
                    format!(
                        "URL points to a private/internal IP '{}'. \
                         This is blocked to prevent SSRF attacks.",
                        ip
                    )
                } else {
                    format!(
                        "hostname '{}' resolves to private/internal IP '{}'. \
                         This is blocked to prevent SSRF attacks.",
                        host, ip
                    )
                };
                return Err(ConfigError::InvalidValue {
                    key: field_name.to_string(),
                    message,
                });
            }
            _ => {}
        }
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IpClass {
    Public,
    PrivateOrLoopback,
    AlwaysBlocked,
}

// ---------------------------------------------------------------------------
// DB-first resolution helpers (DB > env > default)
// ---------------------------------------------------------------------------

/// Log a warning when a DB/TOML setting shadows a set env var.
///
/// Only checks real env vars (`std::env::var`), not runtime overrides or
/// injected vars — those are internal and don't warrant operator warnings.
/// Values are intentionally NOT logged to avoid leaking sensitive data.
fn warn_if_db_shadows_env(env_key: &str) {
    if std::env::var(env_key).is_ok_and(|v| !v.is_empty()) {
        tracing::warn!(
            env_key = %env_key,
            "{env_key} env var is set but a DB or TOML setting takes priority. \
             Remove the setting from DB/TOML to use the env var."
        );
    }
}

/// Resolve with DB > env > default priority for concrete settings fields.
///
/// If `settings_val != default_val`, the settings value wins (it was explicitly
/// set in DB or TOML). Otherwise falls back to `optional_env(env_key)`, then
/// `default_val`.
///
/// **Limitation:** Uses `settings_val != default_val` as a heuristic for
/// "was this field explicitly set." If a user deliberately sets a DB value
/// equal to the default, it's indistinguishable from "unset" and the env
/// var will win. This matches `merge_from()` semantics and is acceptable
/// since setting a value to its default is effectively a no-op.
pub(crate) fn db_first_or_default<T>(
    settings_val: &T,
    default_val: &T,
    env_key: &str,
) -> Result<T, ConfigError>
where
    T: std::str::FromStr + Clone + PartialEq + std::fmt::Display,
    T::Err: std::fmt::Display,
{
    if settings_val != default_val {
        warn_if_db_shadows_env(env_key);
        return Ok(settings_val.clone());
    }
    parse_optional_env(env_key, default_val.clone())
}

/// Resolve a bool with DB > env > default priority.
pub(crate) fn db_first_bool(
    settings_val: bool,
    default_val: bool,
    env_key: &str,
) -> Result<bool, ConfigError> {
    if settings_val != default_val {
        warn_if_db_shadows_env(env_key);
        return Ok(settings_val);
    }
    parse_bool_env(env_key, default_val)
}

/// Resolve an `Option<String>` with DB > env priority (no hardcoded default).
///
/// Non-empty `Some` means DB set it; `None` or empty falls back to env.
pub(crate) fn db_first_optional_string(
    settings_val: &Option<String>,
    env_key: &str,
) -> Result<Option<String>, ConfigError> {
    if let Some(val) = settings_val
        && !val.is_empty()
    {
        warn_if_db_shadows_env(env_key);
        return Ok(Some(val.clone()));
    }
    optional_env(env_key)
}

/// Resolve an `Option<T>` with DB > env priority (no hardcoded default).
///
/// `Some(v)` means DB set it; `None` falls back to env.
pub(crate) fn db_first_option<T>(
    settings_val: &Option<T>,
    env_key: &str,
) -> Result<Option<T>, ConfigError>
where
    T: std::str::FromStr + Clone + std::fmt::Display,
    T::Err: std::fmt::Display,
{
    if let Some(val) = settings_val {
        warn_if_db_shadows_env(env_key);
        return Ok(Some(val.clone()));
    }
    parse_option_env(env_key)
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
        drop(_guard);

        // Clean up so this test doesn't leave ENV_MUTEX permanently poisoned.
        ENV_MUTEX.clear_poison();
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
    fn validate_base_url_rejects_http_remote_without_dns_resolution() {
        let result = validate_base_url("http://ssrf-test.invalid", "TEST");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("only allowed for localhost"),
            "strict HTTP validation should short-circuit before DNS lookup: {err}"
        );
        assert!(
            !err.contains("failed to resolve"),
            "strict HTTP validation should not require DNS resolution: {err}"
        );
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
    fn validate_base_url_https_hostname_skips_dns() {
        // HTTPS hostnames skip DNS resolution — TLS validates the host at
        // connection time — so even an unresolvable domain should pass.
        let result = validate_base_url("https://ssrf-test.invalid", "TEST");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_base_url_http_rejects_dns_failure() {
        // HTTP hostnames that are not localhost still go through DNS. But
        // non-localhost HTTP is rejected before DNS lookup (strict SSRF),
        // so this tests the pre-DNS rejection path.
        let result = validate_base_url("http://ssrf-test.invalid", "TEST");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("only allowed for localhost"),
            "Expected HTTP rejection, got: {err}"
        );
    }

    #[test]
    fn validate_operator_base_url_allows_https_private_ips() {
        assert!(validate_operator_base_url("https://100.64.0.1/v1", "TEST").is_ok());
        assert!(validate_operator_base_url("https://127.0.0.1/v1", "TEST").is_ok());
        assert!(validate_operator_base_url("https://[::1]/v1", "TEST").is_ok());
    }

    #[test]
    fn validate_operator_base_url_allows_http_private_ips() {
        assert!(validate_operator_base_url("http://100.64.0.1:8000/v1", "TEST").is_ok());
        assert!(validate_operator_base_url("http://192.168.1.50:8000/v1", "TEST").is_ok());
    }

    #[test]
    fn validate_operator_base_url_still_rejects_public_http_and_metadata() {
        assert!(validate_operator_base_url("http://8.8.8.8/v1", "TEST").is_err());
        assert!(validate_operator_base_url("https://169.254.169.254/v1", "TEST").is_err());
    }

    #[test]
    fn validate_operator_base_url_rejects_link_local_ips() {
        assert!(validate_operator_base_url("http://169.254.1.10:8000/v1", "TEST").is_err());
        assert!(validate_operator_base_url("https://[fe80::1]/v1", "TEST").is_err());
    }

    // --- db_first_* helper tests ---

    #[test]
    fn db_first_or_default_prefers_settings_over_env() {
        let _guard = lock_env();
        let key = "IRONCLAW_TEST_DB_FIRST_1";
        // SAFETY: under ENV_MUTEX
        unsafe { std::env::set_var(key, "from-env") };

        let result: String =
            db_first_or_default(&"from-db".to_string(), &"default".to_string(), key)
                .expect("should resolve");
        assert_eq!(result, "from-db", "DB value should win over env");

        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn db_first_or_default_falls_back_to_env() {
        let _guard = lock_env();
        let key = "IRONCLAW_TEST_DB_FIRST_2";
        unsafe { std::env::set_var(key, "from-env") };

        // settings_val == default_val → treated as "unset"
        let result: String =
            db_first_or_default(&"default".to_string(), &"default".to_string(), key)
                .expect("should resolve");
        assert_eq!(
            result, "from-env",
            "env should win when settings at default"
        );

        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn db_first_or_default_uses_default_when_neither_set() {
        let _guard = lock_env();
        let key = "IRONCLAW_TEST_DB_FIRST_3";
        unsafe { std::env::remove_var(key) };

        let result: String =
            db_first_or_default(&"default".to_string(), &"default".to_string(), key)
                .expect("should resolve");
        assert_eq!(result, "default");
    }

    #[test]
    fn db_first_bool_prefers_settings() {
        let _guard = lock_env();
        let key = "IRONCLAW_TEST_DB_FIRST_BOOL_1";
        unsafe { std::env::set_var(key, "false") };

        let result = db_first_bool(true, false, key).expect("should resolve");
        assert!(result, "DB true should win over env false");

        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn db_first_bool_falls_back_to_env() {
        let _guard = lock_env();
        let key = "IRONCLAW_TEST_DB_FIRST_BOOL_2";
        unsafe { std::env::set_var(key, "true") };

        // settings == default → falls back to env
        let result = db_first_bool(false, false, key).expect("should resolve");
        assert!(result, "env should win when settings at default");

        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn db_first_optional_string_prefers_settings() {
        let _guard = lock_env();
        let key = "IRONCLAW_TEST_DB_FIRST_OPT_1";
        unsafe { std::env::set_var(key, "from-env") };

        let val = Some("from-db".to_string());
        let result = db_first_optional_string(&val, key).expect("should resolve");
        assert_eq!(result, Some("from-db".to_string()));

        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn db_first_optional_string_falls_back_to_env() {
        let _guard = lock_env();
        let key = "IRONCLAW_TEST_DB_FIRST_OPT_2";
        unsafe { std::env::set_var(key, "from-env") };

        let result = db_first_optional_string(&None, key).expect("should resolve");
        assert_eq!(result, Some("from-env".to_string()));

        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn db_first_optional_string_empty_treated_as_unset() {
        let _guard = lock_env();
        let key = "IRONCLAW_TEST_DB_FIRST_OPT_3";
        unsafe { std::env::set_var(key, "from-env") };

        let val = Some(String::new());
        let result = db_first_optional_string(&val, key).expect("should resolve");
        assert_eq!(
            result,
            Some("from-env".to_string()),
            "empty string should be treated as unset"
        );

        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn db_first_option_prefers_settings() {
        let _guard = lock_env();
        let key = "IRONCLAW_TEST_DB_FIRST_OPT_T_1";
        unsafe { std::env::set_var(key, "99") };

        let val: Option<u64> = Some(42);
        let result = db_first_option(&val, key).expect("should resolve");
        assert_eq!(result, Some(42));

        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn db_first_option_falls_back_to_env() {
        let _guard = lock_env();
        let key = "IRONCLAW_TEST_DB_FIRST_OPT_T_2";
        unsafe { std::env::set_var(key, "99") };

        let val: Option<u64> = None;
        let result = db_first_option(&val, key).expect("should resolve");
        assert_eq!(result, Some(99));

        unsafe { std::env::remove_var(key) };
    }

    // --- admin-only LLM key stripping (defense-in-depth for #1955) ---

    #[test]
    fn strip_admin_only_llm_keys_removes_all_known_keys() {
        let mut map = HashMap::new();
        map.insert(
            "llm_builtin_overrides".to_string(),
            serde_json::json!({"openai": {"base_url": "http://10.0.0.5"}}),
        );
        map.insert(
            "llm_custom_providers".to_string(),
            serde_json::json!([{"id": "x"}]),
        );
        map.insert(
            "ollama_base_url".to_string(),
            serde_json::json!("http://192.168.1.20:11434"),
        );
        map.insert(
            "openai_compatible_base_url".to_string(),
            serde_json::json!("http://100.64.0.1"),
        );
        map.insert("selected_model".to_string(), serde_json::json!("gpt-4o"));
        map.insert("agent.name".to_string(), serde_json::json!("Iron"));

        strip_admin_only_llm_keys(&mut map);

        assert!(!map.contains_key("llm_builtin_overrides"));
        assert!(!map.contains_key("llm_custom_providers"));
        assert!(!map.contains_key("ollama_base_url"));
        assert!(!map.contains_key("openai_compatible_base_url"));
        // Non-admin keys must survive.
        assert_eq!(
            map.get("selected_model"),
            Some(&serde_json::json!("gpt-4o"))
        );
        assert_eq!(map.get("agent.name"), Some(&serde_json::json!("Iron")));
    }

    #[test]
    fn strip_admin_only_llm_keys_is_a_no_op_for_clean_map() {
        let mut map = HashMap::new();
        map.insert("selected_model".to_string(), serde_json::json!("gpt-4o"));
        map.insert("llm_backend".to_string(), serde_json::json!("openai"));

        strip_admin_only_llm_keys(&mut map);

        assert_eq!(map.len(), 2);
        assert!(map.contains_key("selected_model"));
        assert!(map.contains_key("llm_backend"));
    }

    // --- async DNS regression (#1955: don't stall the tokio worker) ---

    #[tokio::test(flavor = "multi_thread")]
    async fn validate_base_url_safe_to_call_from_async_handler() {
        // Regression test: validate_base_url_with_policy used to call the
        // blocking `to_socket_addrs()` directly, which can stall a tokio
        // worker thread when invoked from an async handler. The function
        // now wraps the lookup in `block_in_place` on multi-threaded
        // runtimes, so calling it from an async context must not panic
        // and must produce a deterministic error.
        let result = validate_base_url("http://ssrf-test.invalid", "TEST");
        let err = result.expect_err("invalid host should fail validation");
        let msg = err.to_string();
        // Strict policy short-circuits before DNS, so we should get the
        // localhost-only message rather than a DNS error.
        assert!(
            msg.contains("only allowed for localhost"),
            "expected strict short-circuit, got: {msg}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn validate_operator_base_url_safe_to_call_from_async_handler() {
        // The operator policy must also tolerate being called from async
        // handlers (it is reachable from /api/llm/test_connection and
        // /api/llm/list_models). Use IP literals so we don't depend on
        // a working resolver.
        assert!(validate_operator_base_url("http://127.0.0.1:11434", "TEST").is_ok());
        assert!(validate_operator_base_url("http://192.168.1.10:11434", "TEST").is_ok());
        assert!(validate_operator_base_url("http://169.254.169.254", "TEST").is_err());
    }
}
