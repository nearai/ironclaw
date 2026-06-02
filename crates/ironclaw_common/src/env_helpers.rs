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

static RUNTIME_ENV_OVERRIDES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn runtime_overrides() -> &'static Mutex<HashMap<String, String>> {
    RUNTIME_ENV_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()))
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
    let _ = SECONDARY_FALLBACK.set(Box::new(f));
}

/// Set a runtime env override (thread-safe alternative to `std::env::set_var`).
pub fn set_runtime_env(key: &str, value: &str) {
    runtime_overrides()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_string(), value.to_string());
}

/// Read an env var, checking real env first, then runtime overrides, then any
/// secondary fallback registered by the embedding application.
///
/// Empty values are treated as unset at every layer.
pub fn env_or_override(key: &str) -> Option<String> {
    if let Ok(val) = std::env::var(key)
        && !val.is_empty()
    {
        return Some(val);
    }

    if let Some(val) = runtime_overrides()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .filter(|v| !v.is_empty())
        .cloned()
    {
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
}
