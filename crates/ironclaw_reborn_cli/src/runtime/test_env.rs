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
//! To make "exactly one lock" hold beyond this crate too, [`lock_runtime_env`]
//! does not own a crate-local mutex — it delegates to the canonical
//! workspace-wide [`ironclaw_common::env_helpers::lock_env`], the same
//! `ENV_MUTEX` that `ironclaw_reborn_composition` (which these tests build
//! services against), `ironclaw_llm`, `ironclaw_auth`, and the `src/` crate
//! already serialize on. So a future env-mutating test anywhere in this binary
//! that reaches for the canonical lock still serializes against these — it
//! cannot form the second, non-serializing lock domain that flaked #6015.
//!
//! Not exposed outside `#[cfg(test)]`.

use std::sync::MutexGuard;

/// Acquire the single crate-wide process-env lock. `pub(crate)` so
/// env-mutating tests outside `runtime` (e.g. `commands::serve_sso`) share it.
/// Delegates to [`ironclaw_common::env_helpers::lock_env`] so the whole
/// workspace serializes on one mutex rather than a per-crate copy.
pub(crate) fn lock_runtime_env() -> MutexGuard<'static, ()> {
    ironclaw_common::env_helpers::lock_env()
}

/// RAII guard that snapshots an env var on construction and restores it
/// on drop. Restores on panic too (Drop runs during unwind), which the
/// manual snapshot/restore pattern does not. Tests install this guard so
/// other tests running in parallel cannot observe their mutations.
pub(crate) struct EnvGuard {
    key: &'static str,
    prior: Option<std::ffi::OsString>,
}

impl EnvGuard {
    pub(crate) fn set(key: &'static str, value: &str) -> Self {
        let prior = std::env::var_os(key);
        // SAFETY: env mutation is process-global; restore on Drop covers
        // panic unwind. Callers must hold `lock_runtime_env()` for the
        // life of this guard to serialise against sibling test threads.
        unsafe { std::env::set_var(key, value) };
        Self { key, prior }
    }

    pub(crate) fn clear(key: &'static str) -> Self {
        let prior = std::env::var_os(key);
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
    /// `WEBUI_BASE_URL_ENV_LOCK`, which did not serialize against the runtime
    /// lock, so the two lock domains raced `std::env` mutation (UB on Rust
    /// 1.82+) and flaked the `build_runtime_input_production_*` assertions.
    /// This pins two things: (1) `lock_runtime_env` hands out a genuinely
    /// exclusive mutex, and (2) that mutex *is* the canonical workspace lock
    /// `ironclaw_common::env_helpers::ENV_MUTEX` — so while the accessor's
    /// guard is held, the canonical mutex is contended. Re-introducing a
    /// parallel crate-local lock (which would break serialization against the
    /// rest of the workspace) stands out against this single canonical one.
    #[test]
    fn process_env_lock_is_the_single_canonical_mutex() {
        let _held = lock_runtime_env();
        assert!(
            ironclaw_common::env_helpers::ENV_MUTEX.try_lock().is_err(),
            "lock_runtime_env must hand out the canonical workspace env mutex, \
             not a second crate-local one"
        );
    }
}
