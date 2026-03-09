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
        .get()
        .and_then(|m| m.get(key))
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
        let _guard = ENV_MUTEX.lock().unwrap();
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
}
