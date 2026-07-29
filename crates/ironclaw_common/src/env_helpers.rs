//! Thread-safe runtime env-var overlay shared across the workspace.
//!
//! Replaces `std::env::set_var` (which is UB in multi-threaded programs on
//! Rust 1.82+) with an in-process `Mutex<HashMap>` that callers consult via
//! [`env_or_override`]. The main crate layers an additional secrets overlay
//! on top of this; `ironclaw_llm` and other workspace crates use this module
//! directly when they need the runtime override semantics without pulling in
//! the rest of the binary.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Crate-wide mutex for tests that mutate the process environment.
///
/// Acquire this before any `unsafe { std::env::set_var / remove_var }` call
/// so concurrent tests don't race. Recovers from poison since one panicked
/// test shouldn't cascade.
pub static ENV_MUTEX: Mutex<()> = Mutex::new(());

/// Acquire the env-var mutex, recovering from poison.
pub fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RuntimeEnvOverride {
    Value(String),
    Mask,
}

static RUNTIME_ENV_OVERRIDES: OnceLock<Mutex<HashMap<String, RuntimeEnvOverride>>> =
    OnceLock::new();

fn runtime_overrides() -> &'static Mutex<HashMap<String, RuntimeEnvOverride>> {
    RUNTIME_ENV_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Exact saved runtime-overlay state for one env var.
#[derive(Clone, Debug)]
pub struct RuntimeEnvSnapshot {
    key: String,
    value: Option<RuntimeEnvOverride>,
}

/// Optional secondary env lookup registered by the main crate at startup.
///
/// `ironclaw` keeps a separate `INJECTED_VARS` overlay populated from the
/// encrypted secrets store (so API keys can be read without `set_var`).
/// `ironclaw_llm` does not have direct access to that overlay, so the main
/// crate registers a closure here that consults it. Callers of
/// [`env_or_override`] then see the union of: real env, runtime overrides,
/// and the registered fallback.
type EnvFallback = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;
static SECONDARY_FALLBACK: OnceLock<EnvFallback> = OnceLock::new();

/// Install a secondary env lookup. Idempotent: subsequent calls are ignored.
pub fn register_secondary_fallback(f: impl Fn(&str) -> Option<String> + Send + Sync + 'static) {
    // Idempotent by contract: `set` returns `Err` only when the fallback is
    // already installed, in which case subsequent registrations are ignored.
    #[allow(clippy::let_underscore_must_use)]
    let _ = SECONDARY_FALLBACK.set(Box::new(f));
}

/// Set a runtime env override (thread-safe alternative to `std::env::set_var`).
pub fn set_runtime_env(key: &str, value: &str) {
    runtime_overrides()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            key.to_string(),
            RuntimeEnvOverride::Value(value.to_string()),
        );
}

/// Mask an env var for callers of [`env_or_override`].
///
/// This is primarily useful for tests that need hermetic "env var absent"
/// behavior even when a developer shell exports the variable. Call
/// [`remove_runtime_env`] to clear the mask.
pub fn mask_runtime_env(key: &str) {
    runtime_overrides()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_string(), RuntimeEnvOverride::Mask);
}

/// Remove a runtime env override.
pub fn remove_runtime_env(key: &str) {
    runtime_overrides()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(key);
}

/// Capture the current runtime overlay or mask for one env var.
///
/// This does not capture the real process environment. It is intended for
/// tests that temporarily set or mask a runtime env override and then need to
/// restore the exact prior overlay state.
pub fn snapshot_runtime_env(key: &str) -> RuntimeEnvSnapshot {
    let value = runtime_overrides()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .cloned();
    RuntimeEnvSnapshot {
        key: key.to_string(),
        value,
    }
}

/// Restore an exact runtime overlay or mask captured by [`snapshot_runtime_env`].
pub fn restore_runtime_env(snapshot: RuntimeEnvSnapshot) {
    let mut overrides = runtime_overrides()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match snapshot.value {
        Some(value) => {
            overrides.insert(snapshot.key, value);
        }
        None => {
            overrides.remove(&snapshot.key);
        }
    }
}

/// Read an env var, honoring a runtime mask first, then checking real env,
/// runtime overrides, and any secondary fallback registered by the embedding
/// application.
///
/// Empty values are treated as unset at every layer.
pub fn env_or_override(key: &str) -> Option<String> {
    let runtime_value = {
        let overrides = runtime_overrides()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match overrides.get(key) {
            Some(RuntimeEnvOverride::Mask) => return None,
            Some(RuntimeEnvOverride::Value(value)) if !value.is_empty() => Some(value.clone()),
            _ => None,
        }
    };

    if let Ok(val) = std::env::var(key)
        && !val.is_empty()
    {
        return Some(val);
    }

    if let Some(val) = runtime_value {
        return Some(val);
    }

    if let Some(fallback) = SECONDARY_FALLBACK.get()
        && let Some(val) = fallback(key).filter(|v| !v.is_empty())
    {
        return Some(val);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_override_round_trip() {
        let _guard = lock_env();
        set_runtime_env("IRONCLAW_TEST_RUNTIME_OVERRIDE", "1");
        assert_eq!(
            env_or_override("IRONCLAW_TEST_RUNTIME_OVERRIDE"),
            Some("1".to_string())
        );
    }

    #[test]
    fn empty_runtime_override_treated_as_unset() {
        let _guard = lock_env();
        set_runtime_env("IRONCLAW_TEST_EMPTY", "");
        assert_eq!(env_or_override("IRONCLAW_TEST_EMPTY"), None);
    }

    #[test]
    fn runtime_override_can_be_removed() {
        let _guard = lock_env();
        set_runtime_env("IRONCLAW_TEST_REMOVE", "1");
        remove_runtime_env("IRONCLAW_TEST_REMOVE");
        assert_eq!(env_or_override("IRONCLAW_TEST_REMOVE"), None);
    }

    #[test]
    fn runtime_mask_hides_real_env() {
        let _guard = lock_env();
        let key = "IRONCLAW_TEST_RUNTIME_MASK";
        let original = std::env::var_os(key);
        remove_runtime_env(key);
        unsafe {
            std::env::set_var(key, "real-env-value");
        }

        assert_eq!(env_or_override(key), Some("real-env-value".to_string()));

        mask_runtime_env(key);
        assert_eq!(env_or_override(key), None);

        remove_runtime_env(key);
        unsafe {
            match original {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}
