//! Crate-wide test-only env-var harness. Any test in `ironclaw_reborn_cli`
//! that reads or mutates process env vars (`IRONCLAW_TRIGGER_POLLER_*`,
//! `IRONCLAW_REBORN_RUNNER_*`, `IRONCLAW_REBORN_WEBUI_*`, OAuth knobs,
//! credential-refresh knobs) must hold this single lock and mutate through
//! `EnvGuard`.
//!
//! All of a crate's unit tests link into ONE test binary and run in parallel,
//! so the lock must be process-wide across *every* env-mutating module, not
//! just `runtime`. A second, separate mutex (e.g. the former
//! `commands::serve_sso::WEBUI_BASE_URL_ENV_LOCK`) does not serialize against
//! this one: concurrent `std::env::set_var` from the two lock domains races
//! the shared C environment — UB on Rust 1.82+ — and intermittently corrupts
//! the `runtime::tests::build_runtime_input_production_*` env assertions
//! (#6015). Hence `pub(crate)` and the scope-neutral name.
//!
//! Not exposed outside `#[cfg(test)]`.

use std::sync::{LazyLock, Mutex, MutexGuard};

/// Serializes every test that touches process env vars so they cannot race
/// each other. cargo test runs tests in parallel by default; without this
/// each env-mutating test would observe sibling tests' mutations. Held for
/// the whole body of every env-touching test.
static RUNTIME_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Acquire the single crate-wide process-env lock. `pub(crate)` so
/// env-mutating tests outside `runtime` (e.g. `commands::serve_sso`) share it.
pub(crate) fn lock_runtime_env() -> MutexGuard<'static, ()> {
    RUNTIME_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// RAII guard that snapshots an env var on construction and restores it
/// on drop. Restores on panic too (Drop runs during unwind), which the
/// manual snapshot/restore pattern does not. Tests install this guard so
/// other tests running in parallel cannot observe their mutations.
pub(super) struct EnvGuard {
    key: &'static str,
    prior: Option<String>,
}

impl EnvGuard {
    pub(super) fn set(key: &'static str, value: &str) -> Self {
        let prior = std::env::var(key).ok();
        // SAFETY: env mutation is process-global; restore on Drop covers
        // panic unwind. Callers must hold `lock_runtime_env()` for the
        // life of this guard to serialise against sibling test threads.
        unsafe { std::env::set_var(key, value) };
        Self { key, prior }
    }

    pub(super) fn clear(key: &'static str) -> Self {
        let prior = std::env::var(key).ok();
        // SAFETY: see EnvGuard::set.
        unsafe { std::env::remove_var(key) };
        Self { key, prior }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.prior.take() {
            // SAFETY: see EnvGuard::set.
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            // SAFETY: see EnvGuard::set.
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for #6015. The crate must expose exactly ONE process-env
    /// lock: `commands::serve_sso` previously defined its own
    /// `WEBUI_BASE_URL_ENV_LOCK`, which did not serialize against
    /// `RUNTIME_ENV_LOCK`, so the two lock domains raced `std::env` mutation
    /// (UB on Rust 1.82+) and flaked the `build_runtime_input_production_*`
    /// assertions. This pins that the shared accessor hands out a genuinely
    /// exclusive mutex — a second acquisition while one is held is contended —
    /// so re-introducing a parallel lock (which would break serialization)
    /// stands out against this single canonical one.
    #[test]
    fn process_env_lock_is_a_single_exclusive_mutex() {
        let _held = lock_runtime_env();
        assert!(
            RUNTIME_ENV_LOCK.try_lock().is_err(),
            "the crate-wide process-env lock must be one shared, exclusive mutex"
        );
    }
}
